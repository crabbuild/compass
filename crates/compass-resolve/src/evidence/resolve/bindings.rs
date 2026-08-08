//! Exact binding, alias, and bound-member target resolution.

use super::super::*;

impl UniversalResolutionIndex {
    pub(in crate::evidence) fn binding_target_is_internal(&self, binding: &BindingFact) -> bool {
        let Some(owner) = binding
            .scope_id
            .as_deref()
            .and_then(|scope_id| self.facts.scopes.get(scope_id))
            .and_then(|scope| scope.owner_declaration_id.as_deref())
            .and_then(|declaration_id| self.facts.declarations.get(declaration_id))
        else {
            return false;
        };
        qualified_root(&owner.qualified_name) == qualified_root(&binding.qualified_target)
            || (binding.language == "rust"
                && self
                    .indexes
                    .rust_source_wildcard_targets
                    .contains(&binding.qualified_target))
    }

    pub(in crate::evidence) fn resolve_explicit_binding(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let binding = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.facts.bindings.get(binding_id))?;
        // Wildcards are a search scope, not an exact spelling. Let lexical and
        // module resolution run before the dedicated wildcard stage below.
        if binding.spelling == "*" {
            return None;
        }
        let qualified_occurrence = self
            .occurrence(candidate)
            .is_some_and(|occurrence| occurrence.qualifier.is_some());
        if !qualified_occurrence
            && let Some(target) = binding.target_declaration_id.as_ref()
            && self.declaration_allowed(target, candidate)
        {
            return Some(ResolutionDecision::Resolved {
                declaration_id: target.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExplicitBinding,
                    candidate_count: 1,
                },
            });
        }
        match self.bound_member_target(language, binding, candidate) {
            Ok(Some(qualified)) => {
                let key = (language.to_owned(), qualified.clone());
                if let Some(decision) = self.unique_decision(
                    self.indexes.by_qualified.get(&key),
                    candidate,
                    ResolutionRule::MemberBinding,
                ) {
                    return Some(decision);
                }
                if let Some(decision) = self.member_decision(language, &qualified, candidate) {
                    return Some(decision);
                }
                match self.rust_trait_member_declarations(language, &qualified, candidate) {
                    Ok(declarations) => {
                        if let Some(decision) = self.unique_decision(
                            Some(&declarations),
                            candidate,
                            ResolutionRule::MemberBinding,
                        ) {
                            return Some(decision);
                        }
                    }
                    Err(candidate_count) => {
                        return Some(ResolutionDecision::Ambiguous { candidate_count });
                    }
                }
                if let Some(decision) =
                    self.imported_member_decision(language, &qualified, candidate)
                {
                    return Some(decision);
                }
                if let Some(decision) =
                    self.wildcard_reexport_decision(language, &qualified, candidate)
                {
                    return Some(decision);
                }
                return Some(ResolutionDecision::QualifiedExternal {
                    qualified_name: qualified,
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::QualifiedExternal,
                        candidate_count: 0,
                    },
                });
            }
            Ok(None) if binding.kind == compass_languages::BindingKind::CallResult => {
                if let Some(fallback_binding_id) = binding.fallback_binding_id.as_ref() {
                    let mut fallback = candidate.clone();
                    fallback.binding_id = Some(fallback_binding_id.clone());
                    if let Some(decision) = self.resolve_explicit_binding(language, &fallback)
                        && decision != ResolutionDecision::Unresolved
                    {
                        return Some(decision);
                    }
                }
                // The call-result evidence is an optional refinement. If the
                // project-wide callable or return type is unavailable, keep
                // the candidate on its prior qualified/deferred path instead
                // of suppressing source-valid fallback evidence.
                return None;
            }
            Ok(None) => {}
            Err(candidate_count) => {
                return Some(ResolutionDecision::Ambiguous { candidate_count });
            }
        }
        let binding_lookup = if qualified_occurrence
            || matches!(
                candidate.relation,
                CandidateRelation::Imports | CandidateRelation::Reexports
            ) {
            candidate
                .constraints
                .qualified_name
                .as_deref()
                .unwrap_or(&binding.qualified_target)
        } else {
            &binding.qualified_target
        };
        let qualified = match self.follow_alias(language, binding_lookup) {
            Ok(qualified) => qualified,
            Err(candidate_count) => {
                return Some(ResolutionDecision::Ambiguous { candidate_count });
            }
        };
        let key = (language.to_owned(), qualified.clone());
        let qualified_rule = if language == "rust"
            && qualified_occurrence
            && matches!(
                candidate.relation,
                CandidateRelation::Calls
                    | CandidateRelation::IndirectCalls
                    | CandidateRelation::Constructs
                    | CandidateRelation::AccessesMember
            ) {
            ResolutionRule::MemberBinding
        } else {
            ResolutionRule::ExplicitBinding
        };
        if let Some(decision) = self.unique_decision(
            self.indexes.by_qualified.get(&key),
            candidate,
            qualified_rule,
        ) {
            return Some(decision);
        }
        if let Some(decision) = self.member_decision(language, &qualified, candidate) {
            return Some(decision);
        }
        if let Some(decision) = self.wildcard_reexport_decision(language, &qualified, candidate) {
            return Some(decision);
        }
        if let Some(decision) = self.inventory_decision(language, &qualified, candidate) {
            return Some(decision);
        }
        let imported = self.imported_declarations(
            language,
            &binding.qualified_target,
            &candidate.target_spelling,
        );
        (!imported.is_empty())
            .then(|| {
                self.unique_decision(Some(&imported), candidate, ResolutionRule::ExplicitBinding)
            })
            .flatten()
    }
}
