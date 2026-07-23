use std::collections::{BTreeMap, BTreeSet};

use compass_ir::{
    Capability, Coverage, CoverageState, EvidenceRecord, IrError, ModuleIr, OperationKind,
    ProgramBundle, ProviderDescriptor, ProviderKind, SourceAnchor, TypeRef,
};

use crate::{EvidenceBatch, EvidenceFact, FactKind, ProviderError, normalize_source_path};

pub const MERGER_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("provider {0} was supplied with different evidence batches")]
    ConflictingProvider(String),
    #[error("evidence ID {0} was supplied with different records")]
    ConflictingEvidence(String),
    #[error("conflicting source structures for {0}")]
    ConflictingStructure(String),
    #[error("artifact provider {0} attempted to define source structure")]
    ArtifactDefinedStructure(String),
    #[error("provider {provider} emitted evidence owned by {owner}")]
    EvidenceOwnership { provider: String, owner: String },
    #[error("invalid provider scope for {provider}: {path}")]
    Scope { provider: String, path: String },
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Ir(#[from] IrError),
}

pub fn merge_evidence(batches: Vec<EvidenceBatch>) -> Result<ProgramBundle, MergeError> {
    let batches = canonical_batches(batches)?;
    let mut providers = Vec::with_capacity(batches.len());
    let mut evidence = BTreeMap::<String, EvidenceRecord>::new();
    let mut modules = BTreeMap::<String, ModuleIr>::new();
    let mut facts = Vec::new();
    let mut declared_coverage = BTreeMap::<String, Coverage>::new();

    for batch in batches {
        validate_batch(&batch)?;
        providers.push(batch.descriptor.clone());
        for record in batch.evidence {
            match evidence.get(&record.id) {
                Some(existing) if existing != &record => {
                    return Err(MergeError::ConflictingEvidence(record.id));
                }
                Some(_) => {}
                None => {
                    evidence.insert(record.id.clone(), record);
                }
            }
        }
        for module in batch.modules {
            if batch.descriptor.kind == ProviderKind::Artifact {
                return Err(MergeError::ArtifactDefinedStructure(
                    batch.descriptor.id.clone(),
                ));
            }
            let source_file = normalize_source_path(&module.source_file)?;
            match modules.get(&source_file) {
                Some(existing) if existing != &module => {
                    return Err(MergeError::ConflictingStructure(source_file));
                }
                Some(_) => {}
                None => {
                    modules.insert(source_file, module);
                }
            }
        }
        for (path, coverage) in batch.coverage {
            let path = normalize_source_path(&path)?;
            merge_coverage(declared_coverage.entry(path).or_default(), coverage);
        }
        facts.extend(batch.facts);
    }

    facts.sort_by(|left, right| {
        left.anchor.cmp(&right.anchor).then_with(|| {
            left.evidence_id
                .as_bytes()
                .cmp(right.evidence_id.as_bytes())
        })
    });
    facts.dedup();

    for fact in &facts {
        if let Some(module) = modules.get_mut(&fact.anchor.source_file) {
            let attached = attach_fact(module, fact);
            if !attached {
                mark_partial(
                    &mut module.coverage,
                    fact.capability.clone(),
                    "unmatched_semantic_occurrence",
                );
            }
        }
    }

    for (path, coverage) in declared_coverage {
        if let Some(module) = modules.get_mut(&path) {
            merge_coverage(&mut module.coverage, coverage);
        }
    }
    for module in modules.values_mut() {
        for function in &mut module.functions {
            let mut conflicts = false;
            for block in &function.blocks {
                for operation in &block.operations {
                    if let OperationKind::Call {
                        resolved_symbols, ..
                    } = &operation.kind
                        && resolved_symbols.len() > 1
                    {
                        conflicts = true;
                    }
                }
            }
            if conflicts {
                mark_partial(
                    &mut function.coverage,
                    Capability::CallResolution,
                    "provider_conflict",
                );
                mark_partial(
                    &mut module.coverage,
                    Capability::CallResolution,
                    "provider_conflict",
                );
            }
        }
    }

    let bundle = ProgramBundle {
        schema: compass_ir::PROGRAM_SCHEMA.to_owned(),
        providers,
        evidence: evidence.into_values().collect(),
        modules: modules.into_values().collect(),
    }
    .canonicalized();
    bundle.validate()?;
    Ok(bundle)
}

