//! Explicit, ordered orchestration for resolving one relationship candidate.

use super::super::*;
use super::context::{CandidateContext, ResolutionDb};
use super::outcome::StageOutcome;
use crate::ResolutionAdmission;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolutionStage {
    ExactSource,
    LanguagePolicy,
    ReceiverHierarchy,
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
        Self::LanguagePolicy,
        Self::ReceiverHierarchy,
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
        let indexes = self
            .indexes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ResolutionDb::new(self, &indexes).resolve(candidate_id)
    }
}

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve(&self, candidate_id: &str) -> ResolutionDecision {
        let Some(candidate) = self.facts.candidates.get(candidate_id) else {
            return ResolutionDecision::Unresolved;
        };
        self.resolve_candidate(&candidate, ResolutionAdmission::Max)
    }

    pub(in crate::evidence) fn resolve_candidate(
        &self,
        candidate: &RelationshipCandidate,
        admission: ResolutionAdmission,
    ) -> ResolutionDecision {
        let context = CandidateContext::new(self, candidate);
        for stage in ResolutionStage::ORDER {
            if !stage.is_admitted(admission, candidate) {
                continue;
            }
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
        // The Ruby extractor records the receiver as the lexical fallback
        // (`QueryTest::Arel`, for example) when the source does not declare a
        // same-file constant.  Resolve the raw constant spelling against the
        // enclosing lexical owner before committing to that speculative
        // hierarchy.  The lookup remains exact and language-scoped; it never
        // falls back to a terminal method name.
        if let Some(decision) = self.ruby_lexical_target_decision(candidate) {
            return StageOutcome::Decided(decision);
        }
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
        // Ruby receiver-dispatch candidates intentionally leave
        // `qualified_name` empty: the receiver hierarchy is the authoritative
        // constraint.  A qualified constant receiver can still be proven by
        // Ruby's lexical lookup rules, so run that exact-name pass before the
        // generic qualified-target guard.
        if candidate.constraints.hierarchy.is_none()
            && let Some(decision) = self.ruby_lexical_target_decision(candidate)
        {
            return StageOutcome::Decided(decision);
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
        if let Some(decision) = self.ruby_import_decision(candidate, &qualified) {
            return StageOutcome::Decided(decision);
        }
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

    fn ruby_lexical_target_decision(
        &self,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if candidate.language != "ruby"
            || !matches!(
                candidate.relation,
                CandidateRelation::Calls
                    | CandidateRelation::Constructs
                    | CandidateRelation::Extends
                    | CandidateRelation::UsesTrait
            )
        {
            return None;
        }
        let occurrence = self.occurrence(candidate)?;
        let qualifier = match candidate.relation {
            CandidateRelation::Calls | CandidateRelation::Constructs => occurrence.qualifier()?,
            CandidateRelation::Extends | CandidateRelation::UsesTrait => occurrence.spelling(),
            _ => return None,
        };
        let normalized = qualifier.trim().trim_start_matches("::");
        if normalized.is_empty()
            || !normalized.split("::").all(|part| {
                let mut characters = part.chars();
                characters.next().is_some_and(char::is_uppercase)
            })
        {
            return None;
        }
        let source = self
            .facts
            .declarations
            .get(&candidate.source_declaration_id)?;
        let names = languages::ruby::lexical_names(&source.qualified_name, qualifier);
        let context = self.occurrence(candidate).and_then(OccurrenceRef::context);
        let mut slots = BTreeSet::new();
        for name in names {
            let qualified = match candidate.relation {
                CandidateRelation::Constructs
                | CandidateRelation::Extends
                | CandidateRelation::UsesTrait => name,
                CandidateRelation::Calls => {
                    let separator = match context {
                        Some("singleton") => '.',
                        Some("instance") => '#',
                        _ => continue,
                    };
                    format!("{name}{separator}{}", candidate.target_spelling)
                }
                _ => continue,
            };
            if let Some(ids) = self
                .indexes
                .names
                .by_qualified
                .get(&("ruby".to_owned(), qualified))
            {
                slots.extend(
                    ids.iter()
                        .copied()
                        .filter(|slot| self.declaration_allowed_slot(*slot, candidate)),
                );
            }
        }
        let candidates = slots.into_iter().collect::<Vec<_>>();
        self.unique_decision(
            (!candidates.is_empty()).then_some(&candidates),
            candidate,
            ResolutionRule::ExactLexicalDeclaration,
        )
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
        if is_language_builtin_qualified_target(context.language, &qualified_name)
            && !self.rust_builtin_external_candidate(context, candidate)
        {
            return StageOutcome::Decided(ResolutionDecision::Unresolved);
        }
        StageOutcome::Decided(ResolutionDecision::QualifiedExternal {
            qualified_name,
            evidence: ResolutionEvidence {
                rule: ResolutionRule::QualifiedExternal,
                candidate_count: 0,
            },
        })
    }

    /// Rust's prelude names are intentionally not published as graph hubs for
    /// ordinary constructor calls (`Vec::new`, `Box::new`, and friends).  Two
    /// forms still carry useful, source-backed evidence: a receiver whose
    /// concrete type was inferred from `self`, and a qualified call in a
    /// scope with an explicit wildcard import (where the imported module may
    /// provide a project/dependency symbol that is outside the corpus).
    fn rust_builtin_external_candidate(
        &self,
        context: &CandidateContext<'_>,
        candidate: &RelationshipCandidate,
    ) -> bool {
        if context.language != "rust" || candidate.relation != CandidateRelation::Calls {
            return false;
        }
        if self
            .occurrence(candidate)
            .and_then(OccurrenceRef::qualifier)
            .is_some_and(|qualifier| qualifier == "self")
        {
            return true;
        }
        let mut scope_id = candidate.constraints.scope_id.as_deref();
        let mut visited = BTreeSet::new();
        while let Some(scope) = scope_id.filter(|scope| visited.insert((*scope).to_owned())) {
            if self
                .indexes
                .wildcards
                .by_scope
                .contains_key(&(context.language.to_owned(), scope.to_owned()))
            {
                return true;
            }
            scope_id = self
                .facts
                .scopes
                .get(scope)
                .and_then(|scope| scope.parent_scope_id.as_deref());
        }
        false
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

impl ResolutionStage {
    fn is_admitted(
        self,
        admission: ResolutionAdmission,
        candidate: &RelationshipCandidate,
    ) -> bool {
        match self {
            Self::QualifiedExternal => {
                admission.admits_qualified_external() || candidate.binding_id.is_some()
            }
            Self::DeferredReceiver => admission.admits_deferred_receiver(),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use compass_languages::{CandidateRelation, RelationshipCandidate, ResolutionConstraint};

    use super::ResolutionStage;
    use crate::ResolutionAdmission;

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
                LanguagePolicy,
                ReceiverHierarchy,
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

    #[test]
    fn inference_only_stages_are_never_entered_below_their_admission_level() {
        let mut candidate = RelationshipCandidate {
            id: "candidate".to_owned(),
            language: "python".to_owned(),
            relation: CandidateRelation::Calls,
            source_declaration_id: "caller".to_owned(),
            occurrence_id: Some("occurrence".to_owned()),
            binding_id: None,
            target_spelling: "execute".to_owned(),
            constraints: ResolutionConstraint::default(),
        };

        assert!(
            !ResolutionStage::QualifiedExternal.is_admitted(ResolutionAdmission::Low, &candidate)
        );
        assert!(
            !ResolutionStage::QualifiedExternal
                .is_admitted(ResolutionAdmission::Medium, &candidate)
        );
        assert!(
            ResolutionStage::QualifiedExternal.is_admitted(ResolutionAdmission::High, &candidate)
        );
        assert!(
            !ResolutionStage::DeferredReceiver.is_admitted(ResolutionAdmission::High, &candidate)
        );
        assert!(
            ResolutionStage::DeferredReceiver.is_admitted(ResolutionAdmission::Max, &candidate)
        );

        candidate.binding_id = Some("explicit-import".to_owned());
        assert!(
            ResolutionStage::QualifiedExternal.is_admitted(ResolutionAdmission::Low, &candidate)
        );
    }
}
