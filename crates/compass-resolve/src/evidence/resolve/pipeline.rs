//! Ordered orchestration for resolving one relationship candidate.

use super::super::*;
use super::context::{CandidateContext, ResolutionDb};
use super::outcome::StageOutcome;
use crate::evidence::languages::policy::LanguagePolicyKind;

impl UniversalResolutionIndex {
    pub fn resolve(&self, candidate_id: &str) -> ResolutionDecision {
        ResolutionDb::new(self).resolve(candidate_id)
    }
}

impl ResolutionDb<'_> {
    fn resolve(&self, candidate_id: &str) -> ResolutionDecision {
        let Some(candidate) = self.facts.candidates.get(candidate_id) else {
            return ResolutionDecision::Unresolved;
        };
        let language = candidate
            .constraints
            .exact_language
            .as_deref()
            .unwrap_or(&candidate.language);
        if let Some(target) = candidate.constraints.exact_target_declaration_id.as_ref()
            && self.declaration_allowed(target, candidate)
        {
            return ResolutionDecision::Resolved {
                declaration_id: target.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExactSourceDeclaration,
                    candidate_count: 1,
                },
            };
        }
        if let Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name,
            strategy,
        }) = candidate.constraints.hierarchy.as_ref()
        {
            return self.resolve_c3_receiver_dispatch(
                language,
                receiver_qualified_name,
                *strategy,
                candidate,
            );
        }
        if let Some(HierarchyConstraint::RustAssociatedType {
            receiver_declaration_id,
            receiver_qualified_name,
            trait_qualified_name,
        }) = candidate.constraints.hierarchy.as_ref()
        {
            return self.resolve_rust_associated_type(
                language,
                receiver_declaration_id,
                receiver_qualified_name,
                trait_qualified_name,
                candidate,
            );
        }
        if let StageOutcome::Decided(decision) = StageOutcome::from_optional(
            LanguagePolicyKind::for_language(language)
                .resolve_import_candidate(self, language, candidate),
        ) {
            return decision;
        }
        if let Some(decision) = self.resolve_explicit_binding(language, candidate) {
            return decision;
        }
        let fallback_candidate = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.facts.bindings.get(binding_id))
            .filter(|binding| binding.kind == compass_languages::BindingKind::CallResult)
            .map(|binding| {
                let mut fallback = candidate.clone();
                fallback.binding_id.clone_from(&binding.fallback_binding_id);
                fallback
            });
        let candidate = fallback_candidate.as_ref().unwrap_or(candidate);
        let context = CandidateContext::new(self, candidate);
        let qualifier = context.qualifier;
        let has_unbound_qualified_receiver = context.has_unbound_qualified_receiver();
        let allows_lexical_lookup = context.allows_lexical_lookup();
        if matches!(
            candidate.constraints.hierarchy.as_ref(),
            Some(HierarchyConstraint::DirectBase { .. })
        ) {
            return self.resolve_direct_base(language, candidate);
        }

        if matches!(
            candidate.relation,
            CandidateRelation::Contains | CandidateRelation::Owns
        ) && let Some(qualified) = candidate.constraints.qualified_name.as_ref()
        {
            let qualified = match self.follow_alias(language, qualified) {
                Ok(qualified) => qualified,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
            if let Some(decision) = self.unique_decision(
                self.indexes
                    .by_qualified
                    .get(&(language.to_owned(), qualified.clone())),
                candidate,
                ResolutionRule::ExplicitBinding,
            ) {
                return decision;
            }
        }

        if allows_lexical_lookup && let Some(scope) = candidate.constraints.scope_id.as_deref() {
            let mut cursor = Some(scope);
            let mut visited = BTreeSet::new();
            while let Some(scope) = cursor.filter(|scope| visited.insert((*scope).to_owned())) {
                let key = (
                    language.to_owned(),
                    scope.to_owned(),
                    candidate.target_spelling.clone(),
                );
                if let Some(decision) = self.unique_decision(
                    self.indexes.by_scope_name.get(&key),
                    candidate,
                    ResolutionRule::ExactLexicalDeclaration,
                ) {
                    return decision;
                }
                cursor = self
                    .facts
                    .scopes
                    .get(scope)
                    .and_then(|scope| scope.parent_scope_id.as_deref());
            }
        }

        if let Some(qualified) = candidate.constraints.qualified_name.as_ref() {
            let qualified = match self.follow_alias(language, qualified) {
                Ok(qualified) => qualified,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
            let key = (language.to_owned(), qualified.clone());
            if let Some(decision) = self.unique_decision(
                self.indexes.by_qualified.get(&key),
                candidate,
                ResolutionRule::ExplicitBinding,
            ) {
                return decision;
            }
            if let Some(decision) = self.member_decision(language, &qualified, candidate) {
                return decision;
            }
            if let Some(decision) = self.wildcard_reexport_decision(language, &qualified, candidate)
            {
                return decision;
            }
            if let Some(decision) = self.imported_member_decision(language, &qualified, candidate) {
                return decision;
            }
            if let Some(decision) = self.inventory_decision(language, &qualified, candidate) {
                return decision;
            }
        }

        if !has_unbound_qualified_receiver
            && let Some(module) = candidate.constraints.module_or_package.as_ref()
        {
            let key = (
                language.to_owned(),
                module.clone(),
                candidate.target_spelling.clone(),
            );
            if let Some(decision) = self.unique_decision(
                self.indexes.by_module_name.get(&key),
                candidate,
                ResolutionRule::UniqueModuleOrPackage,
            ) {
                return decision;
            }
        }

        if let Some(decision) = self.resolve_wildcard_binding(language, candidate) {
            return decision;
        }
        if let Some(decision) = self.resolve_visible_wildcard_bindings(language, candidate) {
            return decision;
        }

        if candidate.constraints.allow_external
            && let Some(qualified_name) = candidate.constraints.qualified_name.clone()
        {
            return ResolutionDecision::QualifiedExternal {
                qualified_name,
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::QualifiedExternal,
                    candidate_count: 0,
                },
            };
        }
        if matches!(
            candidate.relation,
            CandidateRelation::Calls | CandidateRelation::IndirectCalls | CandidateRelation::Tests
        ) && let Some(qualified_name) = candidate.constraints.qualified_name.clone()
            && qualifier.is_some_and(is_deferred_receiver)
        {
            return ResolutionDecision::DeferredReceiver {
                qualified_name,
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::DeferredReceiver,
                    candidate_count: 0,
                },
            };
        }
        ResolutionDecision::Unresolved
    }
}
