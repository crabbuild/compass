use std::collections::{BTreeMap, BTreeSet};

use compass_ir::{
    Coverage, IrError, OperationKind, ProgramBundle, SymbolId, canonical_json_bytes, hex_sha256,
};
use serde::{Deserialize, Serialize};

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
    pub errors: Vec<String>,
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
}

pub fn analyze(program: ProgramBundle) -> Result<AnalysisBundle, AnalysisError> {
    let program = program.canonicalized();
    program.validate()?;
    let mut summaries = Vec::new();
    let mut symbols = BTreeSet::new();
    let mut reverse_calls = BTreeMap::<String, Vec<String>>::new();
    for module in &program.modules {
        for function in &module.functions {
            if !symbols.insert(function.symbol_id.clone()) {
                return Err(AnalysisError::DuplicateFunction(
                    function.symbol_id.clone(),
                ));
            }
            let summary = summarize(function)?;
            for target in &summary.resolved_calls {
                reverse_calls
                    .entry(target.clone())
                    .or_default()
                    .push(function.symbol_id.clone());
            }
            summaries.push(summary);
        }
    }
    canonicalize_reverse_calls(&mut reverse_calls);
    summaries.sort_by(|left, right| left.symbol_id.as_bytes().cmp(right.symbol_id.as_bytes()));
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
            sort_dedup(&mut summary.errors);
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

    pub fn digest(&self) -> Result<String, AnalysisError> {
        Ok(hex_sha256(&self.canonical_bytes()?))
    }
}

pub(crate) fn summarize(
    function: &compass_ir::FunctionIr,
) -> Result<FunctionSummary, AnalysisError> {
    let mut resolved_calls = Vec::new();
    let mut unresolved_calls = Vec::new();
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let mut effects = Vec::new();
    let mut errors = Vec::new();
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
                OperationKind::Throw { value } => {
                    effects.push("throw".to_owned());
                    errors.push(value.clone());
                }
            }
        }
    }
    sort_dedup(&mut resolved_calls);
    sort_dedup(&mut unresolved_calls);
    sort_dedup(&mut reads);
    sort_dedup(&mut writes);
    sort_dedup(&mut effects);
    sort_dedup(&mut errors);
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
        &errors,
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
        errors,
        evidence,
        coverage: function.coverage.clone(),
        summary_digest,
    })
}

pub(crate) fn semantic_digest(
    function: &compass_ir::FunctionIr,
) -> Result<String, AnalysisError> {
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
