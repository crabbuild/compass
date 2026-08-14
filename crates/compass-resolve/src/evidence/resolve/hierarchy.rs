//! Bounded direct-base, C3, and receiver-dispatch resolution.

use super::super::*;

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve_c3_receiver_dispatch(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        strategy: ReceiverDispatchStrategy,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        match strategy {
            ReceiverDispatchStrategy::C3FromReceiver => {
                if let Some(decision) =
                    self.resolve_exact_receiver_member(language, receiver_qualified_name, candidate)
                {
                    return decision;
                }
                // A direct member on the first base is source-proven even
                // when a later external base prevents construction of the
                // complete linearization. The same remains true through a
                // chain of single inheritance: no sibling can precede the
                // next class until the chain reaches a multiple-base fork.
                if let Some(decision) = self.resolve_source_proven_receiver_prefix(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
                if let Some(decision) = self.resolve_source_proven_later_direct_base(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
            }
            ReceiverDispatchStrategy::C3AfterReceiver => {
                if let Some(decision) = self.resolve_direct_receiver_successor(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
                if let Some(decision) = self.resolve_source_proven_later_direct_base(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
            }
        }
        let mut memo = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        let Ok(linearization) = self.c3_linearization(
            language,
            receiver_qualified_name,
            &mut memo,
            &mut visiting,
            0,
        ) else {
            return ResolutionDecision::Unresolved;
        };
        if linearization.first().map(String::as_str) != Some(receiver_qualified_name) {
            return ResolutionDecision::Unresolved;
        }
        for owner in linearization.iter().skip(1) {
            let key = (
                language.to_owned(),
                owner.clone(),
                candidate.target_spelling.clone(),
            );
            let Some(members) = self.indexes.members.members_by_owner.get(&key) else {
                continue;
            };
            let eligible = members
                .iter()
                .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                .take(self.budget.candidates_per_lookup().saturating_add(1))
                .cloned()
                .collect::<Vec<_>>();
            match eligible.as_slice() {
                [only] => {
                    let Some(only) = self.declaration_id(*only) else {
                        continue;
                    };
                    return ResolutionDecision::Resolved {
                        declaration_id: only.to_owned(),
                        evidence: ResolutionEvidence {
                            rule: ResolutionRule::LinearizedReceiverDispatch,
                            candidate_count: 1,
                        },
                    };
                }
                [] => {}
                many => {
                    return ResolutionDecision::Ambiguous {
                        candidate_count: many.len(),
                    };
                }
            }
        }
        ResolutionDecision::Unresolved
    }

    pub(in crate::evidence) fn possible_receiver_dispatches(
        &self,
        candidate_id: &str,
        exact_declaration_id: Option<&str>,
    ) -> Vec<(String, ResolutionRule)> {
        let Some(candidate) = self.facts.candidates.get(candidate_id) else {
            return Vec::new();
        };
        let Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name,
            strategy,
        }) = candidate.constraints.hierarchy.as_ref()
        else {
            return Vec::new();
        };
        let language = candidate
            .constraints
            .exact_language
            .as_deref()
            .unwrap_or(&candidate.language);
        let Some(receiver) = self.exact_hierarchy_type(language, receiver_qualified_name) else {
            return Vec::new();
        };
        let mut possible = BTreeMap::<String, ResolutionRule>::new();
        if let Some(declaration_id) =
            self.possible_incomplete_hierarchy_member(language, &receiver, &candidate)
            && exact_declaration_id != Some(declaration_id.as_str())
        {
            possible.insert(
                declaration_id,
                ResolutionRule::IncompleteHierarchyReceiverDispatch,
            );
        }
        let Some(descendants) = self.closed_world_descendants(language, &receiver) else {
            return possible.into_iter().collect();
        };
        for descendant in descendants {
            let mut memo = BTreeMap::new();
            let mut visiting = BTreeSet::new();
            let linearization =
                match self.c3_linearization(language, &descendant, &mut memo, &mut visiting, 0) {
                    Ok(linearization) => linearization,
                    Err(()) => {
                        if *strategy == ReceiverDispatchStrategy::C3FromReceiver
                            && self.hierarchy_has_unresolved_base(language, &descendant)
                        {
                            let decision = self
                                .resolve_exact_receiver_member(language, &descendant, &candidate)
                                .or_else(|| {
                                    self.resolve_source_proven_receiver_prefix(
                                        language,
                                        &descendant,
                                        &candidate,
                                    )
                                });
                            match decision {
                                Some(ResolutionDecision::Resolved { declaration_id, .. }) => {
                                    if exact_declaration_id != Some(declaration_id.as_str()) {
                                        possible.entry(declaration_id).or_insert(
                                            ResolutionRule::IncompleteHierarchyReceiverDispatch,
                                        );
                                    }
                                }
                                Some(ResolutionDecision::Ambiguous { .. }) => return Vec::new(),
                                Some(_) | None => {}
                            }
                        }
                        continue;
                    }
                };
            let start = match strategy {
                ReceiverDispatchStrategy::C3FromReceiver => 0,
                ReceiverDispatchStrategy::C3AfterReceiver => {
                    let Some(position) = linearization.iter().position(|owner| owner == &receiver)
                    else {
                        continue;
                    };
                    position.saturating_add(1)
                }
            };
            if !linearization.iter().any(|owner| owner == &receiver) {
                continue;
            }
            for owner in linearization.iter().skip(start) {
                match self.unique_receiver_member_id(language, owner, &candidate) {
                    Ok(Some(declaration_id)) => {
                        if exact_declaration_id != Some(declaration_id.as_str()) {
                            possible
                                .entry(declaration_id)
                                .or_insert(ResolutionRule::ClosedWorldReceiverDispatch);
                        }
                        break;
                    }
                    Ok(None) => {}
                    Err(()) => return Vec::new(),
                }
            }
            if possible.len() > self.budget.candidates_per_lookup() {
                return Vec::new();
            }
        }
        possible.into_iter().collect()
    }

    pub(in crate::evidence) fn hierarchy_has_unresolved_base(
        &self,
        language: &str,
        root: &str,
    ) -> bool {
        let mut visiting = BTreeSet::new();
        self.hierarchy_incompleteness(language, root, &mut visiting, 0)
            .unwrap_or(false)
    }

    pub(in crate::evidence) fn hierarchy_incompleteness(
        &self,
        language: &str,
        qualified_name: &str,
        visiting: &mut BTreeSet<(String, String)>,
        depth: usize,
    ) -> Result<bool, ()> {
        if depth >= self.budget.candidates_per_lookup() {
            return Err(());
        }
        let canonical = self
            .exact_hierarchy_type(language, qualified_name)
            .ok_or(())?;
        let key = (language.to_owned(), canonical);
        if !visiting.insert(key.clone()) {
            return Err(());
        }
        let result = (|| {
            let Some(bases) = self.indexes.hierarchy.direct_bases.get(&key) else {
                return Ok(false);
            };
            if bases.links.len() > self.budget.candidates_per_lookup() {
                return Err(());
            }
            let mut incomplete = !bases.complete;
            for link in &bases.links {
                let Some(base) = link
                    .qualified_name
                    .as_deref()
                    .and_then(|name| self.exact_hierarchy_type(language, name))
                else {
                    incomplete = true;
                    continue;
                };
                incomplete |= self.hierarchy_incompleteness(
                    language,
                    &base,
                    visiting,
                    depth.saturating_add(1),
                )?;
            }
            Ok(incomplete)
        })();
        visiting.remove(&key);
        result
    }

    pub(in crate::evidence) fn closed_world_descendants(
        &self,
        language: &str,
        receiver: &str,
    ) -> Option<Vec<String>> {
        let mut discovered = BTreeSet::new();
        let mut frontier = vec![receiver.to_owned()];
        let mut cursor = 0usize;
        while let Some(current) = frontier.get(cursor).cloned() {
            cursor = cursor.saturating_add(1);
            let Some(direct) = self
                .indexes
                .hierarchy
                .direct_subtypes
                .get(&(language.to_owned(), current))
            else {
                continue;
            };
            if !direct.complete {
                return None;
            }
            for subtype in &direct.types {
                if discovered.insert(subtype.clone()) {
                    if discovered.len() > self.budget.candidates_per_lookup() {
                        return None;
                    }
                    frontier.push(subtype.clone());
                }
            }
        }
        Some(discovered.into_iter().collect())
    }

    pub(in crate::evidence) fn possible_incomplete_hierarchy_member(
        &self,
        language: &str,
        receiver: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<String> {
        let bases = self
            .indexes
            .hierarchy
            .direct_bases
            .get(&(language.to_owned(), receiver.to_owned()))?;
        if !bases.complete
            || bases.links.len() < 2
            || bases.links.len() > self.budget.candidates_per_lookup()
        {
            return None;
        }
        let mut preceding_hierarchy_unknown = false;
        for link in &bases.links {
            let Some(base) = link
                .qualified_name
                .as_deref()
                .and_then(|name| self.exact_hierarchy_type(language, name))
            else {
                preceding_hierarchy_unknown = true;
                continue;
            };
            match self.unique_receiver_member_id(language, &base, candidate) {
                Ok(Some(declaration_id)) => {
                    return preceding_hierarchy_unknown.then_some(declaration_id);
                }
                Ok(None) => {}
                Err(()) => return None,
            }
            let mut memo = BTreeMap::new();
            let mut visiting = BTreeSet::new();
            if self
                .c3_linearization(language, &base, &mut memo, &mut visiting, 0)
                .is_err()
            {
                preceding_hierarchy_unknown = true;
            }
        }
        None
    }

    pub(in crate::evidence) fn unique_receiver_member_id(
        &self,
        language: &str,
        receiver: &str,
        candidate: &RelationshipCandidate,
    ) -> Result<Option<String>, ()> {
        match self.resolve_exact_receiver_member(language, receiver, candidate) {
            Some(ResolutionDecision::Resolved { declaration_id, .. }) => Ok(Some(declaration_id)),
            Some(ResolutionDecision::Ambiguous { .. }) => Err(()),
            Some(_) | None => Ok(None),
        }
    }

    pub(in crate::evidence) fn resolve_source_proven_receiver_prefix(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let mut receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let mut visited = BTreeSet::new();
        for _ in 0..self.budget.candidates_per_lookup() {
            if !visited.insert(receiver.clone()) {
                return None;
            }
            let base_set = self
                .indexes
                .hierarchy
                .direct_bases
                .get(&(language.to_owned(), receiver.clone()))?;
            if !base_set.complete
                || base_set.links.is_empty()
                || base_set.links.len() > self.budget.candidates_per_lookup()
            {
                return None;
            }
            let first_base = base_set.links[0]
                .qualified_name
                .as_deref()
                .and_then(|name| self.exact_hierarchy_type(language, name))?;
            if let Some(decision) =
                self.resolve_exact_receiver_member(language, &first_base, candidate)
            {
                return Some(decision);
            }
            if base_set.links.len() != 1 {
                return None;
            }
            receiver = first_base;
        }
        None
    }

    pub(in crate::evidence) fn resolve_exact_receiver_member(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let key = (
            language.to_owned(),
            receiver,
            candidate.target_spelling.clone(),
        );
        let mut eligible = BTreeSet::new();
        if let Some(members) = self.indexes.members.members_by_owner.get(&key) {
            eligible.extend(
                members
                    .iter()
                    .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                    .copied(),
            );
        }
        if let Some(targets) = self.indexes.members.members.get(&key) {
            for target in targets {
                let Some(declarations) = self
                    .indexes
                    .names
                    .by_qualified
                    .get(&(language.to_owned(), target.clone()))
                else {
                    continue;
                };
                eligible.extend(
                    declarations
                        .iter()
                        .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                        .copied(),
                );
            }
        }
        let eligible = eligible
            .into_iter()
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .collect::<Vec<_>>();
        match eligible.as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: self.declaration_id(*only)?.to_owned(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::LinearizedReceiverDispatch,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    pub(in crate::evidence) fn resolve_source_proven_later_direct_base(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let base_set = self
            .indexes
            .hierarchy
            .direct_bases
            .get(&(language.to_owned(), receiver))?;
        if !base_set.complete
            || base_set.links.len() < 2
            || base_set.links.len() > self.budget.candidates_per_lookup()
        {
            return None;
        }
        for (index, link) in base_set.links.iter().enumerate() {
            let base = link
                .qualified_name
                .as_deref()
                .and_then(|name| self.exact_hierarchy_type(language, name))?;
            if index > 0
                && let Some(decision) =
                    self.resolve_exact_receiver_member(language, &base, candidate)
            {
                return Some(decision);
            }
            let mut memo = BTreeMap::new();
            let mut visiting = BTreeSet::new();
            let linearization = self
                .c3_linearization(language, &base, &mut memo, &mut visiting, 0)
                .ok()?;
            if linearization.as_slice() != [base.as_str()] {
                return None;
            }
        }
        None
    }

    pub(in crate::evidence) fn resolve_direct_base(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        let Some(qualified_name) = candidate.constraints.qualified_name.as_deref() else {
            return ResolutionDecision::Unresolved;
        };
        let qualified_name = match self.follow_alias(language, qualified_name) {
            Ok(qualified_name) => qualified_name,
            Err(candidate_count) => {
                return ResolutionDecision::Ambiguous { candidate_count };
            }
        };
        let key = (language.to_owned(), qualified_name.clone());
        if let Some(decision) = self.unique_decision(
            self.indexes.names.by_qualified.get(&key),
            candidate,
            ResolutionRule::ExactHierarchyBase,
        ) {
            return decision;
        }
        if candidate.constraints.allow_external {
            return ResolutionDecision::QualifiedExternal {
                qualified_name,
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::QualifiedExternal,
                    candidate_count: 0,
                },
            };
        }
        ResolutionDecision::Unresolved
    }

    pub(in crate::evidence) fn resolve_direct_receiver_successor(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let base_set = self
            .indexes
            .hierarchy
            .direct_bases
            .get(&(language.to_owned(), receiver))?;
        if !base_set.complete
            || base_set.links.is_empty()
            || base_set.links.len() > self.budget.candidates_per_lookup()
        {
            return None;
        }
        let direct_successor = base_set.links[0]
            .qualified_name
            .as_deref()
            .and_then(|name| self.exact_hierarchy_type(language, name))?;
        let members = self.indexes.members.members_by_owner.get(&(
            language.to_owned(),
            direct_successor,
            candidate.target_spelling.clone(),
        ))?;
        let eligible = members
            .iter()
            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .cloned()
            .collect::<Vec<_>>();
        match eligible.as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: self.declaration_id(*only)?.to_owned(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::DirectReceiverSuccessorDispatch,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    pub(in crate::evidence) fn c3_linearization(
        &self,
        language: &str,
        qualified_name: &str,
        memo: &mut BTreeMap<(String, String), Result<Vec<String>, ()>>,
        visiting: &mut BTreeSet<(String, String)>,
        depth: usize,
    ) -> Result<Vec<String>, ()> {
        if depth >= self.budget.candidates_per_lookup() {
            return Err(());
        }
        let canonical = self
            .exact_hierarchy_type(language, qualified_name)
            .ok_or(())?;
        let key = (language.to_owned(), canonical.clone());
        if let Some(cached) = memo.get(&key) {
            return cached.clone();
        }
        if !visiting.insert(key.clone()) {
            return Err(());
        }
        let result = (|| {
            let Some(base_set) = self.indexes.hierarchy.direct_bases.get(&key) else {
                return Ok(vec![canonical.clone()]);
            };
            if !base_set.complete
                || base_set.links.is_empty()
                || base_set.links.len() > self.budget.candidates_per_lookup()
            {
                return Err(());
            }
            let mut bases = Vec::with_capacity(base_set.links.len());
            let mut sequences = Vec::with_capacity(base_set.links.len().saturating_add(1));
            for link in &base_set.links {
                let base = link
                    .qualified_name
                    .as_deref()
                    .and_then(|name| self.exact_hierarchy_type(language, name))
                    .ok_or(())?;
                bases.push(base.clone());
                sequences.push(self.c3_linearization(
                    language,
                    &base,
                    memo,
                    visiting,
                    depth.saturating_add(1),
                )?);
            }
            sequences.push(bases);
            let mut linearization = vec![canonical.clone()];
            linearization.extend(c3_merge(sequences, self.budget.candidates_per_lookup())?);
            Ok(linearization)
        })();
        visiting.remove(&key);
        memo.insert(key, result.clone());
        result
    }

    pub(in crate::evidence) fn exact_hierarchy_type(
        &self,
        language: &str,
        qualified_name: &str,
    ) -> Option<String> {
        let qualified_name = self.follow_alias(language, qualified_name).ok()?;
        let declarations = self
            .indexes
            .names
            .by_qualified
            .get(&(language.to_owned(), qualified_name))?;
        let eligible = declarations
            .iter()
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| {
                matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "record" | "struct"
                )
            })
            .take(2)
            .collect::<Vec<_>>();
        let [declaration] = eligible.as_slice() else {
            return None;
        };
        Some(declaration.qualified_name.clone())
    }
}
