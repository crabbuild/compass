use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use crate::{
    Capability, Coverage, CoverageState, FunctionIr, ModuleIr, OperationKind, ProgramBundle,
    SourceAnchor, Terminator,
};

#[derive(Debug, thiserror::Error)]
pub enum IrError {
    #[error("unsupported Program IR schema {0}")]
    Schema(String),
    #[error("duplicate provider ID {0}")]
    DuplicateProvider(String),
    #[error("duplicate evidence ID {0}")]
    DuplicateEvidence(String),
    #[error("duplicate module source path {0}")]
    DuplicateModule(String),
    #[error("duplicate function symbol ID {0}")]
    DuplicateFunction(String),
    #[error("invalid source path {0}")]
    InvalidPath(String),
    #[error("invalid source anchor {0}:{1}..{2}")]
    InvalidAnchor(String, u64, u64),
    #[error("unknown provider {0}")]
    UnknownProvider(String),
    #[error("unknown evidence {0}")]
    UnknownEvidence(String),
    #[error("invalid coverage for {capability:?}: {detail}")]
    InvalidCoverage {
        capability: Capability,
        detail: String,
    },
    #[error("function {symbol} has duplicate block ID {block}")]
    DuplicateBlock { symbol: String, block: u32 },
    #[error("function {symbol} has duplicate operation ordinal {ordinal}")]
    DuplicateOperation { symbol: String, ordinal: u32 },
    #[error("function {symbol} references missing block {block}")]
    MissingBlock { symbol: String, block: u32 },
    #[error("resolved call in {symbol} has no call-resolution evidence")]
    MissingResolutionEvidence { symbol: String },
    #[error("invalid Program IR: {0}")]
    Invalid(String),
    #[error("canonical encoding failed: {0}")]
    Canonical(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl ProgramBundle {
    pub fn validate(&self) -> Result<(), IrError> {
        let legacy = self.schema == crate::PROGRAM_SCHEMA_V1;
        if !legacy && self.schema != crate::PROGRAM_SCHEMA {
            return Err(IrError::Schema(self.schema.clone()));
        }
        let mut provider_ids = BTreeSet::new();
        for provider in &self.providers {
            if provider.id.is_empty()
                || provider.version.is_empty()
                || provider.scope.is_empty()
                || !is_lower_hex_digest(&provider.input_digest)
                || !is_lower_hex_digest(&provider.configuration_digest)
            {
                return Err(IrError::Invalid(format!(
                    "invalid provider descriptor {}",
                    provider.id
                )));
            }
            if !provider_ids.insert(provider.id.clone()) {
                return Err(IrError::DuplicateProvider(provider.id.clone()));
            }
        }

        let mut evidence_ids = BTreeSet::new();
        let mut evidence_capabilities = BTreeMap::new();
        for evidence in &self.evidence {
            if !provider_ids.contains(&evidence.provider_id) {
                return Err(IrError::UnknownProvider(evidence.provider_id.clone()));
            }
            if !evidence_ids.insert(evidence.id.clone()) {
                return Err(IrError::DuplicateEvidence(evidence.id.clone()));
            }
            if !is_lower_hex_digest(&evidence.id) {
                return Err(IrError::Invalid(format!(
                    "invalid evidence ID {}",
                    evidence.id
                )));
            }
            if let Some(path) = &evidence.source_file {
                validate_path(path)?;
            }
            evidence_capabilities.insert(evidence.id.clone(), evidence.capability.clone());
        }

        let mut modules = BTreeSet::new();
        let mut functions = BTreeSet::new();
        for module in &self.modules {
            if !modules.insert(module.source_file.clone()) {
                return Err(IrError::DuplicateModule(module.source_file.clone()));
            }
            validate_module(module, &evidence_capabilities, &mut functions, legacy)?;
        }
        Ok(())
    }
}

fn validate_module(
    module: &ModuleIr,
    evidence: &BTreeMap<String, Capability>,
    functions: &mut BTreeSet<String>,
    legacy: bool,
) -> Result<(), IrError> {
    validate_path(&module.source_file)?;
    if module.language.is_empty() || !is_lower_hex_digest(&module.source_digest) {
        return Err(IrError::Invalid(format!(
            "invalid module {}",
            module.source_file
        )));
    }
    validate_coverage(&module.coverage, legacy)?;
    validate_evidence(&module.evidence, evidence)?;
    for function in &module.functions {
        if !functions.insert(function.symbol_id.clone()) {
            return Err(IrError::DuplicateFunction(function.symbol_id.clone()));
        }
        validate_function(function, &module.source_file, evidence, legacy)?;
    }
    Ok(())
}

fn validate_function(
    function: &FunctionIr,
    source_file: &str,
    evidence: &BTreeMap<String, Capability>,
    legacy: bool,
) -> Result<(), IrError> {
    if function.symbol_id.is_empty()
        || function.name.is_empty()
        || !is_lower_hex_digest(&function.signature_digest)
        || !is_lower_hex_digest(&function.body_digest)
    {
        return Err(IrError::Invalid(format!(
            "invalid function {}",
            function.symbol_id
        )));
    }
    validate_anchor(&function.anchor, source_file)?;
    validate_evidence(&function.evidence, evidence)?;
    validate_coverage(&function.coverage, legacy)?;
    for parameter in &function.parameters {
        validate_anchor(&parameter.anchor, source_file)?;
        validate_evidence(&parameter.evidence, evidence)?;
        if let Some(type_ref) = &parameter.type_ref {
            validate_evidence(&type_ref.evidence, evidence)?;
        }
    }
    if let Some(type_ref) = &function.return_type {
        validate_evidence(&type_ref.evidence, evidence)?;
    }

    let block_ids = function
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    if block_ids.len() != function.blocks.len() {
        let duplicate = function
            .blocks
            .iter()
            .find(|block| {
                function
                    .blocks
                    .iter()
                    .filter(|item| item.id == block.id)
                    .count()
                    > 1
            })
            .map_or(0, |block| block.id);
        return Err(IrError::DuplicateBlock {
            symbol: function.symbol_id.clone(),
            block: duplicate,
        });
    }
    let mut ordinals = BTreeSet::new();
    for block in &function.blocks {
        validate_evidence(&block.evidence, evidence)?;
        for operation in &block.operations {
            if !ordinals.insert(operation.ordinal) {
                return Err(IrError::DuplicateOperation {
                    symbol: function.symbol_id.clone(),
                    ordinal: operation.ordinal,
                });
            }
            validate_anchor(&operation.anchor, source_file)?;
            validate_evidence(&operation.evidence, evidence)?;
            if operation.anchor.start_byte < function.anchor.start_byte
                || operation.anchor.end_byte > function.anchor.end_byte
            {
                return Err(IrError::Invalid(format!(
                    "operation outside function {}",
                    function.symbol_id
                )));
            }
            if let OperationKind::Call {
                callee_anchor,
                resolved_symbols,
                receiver_type,
                ..
            } = &operation.kind
            {
                validate_anchor(callee_anchor, source_file)?;
                if callee_anchor.start_byte < operation.anchor.start_byte
                    || callee_anchor.end_byte > operation.anchor.end_byte
                {
                    return Err(IrError::Invalid(format!(
                        "callee anchor outside call in {}",
                        function.symbol_id
                    )));
                }
                if !resolved_symbols.is_empty()
                    && !operation.evidence.iter().any(|id| {
                        evidence
                            .get(id)
                            .is_some_and(|capability| *capability == Capability::CallResolution)
                    })
                {
                    return Err(IrError::MissingResolutionEvidence {
                        symbol: function.symbol_id.clone(),
                    });
                }
                if let Some(type_ref) = receiver_type {
                    validate_evidence(&type_ref.evidence, evidence)?;
                }
            }
        }
        for target in terminator_targets(&block.terminator) {
            if !block_ids.contains(&target) {
                return Err(IrError::MissingBlock {
                    symbol: function.symbol_id.clone(),
                    block: target,
                });
            }
        }
    }
    Ok(())
}

fn terminator_targets(terminator: &Terminator) -> Vec<u32> {
    match terminator {
        Terminator::Goto { target } => vec![*target],
        Terminator::Branch {
            then_target,
            else_target,
            ..
        } => vec![*then_target, *else_target],
        Terminator::Return { .. } | Terminator::Unreachable => Vec::new(),
    }
}

fn validate_coverage(coverage: &Coverage, legacy: bool) -> Result<(), IrError> {
    for (capability, state) in coverage {
        if legacy
            && matches!(
                state,
                CoverageState::Indeterminate { .. } | CoverageState::Failed { .. }
            )
        {
            return Err(IrError::InvalidCoverage {
                capability: capability.clone(),
                detail: "schema 1 does not support indeterminate or failed coverage".to_owned(),
            });
        }
        if !legacy && matches!(state, CoverageState::Unavailable { .. }) {
            return Err(IrError::InvalidCoverage {
                capability: capability.clone(),
                detail: "schema 2 uses indeterminate instead of unavailable".to_owned(),
            });
        }
        let reasons = match state {
            CoverageState::Complete => continue,
            CoverageState::Partial { reasons }
            | CoverageState::Indeterminate { reasons }
            | CoverageState::Failed { reasons }
            | CoverageState::Unavailable { reasons } => reasons,
        };
        if reasons.is_empty() || reasons.iter().any(String::is_empty) {
            return Err(IrError::InvalidCoverage {
                capability: capability.clone(),
                detail: "non-complete coverage requires reasons".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_evidence(
    ids: &[String],
    evidence: &BTreeMap<String, Capability>,
) -> Result<(), IrError> {
    for id in ids {
        if !evidence.contains_key(id) {
            return Err(IrError::UnknownEvidence(id.clone()));
        }
    }
    Ok(())
}

fn validate_anchor(anchor: &SourceAnchor, expected_file: &str) -> Result<(), IrError> {
    validate_path(&anchor.source_file)?;
    if anchor.source_file != expected_file || anchor.start_byte > anchor.end_byte {
        return Err(IrError::InvalidAnchor(
            anchor.source_file.clone(),
            anchor.start_byte,
            anchor.end_byte,
        ));
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), IrError> {
    if path.is_empty()
        || path.contains('\\')
        || path.contains('\0')
        || Path::new(path).is_absolute()
        || path.split('/').any(|part| part.is_empty())
        || Path::new(path)
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(IrError::InvalidPath(path.to_owned()));
    }
    Ok(())
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
