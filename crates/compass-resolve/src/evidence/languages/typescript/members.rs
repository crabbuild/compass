//! Structural, generic, indexed, and callable-result member resolution.

use super::*;

impl UniversalResolutionIndex {
    pub(in crate::evidence) fn typescript_structural_member_slots(
        &self,
        owner_slot: DeclarationSlot,
        property: &str,
        candidate: &RelationshipCandidate,
        visiting: &mut BTreeSet<DeclarationSlot>,
    ) -> Result<Option<BTreeSet<DeclarationSlot>>, ()> {
        if !visiting.insert(owner_slot) {
            return Err(());
        }
        let Some(owner) = self.declaration(owner_slot) else {
            visiting.remove(&owner_slot);
            return Err(());
        };
        let owner_language = owner.language.as_str();
        if owner.kind == "module" {
            // Module-owner aliases represent a namespace object (including a
            // CommonJS `module.exports` object). Resolve direct members only
            // through the provider's published export slots; the lexical
            // module scope also contains private declarations that must not be
            // exposed merely because their names match the requested member.
            let mut exported = BTreeSet::new();
            let source_modules =
                typescript_source_module_keys(&owner.range.source_file, &self.project.root);
            for module in source_modules {
                exported.extend(self.typescript_export_slots(
                    owner_language,
                    &module,
                    property,
                    candidate,
                    false,
                ));
            }
            if exported.is_empty() {
                exported.extend(self.typescript_export_slots(
                    owner_language,
                    &owner.qualified_name,
                    property,
                    candidate,
                    false,
                ));
            }
            if !exported.is_empty() {
                visiting.remove(&owner_slot);
                return Ok(Some(exported));
            }
        }
        let direct_key = (
            owner_language.to_owned(),
            owner.qualified_name.clone(),
            property.to_owned(),
        );
        if owner.kind != "module"
            && let Some(members) = self.indexes.members_by_owner.get(&direct_key)
        {
            let direct = members
                .iter()
                .filter(|slot| self.typescript_declaration_allowed_slot(**slot, candidate))
                .copied()
                .collect::<BTreeSet<_>>();
            if !direct.is_empty() {
                visiting.remove(&owner_slot);
                return Ok(Some(direct));
            }
        }
        let alias_key = (owner_language.to_owned(), owner.qualified_name.clone());
        let Some(aliases) = self.indexes.typescript_member_aliases.get(&alias_key) else {
            visiting.remove(&owner_slot);
            return Ok(None);
        };
        if !self.typescript_structural_object_owner(owner_slot) {
            visiting.remove(&owner_slot);
            return Err(());
        }
        let mut inherited = BTreeSet::new();
        let mut unresolved = false;
        for alias in aliases {
            let source_slots =
                self.typescript_member_alias_source_slots(owner_language, owner, alias, candidate);
            let Some(source_slots) = source_slots else {
                unresolved = true;
                continue;
            };
            if source_slots.is_empty() {
                unresolved = true;
                continue;
            }
            for source_slot in source_slots {
                match self.typescript_structural_member_slots(
                    source_slot,
                    property,
                    candidate,
                    visiting,
                ) {
                    Ok(Some(members)) => inherited.extend(members),
                    Ok(None) => {}
                    Err(()) => unresolved = true,
                }
            }
        }
        visiting.remove(&owner_slot);
        if unresolved {
            return Err(());
        }
        Ok(Some(inherited))
    }

    pub(in crate::evidence) fn typescript_structural_object_owner(
        &self,
        slot: DeclarationSlot,
    ) -> bool {
        let Some(owner) = self.declaration(slot) else {
            return false;
        };
        if !matches!(owner.language.as_str(), "typescript" | "javascript") {
            return false;
        }
        // CommonJS object exports retain the file module as their owner. A
        // `Member("*")` alias is emitted for that owner only after the
        // adapter proves every spread source, so treating a module owner as a
        // structural object here does not widen arbitrary module lookups.
        if owner.kind == "module" {
            return true;
        }
        if owner.kind != "variable" {
            return false;
        }
        if owner
            .scope_id
            .as_deref()
            .and_then(|scope_id| self.facts.scopes.get(scope_id))
            .is_some_and(|scope| scope.kind == "object")
        {
            return true;
        }
        self.indexes
            .members_by_owner
            .keys()
            .any(|(language, qualified, _)| {
                language == &owner.language && qualified == &owner.qualified_name
            })
    }

