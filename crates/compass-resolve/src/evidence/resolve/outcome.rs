//! Explicit control flow between ordered resolution stages.

use super::super::ResolutionDecision;

pub(super) enum StageOutcome {
    Continue,
    Decided(ResolutionDecision),
}

impl StageOutcome {
    pub(super) fn from_optional(decision: Option<ResolutionDecision>) -> Self {
        decision.map_or(Self::Continue, Self::Decided)
    }
}
