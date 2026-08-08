//! Validated, phased construction of immutable resolution indexes.

use super::super::*;

impl UniversalResolutionIndex {
    pub fn new(
        batches: &[SemanticEvidenceBatch],
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory(batches, &[], Path::new("."), limits)
    }

    pub fn new_with_inventory(
        batches: &[SemanticEvidenceBatch],
        inventory_nodes: &[NodeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory_owned(batches.to_vec(), inventory_nodes, root, limits)
    }

    pub(crate) fn new_with_inventory_owned(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory_owned_impl(batches, inventory_nodes, &[], root, limits, true)
    }

    pub(crate) fn new_with_project_inventory_owned(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        project_edges: &[EdgeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory_owned_impl(
            batches,
            inventory_nodes,
            project_edges,
            root,
            limits,
            true,
        )
    }

    pub(crate) fn new_with_prevalidated_project_inventory_owned(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        project_edges: &[EdgeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory_owned_impl(
            batches,
            inventory_nodes,
            project_edges,
            root,
            limits,
            false,
        )
    }

    fn new_with_inventory_owned_impl(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        project_edges: &[EdgeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
        validate_batches: bool,
    ) -> Result<Self, String> {
        let mut profile_started = Instant::now();
        if validate_batches {
            batches.par_iter().try_for_each(|batch| {
                validate_evidence(batch, EvidenceLimits::default())
                    .map_err(|error| format!("invalid universal evidence: {error}"))
            })?;
        }

        // Check aggregate input sizes before reserving hash-map capacity. A
        // valid batch is bounded on its own, but an unbounded number of valid
        // batches could otherwise make the old aggregate reservation attempt
        // a very large allocation before the resolver's configured limits are
        // consulted. The scope table has no separate public limit; sharing the
        // candidate ceiling keeps the public limit shape stable while still
        // bounding every resolver-owned primary table.
        let capacities = batches.iter().try_fold(
            [0_usize; 5],
            |mut counts, batch| -> Result<[usize; 5], String> {
                for (index, (name, value)) in [
                    ("declarations", batch.declarations.len()),
                    ("occurrences", batch.occurrences.len()),
                    ("bindings", batch.bindings.len()),
                    ("candidates", batch.candidates.len()),
                    ("scopes", batch.scopes.len()),
                ]
                .into_iter()
                .enumerate()
                {
                    counts[index] = counts[index].checked_add(value).ok_or_else(|| {
                        format!("universal aggregate {name} count overflows usize")
                    })?;
                }
                Ok(counts)
            },
        )?;
        for (name, count, limit) in [
            ("declarations", capacities[0], limits.declarations),
            ("occurrences", capacities[1], limits.occurrences),
            ("bindings", capacities[2], limits.bindings),
            ("candidates", capacities[3], limits.candidates),
            ("scopes", capacities[4], limits.candidates),
        ] {
            if count > limit {
                return Err(format!(
                    "universal aggregate {name} count {count} exceeds limit {limit}"
                ));
            }
        }

        let go_module_path = read_go_module_path(root);
        // Reserve the checked aggregate fact counts before consuming the
        // batches. A corpus-scale evidence index otherwise grows each primary
        // map by repeated rehashes while the old and new tables overlap in
        // memory.
        let mut declarations = AHashMap::with_capacity(capacities[0]);
        let mut occurrences = AHashMap::with_capacity(capacities[1]);
        let mut bindings = AHashMap::with_capacity(capacities[2]);
        let mut candidates = AHashMap::with_capacity(capacities[3]);
        let mut scopes = AHashMap::with_capacity(capacities[4]);
        profile_internal("universal evidence validation", &mut profile_started);
        for batch in batches {
            for fact in batch.declarations {
                insert_unique(&mut declarations, fact.id.clone(), fact)?;
            }
            for fact in batch.occurrences {
                insert_unique(&mut occurrences, fact.id.clone(), fact)?;
            }
            for fact in batch.bindings {
                insert_unique(&mut bindings, fact.id.clone(), fact)?;
            }
            for fact in batch.candidates {
                insert_unique(&mut candidates, fact.id.clone(), fact)?;
            }
            for fact in batch.scopes {
                insert_unique(&mut scopes, fact.id.clone(), fact)?;
            }
        }
        profile_internal("universal fact collection", &mut profile_started);
        let rust_source_wildcard_targets = declarations
            .values()
            .filter(|declaration| {
                declaration.language == "rust"
                    && matches!(declaration.kind.as_str(), "file" | "module" | "enum")
            })
            .map(|declaration| declaration.qualified_name.clone())
            .collect::<AHashSet<_>>();
        let mut declaration_ids = declarations.keys().cloned().collect::<Vec<_>>();
        declaration_ids.sort_unstable();
        if u32::try_from(declaration_ids.len()).is_err() {
            return Err("universal declaration slot count exceeds u32".to_owned());
        }
        let definition_ranges = unique_definition_ranges(&declarations, &scopes);
        let (typescript_project_modules, typescript_project_metadata) =
            typescript_project_module_index(
                project_edges,
                inventory_nodes,
                root,
                limits.candidates,
                limits.candidates_per_lookup,
            )?;
        let (typescript_modules, typescript_export_aliases, typescript_reexport_targets) =
            typescript_module_indices(
                &declarations,
                &declaration_ids,
                &bindings,
                &scopes,
                root,
                &typescript_project_modules,
            );
        let typescript_exported_declarations = bindings
            .values()
            .filter(|binding| binding.kind == compass_languages::BindingKind::Reexport)
            .filter_map(|binding| binding.target_declaration_id.as_deref())
            .filter_map(|id| declaration_slot(&declaration_ids, id))
            .collect::<AHashSet<_>>();
        for (name, count, limit) in [
            ("declarations", declarations.len(), limits.declarations),
            ("bindings", bindings.len(), limits.bindings),
            ("occurrences", occurrences.len(), limits.occurrences),
            ("candidates", candidates.len(), limits.candidates),
        ] {
            if count > limit {
                return Err(format!(
                    "universal {name} count {count} exceeds limit {limit}"
                ));
            }
        }
        let (by_qualified, (by_module_name, (by_scope_name, by_source_directory_name))) =
            rayon::join(
                || {
                    let mut index = AHashMap::<(String, String), Vec<DeclarationSlot>>::new();
                    for declaration in declarations.values() {
                        let Some(slot) = declaration_slot(&declaration_ids, &declaration.id) else {
                            continue;
                        };
                        index
                            .entry((
                                declaration.language.clone(),
                                declaration.qualified_name.clone(),
                            ))
                            .or_default()
                            .push(slot);
                    }
                    sort_declaration_index(
                        &mut index,
                        &declaration_ids,
                        limits.candidates_per_lookup,
                    );
                    index
                },
                || {
                    let (by_module_name, (by_scope_name, by_source_directory_name)) = rayon::join(
                        || {
                            let mut index =
                                AHashMap::<(String, String, String), Vec<DeclarationSlot>>::new();
                            for declaration in declarations.values() {
                                // Go methods live in the receiver method set, not in
                                // the package block. Keeping them in the package-name
                                // index makes an unqualified call ambiguous whenever a
                                // package function shares the method name (for example,
                                // a method forwarding to its package-level helper).
                                if declaration.language == "go" && declaration.kind == "method" {
                                    continue;
                                }
                                let Some(slot) =
                                    declaration_slot(&declaration_ids, &declaration.id)
                                else {
                                    continue;
                                };
                                let Some(module) = declaration.module_or_package.as_ref() else {
                                    continue;
                                };
                                index
                                    .entry((
                                        declaration.language.clone(),
                                        module.clone(),
                                        declaration.name.clone(),
                                    ))
                                    .or_default()
                                    .push(slot);
                            }
                            sort_declaration_index(
                                &mut index,
                                &declaration_ids,
                                limits.candidates_per_lookup,
                            );
                            index
                        },
                        || {
                            let (by_scope_name, by_source_directory_name) = rayon::join(
                                || {
                                    let mut index = AHashMap::<
                                        (String, String, String),
                                        Vec<DeclarationSlot>,
                                    >::new();
                                    for declaration in declarations.values() {
                                        // Go methods are selected through a receiver or
                                        // method expression; they are not lexical names
                                        // in the package/file scope.
                                        if declaration.language == "go"
                                            && declaration.kind == "method"
                                        {
                                            continue;
                                        }
                                        let Some(slot) =
                                            declaration_slot(&declaration_ids, &declaration.id)
                                        else {
                                            continue;
                                        };
                                        let Some(scope) = declaration.scope_id.as_ref() else {
                                            continue;
                                        };
                                        index
                                            .entry((
                                                declaration.language.clone(),
                                                scope.clone(),
                                                declaration.name.clone(),
                                            ))
                                            .or_default()
                                            .push(slot);
                                    }
                                    sort_declaration_index(
                                        &mut index,
                                        &declaration_ids,
                                        limits.candidates_per_lookup,
                                    );
                                    index
                                },
                                || {
                                    let mut index = AHashMap::<
                                        (String, String, String),
                                        Vec<DeclarationSlot>,
                                    >::new();
                                    for declaration in declarations.values() {
                                        let Some(slot) =
                                            declaration_slot(&declaration_ids, &declaration.id)
                                        else {
                                            continue;
                                        };
                                        let Some(directory) =
                                            source_directory(&declaration.range.source_file, root)
                                        else {
                                            continue;
                                        };
                                        index
                                            .entry((
                                                declaration.language.clone(),
                                                directory,
                                                declaration.name.clone(),
                                            ))
                                            .or_default()
                                            .push(slot);
                                    }
                                    sort_declaration_index(
                                        &mut index,
                                        &declaration_ids,
                                        limits.candidates_per_lookup,
                                    );
                                    index
                                },
                            );
                            (by_scope_name, by_source_directory_name)
                        },
                    );
                    (by_module_name, (by_scope_name, by_source_directory_name))
                },
            );
        profile_internal("universal declaration indices", &mut profile_started);
        let mut inventory_by_qualified = AHashMap::<_, Vec<_>>::new();
        for node in inventory_nodes {
            if node.string("symbol_kind") != "file" || node.string("source_file").is_empty() {
                continue;
            }
            let language = node.string("language");
            let qualified = match language.as_str() {
                "python" => python_module_name(&node.string("source_file"), root),
                "go" => {
                    let package = node.string("package");
                    (!package.is_empty()).then_some(package)
                }
                _ => None,
            };
            if let Some(qualified) = qualified {
                inventory_by_qualified
                    .entry((language, qualified))
                    .or_default()
                    .push(node.id.clone());
            }
        }
        for values in inventory_by_qualified.values_mut() {
            values.sort_unstable();
            values.dedup();
            if values.len() > limits.candidates_per_lookup {
                values.truncate(candidate_storage_limit(limits.candidates_per_lookup));
            }
        }
        profile_internal("universal source inventory index", &mut profile_started);
        let (aliases, direct_bases) = rayon::join(
            || {
                let mut aliases = AHashMap::<_, Vec<_>>::new();
                for binding in bindings.values() {
                    let Some(owner) = binding
                        .scope_id
                        .as_deref()
                        .and_then(|id| scopes.get(id))
                        .and_then(|scope| scope.owner_declaration_id.as_deref())
                        .and_then(|id| declarations.get(id))
                    else {
                        continue;
                    };
                    for separator in [".", "::"] {
                        aliases
                            .entry((
                                binding.language.clone(),
                                format!("{}{separator}{}", owner.qualified_name, binding.spelling),
                            ))
                            .or_default()
                            .push(binding.qualified_target.clone());
                    }
                }
                for targets in aliases.values_mut() {
                    targets.sort_unstable();
                    targets.dedup();
                }
                aliases
            },
            || {
                let mut direct_bases = AHashMap::<(String, String), DirectBaseSet>::new();
                for candidate in candidates.values() {
                    let Some(owner) = declarations.get(&candidate.source_declaration_id) else {
                        continue;
                    };
                    let base_set_complete = match candidate.constraints.hierarchy.as_ref() {
                        Some(HierarchyConstraint::DirectBase { base_set_complete }) => {
                            *base_set_complete
                        }
                        None if candidate.language == "java"
                            && matches!(
                                candidate.relation,
                                CandidateRelation::Extends | CandidateRelation::Implements
                            ) =>
                        {
                            owner.direct_bases_complete
                        }
                        _ => continue,
                    };
                    let range = candidate
                        .occurrence_id
                        .as_deref()
                        .and_then(|id| occurrences.get(id))
                        .map(|occurrence| &occurrence.range);
                    let entry = direct_bases
                        .entry((candidate.language.clone(), owner.qualified_name.clone()))
                        .or_insert_with(|| DirectBaseSet {
                            links: Vec::new(),
                            complete: true,
                        });
                    entry.complete &= base_set_complete;
                    if entry.links.len() <= limits.candidates_per_lookup {
                        entry.links.push(DirectBaseLink {
                            qualified_name: candidate.constraints.qualified_name.clone(),
                            source_file: range
                                .map_or_else(String::new, |range| range.source_file.clone()),
                            start_byte: range.map_or(u64::MAX, |range| range.start_byte),
                            end_byte: range.map_or(u64::MAX, |range| range.end_byte),
                            candidate_id: candidate.id.clone(),
                        });
                    } else {
                        entry.complete = false;
                    }
                }
                for bases in direct_bases.values_mut() {
                    bases.links.sort_unstable_by(|left, right| {
                        left.source_file
                            .cmp(&right.source_file)
                            .then_with(|| left.start_byte.cmp(&right.start_byte))
                            .then_with(|| left.end_byte.cmp(&right.end_byte))
                            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
                    });
                    if bases.links.len() > limits.candidates_per_lookup {
                        bases.complete = false;
                        bases.links.truncate(limits.candidates_per_lookup);
                    }
                }
                direct_bases
            },
        );
        profile_internal(
            "universal alias and hierarchy indices",
            &mut profile_started,
        );
        let mut direct_subtypes = AHashMap::<(String, String), DirectSubtypeSet>::new();
        for ((language, subtype), bases) in &direct_bases {
            for link in &bases.links {
                let Some(base) = link.qualified_name.as_ref() else {
                    continue;
                };
                let entry = direct_subtypes
                    .entry((language.clone(), base.clone()))
                    .or_insert_with(|| DirectSubtypeSet {
                        types: Vec::new(),
                        complete: true,
                    });
                if entry.types.len() <= limits.candidates_per_lookup {
                    entry.types.push(subtype.clone());
                } else {
                    entry.complete = false;
                }
            }
        }
        for subtypes in direct_subtypes.values_mut() {
            subtypes.types.sort_unstable();
            subtypes.types.dedup();
            if subtypes.types.len() > limits.candidates_per_lookup {
                subtypes.complete = false;
                subtypes.types.truncate(limits.candidates_per_lookup);
            }
        }
        let mut members_by_owner =
            AHashMap::<(String, String, String), Vec<DeclarationSlot>>::new();
        for declaration in declarations.values() {
            let Some(slot) = declaration_slot(&declaration_ids, &declaration.id) else {
                continue;
            };
            let Some(owner) = declaration
                .scope_id
                .as_deref()
                .and_then(|id| scopes.get(id))
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            else {
                continue;
            };
            members_by_owner
                .entry((
                    declaration.language.clone(),
                    owner.qualified_name.clone(),
                    declaration.name.clone(),
                ))
                .or_default()
                .push(slot);
        }
        // Object-literal members are deliberately collected in their lexical
        // binding scope by the TypeScript adapter: unlike a class/interface,
        // an object value has no syntax-level scope owner.  Recover the
        // source-qualified object variable as an additional owner so imported
        // chains such as `api.make(value).inspect()` can traverse the
        // callable property without widening ordinary lexical members.
        let mut structural_object_owners =
            AHashMap::<(String, String), Vec<DeclarationSlot>>::new();
        for declaration in declarations.values().filter(|declaration| {
            declaration.kind == "variable"
                && matches!(declaration.language.as_str(), "typescript" | "javascript")
        }) {
            let Some(slot) = declaration_slot(&declaration_ids, &declaration.id) else {
                continue;
            };
            structural_object_owners
                .entry((
                    declaration.language.clone(),
                    declaration.qualified_name.clone(),
                ))
                .or_default()
                .push(slot);
        }
        for declaration in declarations.values().filter(|declaration| {
            matches!(declaration.language.as_str(), "typescript" | "javascript")
                && matches!(
                    declaration.kind.as_str(),
                    "property" | "method" | "field" | "constructor"
                )
        }) {
            let Some((owner_name, _)) = declaration.qualified_name.rsplit_once('.') else {
                continue;
            };
            let Some(owner_slots) = structural_object_owners
                .get(&(declaration.language.clone(), owner_name.to_owned()))
            else {
                continue;
            };
            let Some(slot) = declaration_slot(&declaration_ids, &declaration.id) else {
                continue;
            };
            for owner_slot in owner_slots {
                let Some(owner_id) = declaration_ids.get(*owner_slot as usize) else {
                    continue;
                };
                let Some(owner) = declarations.get(owner_id) else {
                    continue;
                };
                if owner.scope_id != declaration.scope_id {
                    continue;
                }
                members_by_owner
                    .entry((
                        declaration.language.clone(),
                        owner_name.to_owned(),
                        declaration.name.clone(),
                    ))
                    .or_default()
                    .push(slot);
            }
        }
        for members in members_by_owner.values_mut() {
            members.sort_unstable_by(|left, right| {
                declaration_ids[*left as usize].cmp(&declaration_ids[*right as usize])
            });
            members.dedup();
            if members.len() > limits.candidates_per_lookup {
                members.truncate(candidate_storage_limit(limits.candidates_per_lookup));
            }
        }
        let rust_impl_associated_types = rust_impl_associated_type_index(
            &declarations,
            &declaration_ids,
            &scopes,
            &candidates,
            &occurrences,
            limits.candidates_per_lookup,
        );
        let rust_impl_associated_trait_names = rust_impl_associated_trait_name_index(
            &rust_impl_associated_types,
            limits.candidates_per_lookup,
        );
        let rust_impl_traits = rust_impl_trait_index(&candidates, limits.candidates_per_lookup);
        profile_internal("universal hierarchy indices", &mut profile_started);
        let mut wildcard_bindings_by_scope = AHashMap::<(String, String), WildcardModuleSet>::new();
        let mut wildcard_bindings_by_module =
            AHashMap::<(String, String), WildcardModuleSet>::new();
        let mut wildcard_reexports_by_module =
            AHashMap::<(String, String), WildcardModuleSet>::new();
        for binding in bindings.values().filter(|binding| binding.spelling == "*") {
            let Some(scope_id) = binding.scope_id.as_ref() else {
                continue;
            };
            let entry = wildcard_bindings_by_scope
                .entry((binding.language.clone(), scope_id.clone()))
                .or_insert_with(|| WildcardModuleSet {
                    modules: Vec::new(),
                    complete: true,
                });
            let target_is_internal = scopes
                .get(scope_id)
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
                .is_some_and(|owner| {
                    qualified_root(&owner.qualified_name)
                        == qualified_root(&binding.qualified_target)
                })
                || (binding.language == "rust"
                    && rust_source_wildcard_targets.contains(&binding.qualified_target));
            entry.complete &= target_is_internal;
            entry.modules.push(binding.qualified_target.clone());
            if let Some(owner) = scopes
                .get(scope_id)
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            {
                let module_entry = wildcard_bindings_by_module
                    .entry((binding.language.clone(), owner.qualified_name.clone()))
                    .or_insert_with(|| WildcardModuleSet {
                        modules: Vec::new(),
                        complete: true,
                    });
                module_entry.complete &= target_is_internal;
                module_entry.modules.push(binding.qualified_target.clone());
            }
            if binding.kind == compass_languages::BindingKind::Reexport
                && let Some(owner) = scopes
                    .get(scope_id)
                    .and_then(|scope| scope.owner_declaration_id.as_deref())
                    .and_then(|id| declarations.get(id))
            {
                wildcard_reexports_by_module
                    .entry((binding.language.clone(), owner.qualified_name.clone()))
                    .or_insert_with(|| WildcardModuleSet {
                        modules: Vec::new(),
                        complete: true,
                    })
                    .modules
                    .push(binding.qualified_target.clone());
            }
        }
        for bindings in wildcard_bindings_by_scope.values_mut() {
            bindings.modules.sort_unstable();
            bindings.modules.dedup();
            if bindings.modules.len() > limits.candidates_per_lookup {
                bindings.complete = false;
                bindings.modules.truncate(limits.candidates_per_lookup);
            }
        }
        for bindings in wildcard_bindings_by_module.values_mut() {
            bindings.modules.sort_unstable();
            bindings.modules.dedup();
            if bindings.modules.len() > limits.candidates_per_lookup {
                bindings.complete = false;
                bindings.modules.truncate(limits.candidates_per_lookup);
            }
        }
        for reexports in wildcard_reexports_by_module.values_mut() {
            reexports.modules.sort_unstable();
            reexports.modules.dedup();
            if reexports.modules.len() > limits.candidates_per_lookup {
                reexports.complete = false;
                reexports.modules.truncate(limits.candidates_per_lookup);
            }
        }
        profile_internal("universal wildcard index", &mut profile_started);
        let mut members = AHashMap::<_, Vec<_>>::new();
        for binding in bindings
            .values()
            .filter(|binding| binding.kind == compass_languages::BindingKind::Member)
        {
            let Some(owner) = binding
                .scope_id
                .as_deref()
                .and_then(|id| scopes.get(id))
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            else {
                continue;
            };
            members
                .entry((
                    binding.language.clone(),
                    owner.qualified_name.clone(),
                    binding.spelling.clone(),
                ))
                .or_default()
                .push(binding.qualified_target.clone());
        }
        for targets in members.values_mut() {
            targets.sort_unstable();
            targets.dedup();
        }
        let mut typescript_member_aliases =
            AHashMap::<(String, String), Vec<TypeScriptMemberAlias>>::new();
        for binding in bindings.values().filter(|binding| {
            binding.kind == compass_languages::BindingKind::Member
                && binding.spelling == "*"
                && matches!(binding.language.as_str(), "typescript" | "javascript")
        }) {
            let Some(owner) = binding
                .scope_id
                .as_deref()
                .and_then(|id| scopes.get(id))
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            else {
                continue;
            };
            typescript_member_aliases
                .entry((binding.language.clone(), owner.qualified_name.clone()))
                .or_default()
                .push(TypeScriptMemberAlias {
                    source: binding.qualified_target.clone(),
                    source_slot: binding
                        .target_declaration_id
                        .as_deref()
                        .and_then(|id| declaration_slot(&declaration_ids, id)),
                    start_byte: binding.range.start_byte,
                });
        }
        for aliases in typescript_member_aliases.values_mut() {
            aliases.sort_unstable_by(|left, right| {
                left.start_byte
                    .cmp(&right.start_byte)
                    .then_with(|| left.source.cmp(&right.source))
                    .then_with(|| left.source_slot.cmp(&right.source_slot))
            });
            aliases.dedup_by(|left, right| {
                left.source == right.source
                    && left.source_slot == right.source_slot
                    && left.start_byte == right.start_byte
            });
            if aliases.len() > limits.candidates_per_lookup {
                aliases.truncate(candidate_storage_limit(limits.candidates_per_lookup));
            }
        }
        let mut return_entries = AHashMap::<_, Vec<_>>::new();
        let mut outer_return_entries = AHashMap::<_, Vec<_>>::new();
        for candidate in candidates
            .values()
            .filter(|candidate| candidate.relation == CandidateRelation::Returns)
        {
            let Some(callable) = declarations.get(&candidate.source_declaration_id) else {
                continue;
            };
            if candidate.constraints.qualified_name.is_none() {
                continue;
            }
            let start_byte = candidate
                .occurrence_id
                .as_deref()
                .and_then(|id| occurrences.get(id))
                .map_or(u64::MAX, |occurrence| occurrence.range.start_byte);
            return_entries
                .entry((candidate.language.clone(), callable.qualified_name.clone()))
                .or_default()
                .push((start_byte, candidate.id.clone()));
            if candidate
                .occurrence_id
                .as_deref()
                .and_then(|id| occurrences.get(id))
                .and_then(|occurrence| occurrence.context.as_deref())
                == Some("rust-outer-nominal-return")
            {
                outer_return_entries
                    .entry((candidate.language.clone(), callable.qualified_name.clone()))
                    .or_default()
                    .push((start_byte, candidate.id.clone()));
            }
        }
        let return_candidates_by_callable = return_entries
            .into_iter()
            .map(|(key, mut entries)| {
                entries.sort_unstable();
                entries.truncate(candidate_storage_limit(limits.candidates_per_lookup));
                (
                    key,
                    entries
                        .into_iter()
                        .map(|(_, candidate)| candidate)
                        .collect(),
                )
            })
            .collect();
        let outer_return_candidates_by_callable = outer_return_entries
            .into_iter()
            .map(|(key, mut entries)| {
                entries.sort_unstable();
                entries.truncate(limits.candidates_per_lookup);
                (
                    key,
                    entries
                        .into_iter()
                        .map(|(_, candidate)| candidate)
                        .collect(),
                )
            })
            .collect();
        profile_internal("universal member index", &mut profile_started);
        Ok(Self {
            facts: FactStore {
                declarations,
                declaration_ids,
                occurrences,
                bindings,
                candidates,
                scopes,
                definition_ranges,
            },
            indexes: ResolutionIndexes {
                by_qualified,
                by_module_name,
                by_scope_name,
                by_source_directory_name,
                typescript_modules,
                typescript_exported_declarations,
                typescript_export_aliases,
                typescript_reexport_targets,
                direct_bases,
                direct_subtypes,
                members_by_owner,
                rust_impl_associated_types,
                rust_impl_associated_trait_names,
                rust_impl_traits,
                inventory_by_qualified,
                aliases,
                rust_source_wildcard_targets,
                wildcard_bindings_by_scope,
                wildcard_bindings_by_module,
                wildcard_reexports_by_module,
                members,
                typescript_member_aliases,
                return_candidates_by_callable,
                outer_return_candidates_by_callable,
            },
            project: ProjectContext {
                root: root.to_path_buf(),
                typescript_project_modules,
                typescript_project_metadata,
                go_module_path,
            },
            budget: LookupBudget::from(limits),
        })
    }
}
