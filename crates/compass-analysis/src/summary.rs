use std::collections::BTreeMap;
use std::io::Write;

use compass_ir::{
    Coverage, ExceptionEffect, IrError, OperationKind, ProgramBundle, SymbolId,
    canonical_json_bytes, hex_sha256,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

const STREAM_CANONICAL_ELEMENT_CHUNK: usize = 16_384;
const STREAM_CANONICAL_CHUNKS_IN_FLIGHT: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FunctionSummary {
    pub symbol_id: SymbolId,
    pub body_digest: String,
    pub semantic_digest: String,
    pub resolved_calls: Vec<SymbolId>,
    pub unresolved_calls: Vec<String>,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub effects: Vec<String>,
    pub exceptions: Vec<ExceptionEffect>,
    pub evidence: Vec<String>,
    pub coverage: Coverage,
    pub summary_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisBundle {
    pub analysis_schema_version: u32,
    pub analyzer_version: u32,
    pub program: ProgramBundle,
    pub summaries: Vec<FunctionSummary>,
    pub reverse_calls: BTreeMap<SymbolId, Vec<SymbolId>>,
}

#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    #[error(transparent)]
    Ir(#[from] IrError),
    #[error("duplicate function symbol {0}")]
    DuplicateFunction(String),
    #[error("analysis version mismatch")]
    VersionMismatch,
    #[error("canonical analysis output write failed: {0}")]
    Output(#[from] std::io::Error),
}

pub fn analyze(program: ProgramBundle) -> Result<AnalysisBundle, AnalysisError> {
    let program = program.into_canonicalized();
    program.validate()?;
    analyze_prevalidated(program)
}

/// Analyze a Program canonicalized and validated by the in-process merger.
///
/// Untrusted artifacts must use [`analyze`].
pub fn analyze_prevalidated(program: ProgramBundle) -> Result<AnalysisBundle, AnalysisError> {
    let functions = program
        .modules
        .iter()
        .flat_map(|module| module.functions.iter())
        .collect::<Vec<_>>();
    let mut summaries = functions
        .par_iter()
        .map(|function| summarize(function))
        .collect::<Result<Vec<_>, _>>()?;
    summaries.sort_by(|left, right| left.symbol_id.as_bytes().cmp(right.symbol_id.as_bytes()));
    if let Some(duplicate) = summaries
        .windows(2)
        .find(|pair| pair[0].symbol_id == pair[1].symbol_id)
    {
        return Err(AnalysisError::DuplicateFunction(
            duplicate[0].symbol_id.clone(),
        ));
    }
    let mut reverse_calls = BTreeMap::<String, Vec<String>>::new();
    for summary in &summaries {
        for target in &summary.resolved_calls {
            reverse_calls
                .entry(target.clone())
                .or_default()
                .push(summary.symbol_id.clone());
        }
    }
    canonicalize_reverse_calls(&mut reverse_calls);
    Ok(AnalysisBundle {
        analysis_schema_version: crate::ANALYSIS_SCHEMA_VERSION,
        analyzer_version: crate::ANALYZER_VERSION,
        program,
        summaries,
        reverse_calls,
    })
}

impl AnalysisBundle {
    pub fn canonicalized(&self) -> Self {
        let mut bundle = self.clone();
        bundle.program = bundle.program.canonicalized();
        for summary in &mut bundle.summaries {
            sort_dedup(&mut summary.resolved_calls);
            sort_dedup(&mut summary.unresolved_calls);
            sort_dedup(&mut summary.reads);
            sort_dedup(&mut summary.writes);
            sort_dedup(&mut summary.effects);
            sort_dedup(&mut summary.exceptions);
            sort_dedup(&mut summary.evidence);
        }
        bundle
            .summaries
            .sort_by(|left, right| left.symbol_id.as_bytes().cmp(right.symbol_id.as_bytes()));
        canonicalize_reverse_calls(&mut bundle.reverse_calls);
        bundle
    }

    pub fn validate(&self) -> Result<(), AnalysisError> {
        if self.analysis_schema_version != crate::ANALYSIS_SCHEMA_VERSION
            || self.analyzer_version != crate::ANALYZER_VERSION
        {
            return Err(AnalysisError::VersionMismatch);
        }
        self.program.validate()?;
        let regenerated = analyze(self.program.clone())?;
        if regenerated.summaries != self.canonicalized().summaries
            || regenerated.reverse_calls != self.canonicalized().reverse_calls
        {
            return Err(AnalysisError::Ir(IrError::Invalid(
                "summaries do not match embedded Program IR".to_owned(),
            )));
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, AnalysisError> {
        let bundle = self.canonicalized();
        bundle.validate()?;
        Ok(canonical_json_bytes(&bundle)?)
    }

    /// Serialize an analysis that was canonicalized and validated by the
    /// in-process merger.
    ///
    /// Large arrays are encoded in parallel while the final byte stream keeps
    /// the same canonical ordering as [`Self::canonical_bytes`]. Untrusted
    /// artifacts must continue to use [`Self::canonical_bytes`].
    pub fn canonical_bytes_prevalidated(&self) -> Result<Vec<u8>, AnalysisError> {
        let ((evidence, modules), (providers, (summaries, reverse_calls))) = rayon::join(
            || {
                rayon::join(
                    || canonical_array_chunks(&self.program.evidence),
                    || canonical_array_chunks(&self.program.modules),
                )
            },
            || {
                rayon::join(
                    || canonical_array_chunks(&self.program.providers),
                    || {
                        rayon::join(
                            || canonical_array_chunks(&self.summaries),
                            || canonical_fragment(&self.reverse_calls),
                        )
                    },
                )
            },
        );
        let evidence = evidence?;
        let modules = modules?;
        let providers = providers?;
        let summaries = summaries?;
        let reverse_calls = reverse_calls?;
        let schema = canonical_fragment(&self.program.schema)?;

        let capacity = evidence
            .iter()
            .chain(&modules)
            .chain(&providers)
            .chain(&summaries)
            .map(Vec::len)
            .sum::<usize>()
            .saturating_add(reverse_calls.len())
            .saturating_add(schema.len())
            .saturating_add(256);
        let mut output = Vec::with_capacity(capacity);
        output.extend_from_slice(b"{\"analysis_schema_version\":");
        output.extend_from_slice(self.analysis_schema_version.to_string().as_bytes());
        output.extend_from_slice(b",\"analyzer_version\":");
        output.extend_from_slice(self.analyzer_version.to_string().as_bytes());
        output.extend_from_slice(b",\"program\":{\"evidence\":");
        append_canonical_array(&mut output, evidence);
        output.extend_from_slice(b",\"modules\":");
        append_canonical_array(&mut output, modules);
        output.extend_from_slice(b",\"providers\":");
        append_canonical_array(&mut output, providers);
        output.extend_from_slice(b",\"schema\":");
        output.extend_from_slice(&schema);
        output.extend_from_slice(b"},\"reverse_calls\":");
        output.extend_from_slice(&reverse_calls);
        output.extend_from_slice(b",\"summaries\":");
        append_canonical_array(&mut output, summaries);
        output.extend_from_slice(b"}\n");
        Ok(output)
    }

    /// Stream the canonical representation without retaining the complete
    /// JSON document or all serialized array chunks in memory at once.
    ///
    /// The in-process builder has already canonicalized and validated this
    /// bundle, so the bounded writer path preserves the same bytes as
    /// [`Self::canonical_bytes_prevalidated`].
    pub fn write_canonical_prevalidated<W: Write>(
        &self,
        mut output: W,
    ) -> Result<(), AnalysisError> {
        output.write_all(b"{\"analysis_schema_version\":")?;
        output.write_all(self.analysis_schema_version.to_string().as_bytes())?;
        output.write_all(b",\"analyzer_version\":")?;
        output.write_all(self.analyzer_version.to_string().as_bytes())?;
        output.write_all(b",\"program\":{\"evidence\":")?;
        write_canonical_array(&mut output, &self.program.evidence)?;
        output.write_all(b",\"modules\":")?;
        write_canonical_array(&mut output, &self.program.modules)?;
        output.write_all(b",\"providers\":")?;
        write_canonical_array(&mut output, &self.program.providers)?;
        output.write_all(b",\"schema\":")?;
        output.write_all(&canonical_fragment(&self.program.schema)?)?;
        output.write_all(b"},\"reverse_calls\":")?;
        output.write_all(&canonical_fragment(&self.reverse_calls)?)?;
        output.write_all(b",\"summaries\":")?;
        write_canonical_array(&mut output, &self.summaries)?;
        output.write_all(b"}\n")?;
        Ok(())
    }

    pub fn digest(&self) -> Result<String, AnalysisError> {
        Ok(hex_sha256(&self.canonical_bytes()?))
    }
}

fn canonical_array_chunks<T: Serialize + Sync>(
    values: &[T],
) -> Result<Vec<Vec<u8>>, AnalysisError> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    // Program serialization runs beside graph assembly. A small fixed number
    // of chunks keeps those independent stages concurrent instead of filling
    // the shared Rayon pool with serialization work.
    let target_chunks = 2;
    let chunk_size = values.len().div_ceil(target_chunks);
    values
        .par_chunks(chunk_size)
        .map(|chunk| canonical_json_bytes(chunk).map_err(AnalysisError::from))
        .collect()
}

fn canonical_fragment<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, AnalysisError> {
    let mut bytes = canonical_json_bytes(value)?;
    debug_assert_eq!(bytes.last(), Some(&b'\n'));
    bytes.pop();
    Ok(bytes)
}

fn append_canonical_array(output: &mut Vec<u8>, chunks: Vec<Vec<u8>>) {
    output.push(b'[');
    for (index, chunk) in chunks.into_iter().enumerate() {
        debug_assert!(chunk.starts_with(b"[") && chunk.ends_with(b"]\n"));
        if index != 0 {
            output.push(b',');
        }
        output.extend_from_slice(&chunk[1..chunk.len() - 2]);
    }
    output.push(b']');
}

fn write_canonical_array<W: Write, T: Serialize + Sync>(
    output: &mut W,
    values: &[T],
) -> Result<(), AnalysisError> {
    output.write_all(b"[")?;
    let mut first = true;
    let mut chunks = values.chunks(STREAM_CANONICAL_ELEMENT_CHUNK);
    loop {
        let batch = (0..STREAM_CANONICAL_CHUNKS_IN_FLIGHT)
            .filter_map(|_| chunks.next())
            .collect::<Vec<_>>();
        if batch.is_empty() {
            break;
        }
        let encoded = batch
            .par_iter()
            .map(canonical_json_bytes)
            .collect::<Result<Vec<_>, _>>()?;
        for bytes in encoded {
            debug_assert!(bytes.starts_with(b"[") && bytes.ends_with(b"]\n"));
            if !first {
                output.write_all(b",")?;
            }
            output.write_all(&bytes[1..bytes.len().saturating_sub(2)])?;
            first = false;
        }
    }
    output.write_all(b"]")?;
    Ok(())
}

pub(crate) fn summarize(
    function: &compass_ir::FunctionIr,
) -> Result<FunctionSummary, AnalysisError> {
    let mut resolved_calls = Vec::new();
    let mut unresolved_calls = Vec::new();
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let mut effects = Vec::new();
    let mut exceptions = Vec::new();
    let mut evidence = function.evidence.clone();
    for block in &function.blocks {
        evidence.extend(block.evidence.clone());
        for operation in &block.operations {
            evidence.extend(operation.evidence.clone());
            match &operation.kind {
                OperationKind::Call {
                    callee,
                    resolved_symbols,
                    ..
                } => {
                    if resolved_symbols.len() == 1 {
                        resolved_calls.push(resolved_symbols[0].clone());
                    } else {
                        unresolved_calls.push(callee.clone());
                    }
                }
                OperationKind::Read { path } => reads.push(path.clone()),
                OperationKind::Write { path } => writes.push(path.clone()),
                OperationKind::Await => effects.push("await".to_owned()),
                OperationKind::Throw { effect } => {
                    effects.push("throw".to_owned());
                    exceptions.push(effect.clone());
                }
            }
        }
    }
    sort_dedup(&mut resolved_calls);
    sort_dedup(&mut unresolved_calls);
    sort_dedup(&mut reads);
    sort_dedup(&mut writes);
    sort_dedup(&mut effects);
    sort_dedup(&mut exceptions);
    sort_dedup(&mut evidence);
    let semantic_digest = semantic_digest(function)?;
    let summary_payload = (
        &function.symbol_id,
        &function.body_digest,
        &semantic_digest,
        &resolved_calls,
        &unresolved_calls,
        &reads,
        &writes,
        &effects,
        &exceptions,
        &evidence,
        &function.coverage,
    );
    let summary_digest = hex_sha256(&canonical_json_bytes(&summary_payload)?);
    Ok(FunctionSummary {
        symbol_id: function.symbol_id.clone(),
        body_digest: function.body_digest.clone(),
        semantic_digest,
        resolved_calls,
        unresolved_calls,
        reads,
        writes,
        effects,
        exceptions,
        evidence,
        coverage: function.coverage.clone(),
        summary_digest,
    })
}

pub(crate) fn semantic_digest(function: &compass_ir::FunctionIr) -> Result<String, AnalysisError> {
    let operations = function
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .operations
                .iter()
                .map(|operation| (&operation.kind, operation.ordinal))
        })
        .collect::<Vec<_>>();
    Ok(hex_sha256(&canonical_json_bytes(&(
        &function.symbol_id,
        &function.signature_digest,
        &function.body_digest,
        function.is_test,
        operations,
        &function.coverage,
    ))?))
}

fn canonicalize_reverse_calls(reverse_calls: &mut BTreeMap<String, Vec<String>>) {
    for callers in reverse_calls.values_mut() {
        sort_dedup(callers);
    }
}

fn sort_dedup<T: Ord>(items: &mut Vec<T>) {
    items.sort();
    items.dedup();
}
