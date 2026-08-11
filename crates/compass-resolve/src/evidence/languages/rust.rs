//! Rust trait, implementation, and associated-type resolution policy.

use super::super::*;

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve_rust_associated_type(
        &self,
        language: &str,
        receiver_declaration_id: &str,
        receiver_qualified_name: &str,
        trait_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        let receiver = self.facts.declarations.get(receiver_declaration_id);
        if language != "rust"
            || receiver.is_none_or(|receiver| {
                receiver.language != "rust"
                    || receiver.qualified_name != receiver_qualified_name
                    || !matches!(receiver.kind.as_str(), "struct" | "enum" | "type_alias")
            })
        {
            return ResolutionDecision::Unresolved;
        }
        let trait_qualified_name =
            match self.canonical_rust_impl_trait(receiver_declaration_id, trait_qualified_name) {
                Ok(qualified_name) => qualified_name,
                Err(0) => return ResolutionDecision::Unresolved,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
        let lineage = match self.rust_trait_lineage(&trait_qualified_name) {
            Ok(lineage) => lineage,
            Err(0) => return ResolutionDecision::Unresolved,
            Err(candidate_count) => {
                return ResolutionDecision::Ambiguous { candidate_count };
            }
        };
        let mut targets = BTreeSet::new();
        let associated_trait_names = self.indexes.rust.impl_associated_trait_names.get(&(
            receiver_declaration_id.to_owned(),
            candidate.target_spelling.clone(),
        ));
        let Some(associated_trait_names) = associated_trait_names else {
            return ResolutionDecision::Unresolved;
        };
        if !associated_trait_names.complete {
            return ResolutionDecision::Ambiguous {
                candidate_count: associated_trait_names.modules.len().saturating_add(1),
            };
        }
        for raw_trait_name in &associated_trait_names.modules {
            let trait_name =
                match self.canonical_rust_impl_trait(receiver_declaration_id, raw_trait_name) {
                    Ok(qualified_name) => qualified_name,
                    Err(0) => return ResolutionDecision::Unresolved,
                    Err(candidate_count) => {
                        return ResolutionDecision::Ambiguous { candidate_count };
                    }
                };
            if !lineage.contains(&trait_name) {
                continue;
            }
            let Some(associated) = self.indexes.rust.impl_associated_types.get(&(
                receiver_declaration_id.to_owned(),
                raw_trait_name.clone(),
                candidate.target_spelling.clone(),
            )) else {
                continue;
            };
            if !associated.complete {
                return ResolutionDecision::Ambiguous {
                    candidate_count: associated.declarations.len().saturating_add(1),
                };
            }
            targets.extend(
                associated
                    .declarations
                    .iter()
                    .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                    .copied(),
            );
            if targets.len() > self.budget.candidates_per_lookup() {
                return ResolutionDecision::Ambiguous {
                    candidate_count: targets.len(),
                };
            }
        }
        match targets.into_iter().collect::<Vec<_>>().as_slice() {
            [only] => {
                let Some(declaration_id) = self.declaration_id(*only) else {
                    return ResolutionDecision::Unresolved;
                };
                ResolutionDecision::Resolved {
                    declaration_id: declaration_id.to_owned(),
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::RustAssociatedType,
                        candidate_count: 1,
                    },
                }
            }
            [] => ResolutionDecision::Unresolved,
            many => ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            },
        }
    }

    pub(in crate::evidence) fn canonical_rust_impl_trait(
        &self,
        receiver_declaration_id: &str,
        raw_trait_name: &str,
    ) -> Result<String, usize> {
        if let Ok(declaration) = self.exact_rust_declaration(raw_trait_name, &["trait"]) {
            return Ok(declaration.qualified_name.clone());
        }
        let Some(candidates) = self.indexes.rust.impl_traits.get(&(
            receiver_declaration_id.to_owned(),
            raw_trait_name.to_owned(),
        )) else {
            return Err(0);
        };
        if !candidates.complete {
            return Err(candidates.candidate_ids.len().saturating_add(1));
        }
        let mut qualified_names = BTreeSet::new();
        for candidate_id in &candidates.candidate_ids {
            let Some(candidate) = self.facts.candidates.get(candidate_id) else {
                return Err(0);
            };
            match self.resolve_rust_impl_trait_candidate(candidate, raw_trait_name) {
                ResolutionDecision::Resolved { declaration_id, .. } => {
                    let Some(declaration) = self.facts.declarations.get(&declaration_id) else {
                        return Err(0);
                    };
                    if declaration.language != "rust" || declaration.kind != "trait" {
                        return Err(0);
                    }
                    qualified_names.insert(declaration.qualified_name.clone());
                }
                ResolutionDecision::Ambiguous { candidate_count } => {
                    return Err(candidate_count);
                }
                _ => return Err(0),
            }
            if qualified_names.len() > self.budget.candidates_per_lookup() {
                return Err(qualified_names.len());
            }
        }
        match qualified_names.into_iter().collect::<Vec<_>>().as_slice() {
            [only] => Ok(only.clone()),
            [] => Err(0),
            many => Err(many.len()),
        }
    }

    pub(in crate::evidence) fn resolve_rust_impl_trait_candidate(
        &self,
        candidate: &RelationshipCandidate,
        raw_trait_name: &str,
    ) -> ResolutionDecision {
        let mut lookup = candidate.clone();
        lookup.target_spelling = raw_trait_name
            .rsplit("::")
            .next()
            .unwrap_or(raw_trait_name)
            .to_owned();
        lookup.constraints.qualified_name = Some(raw_trait_name.to_owned());
        lookup.constraints.allow_external = false;

        if let Some(decision) = self.resolve_explicit_binding("rust", &lookup) {
            return decision;
        }
        if let Some(module) = lookup.constraints.module_or_package.as_ref() {
            let key = (
                "rust".to_owned(),
                module.clone(),
                lookup.target_spelling.clone(),
            );
            if let Some(decision) = self.unique_decision(
                self.indexes.names.by_module_name.get(&key),
                &lookup,
                ResolutionRule::UniqueModuleOrPackage,
            ) {
                return decision;
            }
        }
        if let Some(decision) = self.resolve_wildcard_binding("rust", &lookup) {
            return decision;
        }
        if let Some(decision) = self.resolve_visible_wildcard_bindings("rust", &lookup) {
            return decision;
        }
        ResolutionDecision::Unresolved
    }

    pub(in crate::evidence) fn rust_trait_lineage(
        &self,
        root: &str,
    ) -> Result<BTreeSet<String>, usize> {
        let mut pending = vec![root.to_owned()];
        let mut lineage = BTreeSet::new();
        while let Some(qualified_name) = pending.pop() {
            let trait_declaration = self.exact_rust_declaration(&qualified_name, &["trait"])?;
            if !lineage.insert(trait_declaration.qualified_name.clone()) {
                continue;
            }
            if lineage.len() > self.budget.candidates_per_lookup() {
                return Err(lineage.len());
            }
            if !trait_declaration.direct_bases_complete {
                return Err(lineage.len().saturating_add(1));
            }
            let key = ("rust".to_owned(), trait_declaration.qualified_name.clone());
            let Some(bases) = self.indexes.hierarchy.direct_bases.get(&key) else {
                continue;
            };
            if !bases.complete || bases.links.len() > self.budget.candidates_per_lookup() {
                return Err(bases.links.len().saturating_add(1));
            }
            for base in bases.links.iter().rev() {
                let Some(base_name) = base.qualified_name.as_deref() else {
                    return Err(bases.links.len().saturating_add(1));
                };
                match self.exact_rust_declaration(base_name, &["trait"]) {
                    Ok(base) => pending.push(base.qualified_name.clone()),
                    Err(0) if self.rust_builtin_marker_base(base) => {}
                    Err(candidate_count) => return Err(candidate_count),
                }
            }
        }
        Ok(lineage)
    }

    pub(in crate::evidence) fn rust_builtin_marker_base(&self, link: &DirectBaseLink) -> bool {
        let Some(candidate) = self.facts.candidates.get(&link.candidate_id) else {
            return false;
        };
        if !matches!(candidate.target_spelling.as_str(), "Send" | "Sized")
            || self
                .occurrence(candidate)
                .is_none_or(|occurrence| occurrence.qualifier.is_some())
        {
            return false;
        }
        match candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.facts.bindings.get(binding_id))
        {
            Some(binding) if binding.spelling == "*" => {
                self.resolve_wildcard_binding("rust", candidate).is_none()
            }
            Some(_) => false,
            None => self
                .resolve_visible_wildcard_bindings("rust", candidate)
                .is_none(),
        }
    }

    pub(in crate::evidence) fn exact_rust_declaration<'a>(
        &'a self,
        qualified_name: &str,
        allowed_kinds: &[&str],
    ) -> Result<&'a DeclarationFact, usize> {
        let exact = self
            .indexes
            .names
            .by_qualified
            .get(&("rust".to_owned(), qualified_name.to_owned()))
            .into_iter()
            .flat_map(|slots| slots.iter())
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| allowed_kinds.contains(&declaration.kind.as_str()))
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .collect::<Vec<_>>();
        match exact.as_slice() {
            [only] => return Ok(*only),
            [] => {}
            many => return Err(many.len()),
        }
        let aliased = self.follow_alias("rust", qualified_name)?;
        if aliased == qualified_name {
            return Err(0);
        }
        let declarations = self
            .indexes
            .names
            .by_qualified
            .get(&("rust".to_owned(), aliased))
            .into_iter()
            .flat_map(|slots| slots.iter())
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| allowed_kinds.contains(&declaration.kind.as_str()))
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .collect::<Vec<_>>();
        match declarations.as_slice() {
            [only] => Ok(*only),
            [] => Err(0),
            many => Err(many.len()),
        }
    }
}

