//! PHP case-folded symbol and trait-precedence resolution.

use std::collections::BTreeSet;

use compass_languages::{
    CandidateRelation, HierarchyConstraint, ReceiverDispatchStrategy, RelationshipCandidate,
};

use super::super::{ResolutionDecision, ResolutionEvidence, ResolutionRule};
use crate::evidence::resolve::context::ResolutionDb;

enum PhpMemberLookup {
    Found(BTreeSet<String>),
    Absent,
    Unknown,
}

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve_php_candidate(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if let Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name,
            strategy,
        }) = candidate.constraints.hierarchy.as_ref()
        {
            return Some(self.resolve_php_receiver(
                language,
                receiver_qualified_name,
                *strategy,
                candidate,
            ));
        }
        if candidate.relation != CandidateRelation::Calls
            || candidate.binding_id.is_some()
            || candidate.target_spelling.contains('\\')
        {
            return None;
        }
        let qualified = candidate.constraints.qualified_name.as_ref()?;
        if let Some(decision) = self.unique_decision(
            self.indexes
                .names
                .by_qualified
                .get(&(language.to_owned(), qualified.to_ascii_lowercase())),
            candidate,
            ResolutionRule::ExplicitBinding,
        ) {
            return Some(decision);
        }
        let global = candidate.target_spelling.to_ascii_lowercase();
        if qualified.eq_ignore_ascii_case(&global) {
            return None;
        }
        self.unique_decision(
            self.indexes
                .names
                .by_qualified
                .get(&(language.to_owned(), global)),
            candidate,
            ResolutionRule::PhpGlobalFunctionFallback,
        )
    }

    fn resolve_php_receiver(
        &self,
        language: &str,
        receiver: &str,
        strategy: ReceiverDispatchStrategy,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        let Some(receiver) = self.exact_hierarchy_type(language, &receiver.to_ascii_lowercase())
        else {
            return ResolutionDecision::Unresolved;
        };
        let lookup = match strategy {
            ReceiverDispatchStrategy::C3FromReceiver => {
                self.php_member_lookup(language, &receiver, candidate, &mut BTreeSet::new(), 0)
            }
            ReceiverDispatchStrategy::C3AfterReceiver => self.php_inherited_member_lookup(
                language,
                &receiver,
                candidate,
                &mut BTreeSet::new(),
                0,
            ),
        };
        match lookup {
            PhpMemberLookup::Found(ids) if ids.len() == 1 => {
                let Some(declaration_id) = ids.into_iter().next() else {
                    return ResolutionDecision::Unresolved;
                };
                ResolutionDecision::Resolved {
                    declaration_id,
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::LinearizedReceiverDispatch,
                        candidate_count: 1,
                    },
                }
            }
            PhpMemberLookup::Found(ids) => ResolutionDecision::Ambiguous {
                candidate_count: ids.len(),
            },
            PhpMemberLookup::Absent | PhpMemberLookup::Unknown => ResolutionDecision::Unresolved,
        }
    }

    fn php_member_lookup(
        &self,
        language: &str,
        owner: &str,
        candidate: &RelationshipCandidate,
        visiting: &mut BTreeSet<String>,
        depth: usize,
    ) -> PhpMemberLookup {
        if depth >= self.budget.candidates_per_lookup() || !visiting.insert(owner.to_owned()) {
            return PhpMemberLookup::Unknown;
        }
        let direct = self.php_direct_members(owner, candidate);
        let result = if !direct.is_empty() {
            PhpMemberLookup::Found(direct)
        } else {
            self.php_inherited_member_lookup(language, owner, candidate, visiting, depth)
        };
        visiting.remove(owner);
        result
    }

    fn php_inherited_member_lookup(
        &self,
        language: &str,
        owner: &str,
        candidate: &RelationshipCandidate,
        visiting: &mut BTreeSet<String>,
        depth: usize,
    ) -> PhpMemberLookup {
        let Some(bases) = self
            .indexes
            .hierarchy
            .direct_bases
            .get(&(language.to_owned(), owner.to_owned()))
        else {
            return PhpMemberLookup::Absent;
        };
        if !bases.complete || bases.links.len() > self.budget.candidates_per_lookup() {
            return PhpMemberLookup::Unknown;
        }

        let mut trait_members = BTreeSet::new();
        let mut unknown_trait = false;
        for link in bases
            .links
            .iter()
            .filter(|link| link.relation == CandidateRelation::UsesTrait)
        {
            let Some(trait_name) = link
                .qualified_name
                .as_deref()
                .and_then(|name| self.exact_hierarchy_type(language, name))
            else {
                unknown_trait = true;
                continue;
            };
            match self.php_member_lookup(
                language,
                &trait_name,
                candidate,
                visiting,
                depth.saturating_add(1),
            ) {
                PhpMemberLookup::Found(ids) => trait_members.extend(ids),
                PhpMemberLookup::Unknown => unknown_trait = true,
                PhpMemberLookup::Absent => {}
            }
            if trait_members.len() > self.budget.candidates_per_lookup() {
                return PhpMemberLookup::Unknown;
            }
        }
        if unknown_trait {
            return PhpMemberLookup::Unknown;
        }
        if !trait_members.is_empty() {
            return PhpMemberLookup::Found(trait_members);
        }

        let parents = bases
            .links
            .iter()
            .filter(|link| link.relation == CandidateRelation::Extends)
            .collect::<Vec<_>>();
        let [parent] = parents.as_slice() else {
            return if parents.is_empty() {
                PhpMemberLookup::Absent
            } else {
                PhpMemberLookup::Unknown
            };
        };
        let Some(parent) = parent
            .qualified_name
            .as_deref()
            .and_then(|name| self.exact_hierarchy_type(language, name))
        else {
            return PhpMemberLookup::Unknown;
        };
        self.php_member_lookup(
            language,
            &parent,
            candidate,
            visiting,
            depth.saturating_add(1),
        )
    }

    fn php_direct_members(
        &self,
        owner: &str,
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<String> {
        self.indexes
            .php
            .members_by_owner_folded
            .get(&(
                owner.to_ascii_lowercase(),
                candidate.target_spelling.to_ascii_lowercase(),
            ))
            .into_iter()
            .flatten()
            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .filter_map(|slot| self.declaration_id(*slot).map(str::to_owned))
            .collect()
    }
}