    pub(in crate::evidence) fn typescript_member_alias_source_slots(
        &self,
        language: &str,
        owner: &DeclarationFact,
        alias: &TypeScriptMemberAlias,
        candidate: &RelationshipCandidate,
    ) -> Option<BTreeSet<DeclarationSlot>> {
        let mut source_slots = BTreeSet::new();
        if let Some(slot) = alias.source_slot {
            source_slots.insert(slot);
            return Some(source_slots);
        }
        if let Some((module, exported)) = alias.source.rsplit_once("::") {
            if module.is_empty() || exported.is_empty() {
                return None;
            }
            let mut modules = self
                .typescript_project_module_keys(&owner.range.source_file, module, None)
                .unwrap_or_else(|| {
                    typescript_import_module_keys(
                        &owner.range.source_file,
                        module,
                        &self.project.root,
                    )
                });
            modules.sort_unstable();
            modules.dedup();
            for module in modules {
                source_slots.extend(
                    self.typescript_export_slots(language, &module, exported, candidate, true),
                );
            }
            return Some(source_slots);
        }
        if let Some(slots) = self
            .indexes
            .by_qualified
            .get(&(language.to_owned(), alias.source.clone()))
        {
            source_slots.extend(slots.iter().copied());
        }
        Some(source_slots)
    }

    pub(in crate::evidence) fn typescript_project_module_keys(
        &self,
        importer: &str,
        module: &str,
        context: Option<&str>,
    ) -> Option<Vec<String>> {
        typescript_project_module_keys(
            &self.project.typescript_project_modules,
            importer,
            module,
            &self.project.root,
            context,
        )
    }

    pub(in crate::evidence) fn typescript_project_metadata(
        &self,
        candidate: &RelationshipCandidate,
        target_source_file: Option<&str>,
    ) -> Option<BTreeMap<String, String>> {
        if !matches!(candidate.language.as_str(), "typescript" | "javascript") {
            return None;
        }
        let occurrence = self.occurrence(candidate)?;
        let module = candidate
            .constraints
            .module_or_package
            .clone()
            .or_else(|| {
                candidate
                    .binding_id
                    .as_deref()
                    .and_then(|binding_id| self.facts.bindings.get(binding_id))
                    .and_then(|binding| binding.qualified_target.rsplit_once("::"))
                    .map(|(module, _)| module.to_owned())
            })?;
        let importer =
            typescript_project_importer_key(&occurrence.range.source_file, &self.project.root)?;
        let target_keys = target_source_file
            .map(|source| typescript_source_module_keys(source, &self.project.root))
            .unwrap_or_else(|| vec![String::new()]);
        if target_keys.is_empty() {
            return None;
        }
        let mut matches = Vec::new();
        for ((candidate_importer, candidate_module, _candidate_context, target_key), value) in
            &self.project.typescript_project_metadata
        {
            if candidate_importer != &importer
                || candidate_module != &module
                || !target_keys.contains(target_key)
            {
                continue;
            }
            matches.push(value.clone());
        }
        matches.sort();
        matches.dedup();
        (matches.len() == 1).then(|| matches.pop()).flatten()
    }

