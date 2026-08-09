//! Bounded wildcard scope, module, and re-export traversal.

use super::super::*;

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve_wildcard_binding(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) {
            return None;
        }
        let binding = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.facts.bindings.get(binding_id))
            .filter(|binding| binding.spelling == "*")?;
        let qualifier = self
            .occurrence(candidate)
            .and_then(|occurrence| occurrence.qualifier.as_deref());
        let declarations = match self.wildcard_declarations(
            language,
            std::slice::from_ref(&binding.qualified_target),
            qualifier,
            candidate,
        ) {
            Ok(declarations) => declarations,
            Err(candidate_count) => {
                return Some(ResolutionDecision::Ambiguous { candidate_count });
            }
        };
        match declarations.into_iter().collect::<Vec<_>>().as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: self.declaration_id(*only)?.to_owned(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::WildcardBinding,
                    candidate_count: 1,
                },
            }),
            [] if !self.binding_target_is_internal(binding)
                && (language != "rust"
                    || rust_external_wildcard_target_is_explicit(qualifier, candidate)) =>
            {
                Some(ResolutionDecision::QualifiedExternal {
                    qualified_name: wildcard_qualified_names(
                        language,
                        &binding.qualified_target,
                        qualifier,
                        &candidate.target_spelling,
                    )
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| candidate.target_spelling.clone()),
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::QualifiedExternal,
                        candidate_count: 0,
                    },
                })
            }
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    pub(in crate::evidence) fn resolve_visible_wildcard_bindings(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let qualifier = self
            .occurrence(candidate)
            .and_then(|occurrence| occurrence.qualifier.as_deref());
        if language != "rust"
            || candidate.binding_id.is_some()
            || matches!(
                candidate.relation,
                CandidateRelation::Imports | CandidateRelation::Reexports
            )
            || qualifier.is_some_and(|qualifier| {
                !rust_external_wildcard_target_is_explicit(Some(qualifier), candidate)
            })
        {
            return None;
        }
        let mut scope_id = candidate.constraints.scope_id.as_deref();
        let mut visited_scopes = BTreeSet::new();
        while let Some(current) = scope_id.filter(|scope| visited_scopes.insert(*scope)) {
            if visited_scopes.len() > self.budget.candidates_per_lookup() {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: visited_scopes.len(),
                });
            }
            if let Some(bindings) = self
                .indexes
                .wildcards
                .by_scope
                .get(&(language.to_owned(), current.to_owned()))
            {
                if !bindings.complete {
                    return Some(ResolutionDecision::Ambiguous {
                        candidate_count: bindings.modules.len().saturating_add(1),
                    });
                }
                let declarations = match self.wildcard_declarations(
                    language,
                    &bindings.modules,
                    qualifier,
                    candidate,
                ) {
                    Ok(declarations) => declarations,
                    Err(candidate_count) => {
                        return Some(ResolutionDecision::Ambiguous { candidate_count });
                    }
                };
                match declarations.into_iter().collect::<Vec<_>>().as_slice() {
                    [only] => {
                        return Some(ResolutionDecision::Resolved {
                            declaration_id: self.declaration_id(*only)?.to_owned(),
                            evidence: ResolutionEvidence {
                                rule: ResolutionRule::WildcardBinding,
                                candidate_count: 1,
                            },
                        });
                    }
                    [] => {}
                    many => {
                        return Some(ResolutionDecision::Ambiguous {
                            candidate_count: many.len(),
                        });
                    }
                }
            }
            scope_id = self
                .facts
                .scopes
                .get(current)
                .and_then(|scope| scope.parent_scope_id.as_deref());
        }
        None
    }

    pub(in crate::evidence) fn wildcard_declarations(
        &self,
        language: &str,
        initial_modules: &[String],
        qualifier: Option<&str>,
        candidate: &RelationshipCandidate,
    ) -> Result<BTreeSet<DeclarationSlot>, usize> {
        let mut modules = initial_modules.to_vec();
        let mut visited = BTreeSet::new();
        let mut declarations = BTreeSet::<DeclarationSlot>::new();
        while let Some(module) = modules.pop() {
            if !visited.insert(module.clone()) {
                continue;
            }
            if visited.len() > self.budget.candidates_per_lookup() {
                return Err(visited.len());
            }
            if qualifier.is_none()
                && let Some(ids) = self.indexes.names.by_module_name.get(&(
                    language.to_owned(),
                    module.clone(),
                    candidate.target_spelling.clone(),
                ))
            {
                declarations.extend(
                    ids.iter()
                        .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                        .cloned(),
                );
            }
            for raw_qualified in
                wildcard_qualified_names(language, &module, qualifier, &candidate.target_spelling)
            {
                let qualified = self.follow_alias(language, &raw_qualified)?;
                let mut qualified_declarations = BTreeSet::new();
                if let Some(ids) = self
                    .indexes
                    .names
                    .by_qualified
                    .get(&(language.to_owned(), qualified.clone()))
                {
                    qualified_declarations.extend(
                        ids.iter()
                            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                            .cloned(),
                    );
                }
                if let Some(ids) = self.member_declarations(language, &qualified, candidate) {
                    qualified_declarations.extend(ids);
                }
                if qualified_declarations.is_empty()
                    && qualifier.is_some_and(|qualifier| {
                        rust_external_wildcard_target_is_explicit(Some(qualifier), candidate)
                    })
                    && qualified == raw_qualified
                {
                    let expanded =
                        self.follow_wildcard_qualified_alias(language, &raw_qualified)?;
                    if expanded != qualified {
                        if let Some(ids) = self
                            .indexes
                            .names
                            .by_qualified
                            .get(&(language.to_owned(), expanded.clone()))
                        {
                            qualified_declarations.extend(
                                ids.iter()
                                    .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                                    .cloned(),
                            );
                        }
                        if let Some(ids) = self.member_declarations(language, &expanded, candidate)
                        {
                            qualified_declarations.extend(ids);
                        }
                    }
                }
                declarations.extend(qualified_declarations);
            }
            if declarations.len() > self.budget.candidates_per_lookup() {
                return Err(declarations.len());
            }
            if let Some(reexports) = self
                .indexes
                .wildcards
                .reexports_by_module
                .get(&(language.to_owned(), module.clone()))
            {
                if !reexports.complete {
                    return Err(reexports.modules.len().saturating_add(1));
                }
                modules.extend(reexports.modules.iter().cloned());
            }
            if language == "rust"
                && candidate
                    .constraints
                    .module_or_package
                    .as_deref()
                    .is_some_and(|source_module| rust_module_is_descendant(source_module, &module))
                && let Some(bindings) = self
                    .indexes
                    .wildcards
                    .by_module
                    .get(&(language.to_owned(), module))
            {
                if !bindings.complete {
                    return Err(bindings.modules.len().saturating_add(1));
                }
                modules.extend(bindings.modules.iter().cloned());
            }
        }
        Ok(declarations)
    }
}
