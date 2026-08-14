//! C# namespace and project-aware resolution policy.

use std::collections::BTreeSet;

use compass_languages::{BindingKind, HierarchyConstraint, RelationshipCandidate};

use super::super::{ResolutionDecision, ResolutionEvidence, ResolutionRule};
use crate::evidence::resolve::context::ResolutionDb;

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve_csharp_candidate(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        match candidate.constraints.hierarchy.as_ref() {
            Some(HierarchyConstraint::DirectBase { .. }) => {
                Some(self.resolve_csharp_direct_base(language, candidate))
            }
            Some(HierarchyConstraint::ReceiverDispatch {
                receiver_qualified_name,
                strategy,
            }) => Some(self.resolve_csharp_receiver(
                language,
                receiver_qualified_name,
                *strategy,
                candidate,
            )),
            Some(HierarchyConstraint::RustAssociatedType { .. }) | None => None,
        }
    }

    fn resolve_csharp_receiver(
        &self,
        language: &str,
        receiver: &str,
        strategy: compass_languages::ReceiverDispatchStrategy,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        let qualified_names = match self.csharp_type_names(language, candidate, receiver, None) {
            Ok(names) => names,
            Err(candidate_count) => return ResolutionDecision::Ambiguous { candidate_count },
        };
        let mut resolved = std::collections::BTreeMap::new();
        let mut ambiguous_count = 0_usize;
        for receiver in qualified_names {
            if self.exact_hierarchy_type(language, &receiver).is_none() {
                continue;
            }
            let mut qualified_candidate = candidate.clone();
            qualified_candidate.constraints.hierarchy =
                Some(HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name: receiver.clone(),
                    strategy,
                });
            match self.resolve_c3_receiver_dispatch(
                language,
                &receiver,
                strategy,
                &qualified_candidate,
            ) {
                ResolutionDecision::Resolved {
                    declaration_id,
                    evidence,
                } => {
                    resolved.insert(declaration_id, evidence);
                }
                ResolutionDecision::Ambiguous { candidate_count } => {
                    ambiguous_count = ambiguous_count.saturating_add(candidate_count);
                }
                ResolutionDecision::Unresolved
                | ResolutionDecision::QualifiedExternal { .. }
                | ResolutionDecision::ResolvedInventory { .. }
                | ResolutionDecision::DeferredReceiver { .. } => {}
            }
            if resolved.len().saturating_add(ambiguous_count) > self.budget.candidates_per_lookup()
            {
                return ResolutionDecision::Ambiguous {
                    candidate_count: resolved.len().saturating_add(ambiguous_count),
                };
            }
        }
        match resolved.into_iter().collect::<Vec<_>>().as_slice() {
            [(declaration_id, evidence)] if ambiguous_count == 0 => ResolutionDecision::Resolved {
                declaration_id: declaration_id.clone(),
                evidence: evidence.clone(),
            },
            [] if ambiguous_count == 0 => ResolutionDecision::Unresolved,
            resolved => ResolutionDecision::Ambiguous {
                candidate_count: resolved.len().saturating_add(ambiguous_count),
            },
        }
    }

    fn resolve_csharp_direct_base(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        let qualified_names = match self.csharp_type_names(
            language,
            candidate,
            &candidate.target_spelling,
            candidate.constraints.qualified_name.as_deref(),
        ) {
            Ok(names) => names,
            Err(candidate_count) => return ResolutionDecision::Ambiguous { candidate_count },
        };

        let mut eligible = BTreeSet::new();
        for qualified in &qualified_names {
            if let Some(slots) = self
                .indexes
                .names
                .by_qualified
                .get(&(language.to_owned(), qualified.clone()))
            {
                eligible.extend(
                    slots
                        .iter()
                        .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                        .copied()
                        .take(self.budget.candidates_per_lookup().saturating_add(1)),
                );
            }
            if eligible.len() > self.budget.candidates_per_lookup() {
                return ResolutionDecision::Ambiguous {
                    candidate_count: eligible.len(),
                };
            }
        }
        match eligible.iter().copied().collect::<Vec<_>>().as_slice() {
            [only] => self.declaration_id(*only).map_or(
                ResolutionDecision::Unresolved,
                |declaration_id| ResolutionDecision::Resolved {
                    declaration_id: declaration_id.to_owned(),
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::ExactHierarchyBase,
                        candidate_count: 1,
                    },
                },
            ),
            [] if candidate.constraints.allow_external
                && candidate.constraints.qualified_name.is_some() =>
            {
                ResolutionDecision::QualifiedExternal {
                    qualified_name: qualified_names
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| candidate.target_spelling.clone()),
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::QualifiedExternal,
                        candidate_count: 0,
                    },
                }
            }
            [] => ResolutionDecision::Unresolved,
            many => ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            },
        }
    }

    fn csharp_type_names(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
        spelling: &str,
        explicit_qualified: Option<&str>,
    ) -> Result<BTreeSet<String>, usize> {
        let Some(owner) = self
            .facts
            .declarations
            .get(&candidate.source_declaration_id)
        else {
            return Ok(BTreeSet::new());
        };
        let source_file = self
            .occurrence(candidate)
            .map(|occurrence| occurrence.range().source_file.as_str())
            .unwrap_or(owner.range.source_file.as_str());
        let mut qualified_names = BTreeSet::new();
        if let Some(qualified) = explicit_qualified {
            qualified_names.insert(self.follow_alias(language, qualified)?);
            return Ok(qualified_names);
        }
        if spelling.contains('.') {
            qualified_names.insert(self.follow_alias(language, spelling)?);
            return Ok(qualified_names);
        }
        if let Some(namespace) = owner.module_or_package.as_deref() {
            qualified_names.insert(format!("{namespace}.{spelling}"));
        }
        qualified_names.insert(spelling.to_owned());
        for binding in self
            .indexes
            .csharp
            .bindings_by_source
            .get(source_file)
            .into_iter()
            .flatten()
        {
            match binding.kind {
                BindingKind::ImportAlias if binding.spelling == spelling => {
                    qualified_names.insert(binding.qualified_target.clone());
                }
                BindingKind::Import if !binding.qualified_target.is_empty() => {
                    qualified_names.insert(format!("{}.{}", binding.qualified_target, spelling));
                }
                BindingKind::Import
                | BindingKind::ImportAlias
                | BindingKind::Reexport
                | BindingKind::LocalAlias
                | BindingKind::CallResult
                | BindingKind::Package
                | BindingKind::Member => {}
            }
        }
        Ok(qualified_names)
    }
}