    /// Resolve a bounded TypeScript member-value chain such as
    /// `Box<Item>.item.inspect`. The root export and generic arguments come
    /// from the import binding; each intermediate member must publish a
    /// direct type signature, and every hop must resolve uniquely. Missing,
    /// structural, or competing type evidence remains unresolved.
    pub(in crate::evidence) fn typescript_member_chain_slots(
        &self,
        language: &str,
        module: &str,
        path: &TypeScriptMemberPath,
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<DeclarationSlot> {
        const MAX_MEMBER_CHAIN_DEPTH: usize = 16;
        if path.members.is_empty() || path.members.len() > MAX_MEMBER_CHAIN_DEPTH {
            return BTreeSet::new();
        }
        let root_targets =
            self.typescript_export_slots(language, module, &path.root_export, candidate, true);
        let root_targets = if path.call_result && path.call_member_index.is_none() {
            self.typescript_callable_overload_slots(
                &root_targets,
                &path.call_argument_types,
                &path.call_type_arguments,
            )
        } else {
            root_targets
        };
        if path.call_member_index.is_some() && root_targets.len() != 1 {
            return BTreeSet::new();
        }
        let mut targets = BTreeSet::new();
        for root_slot in root_targets {
            let has_index_signature = self
                .declaration(root_slot)
                .and_then(|declaration| declaration.signature.as_deref())
                .and_then(typescript_index_value_type)
                .is_some();
            let mut owners = if path.call_result && path.call_member_index.is_none() {
                self.typescript_callable_return_contexts(
                    language,
                    module,
                    root_slot,
                    &path.call_argument_types,
                    &path.call_type_arguments,
                    candidate,
                )
            } else if let Some(signature) = self
                .declaration(root_slot)
                .and_then(|declaration| declaration.signature.as_deref())
                && typescript_value_type(signature).is_some()
            {
                self.typescript_value_type_contexts(
                    language,
                    module,
                    root_slot,
                    &path.type_arguments,
                    candidate,
                )
            } else if path.indexed && has_index_signature {
                self.typescript_index_value_contexts(
                    language,
                    module,
                    root_slot,
                    &path.type_arguments,
                    candidate,
                )
            } else {
                self.typescript_expand_type_alias_contexts(
                    language,
                    module,
                    root_slot,
                    path.type_arguments.clone(),
                    candidate,
                )
            };
            for (index, raw_member_name) in path.members.iter().enumerate() {
                let Some((member_name, index_selector)) =
                    typescript_indexed_member_segment(raw_member_name)
                else {
                    owners.clear();
                    break;
                };
                let final_member = index.saturating_add(1) == path.members.len();
                let mut next_owners = BTreeSet::new();
                for (owner_slot, owner_type_arguments) in owners.clone() {
                    let expanded_owners = self.typescript_expand_type_alias_contexts(
                        language,
                        module,
                        owner_slot,
                        owner_type_arguments,
                        candidate,
                    );
                    for (owner_slot, owner_type_arguments) in expanded_owners {
                        let Some(owner) = self.declaration(owner_slot) else {
                            continue;
                        };
                        let Some(members) = self.indexes.members_by_owner.get(&(
                            owner.language.clone(),
                            owner.qualified_name.clone(),
                            member_name.clone(),
                        )) else {
                            continue;
                        };
                        if path.call_result && path.call_member_index == Some(index) {
                            let owner_module = typescript_source_module_keys(
                                &owner.range.source_file,
                                &self.project.root,
                            )
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| module.to_owned());
                            next_owners.extend(self.typescript_member_callable_return_contexts(
                                language,
                                &owner_module,
                                members,
                                &path.call_argument_types,
                                &path.call_type_arguments,
                                candidate,
                            ));
                            continue;
                        }
                        if final_member {
                            next_owners.extend(
                                members
                                    .iter()
                                    .filter(|slot| {
                                        self.typescript_declaration_allowed_slot(**slot, candidate)
                                    })
                                    .map(|slot| (*slot, Vec::new())),
                            );
                        } else {
                            let typed_members = members
                                .iter()
                                .filter_map(|slot| self.declaration(*slot))
                                .filter(|member| member.kind == "property")
                                .filter_map(|member| member.signature.as_deref())
                                .collect::<Vec<_>>();
                            let [type_name] = typed_members.as_slice() else {
                                continue;
                            };
                            next_owners.extend(self.typescript_member_type_contexts(
                                language,
                                module,
                                type_name,
                                TypeScriptMemberContext {
                                    owner_signature: owner.signature.as_deref(),
                                    type_arguments: &owner_type_arguments,
                                    index_selector: index_selector.as_deref(),
                                },
                                candidate,
                            ));
                        }
                    }
                }
                if next_owners.is_empty() {
                    owners.clear();
                    break;
                }
                owners = next_owners;
            }
            targets.extend(owners.into_iter().map(|(slot, _)| slot));
            if targets.len() > self.budget.candidates_per_lookup() {
                break;
            }
        }
        targets
    }