pub(in crate::evidence) fn rust_module_is_descendant(
    source_module: &str,
    ancestor_module: &str,
) -> bool {
    source_module == ancestor_module
        || source_module
            .strip_prefix(ancestor_module)
            .is_some_and(|suffix| suffix.starts_with("::"))
}

pub(in crate::evidence) fn rust_external_wildcard_target_is_explicit(
    qualifier: Option<&str>,
    candidate: &RelationshipCandidate,
) -> bool {
    qualifier
        .and_then(|value| value.split("::").next())
        .and_then(|value| value.chars().next())
        .is_some_and(char::is_uppercase)
        || (qualifier.is_none()
            && candidate
                .target_spelling
                .chars()
                .next()
                .is_some_and(char::is_uppercase))
}

pub(in crate::evidence) fn rust_impl_associated_type_index(
    declarations: &FactTable<DeclarationFact>,
    declaration_ids: &[String],
    scopes: &FactTable<compass_languages::ScopeFact>,
    candidates: &FactTable<RelationshipCandidate>,
    occurrences: &FactTable<OccurrenceFact>,
    candidate_limit: usize,
) -> AHashMap<(String, String, String), AssociatedTypeSet> {
    let mut implementations = AHashMap::<String, Vec<&RelationshipCandidate>>::new();
    for candidate in candidates.values().filter(|candidate| {
        candidate.language == "rust" && candidate.relation == CandidateRelation::Implements
    }) {
        implementations
            .entry(candidate.source_declaration_id.clone())
            .or_default()
            .push(candidate);
    }

    let mut index = AHashMap::<(String, String, String), AssociatedTypeSet>::new();
    for scope in scopes
        .values()
        .filter(|scope| scope.language == "rust" && scope.kind == "impl")
    {
        let Some(owner_id) = scope.owner_declaration_id.as_ref() else {
            continue;
        };
        let Some(_receiver) = declarations.get(owner_id) else {
            continue;
        };
        let traits = implementations
            .get(owner_id)
            .into_iter()
            .flat_map(|values| values.iter())
            .filter_map(|candidate| {
                let occurrence = candidate
                    .occurrence_id
                    .as_deref()
                    .and_then(|id| occurrences.get(id))?;
                range_contains(&scope.range, &occurrence.range)
                    .then_some(candidate.constraints.qualified_name.as_ref())
                    .flatten()
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let [trait_name] = traits.as_slice() else {
            continue;
        };
        for declaration in declarations.values().filter(|declaration| {
            declaration.language == "rust"
                && declaration.kind == "type_alias"
                && declaration.scope_id.as_deref() == Some(scope.id.as_str())
        }) {
            let Some(slot) = declaration_slot(declaration_ids, &declaration.id) else {
                continue;
            };
            let associated = index
                .entry((
                    owner_id.clone(),
                    (*trait_name).clone(),
                    declaration.name.clone(),
                ))
                .or_insert_with(|| AssociatedTypeSet {
                    declarations: Vec::new(),
                    complete: true,
                });
            if associated.declarations.len() < candidate_limit {
                associated.declarations.push(slot);
            } else {
                associated.complete = false;
            }
        }
    }
    for associated in index.values_mut() {
        associated.declarations.sort_unstable_by(|left, right| {
            declaration_ids[*left as usize].cmp(&declaration_ids[*right as usize])
        });
        associated.declarations.dedup();
        if associated.declarations.len() > candidate_limit {
            associated.complete = false;
            associated.declarations.truncate(candidate_limit);
        }
    }
    index
}

pub(in crate::evidence) fn rust_impl_associated_trait_name_index(
    associated_types: &AHashMap<(String, String, String), AssociatedTypeSet>,
    candidate_limit: usize,
) -> AHashMap<(String, String), WildcardModuleSet> {
    let mut index = AHashMap::<(String, String), WildcardModuleSet>::new();
    for (receiver, trait_name, associated_name) in associated_types.keys() {
        let entry = index
            .entry((receiver.clone(), associated_name.clone()))
            .or_insert_with(|| WildcardModuleSet {
                modules: Vec::new(),
                complete: true,
            });
        if entry.modules.len() < candidate_limit {
            entry.modules.push(trait_name.clone());
        } else {
            entry.complete = false;
        }
    }
    for traits in index.values_mut() {
        traits.modules.sort_unstable();
        traits.modules.dedup();
        if traits.modules.len() > candidate_limit {
            traits.complete = false;
            traits.modules.truncate(candidate_limit);
        }
    }
    index
}

pub(in crate::evidence) fn rust_impl_trait_index(
    candidates: &FactTable<RelationshipCandidate>,
    candidate_limit: usize,
) -> AHashMap<(String, String), RustImplTraitSet> {
    let mut index = AHashMap::<(String, String), RustImplTraitSet>::new();
    for candidate in candidates.values().filter(|candidate| {
        candidate.language == "rust" && candidate.relation == CandidateRelation::Implements
    }) {
        let Some(raw_trait_name) = candidate.constraints.qualified_name.as_ref() else {
            continue;
        };
        let entry = index
            .entry((
                candidate.source_declaration_id.clone(),
                raw_trait_name.clone(),
            ))
            .or_insert_with(|| RustImplTraitSet {
                candidate_ids: Vec::new(),
                complete: true,
            });
        if entry.candidate_ids.len() < candidate_limit {
            entry.candidate_ids.push(candidate.id.clone());
        } else {
            entry.complete = false;
        }
    }
    for traits in index.values_mut() {
        traits.candidate_ids.sort_unstable();
        traits.candidate_ids.dedup();
        if traits.candidate_ids.len() > candidate_limit {
            traits.complete = false;
            traits.candidate_ids.truncate(candidate_limit);
        }
    }
    index
}
