use std::collections::BTreeSet;

use compass_ir::{Capability, CoverageState, ProviderKind, SourceAnchor};

use crate::{EvidenceBatch, FactKind, Role};

const UNVERIFIED_REVISION: &str = "artifact_revision_unverified";
const STALE_DOCUMENT: &str = "stale_artifact_document";
const SCIP_INDEX_SCOPE: &str = "scip_index_scope";

/// Compact, source-anchored compiler facts that can safely enrich graph output.
///
/// This intentionally does not infer whether a reference is a call. Consumers
/// must join `calls` to a syntax-proven call occurrence at the exact anchor.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompilerProjection {
    pub definitions: Vec<CompilerDefinition>,
    pub calls: Vec<CompilerCall>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CompilerDefinition {
    pub provider_id: String,
    pub symbol: String,
    pub anchor: SourceAnchor,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CompilerCall {
    pub provider_id: String,
    pub target: String,
    pub anchor: SourceAnchor,
}

#[must_use]
pub fn compiler_projection(batches: &[EvidenceBatch]) -> CompilerProjection {
    let mut definitions = BTreeSet::new();
    let mut calls = BTreeSet::new();
    for batch in batches.iter().filter(|batch| {
        matches!(
            batch.descriptor.kind,
            ProviderKind::Artifact | ProviderKind::Project
        )
    }) {
        for fact in &batch.facts {
            if !coverage_allows_projection(batch, &fact.anchor.source_file, &fact.capability) {
                continue;
            }
            match &fact.kind {
                FactKind::Symbol { symbol, roles } if roles.contains(&Role::Definition) => {
                    definitions.insert(CompilerDefinition {
                        provider_id: batch.descriptor.id.clone(),
                        symbol: symbol.clone(),
                        anchor: fact.anchor.clone(),
                    });
                }
                FactKind::CallResolution { target } => {
                    calls.insert(CompilerCall {
                        provider_id: batch.descriptor.id.clone(),
                        target: target.clone(),
                        anchor: fact.anchor.clone(),
                    });
                }
                _ => {}
            }
        }
    }
    CompilerProjection {
        definitions: definitions.into_iter().collect(),
        calls: calls.into_iter().collect(),
    }
}

fn coverage_allows_projection(
    batch: &EvidenceBatch,
    source_file: &str,
    capability: &Capability,
) -> bool {
    let Some(state) = batch
        .coverage
        .get(source_file)
        .and_then(|coverage| coverage.get(capability))
    else {
        return false;
    };
    match state {
        CoverageState::Complete => true,
        CoverageState::Partial { reasons } => {
            !reasons.is_empty()
                && !reasons
                    .iter()
                    .any(|reason| matches!(reason.as_str(), UNVERIFIED_REVISION | STALE_DOCUMENT))
                && reasons.iter().all(|reason| reason == SCIP_INDEX_SCOPE)
        }
        CoverageState::Indeterminate { .. } | CoverageState::Failed { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use compass_ir::{ProviderDescriptor, ProviderKind};

    use super::*;
    use crate::{EvidenceFact, coverage_with};

    fn batch(reason: Option<&str>) -> EvidenceBatch {
        let anchor = SourceAnchor {
            source_file: "src/A.java".to_owned(),
            start_byte: 10,
            end_byte: 13,
        };
        let state = reason.map_or(CoverageState::Complete, |reason| CoverageState::Partial {
            reasons: vec![reason.to_owned()],
        });
        EvidenceBatch {
            descriptor: ProviderDescriptor {
                id: "scip/test".to_owned(),
                kind: ProviderKind::Artifact,
                version: "1".to_owned(),
                scope: "repository".to_owned(),
                input_digest: "digest".to_owned(),
                configuration_digest: "config".to_owned(),
            },
            evidence: Vec::new(),
            modules: Vec::new(),
            facts: vec![
                EvidenceFact {
                    evidence_id: "definition".to_owned(),
                    capability: Capability::Definitions,
                    anchor: anchor.clone(),
                    kind: FactKind::Symbol {
                        symbol: "java method A#run().".to_owned(),
                        roles: vec![Role::Definition],
                    },
                },
                EvidenceFact {
                    evidence_id: "call".to_owned(),
                    capability: Capability::CallResolution,
                    anchor,
                    kind: FactKind::CallResolution {
                        target: "java method A#run().".to_owned(),
                    },
                },
            ],
            coverage: BTreeMap::from([(
                "src/A.java".to_owned(),
                coverage_with([
                    (Capability::Definitions, state.clone()),
                    (Capability::CallResolution, state),
                ]),
            )]),
        }
    }

    #[test]
    fn keeps_fresh_artifact_facts() {
        let projection = compiler_projection(&[batch(Some(SCIP_INDEX_SCOPE))]);
        assert_eq!(projection.definitions.len(), 1);
        assert_eq!(projection.calls.len(), 1);
    }

    #[test]
    fn rejects_unverified_and_stale_artifact_facts() {
        for reason in [UNVERIFIED_REVISION, STALE_DOCUMENT, "unknown_partial_scope"] {
            let projection = compiler_projection(&[batch(Some(reason))]);
            assert!(projection.definitions.is_empty());
            assert!(projection.calls.is_empty());
        }
    }
}
