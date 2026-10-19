//! Semantic wrappers around public universal resolution limits.

use super::UniversalResolutionLimits;

pub(super) const MAX_AGGREGATE_MERGE_DEPENDENCIES: usize = 65_536;
pub(super) const MAX_AGGREGATE_MERGE_DEPENDENCY_BYTES: usize = 16 * 1024 * 1024;

/// Shared project ceilings, checked before borrowed batch cloning and owned
/// proof-table retention. Repeated proof copies count independently.
#[derive(Clone, Copy, Debug)]
pub(super) struct AuxiliaryLimits {
    pub(super) type_nodes: usize,
    pub(super) type_bytes: usize,
    pub(super) merge_dependencies: usize,
    pub(super) merge_bytes: usize,
}

impl Default for AuxiliaryLimits {
    fn default() -> Self {
        Self {
            type_nodes: 2_000_000,
            type_bytes: 128 * 1024 * 1024,
            merge_dependencies: MAX_AGGREGATE_MERGE_DEPENDENCIES,
            merge_bytes: MAX_AGGREGATE_MERGE_DEPENDENCY_BYTES,
        }
    }
}

#[derive(Debug)]
pub(super) enum AuxiliaryLimitError {
    InvalidBatch(compass_languages::EvidenceError),
    Aggregate {
        resource: &'static str,
        count: usize,
        limit: usize,
    },
}

impl std::fmt::Display for AuxiliaryLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBatch(error) => error.fmt(formatter),
            Self::Aggregate {
                resource,
                count,
                limit,
            } => write!(
                formatter,
                "universal aggregate {resource} {count} exceeds limit {limit}"
            ),
        }
    }
}

impl std::error::Error for AuxiliaryLimitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidBatch(error) => Some(error),
            Self::Aggregate { .. } => None,
        }
    }
}

impl AuxiliaryLimits {
    pub(super) fn check(
        self,
        batches: &[compass_languages::SemanticEvidenceBatch],
    ) -> Result<(), AuxiliaryLimitError> {
        let mut totals = [0_usize; 4];
        let mut exceeded = None;
        for batch in batches {
            let proof = compass_languages::TypeScriptArgumentUsage::for_batch(batch)
                .map_err(AuxiliaryLimitError::InvalidBatch)?;
            let loads = compass_languages::TypeScriptMergeDependencyUsage::for_dependencies(
                &batch.typescript_merge_dependencies,
            )
            .map_err(AuxiliaryLimitError::InvalidBatch)?;
            for (index, (resource, count, limit)) in [
                (
                    "TypeScript type-argument nodes",
                    proof.nodes,
                    self.type_nodes,
                ),
                (
                    "TypeScript type-argument bytes",
                    proof.bytes,
                    self.type_bytes,
                ),
                (
                    "TypeScript merge dependencies",
                    loads.count,
                    self.merge_dependencies,
                ),
                (
                    "TypeScript merge dependency bytes",
                    loads.text_bytes,
                    self.merge_bytes,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                totals[index] = totals[index].saturating_add(count);
                if totals[index] > limit && exceeded.is_none() {
                    exceeded = Some(AuxiliaryLimitError::Aggregate {
                        resource,
                        count: totals[index],
                        limit,
                    });
                }
            }
        }
        // Even after aggregate overflow, inspect every remaining batch's own
        // hard limits. The exact-only fallback may discard auxiliary payloads
        // only after proving they were valid individually.
        exceeded.map_or(Ok(()), Err)
    }
}

/// Immutable per-lookup budget used by bounded resolver traversals.
///
/// The public contract currently exposes one limit for candidate storage,
/// traversal depth, and visited-state bounds. Keeping that mapping here makes
/// those uses explicit without changing their numeric behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LookupBudget {
    candidates_per_lookup: usize,
}

impl From<UniversalResolutionLimits> for LookupBudget {
    fn from(limits: UniversalResolutionLimits) -> Self {
        Self {
            candidates_per_lookup: limits.candidates_per_lookup,
        }
    }
}

impl LookupBudget {
    pub(super) const fn candidates_per_lookup(self) -> usize {
        self.candidates_per_lookup
    }
}