    pub(in crate::evidence) fn typescript_value_type_contexts(
        &self,
        language: &str,
        module: &str,
        owner_slot: DeclarationSlot,
        type_arguments: &[String],
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<(DeclarationSlot, Vec<String>)> {
        let Some(owner) = self.declaration(owner_slot) else {
            return BTreeSet::new();
        };
        let Some(signature) = owner.signature.as_deref() else {
            return BTreeSet::new();
        };
        let Some(type_name) = typescript_value_type(signature) else {
            return BTreeSet::new();
        };
        self.typescript_member_type_contexts(
            language,
            module,
            type_name,
            TypeScriptMemberContext {
                owner_signature: None,
                type_arguments,
                index_selector: None,
            },
            candidate,
        )
    }

    /// Resolve the nominal receiver produced by an imported callable result.
    /// Only source-published fixed parameters and direct/container generic
    /// inference are accepted; overloads, contextual inference, and
    /// structural assignability remain unresolved.
    pub(in crate::evidence) fn typescript_callable_return_contexts(
        &self,
        language: &str,
        module: &str,
        callable_slot: DeclarationSlot,
        call_argument_types: &[String],
        call_type_arguments: &[String],
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<(DeclarationSlot, Vec<String>)> {
        let Some(callable) = self.declaration(callable_slot) else {
            return BTreeSet::new();
        };
        let Some(signature) = callable.signature.as_deref() else {
            return BTreeSet::new();
        };
        let Some(return_type) = typescript_callable_return_type(signature) else {
            return BTreeSet::new();
        };
        let parameters = typescript_generic_parameter_names(signature);
        let mut type_arguments = if call_type_arguments.is_empty() {
            parameters.clone()
        } else if call_type_arguments.len() == parameters.len() {
            call_type_arguments.to_vec()
        } else {
            return BTreeSet::new();
        };
        if let Some(parameter_types) = typescript_callable_parameter_types(signature) {
            if parameter_types.len() != call_argument_types.len() {
                return BTreeSet::new();
            }
            for (parameter_type, argument_type) in
                parameter_types.iter().zip(call_argument_types.iter())
            {
                if argument_type == "__unknown" {
                    continue;
                }
                if !typescript_infer_type_arguments(
                    parameter_type,
                    argument_type,
                    &parameters,
                    &mut type_arguments,
                ) {
                    return BTreeSet::new();
                }
            }
        } else if call_type_arguments.is_empty()
            && !parameters.is_empty()
            && typescript_type_mentions_parameter(return_type, &parameters)
        {
            return BTreeSet::new();
        }
        if !parameters.is_empty()
            && typescript_type_mentions_parameter(return_type, &parameters)
            && type_arguments == parameters
        {
            return BTreeSet::new();
        }
        self.typescript_member_type_contexts(
            language,
            module,
            return_type,
            TypeScriptMemberContext {
                owner_signature: Some(signature),
                type_arguments: &type_arguments,
                index_selector: None,
            },
            candidate,
        )
    }

    pub(in crate::evidence) fn typescript_member_callable_return_contexts(
        &self,
        language: &str,
        module: &str,
        members: &[DeclarationSlot],
        call_argument_types: &[String],
        call_type_arguments: &[String],
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<(DeclarationSlot, Vec<String>)> {
        let mut contexts = BTreeSet::new();
        let member_slots = members
            .iter()
            .copied()
            .filter(|slot| {
                self.declaration(*slot)
                    .and_then(|declaration| declaration.signature.as_deref())
                    .and_then(typescript_callable_return_type)
                    .is_some()
            })
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .collect::<BTreeSet<_>>();
        let callable_members = self.typescript_callable_overload_slots(
            &member_slots,
            call_argument_types,
            call_type_arguments,
        );
        if callable_members.len() != 1 {
            return BTreeSet::new();
        }
        for slot in callable_members {
            contexts.extend(self.typescript_callable_return_contexts(
                language,
                module,
                slot,
                call_argument_types,
                call_type_arguments,
                candidate,
            ));
            if contexts.len() > self.budget.candidates_per_lookup() {
                break;
            }
        }
        contexts
    }

    /// Select one source-proven callable overload when the call carries a
    /// complete, exact argument shape.  TypeScript overload assignability is
    /// intentionally not modeled here: unsupported unions, contextual
    /// inference, optional/rest parameters, and competing exact matches stay
    /// unresolved instead of being collapsed to a convenient declaration.
    pub(in crate::evidence) fn typescript_callable_overload_slots(
        &self,
        slots: &BTreeSet<DeclarationSlot>,
        call_argument_types: &[String],
        call_type_arguments: &[String],
    ) -> BTreeSet<DeclarationSlot> {
        let callable_slots = slots
            .iter()
            .filter(|slot| {
                self.declaration(**slot)
                    .and_then(|declaration| declaration.signature.as_deref())
                    .is_some_and(|signature| {
                        typescript_callable_return_type(signature).is_some()
                            || typescript_callable_parameter_types(signature).is_some()
                    })
            })
            .take(self.budget.candidates_per_lookup().saturating_add(1))
            .copied()
            .collect::<Vec<_>>();
        if callable_slots.len() > self.budget.candidates_per_lookup() {
            return BTreeSet::new();
        }
        if callable_slots.len() <= 1 {
            return slots.clone();
        }
        let matching = callable_slots
            .iter()
            .copied()
            .filter(|slot| {
                self.declaration(*slot).is_some_and(|declaration| {
                    let normalized_argument_types = call_argument_types
                        .iter()
                        .map(|argument| {
                            self.typescript_normalize_overload_type(
                                &declaration.range.source_file,
                                argument,
                            )
                        })
                        .collect::<Vec<_>>();
                    let normalized_type_arguments = call_type_arguments
                        .iter()
                        .map(|argument| {
                            self.typescript_normalize_overload_type(
                                &declaration.range.source_file,
                                argument,
                            )
                        })
                        .collect::<Vec<_>>();
                    typescript_callable_overload_matches(
                        declaration,
                        &normalized_argument_types,
                        &normalized_type_arguments,
                        &self.typescript_callable_parameter_aliases(declaration),
                    )
                })
            })
            .collect::<Vec<_>>();
        match matching.as_slice() {
            [only] => BTreeSet::from([*only]),
            _ => BTreeSet::new(),
        }
    }

    pub(in crate::evidence) fn typescript_callable_parameter_aliases(
        &self,
        declaration: &DeclarationFact,
    ) -> AHashMap<String, String> {
        let mut aliases = AHashMap::<String, Vec<String>>::new();
        let mut scope = declaration.scope_id.clone();
        let mut visited = BTreeSet::new();
        let mut resolved_spellings = BTreeSet::new();
        while let Some(scope_id) = scope.filter(|scope_id| visited.insert(scope_id.clone())) {
            let mut scope_spellings = BTreeSet::new();
            for binding in self.facts.bindings.values().filter(|binding| {
                binding.language == declaration.language
                    && binding.scope_id.as_deref() == Some(scope_id.as_str())
                    && matches!(
                        binding.kind,
                        compass_languages::BindingKind::Import
                            | compass_languages::BindingKind::ImportAlias
                            | compass_languages::BindingKind::Reexport
                    )
                    && binding.namespace.is_none_or(|namespace| {
                        matches!(
                            namespace,
                            compass_languages::SymbolNamespace::Type
                                | compass_languages::SymbolNamespace::ValueAndType
                                | compass_languages::SymbolNamespace::Namespace
                        )
                    })
            }) {
                if resolved_spellings.contains(&binding.spelling)
                    && !scope_spellings.contains(&binding.spelling)
                {
                    continue;
                }
                if !scope_spellings.insert(binding.spelling.clone()) {
                    let targets = aliases.entry(binding.spelling.clone()).or_default();
                    if targets.len() < 2 {
                        targets.push(self.typescript_normalize_overload_type(
                            &declaration.range.source_file,
                            &binding.qualified_target,
                        ));
                    }
                    continue;
                }
                aliases.insert(
                    binding.spelling.clone(),
                    vec![self.typescript_normalize_overload_type(
                        &declaration.range.source_file,
                        &binding.qualified_target,
                    )],
                );
            }
            resolved_spellings.extend(scope_spellings);
            scope = self
                .facts
                .scopes
                .get(&scope_id)
                .and_then(|scope| scope.parent_scope_id.clone());
        }
        aliases
            .into_iter()
            .filter_map(|(spelling, targets)| {
                let [target] = targets.as_slice() else {
                    return None;
                };
                Some((spelling, target.clone()))
            })
            .collect()
    }

    pub(in crate::evidence) fn typescript_normalize_overload_type(
        &self,
        source_file: &str,
        value: &str,
    ) -> String {
        self.typescript_normalize_overload_type_at_depth(source_file, value, 0)
    }

    pub(in crate::evidence) fn typescript_normalize_overload_type_at_depth(
        &self,
        source_file: &str,
        value: &str,
        depth: usize,
    ) -> String {
        let value = value.trim();
        if depth > 32 || value.is_empty() || value == "__unknown" || value.len() > 1024 {
            return value.to_owned();
        }
        if let Some((base, arguments)) = typescript_generic_type_parts(value) {
            let arguments = arguments
                .iter()
                .map(|argument| {
                    self.typescript_normalize_overload_type_at_depth(
                        source_file,
                        argument,
                        depth.saturating_add(1),
                    )
                })
                .collect::<Vec<_>>();
            return format!("{base}<{}>", arguments.join(","));
        }
        let (module, exported) = split_typescript_module_qualified(value);
        let Some(module) = module else {
            return value.to_owned();
        };
        let Some(key) = typescript_import_module_keys(source_file, module, &self.project.root)
            .into_iter()
            .next()
        else {
            return value.to_owned();
        };
        format!("{key}::{exported}")
    }

    pub(in crate::evidence) fn typescript_index_value_contexts(
        &self,
        language: &str,
        module: &str,
        owner_slot: DeclarationSlot,
        type_arguments: &[String],
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<(DeclarationSlot, Vec<String>)> {
        let Some(owner) = self.declaration(owner_slot) else {
            return BTreeSet::new();
        };
        let Some(signature) = owner.signature.as_deref() else {
            return BTreeSet::new();
        };
        let Some(type_name) = typescript_index_value_type(signature) else {
            return BTreeSet::new();
        };
        self.typescript_member_type_contexts(
            language,
            module,
            type_name,
            TypeScriptMemberContext {
                owner_signature: Some(signature),
                type_arguments,
                index_selector: None,
            },
            candidate,
        )
    }

    pub(in crate::evidence) fn typescript_expand_type_alias_contexts(
        &self,
        language: &str,
        module: &str,
        owner_slot: DeclarationSlot,
        type_arguments: Vec<String>,
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<(DeclarationSlot, Vec<String>)> {
        const MAX_ALIAS_DEPTH: usize = 16;
        let mut current = BTreeSet::from([(owner_slot, type_arguments)]);
        let mut seen = BTreeSet::new();
        for _ in 0..=MAX_ALIAS_DEPTH {
            let mut next = BTreeSet::new();
            let mut expanded = false;
            for (slot, arguments) in current {
                if !seen.insert((slot, arguments.clone())) {
                    continue;
                }
                let Some(owner) = self.declaration(slot) else {
                    continue;
                };
                let Some(signature) = owner.signature.as_deref() else {
                    next.insert((slot, arguments));
                    continue;
                };
                let Some(alias_target) = typescript_type_alias_target(signature) else {
                    next.insert((slot, arguments));
                    continue;
                };
                let alias_module =
                    typescript_source_module_keys(&owner.range.source_file, &self.project.root)
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| module.to_owned());
                let substituted = typescript_substitute_type_parameters(
                    alias_target,
                    &typescript_generic_parameter_names(signature),
                    &arguments,
                );
                let contexts = self.typescript_member_type_contexts(
                    language,
                    &alias_module,
                    &substituted,
                    TypeScriptMemberContext {
                        owner_signature: Some(signature),
                        type_arguments: &arguments,
                        index_selector: substituted.strip_suffix("[]").map(|_| ""),
                    },
                    candidate,
                );
                if contexts.is_empty() {
                    continue;
                }
                expanded = true;
                next.extend(contexts);
            }
            if !expanded {
                return next;
            }
            current = next;
        }
        BTreeSet::new()
    }

    pub(in crate::evidence) fn typescript_member_type_contexts(
        &self,
        language: &str,
        module: &str,
        type_name: &str,
        context: TypeScriptMemberContext<'_>,
        candidate: &RelationshipCandidate,
    ) -> BTreeSet<(DeclarationSlot, Vec<String>)> {
        let type_name = type_name.trim();
        if type_name.is_empty() || type_name.len() > 1024 {
            return BTreeSet::new();
        }
        let generic_parameters = context
            .owner_signature
            .map(typescript_generic_parameter_names)
            .unwrap_or_default();
        let substituted = typescript_substitute_type_parameters(
            type_name,
            &generic_parameters,
            context.type_arguments,
        );
        let substituted = typescript_utility_receiver_type(&substituted).unwrap_or(substituted);
        if let Some((base, property)) = typescript_literal_indexed_type(&substituted) {
            let base_contexts = self.typescript_member_type_contexts(
                language,
                module,
                base,
                TypeScriptMemberContext {
                    owner_signature: None,
                    type_arguments: &[],
                    index_selector: None,
                },
                candidate,
            );
            let mut indexed_contexts = BTreeSet::new();
            for (owner_slot, owner_type_arguments) in base_contexts {
                let Some(owner) = self.declaration(owner_slot) else {
                    continue;
                };
                let Some(members) = self.indexes.members_by_owner.get(&(
                    owner.language.clone(),
                    owner.qualified_name.clone(),
                    property.clone(),
                )) else {
                    continue;
                };
                if members.len() != 1 {
                    continue;
                }
                let Some(&member_slot) = members.first() else {
                    continue;
                };
                let Some(member) = self.declaration(member_slot) else {
                    continue;
                };
                let Some(signature) = member.signature.as_deref() else {
                    continue;
                };
                let direct_property_type = (member.kind == "property"
                    && !signature.contains("|params:")
                    && !signature.contains("|return:"))
                .then_some(signature.trim());
                if let Some(value_type) = typescript_value_type(signature).or(direct_property_type)
                {
                    let owner_parameters = typescript_generic_parameter_names(
                        owner.signature.as_deref().unwrap_or_default(),
                    );
                    let value_type = typescript_substitute_type_parameters(
                        value_type,
                        &owner_parameters,
                        &owner_type_arguments,
                    );
                    let member_module = typescript_source_module_keys(
                        &member.range.source_file,
                        &self.project.root,
                    )
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| module.to_owned());
                    indexed_contexts.extend(self.typescript_member_type_contexts(
                        language,
                        &member_module,
                        &value_type,
                        TypeScriptMemberContext {
                            owner_signature: Some(signature),
                            type_arguments: &[],
                            index_selector: None,
                        },
                        candidate,
                    ));
                } else if typescript_callable_parameter_types(signature).is_some()
                    || typescript_callable_return_type(signature).is_some()
                {
                    indexed_contexts.insert((member_slot, Vec::new()));
                }
            }
            return indexed_contexts;
        }
        if let Some((utility, arguments)) = typescript_generic_type_parts(&substituted)
            && matches!(utility, "Pick" | "Omit")
            && arguments.len() == 2
        {
            let base = arguments[0].trim();
            let keys = arguments[1].trim();
            if typescript_keyof_type_base(keys) == Some(base) {
                if utility == "Omit" {
                    return BTreeSet::new();
                }
                // `Pick<Base, keyof Base>` is an identity projection. Keep
                // this resolver branch deliberately narrow: literal Pick or
                // Omit sets need member-level projection metadata, while a
                // broad structural key expression must not widen a receiver.
                return self.typescript_member_type_contexts(
                    language,
                    module,
                    base,
                    TypeScriptMemberContext {
                        owner_signature: None,
                        type_arguments: &[],
                        index_selector: None,
                    },
                    candidate,
                );
            }
            if let Some(key_names) = typescript_literal_key_names(keys) {
                let selected = key_names.contains(candidate.target_spelling.as_str());
                if (utility == "Pick" && !selected) || (utility == "Omit" && selected) {
                    return BTreeSet::new();
                }
                // A literal projection is safe to resolve through its nominal
                // base: the current member candidate is the only downstream
                // property being adjudicated, and the key-set check above
                // prevents a projected-away member from inheriting the base.
                // This works across imported aliases while remaining bounded;
                // dynamic and structural key spaces still fail closed.
                return self.typescript_member_type_contexts(
                    language,
                    module,
                    base,
                    TypeScriptMemberContext {
                        owner_signature: None,
                        type_arguments: &[],
                        index_selector: None,
                    },
                    candidate,
                );
            }
        }
        let substituted = match context.index_selector {
            Some("") => match typescript_array_element_type(&substituted) {
                Some(element) => element.to_owned(),
                None => return BTreeSet::new(),
            },
            Some(index) => {
                if let Some(element) = typescript_array_element_type(&substituted) {
                    element
                } else {
                    match typescript_tuple_element_type(&substituted, index) {
                        Some(element) => element,
                        None => return BTreeSet::new(),
                    }
                }
            }
            None => substituted,
        };
        let (base, nested_type_arguments) = typescript_generic_type_parts(&substituted)
            .unwrap_or((substituted.as_str(), Vec::new()));
        let (qualified_module, exported) = split_typescript_module_qualified(base);
        if exported.is_empty() {
            return BTreeSet::new();
        }
        let mut modules = Vec::new();
        if let Some(qualified_module) = qualified_module {
            modules.push(qualified_module.to_owned());
            modules.extend(typescript_import_module_keys(
                module,
                qualified_module,
                &self.project.root,
            ));
        } else {
            modules.push(module.to_owned());
        }
        modules.sort_unstable();
        modules.dedup();
        let mut contexts = BTreeSet::new();
        let exported_name = exported
            .rsplit_once('.')
            .map_or(exported, |(_, name)| name)
            .to_owned();
        for module in modules {
            for slot in
                self.typescript_export_slots(language, &module, &exported_name, candidate, true)
            {
                if exported.contains('.')
                    && self
                        .declaration(slot)
                        .is_none_or(|declaration| declaration.qualified_name != exported)
                {
                    continue;
                }
                contexts.insert((slot, nested_type_arguments.clone()));
            }
            if contexts.len() > self.budget.candidates_per_lookup() {
                break;
            }
        }
        if qualified_module.is_none() && !exported.contains('.') {
            let mut imported_contexts = BTreeSet::new();
            for binding in self.facts.bindings.values().filter(|binding| {
                binding.language == language
                    && binding.spelling == exported
                    && matches!(
                        binding.kind,
                        compass_languages::BindingKind::Import
                            | compass_languages::BindingKind::ImportAlias
                            | compass_languages::BindingKind::Reexport
                    )
                    && binding.namespace.is_none_or(|namespace| {
                        matches!(
                            namespace,
                            compass_languages::SymbolNamespace::Type
                                | compass_languages::SymbolNamespace::ValueAndType
                                | compass_languages::SymbolNamespace::Namespace
                        )
                    })
            }) {
                let Some(owner) = binding
                    .scope_id
                    .as_deref()
                    .and_then(|scope_id| self.facts.scopes.get(scope_id))
                    .and_then(|scope| scope.owner_declaration_id.as_deref())
                    .and_then(|declaration_id| self.facts.declarations.get(declaration_id))
                else {
                    continue;
                };
                let owner_modules =
                    typescript_source_module_keys(&owner.range.source_file, &self.project.root);
                let mut module_candidates = vec![module.to_owned()];
                module_candidates.extend(typescript_import_module_keys(
                    &owner.range.source_file,
                    module,
                    &self.project.root,
                ));
                if !module_candidates
                    .iter()
                    .any(|candidate_module| owner_modules.contains(candidate_module))
                {
                    continue;
                }
                let Some((target_module, target_export)) =
                    binding.qualified_target.rsplit_once("::")
                else {
                    continue;
                };
                let mut target_modules = vec![target_module.to_owned()];
                target_modules.extend(typescript_import_module_keys(
                    &owner.range.source_file,
                    target_module,
                    &self.project.root,
                ));
                target_modules.sort_unstable();
                target_modules.dedup();
                for target_module in target_modules {
                    for slot in self.typescript_export_slots(
                        language,
                        &target_module,
                        target_export,
                        candidate,
                        true,
                    ) {
                        imported_contexts.insert((slot, nested_type_arguments.clone()));
                    }
                }
            }
            if !imported_contexts.is_empty() {
                contexts = imported_contexts;
            }
        }
        contexts
    }
}