fn canonical_batches(batches: Vec<EvidenceBatch>) -> Result<Vec<EvidenceBatch>, MergeError> {
    let mut by_provider = BTreeMap::<String, EvidenceBatch>::new();
    for batch in batches {
        let canonical = batch.canonicalized();
        match by_provider.get(&canonical.descriptor.id) {
            Some(existing) if existing != &canonical => {
                return Err(MergeError::ConflictingProvider(
                    canonical.descriptor.id.clone(),
                ));
            }
            Some(_) => {}
            None => {
                by_provider.insert(canonical.descriptor.id.clone(), canonical);
            }
        }
    }
    Ok(by_provider.into_values().collect())
}

fn validate_batch(batch: &EvidenceBatch) -> Result<(), MergeError> {
    let mut evidence_ids = BTreeSet::new();
    for record in &batch.evidence {
        if record.provider_id != batch.descriptor.id {
            return Err(MergeError::EvidenceOwnership {
                provider: batch.descriptor.id.clone(),
                owner: record.provider_id.clone(),
            });
        }
        evidence_ids.insert(record.id.as_str());
        if let Some(path) = &record.source_file {
            validate_scope(&batch.descriptor, path)?;
        }
    }
    for module in &batch.modules {
        validate_scope(&batch.descriptor, &module.source_file)?;
    }
    for fact in &batch.facts {
        validate_scope(&batch.descriptor, &fact.anchor.source_file)?;
        if !evidence_ids.contains(fact.evidence_id.as_str()) {
            return Err(MergeError::Ir(IrError::UnknownEvidence(
                fact.evidence_id.clone(),
            )));
        }
    }
    Ok(())
}

fn validate_scope(descriptor: &ProviderDescriptor, path: &str) -> Result<(), MergeError> {
    let path = normalize_source_path(path)?;
    if descriptor.kind == ProviderKind::Syntax && descriptor.scope != path {
        return Err(MergeError::Scope {
            provider: descriptor.id.clone(),
            path,
        });
    }
    Ok(())
}

fn attach_fact(module: &mut ModuleIr, fact: &EvidenceFact) -> bool {
    let Some(function_index) = unique_smallest_function(&module.functions, &fact.anchor) else {
        return false;
    };
    let function = &mut module.functions[function_index];
    let attached = match &fact.kind {
        FactKind::CallResolution { target } => attach_call_resolution(function, fact, target),
        FactKind::TypeResolution { spelling, target } => {
            attach_type_resolution(function, fact, spelling, target)
        }
        FactKind::Symbol { .. } | FactKind::Relationship { .. } => {
            function.evidence.push(fact.evidence_id.clone());
            true
        }
    };
    if attached {
        function
            .evidence
            .sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        function.evidence.dedup();
        module.evidence.push(fact.evidence_id.clone());
        module.evidence.sort();
        module.evidence.dedup();
    }
    attached
}

fn unique_smallest_function(
    functions: &[compass_ir::FunctionIr],
    anchor: &SourceAnchor,
) -> Option<usize> {
    let candidates = functions
        .iter()
        .enumerate()
        .filter(|(_, function)| contains(&function.anchor, anchor))
        .map(|(index, function)| (index, span_len(&function.anchor)))
        .collect::<Vec<_>>();
    unique_smallest(candidates)
}

fn call_candidates(
    function: &compass_ir::FunctionIr,
    fact: &SourceAnchor,
) -> Vec<(usize, usize, u64, bool)> {
    function
        .blocks
        .iter()
        .enumerate()
        .flat_map(|(block_index, block)| {
            block
                .operations
                .iter()
                .enumerate()
                .filter_map(move |(operation_index, operation)| {
                    let OperationKind::Call { callee_anchor, .. } = &operation.kind else {
                        return None;
                    };
                    let exact = callee_anchor == fact;
                    (exact || contains(callee_anchor, fact) || contains(fact, callee_anchor))
                        .then_some((block_index, operation_index, span_len(callee_anchor), exact))
                })
        })
        .collect()
}

fn selected_call(function: &compass_ir::FunctionIr, fact: &SourceAnchor) -> Option<(usize, usize)> {
    let candidates = call_candidates(function, fact);
    let exact = candidates
        .iter()
        .filter(|(_, _, _, is_exact)| *is_exact)
        .map(|(block, operation, span, _)| ((*block, *operation), *span))
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return unique_smallest_indexed(exact);
    }
    unique_smallest_indexed(
        candidates
            .into_iter()
            .map(|(block, operation, span, _)| ((block, operation), span))
            .collect(),
    )
}

