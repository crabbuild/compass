//! Explicit, ordered orchestration for resolving one relationship candidate.

use super::super::*;
use super::context::{CandidateContext, ResolutionDb};
use super::outcome::StageOutcome;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolutionStage {
    ExactSource,
    ReceiverHierarchy,
    LanguagePolicy,
    ExplicitBinding,
    DirectBase,
    QualifiedOwnership,
    Lexical,
    QualifiedTarget,
    ModuleOrPackage,
    Wildcards,
    QualifiedExternal,
    DeferredReceiver,
}

impl ResolutionStage {
    const ORDER: [Self; 12] = [
        Self::ExactSource,
        Self::ReceiverHierarchy,
        Self::LanguagePolicy,
        Self::ExplicitBinding,
        Self::DirectBase,
        Self::QualifiedOwnership,
        Self::Lexical,
        Self::QualifiedTarget,
        Self::ModuleOrPackage,
        Self::Wildcards,
        Self::QualifiedExternal,
        Self::DeferredReceiver,
    ];
}

impl UniversalResolutionIndex {
    pub fn resolve(&self, candidate_id: &str) -> ResolutionDecision {
        ResolutionDb::new(self).resolve(candidate_id)
    }
}

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve(&self, candidate_id: &str) -> ResolutionDecision {
        let Some(candidate) = self.facts.candidates.get(candidate_id) else {
            return ResolutionDecision::Unresolved;
        };
        self.resolve_candidate(candidate)
    }

    pub(in crate::evidence) fn resolve_candidate(
        &self,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        let context = CandidateContext::new(self, candidate);
        for stage in ResolutionStage::ORDER {
            if let StageOutcome::Decided(decision) = self.run_stage(stage, &context) {
                return decision;
            }
        }
        ResolutionDecision::Unresolved
    }

    fn run_stage(&self, stage: ResolutionStage, context: &CandidateContext<'_>) -> StageOutcome {
        match stage {
            ResolutionStage::ExactSource => self.stage_exact_source(context),
            ResolutionStage::ReceiverHierarchy => self.stage_receiver_hierarchy(context),
            ResolutionStage::LanguagePolicy => self.stage_language_policy(context),
            ResolutionStage::ExplicitBinding => self.stage_explicit_binding(context),
            ResolutionStage::DirectBase => self.stage_direct_base(context),
            ResolutionStage::QualifiedOwnership => self.stage_qualified_ownership(context),
            ResolutionStage::Lexical => self.stage_lexical(context),
            ResolutionStage::QualifiedTarget => self.stage_qualified_target(context),
            ResolutionStage::ModuleOrPackage => self.stage_module_or_package(context),
            ResolutionStage::Wildcards => self.stage_wildcards(context),
            ResolutionStage::QualifiedExternal => self.stage_qualified_external(context),
            ResolutionStage::DeferredReceiver => self.stage_deferred_receiver(context),
        }
    }

    fn stage_exact_source(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.original();
        let Some(target) = candidate.constraints.exact_target_declaration_id.as_ref() else {
            return StageOutcome::Continue;
        };
        if !self.declaration_allowed(target, candidate) {
            return StageOutcome::Continue;
        }
        StageOutcome::Decided(ResolutionDecision::Resolved {
            declaration_id: target.clone(),
            evidence: ResolutionEvidence {
                rule: ResolutionRule::ExactSourceDeclaration,
                candidate_count: 1,
            },
        })
    }

    fn stage_receiver_hierarchy(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.original();
        let Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name,
            strategy,
        }) = candidate.constraints.hierarchy.as_ref()
        else {
            return StageOutcome::Continue;
        };
        StageOutcome::Decided(self.resolve_c3_receiver_dispatch(
            context.language,
            receiver_qualified_name,
            *strategy,
            candidate,
        ))
    }

    fn stage_language_policy(&self, context: &CandidateContext<'_>) -> StageOutcome {
        StageOutcome::from_optional(
            LanguagePolicyKind::for_language(context.language).resolve_candidate(
                self,
                context.language,
                context.original(),
            ),
        )
    }

    fn stage_explicit_binding(&self, context: &CandidateContext<'_>) -> StageOutcome {
        StageOutcome::from_optional(
            self.resolve_explicit_binding(context.language, context.original()),
        )
    }

    fn stage_direct_base(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.candidate();
        if !matches!(
            candidate.constraints.hierarchy.as_ref(),
            Some(HierarchyConstraint::DirectBase { .. })
        ) {
            return StageOutcome::Continue;
        }
        StageOutcome::Decided(self.resolve_direct_base(context.language, candidate))
    }

    fn stage_qualified_ownership(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.candidate();
        if !matches!(
            candidate.relation,
            CandidateRelation::Contains | CandidateRelation::Owns
        ) {
            return StageOutcome::Continue;
        }
        let Some(qualified) = candidate.constraints.qualified_name.as_ref() else {
            return StageOutcome::Continue;
        };
        let qualified = match self.follow_alias(context.language, qualified) {
            Ok(qualified) => qualified,
            Err(candidate_count) => {
                return StageOutcome::Decided(ResolutionDecision::Ambiguous { candidate_count });
            }
        };
        StageOutcome::from_optional(
            self.unique_decision(
                self.indexes
                    .names
                    .by_qualified
                    .get(&(context.language.to_owned(), qualified)),
                candidate,
                ResolutionRule::ExplicitBinding,
            ),
        )
    }

    fn stage_lexical(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.candidate();
        if !context.allows_lexical_lookup(self) {
            return StageOutcome::Continue;
        }
        let Some(scope) = candidate.constraints.scope_id.as_deref() else {
            return StageOutcome::Continue;
        };
        let mut cursor = Some(scope);
        let mut visited = BTreeSet::new();
        while let Some(scope) = cursor.filter(|scope| visited.insert((*scope).to_owned())) {
            let key = (
                context.language.to_owned(),
                scope.to_owned(),
                candidate.target_spelling.clone(),
            );
            if let Some(decision) = self.unique_decision(
                self.indexes.names.by_scope_name.get(&key),
                candidate,
                ResolutionRule::ExactLexicalDeclaration,
            ) {
                return StageOutcome::Decided(decision);
            }
            cursor = self
                .facts
                .scopes
                .get(scope)
                .and_then(|scope| scope.parent_scope_id.as_deref());
        }
        StageOutcome::Continue
    }

    fn stage_qualified_target(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.candidate();
        let Some(qualified) = candidate.constraints.qualified_name.as_ref() else {
            return StageOutcome::Continue;
        };
        let qualified = match self.follow_alias(context.language, qualified) {
            Ok(qualified) => qualified,
            Err(candidate_count) => {
                return StageOutcome::Decided(ResolutionDecision::Ambiguous { candidate_count });
            }
        };
        let key = (context.language.to_owned(), qualified.clone());
        if let Some(decision) = [
            self.unique_decision(
                self.indexes.names.by_qualified.get(&key),
                candidate,
                ResolutionRule::ExplicitBinding,
            ),
            self.member_decision(context.language, &qualified, candidate),
            self.wildcard_reexport_decision(context.language, &qualified, candidate),
            self.imported_member_decision(context.language, &qualified, candidate),
            self.inventory_decision(context.language, &qualified, candidate),
        ]
        .into_iter()
        .flatten()
        .next()
        {
            return StageOutcome::Decided(decision);
        }
        StageOutcome::Continue
    }

    fn stage_module_or_package(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.candidate();
        if context.has_unbound_qualified_receiver(self) {
            return StageOutcome::Continue;
        }
        let Some(module) = candidate.constraints.module_or_package.as_ref() else {
            return StageOutcome::Continue;
        };
        let key = (
            context.language.to_owned(),
            module.clone(),
            candidate.target_spelling.clone(),
        );
        StageOutcome::from_optional(self.unique_decision(
            self.indexes.names.by_module_name.get(&key),
            candidate,
            ResolutionRule::UniqueModuleOrPackage,
        ))
    }

    fn stage_wildcards(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.candidate();
        StageOutcome::from_optional(
            self.resolve_wildcard_binding(context.language, candidate)
                .or_else(|| self.resolve_visible_wildcard_bindings(context.language, candidate)),
        )
    }

    fn stage_qualified_external(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.candidate();
        let Some(qualified_name) = candidate
            .constraints
            .allow_external
            .then(|| candidate.constraints.qualified_name.clone())
            .flatten()
        else {
            return StageOutcome::Continue;
        };
        StageOutcome::Decided(ResolutionDecision::QualifiedExternal {
            qualified_name,
            evidence: ResolutionEvidence {
                rule: ResolutionRule::QualifiedExternal,
                candidate_count: 0,
            },
        })
    }

    fn stage_deferred_receiver(&self, context: &CandidateContext<'_>) -> StageOutcome {
        let candidate = context.candidate();
        if !matches!(
            candidate.relation,
            CandidateRelation::Calls | CandidateRelation::IndirectCalls | CandidateRelation::Tests
        ) {
            return StageOutcome::Continue;
        }
        let Some(qualified_name) = candidate.constraints.qualified_name.clone() else {
            return StageOutcome::Continue;
        };
        if !context.qualifier(self).is_some_and(is_deferred_receiver) {
            return StageOutcome::Continue;
        }
        StageOutcome::Decided(ResolutionDecision::DeferredReceiver {
            qualified_name,
            evidence: ResolutionEvidence {
                rule: ResolutionRule::DeferredReceiver,
                candidate_count: 0,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ResolutionStage;

    #[test]
    fn stage_order_is_the_resolution_precedence_contract() {
        use ResolutionStage::{
            DeferredReceiver, DirectBase, ExactSource, ExplicitBinding, LanguagePolicy, Lexical,
            ModuleOrPackage, QualifiedExternal, QualifiedOwnership, QualifiedTarget,
            ReceiverHierarchy, Wildcards,
        };

        assert!(matches!(
            ResolutionStage::ORDER,
            [
                ExactSource,
                ReceiverHierarchy,
                LanguagePolicy,
                ExplicitBinding,
                DirectBase,
                QualifiedOwnership,
                Lexical,
                QualifiedTarget,
                ModuleOrPackage,
                Wildcards,
                QualifiedExternal,
                DeferredReceiver,
            ]
        ));
    }
}
