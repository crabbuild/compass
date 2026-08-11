//! Read-only views used while resolving one candidate.

use super::super::*;

/// Read-only resolver database passed through the ordered pipeline.
pub(in crate::evidence) struct ResolutionDb<'a> {
    pub(in crate::evidence) facts: &'a FactStore,
    pub(in crate::evidence) indexes: &'a ResolutionIndexes,
    pub(in crate::evidence) project: &'a ProjectContext,
    pub(in crate::evidence) budget: LookupBudget,
}

impl<'a> ResolutionDb<'a> {
    pub(in crate::evidence) const fn new(index: &'a UniversalResolutionIndex) -> Self {
        Self {
            facts: &index.facts,
            indexes: &index.indexes,
            project: &index.project,
            budget: index.budget,
        }
    }
}

/// Normalized, borrowed context shared by the generic resolution stages.
pub(super) struct CandidateContext<'a> {
    original: &'a RelationshipCandidate,
    fallback: Option<RelationshipCandidate>,
    pub(super) language: &'a str,
}

impl<'a> CandidateContext<'a> {
    pub(super) fn new(db: &'a ResolutionDb<'a>, candidate: &'a RelationshipCandidate) -> Self {
        let language = candidate
            .constraints
            .exact_language
            .as_deref()
            .unwrap_or(&candidate.language);
        let fallback = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| db.facts.bindings.get(binding_id))
            .filter(|binding| binding.kind == compass_languages::BindingKind::CallResult)
            .map(|binding| {
                let mut fallback = candidate.clone();
                fallback.binding_id.clone_from(&binding.fallback_binding_id);
                fallback
            });
        Self {
            original: candidate,
            fallback,
            language,
        }
    }

    pub(super) const fn original(&self) -> &RelationshipCandidate {
        self.original
    }

    pub(super) fn candidate(&self) -> &RelationshipCandidate {
        self.fallback.as_ref().unwrap_or(self.original)
    }

    pub(super) fn qualifier<'b>(&'b self, db: &'b ResolutionDb<'_>) -> Option<&'b str> {
        db.occurrence(self.candidate())
            .and_then(OccurrenceRef::qualifier)
    }

    pub(super) fn has_unbound_qualified_receiver(&self, db: &ResolutionDb<'_>) -> bool {
        self.qualifier(db).is_some_and(|qualifier| {
            self.candidate().binding_id.is_none()
                && !matches!((self.language, qualifier), ("python", "self" | "cls"))
        })
    }

    pub(super) fn allows_lexical_lookup(&self, db: &ResolutionDb<'_>) -> bool {
        self.qualifier(db).is_none()
            || self.qualifier(db).is_some_and(|qualifier| {
                self.candidate().binding_id.is_none()
                    && matches!((self.language, qualifier), ("python", "self" | "cls"))
            })
    }
}