fn attach_call_resolution(
    function: &mut compass_ir::FunctionIr,
    fact: &EvidenceFact,
    target: &str,
) -> bool {
    let Some((block_index, operation_index)) = selected_call(function, &fact.anchor) else {
        return false;
    };
    let operation = &mut function.blocks[block_index].operations[operation_index];
    let OperationKind::Call {
        resolved_symbols, ..
    } = &mut operation.kind
    else {
        return false;
    };
    resolved_symbols.push(target.to_owned());
    operation.evidence.push(fact.evidence_id.clone());
    true
}

fn attach_type_resolution(
    function: &mut compass_ir::FunctionIr,
    fact: &EvidenceFact,
    spelling: &str,
    target: &str,
) -> bool {
    let Some((block_index, operation_index)) = selected_call(function, &fact.anchor) else {
        return false;
    };
    let operation = &mut function.blocks[block_index].operations[operation_index];
    let OperationKind::Call { receiver_type, .. } = &mut operation.kind else {
        return false;
    };
    let type_ref = receiver_type.get_or_insert_with(|| TypeRef {
        spelling: spelling.to_owned(),
        resolved_symbol: None,
        evidence: Vec::new(),
    });
    if type_ref
        .resolved_symbol
        .as_ref()
        .is_some_and(|value| value != target)
    {
        return false;
    }
    type_ref.resolved_symbol = Some(target.to_owned());
    type_ref.evidence.push(fact.evidence_id.clone());
    true
}

fn unique_smallest(candidates: Vec<(usize, u64)>) -> Option<usize> {
    let minimum = candidates.iter().map(|(_, span)| *span).min()?;
    let mut smallest = candidates
        .into_iter()
        .filter(|(_, span)| *span == minimum)
        .map(|(index, _)| index);
    let selected = smallest.next()?;
    smallest.next().is_none().then_some(selected)
}

fn unique_smallest_indexed(candidates: Vec<((usize, usize), u64)>) -> Option<(usize, usize)> {
    let minimum = candidates.iter().map(|(_, span)| *span).min()?;
    let mut smallest = candidates
        .into_iter()
        .filter(|(_, span)| *span == minimum)
        .map(|(index, _)| index);
    let selected = smallest.next()?;
    smallest.next().is_none().then_some(selected)
}

fn span_len(anchor: &SourceAnchor) -> u64 {
    anchor.end_byte.saturating_sub(anchor.start_byte)
}

fn contains(outer: &SourceAnchor, inner: &SourceAnchor) -> bool {
    outer.source_file == inner.source_file
        && outer.start_byte <= inner.start_byte
        && outer.end_byte >= inner.end_byte
}

fn merge_coverage(target: &mut Coverage, incoming: Coverage) {
    for (capability, state) in incoming {
        target
            .entry(capability)
            .and_modify(|current| *current = combined_state(current, &state))
            .or_insert(state);
    }
}

fn combined_state(left: &CoverageState, right: &CoverageState) -> CoverageState {
    if matches!(left, CoverageState::Complete) || matches!(right, CoverageState::Complete) {
        return CoverageState::Complete;
    }
    let reasons = union_reasons(state_reasons(left), state_reasons(right));
    if matches!(left, CoverageState::Partial { .. })
        || matches!(right, CoverageState::Partial { .. })
    {
        CoverageState::Partial { reasons }
    } else if matches!(left, CoverageState::Indeterminate { .. })
        || matches!(right, CoverageState::Indeterminate { .. })
    {
        CoverageState::Indeterminate { reasons }
    } else {
        CoverageState::Failed { reasons }
    }
}

fn state_reasons(state: &CoverageState) -> &[String] {
    match state {
        CoverageState::Complete => &[],
        CoverageState::Partial { reasons }
        | CoverageState::Indeterminate { reasons }
        | CoverageState::Failed { reasons } => reasons,
    }
}

fn mark_partial(coverage: &mut Coverage, capability: Capability, reason: &str) {
    let partial = CoverageState::Partial {
        reasons: vec![reason.to_owned()],
    };
    coverage
        .entry(capability)
        .and_modify(|state| {
            *state = match state {
                CoverageState::Complete => partial.clone(),
                CoverageState::Partial { reasons }
                | CoverageState::Indeterminate { reasons }
                | CoverageState::Failed { reasons } => CoverageState::Partial {
                    reasons: union_reasons(reasons, &[reason.to_owned()]),
                },
            };
        })
        .or_insert(partial);
}

fn union_reasons(left: &[String], right: &[String]) -> Vec<String> {
    left.iter()
        .chain(right)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
