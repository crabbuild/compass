//! Project-aware imports, exports, aliases, and re-export traversal.

use super::*;

impl ResolutionDb<'_> {
    pub(in crate::evidence) fn resolve_typescript_import_candidate(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if !matches!(language, "typescript" | "javascript") {
            return None;
        }
        let occurrence = self.occurrence(candidate)?;
        let mut module_and_export = None;
        if let Some(binding) = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.facts.bindings.get(binding_id))
            && matches!(
                binding.kind,
                compass_languages::BindingKind::Import
                    | compass_languages::BindingKind::ImportAlias
                    | compass_languages::BindingKind::Reexport
            )
            && let Some((module, exported)) = binding.qualified_target.rsplit_once("::")
            && !module.is_empty()
            && !exported.is_empty()
        {
            module_and_export = Some((module.to_owned(), exported.to_owned()));
        } else if matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) && let Some(qualified) = candidate.constraints.qualified_name.as_deref()
            && let Some((module, exported)) = qualified.rsplit_once("::")
            && !module.is_empty()
            && !exported.is_empty()
        {
            module_and_export = Some((module.to_owned(), exported.to_owned()));
        }

        let (module, exported) = if let Some(module_and_export) = module_and_export {
            module_and_export
        } else if matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) {
            let module = candidate
                .constraints
                .module_or_package
                .as_deref()
                .filter(|module| !module.is_empty())?
                .to_owned();
            (module, "module".to_owned())
        } else {
            return None;
        };
        let (keys, project_resolved) = self
            .typescript_project_module_keys(
                &occurrence.range.source_file,
                &module,
                if matches!(
                    candidate.relation,
                    CandidateRelation::Imports | CandidateRelation::Reexports
                ) {
                    occurrence.context.as_deref()
                } else {
                    None
                },
            )
            .map_or_else(
                || {
                    (
                        typescript_import_module_keys(
                            &occurrence.range.source_file,
                            &module,
                            &self.project.root,
                        ),
                        false,
                    )
                },
                |keys| (keys, true),
            );
        if keys.is_empty() {
            return None;
        }
        let member_path = if matches!(
            candidate.relation,
            CandidateRelation::Calls
                | CandidateRelation::IndirectCalls
                | CandidateRelation::AccessesMember
        ) {
            candidate
                .constraints
                .qualified_name
                .as_deref()
                .and_then(|qualified| typescript_member_path(qualified, &candidate.target_spelling))
        } else {
            None
        };
        let member_owner_export = member_path
            .as_ref()
            .filter(|path| path.members.len() == 1)
            .map(|path| path.root_export.clone());
        // A namespace import (`import * as api` or `const api =
        // require("./api")`) carries the module target as `module::*`, while
        // the source-grounded member occurrence is represented as
        // `module::member`.  It is a direct export lookup, not a nominal
        // owner/member lookup; preserve that distinction so an exact provider
        // re-export can resolve without manufacturing a qualified external.
        let namespace_member_export = (exported == "*")
            .then(|| {
                candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .and_then(|qualified| {
                        let (qualified_module, path) = split_typescript_module_qualified(qualified);
                        let segments = split_typescript_member_segments(path);
                        let [member] = segments.as_slice() else {
                            return None;
                        };
                        (qualified_module == Some(module.as_str())
                            && member == &candidate.target_spelling)
                            .then(|| member.clone())
                    })
            })
            .flatten();
        let commonjs_namespace_call = exported == "*"
            && candidate.relation == CandidateRelation::Calls
            && namespace_member_export.is_none();
        let is_member_candidate = member_path.is_some() || namespace_member_export.is_some();
        let mut source_member_target_seen = false;
        let mut structural_alias_seen = false;
        let mut targets = BTreeSet::new();
        for key in keys {
            let expand_member_chain = member_path.as_ref().is_some_and(|path| {
                path.call_result
                    || path.indexed
                    || path.members.len() > 1
                    || !path.type_arguments.is_empty()
                    || self
                        .typescript_export_slots(language, &key, &path.root_export, candidate, true)
                        .iter()
                        .any(|slot| {
                            self.declaration(*slot)
                                .is_some_and(|declaration| declaration.kind == "type_alias")
                        })
            });
            if expand_member_chain {
                let Some(path) = member_path.as_ref() else {
                    continue;
                };
                source_member_target_seen |= !self
                    .typescript_export_slots(language, &key, &path.root_export, candidate, true)
                    .is_empty();
                targets.extend(self.typescript_member_chain_slots(language, &key, path, candidate));
                continue;
            }
            let owner_export = namespace_member_export
                .as_deref()
                .or(member_owner_export.as_deref())
                .unwrap_or(&exported)
                .to_owned();
            // A member candidate constrains the final target to
            // callable/value kinds, but the exported owner can be a
            // type-only declaration such as an interface. Widen only this
            // internal owner lookup; member filtering below still uses the
            // original candidate, so an interface itself is never published
            // as the callable/access target.
            let namespace_alias_targets = namespace_member_export.as_ref().map(|_| {
                self.typescript_export_alias_slots(
                    language,
                    &key,
                    &owner_export,
                    candidate,
                    member_owner_export.is_some(),
                )
            });
            let mut owner_targets = namespace_alias_targets
                .filter(|targets| !targets.is_empty())
                .unwrap_or_else(|| {
                    self.typescript_export_slots(
                        language,
                        &key,
                        &owner_export,
                        candidate,
                        member_owner_export.is_some(),
                    )
                });
            // A CommonJS namespace can inherit a member through a proven
            // object spread without publishing that inherited name as a
            // direct reexport. If the direct export lookup is empty, recover
            // the module owner itself so the bounded `Member("*")` alias can
            // project the source member. Direct named reexports still win
            // because this fallback only runs for an empty owner set.
            let mut structural_namespace_owner = false;
            if owner_targets.is_empty() && namespace_member_export.is_some() {
                let module_targets =
                    self.typescript_export_slots(language, &key, "*", candidate, true);
                if !module_targets.is_empty() {
                    owner_targets = module_targets;
                    structural_namespace_owner = true;
                }
            }
            if owner_targets.is_empty() && commonjs_namespace_call {
                // A direct CommonJS require can return a callable
                // `module.exports = fn`.  The namespace binding is retained
                // for member access, but a direct call may resolve through the
                // provider's source-grounded default export. Object-valued
                // defaults remain filtered by the candidate's callable kinds.
                owner_targets =
                    self.typescript_export_slots(language, &key, "default", candidate, true);
            }
            if owner_targets.is_empty()
                && !is_member_candidate
                && exported != "*"
                && exported != "default"
            {
                // Project a named import through a bounded CommonJS barrel
                // owner alias when no direct export slot exists.
                let module_targets =
                    self.typescript_export_slots(language, &key, "*", candidate, true);
                for owner_slot in module_targets {
                    let Some(owner) = self.declaration(owner_slot) else {
                        continue;
                    };
                    if self
                        .indexes
                        .typescript
                        .member_aliases
                        .contains_key(&(owner.language.clone(), owner.qualified_name.clone()))
                    {
                        structural_alias_seen = true;
                        if let Ok(Some(alias_members)) = self.typescript_structural_member_slots(
                            owner_slot,
                            &exported,
                            candidate,
                            &mut BTreeSet::new(),
                        ) {
                            targets.extend(alias_members);
                        }
                    }
                }
            }
            source_member_target_seen |= !owner_targets.is_empty();
            if member_owner_export.is_some() || structural_namespace_owner {
                for owner_slot in &owner_targets {
                    let Some(owner) = self.declaration(*owner_slot) else {
                        continue;
                    };
                    if member_owner_export.is_some()
                        && let Some(members) = self.indexes.members.members_by_owner.get(&(
                            owner.language.clone(),
                            owner.qualified_name.clone(),
                            candidate.target_spelling.clone(),
                        ))
                    {
                        let members = members.iter().copied().collect::<BTreeSet<_>>();
                        let members = if candidate.relation == CandidateRelation::Calls {
                            let argument_types = typescript_candidate_argument_types(candidate);
                            self.typescript_callable_overload_slots(&members, &argument_types, &[])
                        } else {
                            members
                        };
                        targets.extend(
                            members
                                .iter()
                                .filter(|slot| {
                                    self.typescript_declaration_allowed_slot(**slot, candidate)
                                })
                                .copied(),
                        );
                    }
                    if self
                        .indexes
                        .typescript
                        .member_aliases
                        .contains_key(&(owner.language.clone(), owner.qualified_name.clone()))
                    {
                        structural_alias_seen = true;
                        if let Ok(Some(alias_members)) = self.typescript_structural_member_slots(
                            *owner_slot,
                            &candidate.target_spelling,
                            candidate,
                            &mut BTreeSet::new(),
                        ) {
                            targets.extend(alias_members);
                        }
                    }
                }
                // A value import can be declared with a nominal object type
                // (`declare const api: Api`).  Its callable members live on
                // the interface/type declaration rather than on the value
                // declaration itself; expand that direct type evidence before
                // falling back to an external target.
                if targets.is_empty()
                    && let Some(path) = member_path.as_ref()
                    && path.members.len() == 1
                {
                    for owner_slot in &owner_targets {
                        for (typed_owner, _) in self.typescript_value_type_contexts(
                            language,
                            &key,
                            *owner_slot,
                            &path.type_arguments,
                            candidate,
                        ) {
                            let Some(typed_owner) = self.declaration(typed_owner) else {
                                continue;
                            };
                            if let Some(members) = self.indexes.members.members_by_owner.get(&(
                                language.to_owned(),
                                typed_owner.qualified_name.clone(),
                                candidate.target_spelling.clone(),
                            )) {
                                targets.extend(
                                    members
                                        .iter()
                                        .filter(|slot| {
                                            self.typescript_declaration_allowed_slot(
                                                **slot, candidate,
                                            )
                                        })
                                        .copied(),
                                );
                            }
                        }
                    }
                }
            } else {
                targets.extend(owner_targets);
            }
        }
        let targets = targets.into_iter().collect::<Vec<_>>();
        if is_member_candidate && structural_alias_seen && targets.is_empty() {
            return Some(ResolutionDecision::Unresolved);
        }
        if !is_member_candidate && structural_alias_seen && targets.is_empty() {
            return Some(ResolutionDecision::Unresolved);
        }
        if is_member_candidate
            && candidate.relation == CandidateRelation::Calls
            && targets.is_empty()
            && source_member_target_seen
        {
            return Some(ResolutionDecision::Unresolved);
        }
        self.unique_typescript_decision(
            Some(&targets),
            candidate,
            if is_member_candidate {
                ResolutionRule::MemberBinding
            } else if project_resolved {
                ResolutionRule::ProjectModuleBinding
            } else {
                ResolutionRule::ExplicitBinding
            },
        )
    }

    /// Resolve one member through a source-proven object-owner alias. Direct
    /// members always win; inherited aliases are followed only when the
    /// source owner is itself a bounded object owner. Multiple distinct
    /// source declarations remain ambiguous instead of relying on hash or
    /// traversal order.
    pub(in crate::evidence) fn typescript_export_slots(
        &self,
        language: &str,
        module: &str,
        exported: &str,
        candidate: &RelationshipCandidate,
        allow_type_owner: bool,
    ) -> BTreeSet<DeclarationSlot> {
        const MAX_TYPESCRIPT_REEXPORT_DEPTH: usize = 64;
        let mut walk = TypeScriptExportWalk {
            candidate,
            allow_type_owner,
            visiting: BTreeSet::new(),
            slots: BTreeSet::new(),
        };
        self.collect_typescript_export_slots(
            language,
            module,
            exported,
            0,
            MAX_TYPESCRIPT_REEXPORT_DEPTH,
            &mut walk,
        );
        walk.slots
    }

    pub(in crate::evidence) fn typescript_export_alias_slots(
        &self,
        language: &str,
        module: &str,
        exported: &str,
        candidate: &RelationshipCandidate,
        allow_type_owner: bool,
    ) -> BTreeSet<DeclarationSlot> {
        let mut slots = BTreeSet::new();
        for target_language in typescript_language_family(language) {
            if let Some(values) = self.indexes.typescript.export_aliases.get(&(
                (*target_language).to_owned(),
                module.to_owned(),
                exported.to_owned(),
            )) {
                slots.extend(
                    values
                        .iter()
                        .filter(|slot| {
                            if allow_type_owner {
                                self.typescript_declaration_allowed_owner_slot(**slot, candidate)
                            } else {
                                self.typescript_declaration_allowed_slot(**slot, candidate)
                            }
                        })
                        .copied(),
                );
            }
        }
        slots
    }

    fn collect_typescript_export_slots(
        &self,
        language: &str,
        module: &str,
        exported: &str,
        depth: usize,
        max_depth: usize,
        walk: &mut TypeScriptExportWalk<'_>,
    ) {
        if depth >= max_depth
            || walk.slots.len() > self.budget.candidates_per_lookup()
            || !walk
                .visiting
                .insert((language.to_owned(), module.to_owned(), exported.to_owned()))
        {
            return;
        }
        for target_language in typescript_language_family(language) {
            let target_language = *target_language;
            // An export-star namespace alias carries a reexport target whose
            // exported spelling is a wildcard. Treat that spelling as the
            // provider module owner so \`ns.member\` can use the same exact
            // member index as a namespace import. Ordinary wildcard barrel
            // traversal still requests the concrete exported name below.
            let direct_export = if exported == "*" { "module" } else { exported };
            for (direct_module_index, index) in [
                (true, &self.indexes.typescript.modules),
                (false, &self.indexes.typescript.export_aliases),
            ] {
                if let Some(values) = index.get(&(
                    target_language.to_owned(),
                    module.to_owned(),
                    direct_export.to_owned(),
                )) {
                    let remaining = candidate_storage_limit(self.budget.candidates_per_lookup())
                        .saturating_sub(walk.slots.len());
                    if remaining > 0 {
                        walk.slots.extend(
                            values
                                .iter()
                                .filter(|slot| {
                                    if direct_module_index
                                        && !walk.allow_type_owner
                                        && !self
                                            .indexes
                                            .typescript
                                            .exported_declarations
                                            .contains(slot)
                                        && self
                                            .declaration(**slot)
                                            .is_none_or(|declaration| declaration.kind != "module")
                                    {
                                        return false;
                                    }
                                    if walk.allow_type_owner {
                                        (exported == "*"
                                            && self.declaration(**slot).is_some_and(
                                                |declaration| declaration.kind == "module",
                                            ))
                                            || self.typescript_declaration_allowed_owner_slot(
                                                **slot,
                                                walk.candidate,
                                            )
                                    } else {
                                        self.typescript_declaration_allowed_slot(
                                            **slot,
                                            walk.candidate,
                                        )
                                    }
                                })
                                .take(remaining)
                                .copied(),
                        );
                    }
                }
            }
            if exported != "default"
                && let Some(values) = self.indexes.typescript.reexport_targets.get(&(
                    target_language.to_owned(),
                    module.to_owned(),
                    "*".to_owned(),
                ))
            {
                for target in values {
                    self.collect_typescript_export_slots(
                        target_language,
                        &target.module,
                        exported,
                        depth.saturating_add(1),
                        max_depth,
                        walk,
                    );
                }
            }
            if let Some(values) = self.indexes.typescript.reexport_targets.get(&(
                target_language.to_owned(),
                module.to_owned(),
                exported.to_owned(),
            )) {
                for target in values {
                    self.collect_typescript_export_slots(
                        target_language,
                        &target.module,
                        &target.exported,
                        depth.saturating_add(1),
                        max_depth,
                        walk,
                    );
                }
            }
        }
        walk.visiting
            .remove(&(language.to_owned(), module.to_owned(), exported.to_owned()));
    }
}
