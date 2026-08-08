//! Read-only views used while resolving one candidate.

use std::ops::Deref;

use compass_languages::RelationshipCandidate;

use super::super::UniversalResolutionIndex;

/// Read-only resolver database passed through the ordered pipeline.
pub(super) struct ResolutionDb<'a> {
    index: &'a UniversalResolutionIndex,
}

impl<'a> ResolutionDb<'a> {
    pub(super) const fn new(index: &'a UniversalResolutionIndex) -> Self {
        Self { index }
    }
}

impl Deref for ResolutionDb<'_> {
    type Target = UniversalResolutionIndex;

    fn deref(&self) -> &Self::Target {
        self.index
    }
}

/// Normalized, borrowed context shared by the generic resolution stages.
pub(super) struct CandidateContext<'a> {
    pub(super) candidate: &'a RelationshipCandidate,
    pub(super) language: &'a str,
    pub(super) qualifier: Option<&'a str>,
}

impl<'a> CandidateContext<'a> {
    pub(super) fn new(db: &'a ResolutionDb<'a>, candidate: &'a RelationshipCandidate) -> Self {
        let language = candidate
            .constraints
            .exact_language
            .as_deref()
            .unwrap_or(&candidate.language);
        let qualifier = db
            .occurrence(candidate)
            .and_then(|occurrence| occurrence.qualifier.as_deref());
        Self {
            candidate,
            language,
            qualifier,
        }
    }

    pub(super) fn has_unbound_qualified_receiver(&self) -> bool {
        self.qualifier.is_some_and(|qualifier| {
            self.candidate.binding_id.is_none()
                && !matches!((self.language, qualifier), ("python", "self" | "cls"))
        })
    }

    pub(super) fn allows_lexical_lookup(&self) -> bool {
        self.qualifier.is_none()
            || self.qualifier.is_some_and(|qualifier| {
                self.candidate.binding_id.is_none()
                    && matches!((self.language, qualifier), ("python", "self" | "cls"))
            })
    }
}
