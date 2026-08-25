//! Generic member, inventory, return-chain, reexport, and alias resolution.

use super::super::*;

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn ruby_import_decision(
        &self,
        candidate: &RelationshipCandidate,
        requested: &str,
    ) -> Option<ResolutionDecision> {
        if candidate.language != "ruby" || candidate.relation != CandidateRelation::Imports {
            return None;
        }
        let occurrence = self.occurrence(candidate)?;
        let source_file = normalize_ruby_path(&occurrence.range().source_file);
        let operation = occurrence.context().unwrap_or_default();
        let target = normalize_ruby_path(requested);
        if target.is_empty()
            || target.starts_with('/')
            || target.split('/').any(|part| part == "..")
        {
            return None;
        }
        let base = if operation == "require_relative" {
            source_file
                .rsplit_once('/')
                .map_or_else(String::new, |(parent, _)| parent.to_owned())
        } else {
            String::new()
        };
        let joined = if base.is_empty() {
            target
        } else if target.starts_with("./") {
            format!("{base}/{}", target.trim_start_matches("./"))
        } else {
            format!("{base}/{target}")
        };
        let candidates = [
            joined.clone(),
            format!("{joined}.rb"),
            format!("{joined}.rake"),
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        let declarations = self
            .facts
            .declarations
            .values()
            .filter(|declaration| {
                declaration.language == "ruby"
                    && declaration.kind == "file"
                    && candidates.contains(&normalize_ruby_path(&declaration.range.source_file))
                    && self.declaration_allowed(&declaration.id, candidate)
            })
            .collect::<Vec<_>>();
        match declarations.as_slice() {
            [declaration] => Some(ResolutionDecision::ResolvedInventory {
                graph_node_id: declaration.graph_node_id.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExactSourceInventory,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    pub(in crate::evidence) fn inventory_decision(
        &self,
        language: &str,
        qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if !matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) || (!candidate.constraints.allowed_target_kinds.is_empty()
            && !candidate
                .constraints
                .allowed_target_kinds
                .iter()
                .any(|kind| matches!(kind.as_str(), "file" | "module" | "package")))
        {
            return None;
        }
        if language == "python"
            && let Some(sources) =
                python_project_sources(&self.project.python_project_modules, qualified_name)
            && sources.len() > 1
        {
            return Some(ResolutionDecision::Ambiguous {
                candidate_count: sources.len(),
            });
        }
        let candidates = self
            .indexes
            .names
            .inventory_by_qualified
            .get(&(language.to_owned(), qualified_name.to_owned()))?;
        match candidates.as_slice() {
            [graph_node_id] => Some(ResolutionDecision::ResolvedInventory {
                graph_node_id: graph_node_id.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExactSourceInventory,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    pub(in crate::evidence) fn imported_declarations(
        &self,
        language: &str,
        import_path: &str,
        spelling: &str,
    ) -> Vec<DeclarationSlot> {
        if language != "go" {
            return Vec::new();
        }
        let components = import_path
            .split('/')
            .filter(|component| !component.is_empty() && *component != ".")
            .collect::<Vec<_>>();
        for start in 0..components.len().min(64) {
            let directory = components[start..].join("/");
            let key = (language.to_owned(), directory, spelling.to_owned());
            if let Some(candidates) = self.indexes.names.by_source_directory_name.get(&key) {
                let imported = candidates
                    .iter()
                    .filter_map(|slot| {
                        self.declaration(*slot)
                            .filter(|declaration| !declaration.qualified_name.contains("::"))
                            .map(|_| *slot)
                    })
                    .take(self.budget.candidates_per_lookup().saturating_add(1))
                    .collect::<BTreeSet<_>>();
                if !imported.is_empty() {
                    return imported.into_iter().collect();
                }
            }
        }
        if self.project.go_module_path.as_deref() != Some(import_path) {
            return Vec::new();
        }
        let Some(package) = components.last() else {
            return Vec::new();
        };
        self.indexes
            .names
            .by_module_name
            .get(&(
                language.to_owned(),
                (*package).to_owned(),
                spelling.to_owned(),
            ))
            .into_iter()
            .flatten()
            .filter_map(|slot| {
                self.declaration(*slot)
                    .filter(|declaration| !declaration.qualified_name.contains("::"))
                    .map(|_| *slot)
            })
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(in crate::evidence) fn imported_member_decision(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if language != "go" {
            return None;
        }
        let (owner, member) = qualified.rsplit_once("::")?;
        let (import_path, owner_spelling) = owner.rsplit_once('.')?;
        let owner_ids = self.imported_declarations(language, import_path, owner_spelling);
        let mut declarations = BTreeSet::<DeclarationSlot>::new();
        for owner_id in owner_ids
            .into_iter()
            .take(candidate_storage_limit(self.budget.candidates_per_lookup()))
        {
            let owner = &self.declaration(owner_id)?.qualified_name;
            if let Some(ids) = self
                .indexes
                .names
                .by_qualified
                .get(&(language.to_owned(), format!("{owner}::{member}")))
            {
                declarations.extend(
                    ids.iter()
                        .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                        .cloned(),
                );
            }
        }
        self.unique_decision(
            Some(&declarations.into_iter().collect::<Vec<_>>()),
            candidate,
            ResolutionRule::MemberBinding,
        )
    }

    pub(in crate::evidence) fn bound_member_target(
        &self,
        language: &str,
        binding: &BindingFact,
        candidate: &RelationshipCandidate,
    ) -> Result<Option<String>, usize> {
        if binding.kind == compass_languages::BindingKind::CallResult {
            let Some(return_type) = self.call_result_return_type(
                language,
                binding,
                candidate,
                &mut BTreeSet::new(),
                0,
            )?
            else {
                return Ok(None);
            };
            return Ok(Some(format!(
                "{return_type}::{}",
                candidate.target_spelling
            )));
        }
        if binding.kind != compass_languages::BindingKind::LocalAlias {
            return Ok(None);
        }
        let Some(qualifier) = self.occurrence(candidate).and_then(|occurrence| {
            occurrence
                .qualifier()
                .filter(|qualifier| qualifier.contains('.'))
        }) else {
            return Ok(None);
        };
        let mut parts = qualifier.split('.');
        if parts.next() != Some(binding.spelling.as_str()) {
            return Ok(None);
        }
        let mut target = binding.qualified_target.clone();
        for member in parts {
            let Some(targets) = self.indexes.members.members.get(&(
                language.to_owned(),
                target.clone(),
                member.to_owned(),
            )) else {
                return Ok(None);
            };
            let [next] = targets.as_slice() else {
                return Err(targets.len());
            };
            target.clone_from(next);
        }
        Ok(Some(format!("{target}::{}", candidate.target_spelling)))
    }

    pub(in crate::evidence) fn call_result_return_type(
        &self,
        language: &str,
        binding: &BindingFact,
        candidate: &RelationshipCandidate,
        visiting: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<Option<String>, usize> {
        const MAX_CALL_RESULT_DEPTH: usize = 64;

        if depth >= MAX_CALL_RESULT_DEPTH || !visiting.insert(binding.id.clone()) {
            return Err(visiting.len().saturating_add(1));
        }
        let result =
            self.call_result_return_type_inner(language, binding, candidate, visiting, depth);
        visiting.remove(&binding.id);
        result
    }

    pub(in crate::evidence) fn call_result_return_type_inner(
        &self,
        language: &str,
        binding: &BindingFact,
        candidate: &RelationshipCandidate,
        visiting: &mut BTreeSet<String>,
        depth: usize,
    ) -> Result<Option<String>, usize> {
        if let Some(return_type) = binding.result_type_qualified_name.as_ref() {
            return Ok(Some(return_type.clone()));
        }
        let qualified_callable =
            if let Some(receiver_binding_id) = binding.receiver_binding_id.as_ref() {
                let Some(receiver_binding) = self.facts.bindings.get(receiver_binding_id) else {
                    return Ok(None);
                };
                let Some(receiver_type) = self.call_result_return_type(
                    language,
                    receiver_binding,
                    candidate,
                    visiting,
                    depth.saturating_add(1),
                )?
                else {
                    return Ok(None);
                };
                format!("{receiver_type}::{}", binding.qualified_target)
            } else {
                binding.qualified_target.clone()
            };

        let mut callable_ids = BTreeSet::new();
        callable_ids.extend(
            self.callable_declarations(language, &qualified_callable)
                .into_iter()
                .filter(|slot| self.declaration_allowed_slot(*slot, candidate)),
        );
        if callable_ids.is_empty()
            && let Some(member_ids) =
                self.member_declarations(language, &qualified_callable, candidate)
        {
            callable_ids.extend(member_ids);
        }
        if callable_ids.is_empty() {
            callable_ids.extend(self.rust_trait_member_declarations(
                language,
                &qualified_callable,
                candidate,
            )?);
        }
        if callable_ids.is_empty() {
            callable_ids.extend(self.rust_wildcard_callable_declarations(
                language,
                binding,
                &qualified_callable,
                candidate,
            )?);
        }
        if callable_ids.len() > self.budget.candidates_per_lookup() {
            return Err(callable_ids.len());
        }
        let callable_ids = callable_ids.into_iter().collect::<Vec<_>>();
        let [callable_id] = callable_ids.as_slice() else {
            return if callable_ids.is_empty() {
                Ok(None)
            } else {
                Err(callable_ids.len())
            };
        };
        let Some(callable) = self.declaration(*callable_id) else {
            return Ok(None);
        };
        let key = (language.to_owned(), callable.qualified_name.clone());
        let return_candidates = if language == "rust" {
            self.indexes
                .members
                .outer_return_candidates_by_callable
                .get(&key)
        } else {
            self.indexes.members.return_candidates_by_callable.get(&key)
        };
        let Some(return_candidates) = return_candidates else {
            return Ok(None);
        };
        if let Some(output_index) = binding.output_index {
            let Ok(output_index) = usize::try_from(output_index) else {
                return Ok(None);
            };
            let Some(return_candidate) = return_candidates.get(output_index) else {
                return Ok(None);
            };
            return self.resolved_return_type(return_candidate);
        }
        let [return_candidate] = return_candidates.as_slice() else {
            return Err(return_candidates.len());
        };
        self.resolved_return_type(return_candidate)
    }

    pub(in crate::evidence) fn rust_wildcard_callable_declarations(
        &self,
        language: &str,
        call_result_binding: &BindingFact,
        qualified_callable: &str,
        candidate: &RelationshipCandidate,
    ) -> Result<BTreeSet<DeclarationSlot>, usize> {
        if language != "rust" || call_result_binding.receiver_binding_id.is_some() {
            return Ok(BTreeSet::new());
        }
        let Some((qualifier, spelling)) = split_qualified_member(language, qualified_callable)
        else {
            return Ok(BTreeSet::new());
        };

        let mut callable_candidate = candidate.clone();
        callable_candidate.target_spelling = spelling.to_owned();
        callable_candidate.constraints.exact_target_declaration_id = None;
        callable_candidate.constraints.qualified_name = Some(qualified_callable.to_owned());
        callable_candidate.constraints.argument_count = None;
        callable_candidate.constraints.argument_types.clear();
        callable_candidate.constraints.allowed_target_kinds =
            vec!["function".to_owned(), "method".to_owned()];
        callable_candidate.constraints.hierarchy = None;
        callable_candidate.constraints.allow_external = false;

        if let Some(wildcard_binding) = call_result_binding
            .fallback_binding_id
            .as_deref()
            .and_then(|binding_id| self.facts.bindings.get(binding_id))
            .filter(|binding| binding.spelling == "*")
        {
            callable_candidate.binding_id = Some(wildcard_binding.id.clone());
            return self.wildcard_declarations(
                language,
                std::slice::from_ref(&wildcard_binding.qualified_target),
                Some(qualifier),
                &callable_candidate,
            );
        }

        let mut scope_id = call_result_binding.scope_id.as_deref();
        let mut visited_scopes = BTreeSet::new();
        while let Some(current) = scope_id.filter(|scope| visited_scopes.insert(*scope)) {
            if visited_scopes.len() > self.budget.candidates_per_lookup() {
                return Err(visited_scopes.len());
            }
            if let Some(bindings) = self
                .indexes
                .wildcards
                .by_scope
                .get(&(language.to_owned(), current.to_owned()))
            {
                if !bindings.complete {
                    return Err(bindings.modules.len().saturating_add(1));
                }
                let declarations = self.wildcard_declarations(
                    language,
                    &bindings.modules,
                    Some(qualifier),
                    &callable_candidate,
                )?;
                if !declarations.is_empty() {
                    return Ok(declarations);
                }
            }
            scope_id = self
                .facts
                .scopes
                .get(current)
                .and_then(|scope| scope.parent_scope_id.as_deref());
        }
        Ok(BTreeSet::new())
    }

    pub(in crate::evidence) fn resolved_return_type(
        &self,
        candidate_id: &str,
    ) -> Result<Option<String>, usize> {
        let Some(candidate) = self.facts.candidates.get(candidate_id) else {
            return Ok(None);
        };
        if candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.facts.bindings.get(binding_id))
            .is_some_and(|binding| binding.kind == compass_languages::BindingKind::CallResult)
        {
            return Ok(None);
        }
        match self.resolve(candidate_id) {
            ResolutionDecision::Resolved { declaration_id, .. } => Ok(self
                .facts
                .declarations
                .get(&declaration_id)
                .map(|declaration| declaration.qualified_name.clone())),
            ResolutionDecision::QualifiedExternal { qualified_name, .. } => {
                Ok(Some(qualified_name))
            }
            ResolutionDecision::Ambiguous { candidate_count } => Err(candidate_count),
            ResolutionDecision::ResolvedInventory { .. }
            | ResolutionDecision::DeferredReceiver { .. }
            | ResolutionDecision::Unresolved => Ok(None),
        }
    }

    pub(in crate::evidence) fn callable_declarations(
        &self,
        language: &str,
        qualified: &str,
    ) -> Vec<DeclarationSlot> {
        let qualified = match self.follow_alias(language, qualified) {
            Ok(qualified) => qualified,
            Err(_) => return Vec::new(),
        };
        let qualified = if language == "rust" {
            let Some((owner, member)) = qualified.rsplit_once("::") else {
                return Vec::new();
            };
            let owner = match self.follow_alias(language, owner) {
                Ok(owner) => owner,
                Err(_) => return Vec::new(),
            };
            format!("{owner}::{member}")
        } else {
            qualified
        };
        if let Some(declarations) = self
            .indexes
            .names
            .by_qualified
            .get(&(language.to_owned(), qualified.clone()))
        {
            return declarations.clone();
        }
        if language != "go" {
            return Vec::new();
        }
        let mut declarations = BTreeSet::new();
        if let Some((owner, member)) = qualified.rsplit_once("::")
            && let Some((import_path, owner_spelling)) = owner.rsplit_once('.')
        {
            for owner_id in self
                .imported_declarations(language, import_path, owner_spelling)
                .into_iter()
                .take(self.budget.candidates_per_lookup())
            {
                let Some(owner) = self.declaration(owner_id) else {
                    continue;
                };
                if let Some(ids) = self.indexes.names.by_qualified.get(&(
                    language.to_owned(),
                    format!("{}::{member}", owner.qualified_name),
                )) {
                    declarations.extend(ids.iter().cloned());
                }
            }
        } else if let Some((import_path, spelling)) = qualified.rsplit_once('.') {
            declarations.extend(self.imported_declarations(language, import_path, spelling));
        }
        declarations
            .into_iter()
            .take(candidate_storage_limit(self.budget.candidates_per_lookup()))
            .collect()
    }

    pub(in crate::evidence) fn member_decision(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let declarations = self.member_declarations(language, qualified, candidate)?;
        self.unique_decision(
            Some(&declarations),
            candidate,
            ResolutionRule::MemberBinding,
        )
    }

    pub(in crate::evidence) fn rust_trait_member_declarations(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Result<Vec<DeclarationSlot>, usize> {
        if language != "rust" {
            return Ok(Vec::new());
        }
        let Some((receiver, member)) = split_qualified_member(language, qualified) else {
            return Ok(Vec::new());
        };
        let receiver_slots = self
            .indexes
            .names
            .by_qualified
            .get(&(language.to_owned(), receiver.to_owned()))
            .into_iter()
            .flatten()
            .filter(|slot| {
                self.declaration(**slot).is_some_and(|declaration| {
                    matches!(
                        declaration.kind.as_str(),
                        "class" | "enum" | "struct" | "trait" | "type_alias"
                    )
                })
            })
            .copied()
            .collect::<Vec<_>>();
        let [receiver_slot] = receiver_slots.as_slice() else {
            return if receiver_slots.is_empty() {
                Ok(Vec::new())
            } else {
                Err(receiver_slots.len())
            };
        };
        let Some(receiver_id) = self.declaration_id(*receiver_slot) else {
            return Ok(Vec::new());
        };
        let mut matching_trait_sets = self
            .indexes
            .rust
            .impl_traits
            .iter()
            .filter(|((implementer_id, _), _)| implementer_id == receiver_id)
            .collect::<Vec<_>>();
        matching_trait_sets.sort_by(|left, right| left.0.1.cmp(&right.0.1));
        if matching_trait_sets.len() > self.budget.candidates_per_lookup() {
            return Err(matching_trait_sets.len());
        }
        let mut declarations = BTreeSet::new();
        for ((_, raw_trait_name), traits) in matching_trait_sets {
            if !traits.complete {
                return Err(traits.candidate_ids.len().saturating_add(1));
            }
            for candidate_id in &traits.candidate_ids {
                let Some(implementation) = self.facts.candidates.get(candidate_id) else {
                    return Err(1);
                };
                let trait_name = match self
                    .resolve_rust_impl_trait_candidate(&implementation, raw_trait_name)
                {
                    ResolutionDecision::Resolved { declaration_id, .. } => {
                        let Some(declaration) = self.facts.declarations.get(&declaration_id) else {
                            return Err(1);
                        };
                        declaration.qualified_name.as_str()
                    }
                    ResolutionDecision::Ambiguous { candidate_count } => {
                        return Err(candidate_count);
                    }
                    ResolutionDecision::QualifiedExternal { .. }
                    | ResolutionDecision::DeferredReceiver { .. }
                    | ResolutionDecision::ResolvedInventory { .. }
                    | ResolutionDecision::Unresolved => continue,
                };
                if let Some(slots) = self
                    .indexes
                    .names
                    .by_qualified
                    .get(&(language.to_owned(), format!("{trait_name}::{member}")))
                {
                    declarations.extend(
                        slots
                            .iter()
                            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                            .copied(),
                    );
                }
                if declarations.len() > self.budget.candidates_per_lookup() {
                    return Err(declarations.len());
                }
            }
        }
        Ok(declarations.into_iter().collect())
    }

    pub(in crate::evidence) fn wildcard_reexport_decision(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let (facade, spelling) = qualified.rsplit_once('.')?;
        if !self
            .indexes
            .wildcards
            .reexports_by_module
            .contains_key(&(language.to_owned(), facade.to_owned()))
        {
            return None;
        }
        let mut modules = vec![facade.to_owned()];
        let mut visited = BTreeSet::new();
        let mut declarations = BTreeSet::<DeclarationSlot>::new();
        while let Some(module) = modules.pop() {
            if !visited.insert(module.clone()) {
                continue;
            }
            if visited.len() > self.budget.candidates_per_lookup() {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: visited.len(),
                });
            }
            let Some(reexports) = self
                .indexes
                .wildcards
                .reexports_by_module
                .get(&(language.to_owned(), module))
            else {
                continue;
            };
            if !reexports.complete {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: reexports.modules.len().saturating_add(1),
                });
            }
            for reexport in &reexports.modules {
                let reexported = format!("{reexport}.{spelling}");
                let canonical = match self.follow_alias(language, &reexported) {
                    Ok(canonical) => canonical,
                    Err(candidate_count) => {
                        return Some(ResolutionDecision::Ambiguous { candidate_count });
                    }
                };
                if let Some(ids) = self
                    .indexes
                    .names
                    .by_qualified
                    .get(&(language.to_owned(), canonical.clone()))
                {
                    declarations.extend(
                        ids.iter()
                            .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                            .copied(),
                    );
                }
                if let Some(ids) = self.member_declarations(language, &canonical, candidate) {
                    declarations.extend(ids);
                }
                modules.push(reexport.clone());
            }
            if declarations.len() > self.budget.candidates_per_lookup() {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: declarations.len(),
                });
            }
        }
        self.unique_decision(
            Some(&declarations.into_iter().collect::<Vec<_>>()),
            candidate,
            ResolutionRule::WildcardBinding,
        )
    }

    pub(in crate::evidence) fn member_declarations(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<Vec<DeclarationSlot>> {
        let (owner, spelling) = split_qualified_member(language, qualified)?;
        let targets = self.indexes.members.members.get(&(
            language.to_owned(),
            owner.to_owned(),
            spelling.to_owned(),
        ))?;
        let mut declarations = BTreeSet::new();
        for target in targets {
            if let Some(ids) = self
                .indexes
                .names
                .by_qualified
                .get(&(language.to_owned(), target.clone()))
            {
                declarations.extend(
                    ids.iter()
                        .filter(|slot| self.declaration_allowed_slot(**slot, candidate))
                        .cloned(),
                );
            }
        }
        Some(declarations.into_iter().collect())
    }

    pub(in crate::evidence) fn declaration_allowed(
        &self,
        declaration_id: &str,
        candidate: &RelationshipCandidate,
    ) -> bool {
        let Some(target) = self.facts.declarations.get(declaration_id) else {
            return false;
        };
        if !declaration_basic_allowed(target, candidate) {
            return false;
        }
        let argument_types = &candidate.constraints.argument_types;
        if target.language != "java"
            || argument_types.is_empty()
            || argument_types.iter().any(Option::is_none)
        {
            return true;
        }
        let Some(overloads) = self
            .indexes
            .names
            .by_qualified
            .get(&(target.language.clone(), target.qualified_name.clone()))
        else {
            return true;
        };
        let eligible = overloads
            .iter()
            .filter_map(|slot| self.declaration(*slot))
            .filter(|declaration| declaration_basic_allowed(declaration, candidate))
            .collect::<Vec<_>>();
        let exact = eligible
            .iter()
            .copied()
            .filter(|declaration| {
                declaration.parameter_types.len() == argument_types.len()
                    && declaration
                        .parameter_types
                        .iter()
                        .zip(argument_types)
                        .all(|(parameter, argument)| argument.as_deref() == Some(parameter))
            })
            .take(2)
            .collect::<Vec<_>>();
        match exact.as_slice() {
            [only] => only.id == target.id,
            [] => LanguagePolicyKind::for_language(&target.language)
                .unique_applicable_overload(self, &eligible, argument_types)
                .is_none_or(|only| only == target.id),
            _ => true,
        }
    }

    pub(in crate::evidence) fn follow_alias(
        &self,
        language: &str,
        qualified: &str,
    ) -> Result<String, usize> {
        let mut current = qualified.to_owned();
        let mut seen = BTreeSet::new();
        for _ in 0..64 {
            if !seen.insert(current.clone()) {
                return Err(seen.len());
            }
            let Some(targets) = self
                .indexes
                .names
                .aliases
                .get(&(language.to_owned(), current.clone()))
            else {
                return Ok(current);
            };
            let [target] = targets.as_slice() else {
                return Err(targets.len());
            };
            if target == &current {
                return Ok(current);
            }
            current.clone_from(target);
        }
        Err(64)
    }

    pub(in crate::evidence) fn follow_wildcard_qualified_alias(
        &self,
        language: &str,
        qualified: &str,
    ) -> Result<String, usize> {
        if language != "rust" {
            return self.follow_alias(language, qualified);
        }

        const MAX_ALIAS_DEPTH: usize = 64;
        let mut current = qualified.to_owned();
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_ALIAS_DEPTH {
            if !seen.insert(current.clone()) {
                return Err(seen.len());
            }
            if let Some(targets) = self
                .indexes
                .names
                .aliases
                .get(&(language.to_owned(), current.clone()))
            {
                let [target] = targets.as_slice() else {
                    return Err(targets.len());
                };
                if target == &current {
                    return Ok(current);
                }
                current.clone_from(target);
                continue;
            }

            let separators = current
                .match_indices("::")
                .map(|(index, _)| index)
                .take(MAX_ALIAS_DEPTH.saturating_add(1))
                .collect::<Vec<_>>();
            if separators.len() > MAX_ALIAS_DEPTH {
                return Err(separators.len());
            }
            let mut expanded = None;
            for separator in separators.into_iter().rev() {
                let prefix = &current[..separator];
                let Some(targets) = self
                    .indexes
                    .names
                    .aliases
                    .get(&(language.to_owned(), prefix.to_owned()))
                else {
                    continue;
                };
                let [target] = targets.as_slice() else {
                    return Err(targets.len());
                };
                if target == prefix {
                    return Ok(current);
                }
                expanded = Some(format!("{target}{}", &current[separator..]));
                break;
            }
            let Some(next) = expanded else {
                return Ok(current);
            };
            current = next;
        }
        Err(MAX_ALIAS_DEPTH)
    }
}

fn normalize_ruby_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_owned()
}
