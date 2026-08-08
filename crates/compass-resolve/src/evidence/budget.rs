//! Semantic wrappers around public universal resolution limits.

use super::UniversalResolutionLimits;

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
