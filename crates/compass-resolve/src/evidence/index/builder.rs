//! Validated, phased construction of immutable resolution indexes.

use super::super::budget::{AuxiliaryLimits, MAX_AGGREGATE_MERGE_DEPENDENCIES};
use super::super::*;
use super::{
    CSharpBinding, CSharpIndexes, HierarchyIndexes, JavaIndexes, MemberIndexes, NameIndexes,
    PhpIndexes, RustIndexes, TypeScriptIndexes, WildcardIndexes,
};
use crate::ResolutionAdmission;

#[derive(Clone, Copy, Debug, Default)]
struct LanguagePresence {
    python: bool,
    typescript: bool,
    rust: bool,
    go: bool,
    csharp: bool,
    php: bool,
}

impl LanguagePresence {
    fn detect(batches: &[SemanticEvidenceBatch], inventory_nodes: &[NodeRecord]) -> Self {
        let mut presence = Self::default();
        for language in batches.iter().map(|batch| batch.pipeline.language.as_str()) {
            presence.observe(language);
        }
        for node in inventory_nodes {
            presence.observe(&node.string("language"));
        }
        presence
    }

    fn observe(&mut self, language: &str) {
        match language {
            "python" => self.python = true,
            "javascript" | "javascriptreact" | "typescript" | "typescriptreact" => {
                self.typescript = true;
            }
            "rust" => self.rust = true,
            "go" => self.go = true,
            "csharp" => self.csharp = true,
            "php" => self.php = true,
            _ => {}
        }
    }
}

struct IndexBuilder<'a> {
    batches: Vec<SemanticEvidenceBatch>,
    inventory_nodes: &'a [NodeRecord],
    project_edges: &'a [EdgeRecord],
    root: &'a Path,
    limits: UniversalResolutionLimits,
    validate_batches: bool,
    admission: ResolutionAdmission,
    auxiliary_limits: AuxiliaryLimits,
}

impl<'a> IndexBuilder<'a> {
    fn new(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &'a [NodeRecord],
        project_edges: &'a [EdgeRecord],
        root: &'a Path,
        limits: UniversalResolutionLimits,
        validate_batches: bool,
        admission: ResolutionAdmission,
    ) -> Self {
        Self {
            batches,
            inventory_nodes,
            project_edges,
            root,
            limits,
            validate_batches,
            admission,
            auxiliary_limits: AuxiliaryLimits::default(),
        }
    }

    fn with_auxiliary_limits(mut self, limits: AuxiliaryLimits) -> Self {
        self.auxiliary_limits = limits;
        self
    }
}

const MAX_AGGREGATE_DOCUMENTATION_BYTES: usize = 128 * 1024 * 1024;

fn typescript_inventory_sources_are_present(
    batches: &[SemanticEvidenceBatch],
    inventory_nodes: &[NodeRecord],
    root: &Path,
    limit: usize,
    lookup_limit: usize,
    project_edges: &[EdgeRecord],
) -> Result<bool, String> {
    use compass_languages::TypeScriptMergeDependencyKind as Kind;
    if inventory_nodes.len() > limit || project_edges.len() > limit {
        return Err(format!(
            "universal source-load inventory node or edge count exceeds limit {limit}"
        ));
    }
    let mut sources = BTreeSet::new();
    for batch in batches
        .iter()
        .filter(|batch| batch.pipeline.language == "typescript")
    {
        let mut file_present = false;
        for declaration in batch.declarations.iter().filter(|fact| {
            matches!(fact.kind.as_str(), "file" | "module") && fact.scope_id.is_none()
        }) {
            let Some(source) =
                typescript_project_importer_key(&declaration.range.source_file, root)
            else {
                return Ok(false);
            };
            sources.insert(source);
            file_present = true;
        }
        if !file_present {
            return Ok(false);
        }
    }
    // Borrow inventory strings. Duplicate graph IDs with different sources are
    // explicitly unknown; they cannot be rescued with an edge spelling.
    let mut node_sources = BTreeMap::<&str, Option<&str>>::new();
    for node in inventory_nodes {
        let source = node
            .attributes
            .get("universal_evidence_source_file")
            .and_then(Value::as_str)
            .filter(|source| !source.is_empty())
            .or_else(|| node.attributes.get("source_file").and_then(Value::as_str));
        if let Some(source) = source.filter(|source| !source.is_empty()) {
            node_sources
                .entry(&node.id)
                .and_modify(|previous| {
                    if *previous != Some(source) {
                        *previous = None;
                    }
                })
                .or_insert(Some(source));
        }
        let typescript = node
            .attributes
            .get("language")
            .and_then(Value::as_str)
            .is_some_and(|language| matches!(language, "typescript" | "tsx" | "typescriptreact"))
            || source.is_some_and(|source| {
                [".ts", ".tsx", ".mts", ".cts"]
                    .iter()
                    .any(|suffix| source.ends_with(suffix))
            });
        if typescript
            && source
                .and_then(|source| typescript_project_importer_key(source, root))
                .is_none_or(|source| !sources.contains(&source))
        {
            return Ok(false);
        }
    }
    let dependencies = batches
        .iter()
        .flat_map(|batch| &batch.typescript_merge_dependencies)
        .collect::<Vec<_>>();
    let mut needed = BTreeMap::<
        String,
        BTreeMap<&str, Vec<(usize, &compass_languages::TypeScriptMergeDependency)>>,
    >::new();
    for (slot, dependency) in dependencies
        .iter()
        .enumerate()
        .filter(|(_, dependency)| dependency.kind != Kind::ReferencePath)
    {
        let Some(source) = typescript_project_importer_key(&dependency.range.source_file, root)
        else {
            return Ok(false);
        };
        let obligations = needed
            .entry(source)
            .or_default()
            .entry(&dependency.specifier)
            .or_default();
        if obligations.len() >= lookup_limit {
            return Err(format!(
                "TypeScript observed source-load obligations exceed limit {lookup_limit}"
            ));
        }
        obligations.push((slot, dependency));
    }
    // This dedicated census consumes original owning project evidence. The
    // ordinary module lookup intentionally omits unavailable targets and is
    // therefore insufficient for an absence proof. Every context participates.
    let mut mapped = vec![None::<(usize, bool)>; dependencies.len()];
    let mut project_seen = BTreeSet::new();
    let mut proof_work = MAX_AGGREGATE_MERGE_DEPENDENCIES * 16;
    for edge in project_edges {
        if edge.attributes.get("relation").and_then(Value::as_str) != Some("imports_from") {
            continue;
        }
        let Some(module) = edge.attributes.get("module").and_then(Value::as_str) else {
            return Ok(false);
        };
        let direct_source = edge
            .attributes
            .get("universal_evidence_source_file")
            .and_then(Value::as_str)
            .filter(|source| !source.is_empty())
            .or_else(|| {
                edge.attributes
                    .get("source_file")
                    .and_then(Value::as_str)
                    .filter(|source| !source.is_empty())
            });
        let source =
            direct_source.or_else(|| node_sources.get(edge.source.as_str()).copied().flatten());
        let Some(source) = source
            .filter(|source| source.len() <= 4_096)
            .and_then(|source| typescript_project_importer_key(source, root))
        else {
            return Ok(false);
        };
        let Some(obligations) = needed.get(&source).and_then(|modules| modules.get(module)) else {
            continue;
        };
        project_seen.insert((source, module.to_owned()));
        let proof = edge
            .attributes
            .get(TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE)
            .map(typescript_source_load_proof_payload)
            .transpose()?;
        let module_count = obligations
            .iter()
            .filter(|(_, dependency)| dependency.kind == Kind::Module)
            .count();
        let target = match node_sources.get(edge.target.as_str()) {
            Some(target) => *target,
            None => edge
                .attributes
                .get("target_file")
                .and_then(Value::as_str)
                .filter(|source| !source.is_empty()),
        };
        let target_present = target
            .filter(|target| target.len() <= 4_096)
            .and_then(|target| typescript_project_importer_key(target, root))
            .is_some_and(|target| sources.contains(&target));
        let mut matched = false;
        for (slot, dependency) in obligations {
            charge_source_load_proof_work(&mut proof_work)?;
            let (applies, unambiguous) = if let Some(proof) = proof {
                (
                    typescript_source_load_proof_matches(proof, dependency),
                    true,
                )
            } else {
                // An unanchored legacy module mapping cannot certify a types
                // directive or distinguish import/require occurrences. For one
                // observed module load its existing meaning is preserved.
                (dependency.kind == Kind::Module, module_count == 1)
            };
            if !applies {
                continue;
            }
            matched = true;
            let entry = mapped[*slot].get_or_insert((0, true));
            entry.0 = entry.0.saturating_add(1);
            if entry.0 > lookup_limit {
                return Err(format!(
                    "TypeScript source-load alternatives exceed limit {lookup_limit}"
                ));
            }
            entry.1 &= target_present && unambiguous;
        }
        if !matched {
            return Ok(false);
        }
    }
    for (slot, dependency) in dependencies.iter().enumerate() {
        let Some(source) = typescript_project_importer_key(&dependency.range.source_file, root)
        else {
            return Ok(false);
        };
        match dependency.kind {
            Kind::ReferencePath => {
                let base = Path::new(&source).parent().unwrap_or_else(|| Path::new(""));
                let Some(target) =
                    normalize_relative_module_path(&base.join(&dependency.specifier))
                        .and_then(|path| path.to_str().map(str::to_owned))
                else {
                    return Ok(false);
                };
                if !sources.contains(&target) {
                    return Ok(false);
                }
            }
            Kind::Module | Kind::ReferenceTypes => {
                if let Some((_, complete)) = mapped[slot] {
                    if !complete {
                        return Ok(false);
                    }
                } else if project_seen.contains(&(source.clone(), dependency.specifier.clone())) {
                    // A mapping for another occurrence/kind is not a license
                    // to fall back for an observed but unproved obligation.
                    return Ok(false);
                } else if dependency.kind == Kind::Module && dependency.specifier.starts_with('.') {
                    if !typescript_relative_load_is_covered(
                        &source,
                        &dependency.specifier,
                        &sources,
                    ) {
                        return Ok(false);
                    }
                } else {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn charge_source_load_proof_work(remaining: &mut usize) -> Result<(), String> {
    *remaining = remaining
        .checked_sub(1)
        .ok_or_else(|| "TypeScript source-load proof work limit exceeded".to_owned())?;
    Ok(())
}

fn typescript_source_load_proof_matches(
    proof: &Map<String, Value>,
    dependency: &compass_languages::TypeScriptMergeDependency,
) -> bool {
    use compass_languages::TypeScriptMergeDependencyKind as Kind;
    let kind = match dependency.kind {
        Kind::Module => "module",
        Kind::ReferencePath => "reference_path",
        Kind::ReferenceTypes => "reference_types",
    };
    let Some(range) = proof.get("range").and_then(Value::as_object) else {
        return false;
    };
    proof.get("kind").and_then(Value::as_str) == Some(kind)
        && proof.get("specifier").and_then(Value::as_str) == Some(dependency.specifier.as_str())
        && range.get("sourceFile").and_then(Value::as_str)
            == Some(dependency.range.source_file.as_str())
        && [
            ("startByte", dependency.range.start_byte),
            ("endByte", dependency.range.end_byte),
            ("startLine", u64::from(dependency.range.start_line)),
            ("endLine", u64::from(dependency.range.end_line)),
            ("startColumn", u64::from(dependency.range.start_column)),
            ("endColumn", u64::from(dependency.range.end_column)),
        ]
        .iter()
        .all(|(field, expected)| range.get(*field).and_then(Value::as_u64) == Some(*expected))
}

fn typescript_relative_load_is_covered(
    source: &str,
    specifier: &str,
    sources: &BTreeSet<String>,
) -> bool {
    let base = Path::new(source).parent().unwrap_or_else(|| Path::new(""));
    let Some(path) = normalize_relative_module_path(&base.join(specifier)) else {
        return false;
    };
    let Some(path) = path.to_str().filter(|path| path.len() <= 4_096) else {
        return false;
    };
    // Preserve the actual extension families rather than coalescing every
    // suffix to one module stem. TypeScript permits source/declaration
    // substitution within a family, including explicit TS extensions.
    // See TypeScript v5.9.3 moduleNameResolver.ts::tryAddingExtensions.
    for (suffix, replacements) in [
        (".d.mts", &[".mts", ".d.mts"][..]),
        (".mts", &[".mts", ".d.mts"][..]),
        (".mjs", &[".mts", ".d.mts"][..]),
        (".d.cts", &[".cts", ".d.cts"][..]),
        (".cts", &[".cts", ".d.cts"][..]),
        (".cjs", &[".cts", ".d.cts"][..]),
        (".d.ts", &[".ts", ".tsx", ".d.ts"][..]),
        (".tsx", &[".tsx", ".ts", ".d.ts"][..]),
        (".ts", &[".ts", ".tsx", ".d.ts"][..]),
        (".jsx", &[".tsx", ".ts", ".d.ts"][..]),
        (".js", &[".ts", ".tsx", ".d.ts"][..]),
    ] {
        if let Some(stem) = path.strip_suffix(suffix) {
            return replacements
                .iter()
                .any(|extension| sources.contains(&format!("{stem}{extension}")));
        }
    }
    if Path::new(path).extension().is_some() {
        return false;
    }
    [
        ".ts",
        ".tsx",
        ".d.ts",
        "/index.ts",
        "/index.tsx",
        "/index.d.ts",
    ]
    .iter()
    .any(|extension| sources.contains(&format!("{path}{extension}")))
}

fn checked_documentation_bytes(
    batches: &[SemanticEvidenceBatch],
    limit: usize,
) -> Result<usize, String> {
    let mut total = 0_usize;
    for documentation in batches
        .iter()
        .flat_map(|batch| &batch.declarations)
        .filter_map(|declaration| declaration.documentation.as_ref())
    {
        total = total
            .checked_add(documentation.content.len())
            .ok_or_else(|| "universal aggregate documentation bytes overflow usize".to_owned())?;
        if total > limit {
            return Err(format!(
                "universal aggregate documentation bytes {total} exceed limit {limit}"
            ));
        }
    }
    Ok(total)
}

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
        Self::new_with_inventory_and_auxiliary_limits(
            batches,
            inventory_nodes,
            root,
            limits,
            AuxiliaryLimits::default(),
        )
    }

    fn new_with_inventory_and_auxiliary_limits(
        batches: &[SemanticEvidenceBatch],
        inventory_nodes: &[NodeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
        auxiliary_limits: AuxiliaryLimits,
    ) -> Result<Self, String> {
        checked_documentation_bytes(batches, MAX_AGGREGATE_DOCUMENTATION_BYTES)?;
        auxiliary_limits
            .check(batches)
            .map_err(|error| error.to_string())?;
        IndexBuilder::new(
            batches.to_vec(),
            inventory_nodes,
            &[],
            root,
            limits,
            true,
            ResolutionAdmission::Max,
        )
        .with_auxiliary_limits(auxiliary_limits)
        .build()
    }

    pub(crate) fn new_with_project_inventory_owned(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        project_edges: &[EdgeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        IndexBuilder::new(
            batches,
            inventory_nodes,
            project_edges,
            root,
            limits,
            true,
            ResolutionAdmission::Max,
        )
        .build()
    }

    pub(crate) fn new_with_prevalidated_project_inventory_owned_at_inference(
        batches: Vec<SemanticEvidenceBatch>,
        inventory_nodes: &[NodeRecord],
        project_edges: &[EdgeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
        admission: ResolutionAdmission,
    ) -> Result<Self, String> {
        IndexBuilder::new(
            batches,
            inventory_nodes,
            project_edges,
            root,
            limits,
            false,
            admission,
        )
        .build()
    }
}

impl IndexBuilder<'_> {
    fn build(self) -> Result<UniversalResolutionIndex, String> {
        let Self {
            mut batches,
            inventory_nodes,
            project_edges,
            root,
            limits,
            validate_batches,
            admission,
            auxiliary_limits,
        } = self;
        checked_documentation_bytes(&batches, MAX_AGGREGATE_DOCUMENTATION_BYTES)?;
        auxiliary_limits
            .check(&batches)
            .map_err(|error| error.to_string())?;
        let languages = LanguagePresence::detect(&batches, inventory_nodes);
        let typescript_merge_inventory_complete = batches
            .iter()
            .any(|batch| batch.pipeline.language == "typescript")
            && batches
                .iter()
                .filter(|batch| batch.pipeline.language == "typescript")
                .all(|batch| {
                    batch.typescript_merge_inventory_complete == Some(true)
                        && batch.typescript_merge_inventory_version
                            == Some(compass_languages::TYPESCRIPT_MERGE_INVENTORY_VERSION)
                });
        let mut profile_started = Instant::now();
        if validate_batches {
            batches.par_iter().try_for_each(|batch| {
                validate_evidence(batch, EvidenceLimits::default())
                    .map_err(|error| format!("invalid universal evidence: {error}"))
            })?;
        }
        let _stub_diagnostics = crate::prepare_python_source_stubs(&mut batches);

        // Check aggregate input sizes before reserving hash-map capacity. A
        // valid batch is bounded on its own, but an unbounded number of valid
        // batches could otherwise make the old aggregate reservation attempt
        // a very large allocation before the resolver's configured limits are
        // consulted. The scope table has no separate public limit; sharing the
        // candidate ceiling keeps the public limit shape stable while still
        // bounding every resolver-owned primary table.
        let mut capacities = batches.iter().try_fold(
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

        let typescript_merge_inventory_complete = typescript_merge_inventory_complete
            && typescript_inventory_sources_are_present(
                &batches,
                inventory_nodes,
                root,
                limits.candidates,
                limits.candidates_per_lookup,
                project_edges,
            )?;

        let low_test_aliases = if admission == ResolutionAdmission::Low {
            let aliases = compact_low_test_candidates(&mut batches);
            capacities[3] = batches.iter().map(|batch| batch.candidates.len()).sum();
            if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
                eprintln!(
                    "[compass internal] universal low test compaction: aliases={} retained_candidates={}",
                    aliases.values().map(Vec::len).sum::<usize>(),
                    capacities[3],
                );
            }
            aliases
        } else {
            AHashMap::new()
        };

        let go_module_path = languages.go.then(|| read_go_module_path(root)).flatten();
        // The typed export index has stricter source-closure requirements.
        // Do not impose them on legacy/untyped JavaScript or inventories that
        // cannot enter the typed receiver consumer at all.
        let has_typed_imports = batches
            .iter()
            .flat_map(|batch| &batch.candidates)
            .any(|candidate| candidate.constraints.typescript_type_arguments.is_some());
        // Reserve the checked aggregate fact counts before consuming the
        // batches. Moving facts into one allocation per sorted primary table
        // avoids repeated vector growth while the per-file batch containers
        // are still being drained.
        let mut declarations = Vec::with_capacity(capacities[0]);
        let mut occurrences = Vec::with_capacity(capacities[1]);
        let mut bindings = Vec::with_capacity(capacities[2]);
        let mut candidates = CandidateTableBuilder::with_capacity(capacities[3]);
        let mut scopes = Vec::with_capacity(capacities[4]);
        profile_internal("universal evidence validation", &mut profile_started);
        for batch in batches {
            declarations.extend(batch.declarations);
            occurrences.extend(batch.occurrences);
            bindings.extend(batch.bindings);
            candidates.extend(batch.candidates)?;
            scopes.extend(batch.scopes);
        }
        let declarations = FactTable::from_values(declarations)?;
        let occurrences = OccurrenceTable::from_values(occurrences)?;
        let bindings = FactTable::from_values(bindings)?;
        let candidates = candidates.finish()?;
        let scopes = FactTable::from_values(scopes)?;
        let mut csharp_bindings_by_source = AHashMap::<String, Vec<CSharpBinding>>::new();
        if languages.csharp {
            for binding in bindings
                .values()
                .filter(|binding| binding.language == "csharp")
            {
                csharp_bindings_by_source
                    .entry(binding.range.source_file.clone())
                    .or_default()
                    .push(CSharpBinding {
                        kind: binding.kind,
                        spelling: binding.spelling.clone(),
                        qualified_target: binding.qualified_target.clone(),
                    });
            }
            for source_bindings in csharp_bindings_by_source.values_mut() {
                source_bindings.sort_unstable_by(|left, right| {
                    left.spelling
                        .cmp(&right.spelling)
                        .then_with(|| left.qualified_target.cmp(&right.qualified_target))
                });
                source_bindings.truncate(candidate_storage_limit(limits.candidates_per_lookup));
            }
        }
        profile_internal("universal fact collection", &mut profile_started);
        let rust_source_wildcard_targets = if languages.rust {
            declarations
                .values()
                .filter(|declaration| {
                    declaration.language == "rust"
                        && matches!(declaration.kind.as_str(), "file" | "module" | "enum")
                })
                .map(|declaration| declaration.qualified_name.clone())
                .collect::<AHashSet<_>>()
        } else {
            AHashSet::new()
        };
        let declaration_ids = declarations
            .values()
            .map(|declaration| declaration.id.clone())
            .collect::<Vec<_>>();
        if u32::try_from(declaration_ids.len()).is_err() {
            return Err("universal declaration slot count exceeds u32".to_owned());
        }
        let definition_ranges = unique_definition_ranges(&declarations, &scopes);
        let python_project_modules = if languages.python {
            python_project_module_index(
                &declarations,
                root,
                limits.candidates,
                limits.candidates_per_lookup,
            )?
        } else {
            PythonProjectModuleIndex::default()
        };
        let (typescript_project_modules, typescript_project_metadata) = if languages.typescript {
            typescript_project_module_index(
                project_edges,
                inventory_nodes,
                root,
                limits.candidates,
                limits.candidates_per_lookup,
            )?
        } else {
            (AHashMap::new(), AHashMap::new())
        };
        let (typescript_modules, typescript_export_aliases, typescript_reexport_targets) =
            if languages.typescript {
                typescript_module_indices(
                    &declarations,
                    &declaration_ids,
                    &bindings,
                    &scopes,
                    root,
                    &typescript_project_modules,
                )
            } else {
                (AHashMap::new(), AHashMap::new(), AHashMap::new())
            };
        let typescript_exported_declarations = if languages.typescript {
            bindings
                .values()
                .filter(|binding| binding.kind == compass_languages::BindingKind::Reexport)
                .filter_map(|binding| binding.target_declaration_id.as_deref())
                .filter_map(|id| declaration_slot(&declaration_ids, id))
                .collect::<AHashSet<_>>()
        } else {
            AHashSet::new()
        };
        let typescript_typed_exports = if languages.typescript && has_typed_imports {
            typescript_typed_export_indices(
                &declarations,
                &declaration_ids,
                &bindings,
                &scopes,
                inventory_nodes,
                project_edges,
                root,
                limits,
            )?
        } else {
            Default::default()
        };
        profile_internal("universal language module indices", &mut profile_started);
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
        // At most one source-spelling entry per admitted Java nominal declaration.
        // This does not guess where a package name ends or select by a member.
        let mut java_receivers_by_source_name = AHashMap::<String, Vec<DeclarationSlot>>::new();
        for declaration in declarations.values().filter(|declaration| {
            declaration.language == "java"
                && matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "enum" | "record" | "annotation_type" | "enum_member"
                )
        }) {
            if let Some(slot) = declaration_slot(&declaration_ids, &declaration.id) {
                java_receivers_by_source_name
                    .entry(declaration.qualified_name.replace("::", "."))
                    .or_default()
                    .push(slot);
            }
        }
        sort_declaration_index(
            &mut java_receivers_by_source_name,
            &declaration_ids,
            limits.candidates_per_lookup,
        );
        profile_internal("universal declaration indices", &mut profile_started);
        let mut inventory_by_qualified = AHashMap::<_, Vec<_>>::new();
        for node in inventory_nodes {
            if node.string("symbol_kind") != "file" || node.string("source_file").is_empty() {
                continue;
            }
            let language = node.string("language");
            let qualified = match language.as_str() {
                "python" => python_project_module_keys(
                    &python_project_modules,
                    &node.string("source_file"),
                    root,
                )
                .map(<[String]>::to_vec)
                .or_else(|| {
                    python_module_name(&node.string("source_file"), root).map(|module| vec![module])
                })
                .unwrap_or_default(),
                "go" => {
                    let package = node.string("package");
                    (!package.is_empty())
                        .then_some(vec![package])
                        .unwrap_or_default()
                }
                _ => Vec::new(),
            };
            for qualified in qualified {
                if language == "python"
                    && !python_project_source_is_indexed(
                        &python_project_modules,
                        &qualified,
                        &node.string("source_file"),
                        root,
                    )
                {
                    continue;
                }
                inventory_by_qualified
                    .entry((language.clone(), qualified))
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
                let mut rust_reexport_aliases = AHashMap::<_, Vec<_>>::new();
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
                        let key = (
                            binding.language.clone(),
                            format!("{}{separator}{}", owner.qualified_name, binding.spelling),
                        );
                        aliases
                            .entry(key.clone())
                            .or_default()
                            .push(binding.qualified_target.clone());
                        if binding.language == "rust"
                            && binding.kind == compass_languages::BindingKind::Reexport
                        {
                            rust_reexport_aliases
                                .entry(key)
                                .or_default()
                                .push(binding.qualified_target.clone());
                        }
                    }
                }
                // A Rust package library and binary share the crate-level
                // qualified name. When both bind the same spelling, only the
                // public reexport defines that spelling for downstream
                // modules; private binary imports remain lexical facts.
                aliases.extend(rust_reexport_aliases);
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
                        .map(|occurrence| occurrence.range());
                    let entry = direct_bases
                        .entry((candidate.language.clone(), owner.qualified_name.clone()))
                        .or_insert_with(|| DirectBaseSet {
                            links: Vec::new(),
                            complete: true,
                        });
                    entry.complete &= base_set_complete;
                    if entry.links.len() <= limits.candidates_per_lookup {
                        entry.links.push(DirectBaseLink {
                            relation: candidate.relation,
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
        let mut php_members_by_owner_folded =
            AHashMap::<(String, String), Vec<DeclarationSlot>>::new();
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
            if languages.php && declaration.language == "php" {
                php_members_by_owner_folded
                    .entry((
                        owner.qualified_name.to_ascii_lowercase(),
                        declaration.name.to_ascii_lowercase(),
                    ))
                    .or_default()
                    .push(slot);
            }
        }
        // Object-literal members are deliberately collected in their lexical
        // binding scope by the TypeScript producer: unlike a class/interface,
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
        for members in php_members_by_owner_folded.values_mut() {
            members.sort_unstable_by(|left, right| {
                declaration_ids[*left as usize].cmp(&declaration_ids[*right as usize])
            });
            members.dedup();
            if members.len() > limits.candidates_per_lookup {
                members.truncate(candidate_storage_limit(limits.candidates_per_lookup));
            }
        }
        let (rust_impl_associated_types, rust_impl_associated_trait_names, rust_impl_traits) =
            if languages.rust {
                let associated_types = rust_impl_associated_type_index(
                    &declarations,
                    &declaration_ids,
                    &scopes,
                    &candidates,
                    &occurrences,
                    limits.candidates_per_lookup,
                );
                let associated_trait_names = rust_impl_associated_trait_name_index(
                    &associated_types,
                    limits.candidates_per_lookup,
                );
                let impl_traits = rust_impl_trait_index(&candidates, limits.candidates_per_lookup);
                (associated_types, associated_trait_names, impl_traits)
            } else {
                (AHashMap::new(), AHashMap::new(), AHashMap::new())
            };
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
        if languages.typescript {
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
                .map_or(u64::MAX, |occurrence| occurrence.range().start_byte);
            return_entries
                .entry((candidate.language.clone(), callable.qualified_name.clone()))
                .or_default()
                .push((start_byte, candidate.id.clone()));
            if candidate
                .occurrence_id
                .as_deref()
                .and_then(|id| occurrences.get(id))
                .and_then(OccurrenceRef::context)
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
        let mut typescript_type_bindings = AHashMap::<(String, String, String), Vec<String>>::new();
        let mut typescript_definition_scopes = AHashMap::<DeclarationSlot, Option<String>>::new();
        if languages.typescript {
            for binding in bindings.values().filter(|binding| {
                matches!(binding.language.as_str(), "typescript" | "javascript")
                    && matches!(
                        binding.kind,
                        compass_languages::BindingKind::Import
                            | compass_languages::BindingKind::ImportAlias
                    )
            }) {
                if let Some(scope) = &binding.scope_id {
                    let slots = typescript_type_bindings
                        .entry((
                            binding.language.clone(),
                            scope.clone(),
                            binding.spelling.clone(),
                        ))
                        .or_default();
                    if slots.len() < candidate_storage_limit(limits.candidates_per_lookup) {
                        slots.push(binding.id.clone());
                    }
                }
            }
            for scope in scopes.values() {
                let Some(owner) = scope
                    .owner_declaration_id
                    .as_deref()
                    .and_then(|id| declarations.get(id))
                else {
                    continue;
                };
                if !matches!(owner.language.as_str(), "typescript" | "javascript")
                    || scope.parent_scope_id != owner.scope_id
                {
                    continue;
                }
                let Some(slot) = declaration_slot(&declaration_ids, &owner.id) else {
                    continue;
                };
                match typescript_definition_scopes.entry(slot) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(Some(scope.id.clone()));
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        entry.insert(None);
                    }
                }
            }
        }
        profile_internal("universal member index", &mut profile_started);
        Ok(UniversalResolutionIndex {
            facts: FactStore {
                declarations,
                declaration_ids,
                occurrences,
                bindings,
                candidates,
                scopes,
                definition_ranges,
            },
            indexes: Mutex::new(ResolutionIndexes {
                names: NameIndexes {
                    by_qualified,
                    by_module_name,
                    by_scope_name,
                    by_source_directory_name,
                    inventory_by_qualified,
                    aliases,
                },
                hierarchy: HierarchyIndexes {
                    direct_bases,
                    direct_subtypes,
                },
                members: MemberIndexes {
                    members_by_owner,
                    members,
                    return_candidates_by_callable,
                    outer_return_candidates_by_callable,
                },
                wildcards: WildcardIndexes {
                    by_scope: wildcard_bindings_by_scope,
                    by_module: wildcard_bindings_by_module,
                    reexports_by_module: wildcard_reexports_by_module,
                },
                typescript: TypeScriptIndexes {
                    merge_inventory_complete: typescript_merge_inventory_complete,
                    type_bindings_by_scope: typescript_type_bindings,
                    definition_scopes: typescript_definition_scopes,
                    modules: typescript_modules,
                    exported_declarations: typescript_exported_declarations,
                    export_aliases: typescript_export_aliases,
                    reexport_targets: typescript_reexport_targets,
                    reexport_bindings: typescript_typed_exports.reexport_bindings,
                    binding_sources: typescript_typed_exports.binding_sources,
                    source_files: typescript_typed_exports.source_files,
                    complete_export_sources: typescript_typed_exports.complete_sources,
                    direct_exports: typescript_typed_exports.direct_exports,
                    member_aliases: typescript_member_aliases,
                },
                rust: RustIndexes {
                    impl_associated_types: rust_impl_associated_types,
                    impl_associated_trait_names: rust_impl_associated_trait_names,
                    impl_traits: rust_impl_traits,
                    source_wildcard_targets: rust_source_wildcard_targets,
                },
                csharp: CSharpIndexes {
                    bindings_by_source: csharp_bindings_by_source,
                },
                java: JavaIndexes {
                    receivers_by_source_name: java_receivers_by_source_name,
                },
                php: PhpIndexes {
                    members_by_owner_folded: php_members_by_owner_folded,
                },
            }),
            project: ProjectContext {
                root: root.to_path_buf(),
                python_project_modules,
                typescript_project_modules,
                typescript_project_metadata,
                go_module_path,
            },
            budget: LookupBudget::from(limits),
            low_test_aliases,
        })
    }
}

type TestAliasKey<'a> = (&'a str, &'a str, Option<&'a str>, Option<&'a str>, &'a str);

fn compact_low_test_candidates(
    batches: &mut [SemanticEvidenceBatch],
) -> AHashMap<String, Vec<String>> {
    let mut aliases = AHashMap::<String, Vec<String>>::new();
    for batch in batches {
        compact_low_test_candidates_in_batch(&mut batch.candidates, &mut aliases);
    }
    for test_ids in aliases.values_mut() {
        test_ids.sort_unstable();
        test_ids.dedup();
    }
    aliases
}

fn compact_low_test_candidates_in_batch(
    candidates: &mut Vec<RelationshipCandidate>,
    aliases: &mut AHashMap<String, Vec<String>>,
) {
    let (batch_aliases, removed) = {
        let mut calls = AHashMap::<TestAliasKey<'_>, Vec<&RelationshipCandidate>>::new();
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        {
            calls
                .entry(test_alias_key(candidate))
                .or_default()
                .push(candidate);
        }
        let mut batch_aliases = Vec::new();
        let mut removed = AHashSet::new();
        for candidate in candidates
            .iter()
            .filter(|candidate| candidate.relation == CandidateRelation::Tests)
        {
            let mut matching = calls
                .get(&test_alias_key(candidate))
                .into_iter()
                .flatten()
                .filter(|call| call.constraints == candidate.constraints)
                .take(2);
            let Some(call) = matching.next() else {
                continue;
            };
            if matching.next().is_some() {
                continue;
            }
            batch_aliases.push((call.id.clone(), candidate.id.clone()));
            removed.insert(candidate.id.clone());
        }
        (batch_aliases, removed)
    };
    if !removed.is_empty() {
        candidates.retain(|candidate| !removed.contains(&candidate.id));
    }
    for (call_id, test_id) in batch_aliases {
        aliases.entry(call_id).or_default().push(test_id);
    }
}

fn test_alias_key(candidate: &RelationshipCandidate) -> TestAliasKey<'_> {
    (
        &candidate.language,
        &candidate.source_declaration_id,
        candidate.occurrence_id.as_deref(),
        candidate.binding_id.as_deref(),
        &candidate.target_spelling,
    )
}

#[cfg(test)]
mod tests {
    use compass_languages::ResolutionConstraint;

    use super::*;

    #[test]
    fn low_test_compaction_requires_one_identical_call_candidate() {
        let call = candidate("call", CandidateRelation::Calls, 0);
        let matching_test = candidate("test-matching", CandidateRelation::Tests, 0);
        let distinct_test = candidate("test-distinct", CandidateRelation::Tests, 1);
        let mut candidates = vec![call, matching_test, distinct_test];
        let mut aliases = AHashMap::new();

        compact_low_test_candidates_in_batch(&mut candidates, &mut aliases);

        assert_eq!(aliases.get("call"), Some(&vec!["test-matching".to_owned()]));
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.id.as_str())
                .collect::<Vec<_>>(),
            ["call", "test-distinct"]
        );
    }

    #[test]
    fn low_test_compaction_keeps_ambiguous_call_pairs() {
        let mut candidates = vec![
            candidate("call-a", CandidateRelation::Calls, 0),
            candidate("call-b", CandidateRelation::Calls, 0),
            candidate("test", CandidateRelation::Tests, 0),
        ];
        let mut aliases = AHashMap::new();

        compact_low_test_candidates_in_batch(&mut candidates, &mut aliases);

        assert!(aliases.is_empty());
        assert_eq!(candidates.len(), 3);
    }

    fn candidate(
        id: &str,
        relation: CandidateRelation,
        argument_count: u32,
    ) -> RelationshipCandidate {
        RelationshipCandidate {
            id: id.to_owned(),
            language: "rust".to_owned(),
            relation,
            source_declaration_id: "declaration:test".to_owned(),
            occurrence_id: Some("occurrence:call".to_owned()),
            binding_id: Some("binding:target".to_owned()),
            target_spelling: "target".to_owned(),
            constraints: ResolutionConstraint {
                argument_count: Some(argument_count),
                ..ResolutionConstraint::default()
            },
        }
    }
}

#[cfg(test)]
mod documentation_limit_tests {
    use super::checked_documentation_bytes;
    use compass_languages::Engine;

    #[test]
    fn aggregate_documentation_is_checked_at_the_exact_byte_boundary()
    -> Result<(), Box<dyn std::error::Error>> {
        let batch = Engine::default()
            .extract_source_combined(
                std::path::Path::new("sample.rs"),
                "sample.rs",
                b"/// documented\nfn work() {}\n",
            )?
            .graph
            .semantic_evidence
            .ok_or("missing evidence")?;
        let size = batch
            .declarations
            .iter()
            .filter_map(|declaration| declaration.documentation.as_ref())
            .map(|documentation| documentation.content.len())
            .sum::<usize>();
        assert!(size > 0);
        let batches = vec![batch.clone(), batch];
        assert_eq!(checked_documentation_bytes(&batches, size * 2)?, size * 2);
        assert!(checked_documentation_bytes(&batches, size * 2 - 1).is_err());
        assert_eq!(checked_documentation_bytes(&[], 0)?, 0);
        Ok(())
    }
}

#[cfg(test)]
mod merge_dependency_tests {
    use super::*;
    #[test]
    fn repeated_module_loads_require_exact_occurrence_proofs()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut batches = Vec::new();
        for (file, source) in [
            (
                "main.ts",
                "import 'augment'; export const value = require('augment');",
            ),
            ("first.ts", "export {};"),
            ("second.ts", "export {};"),
        ] {
            let batch = compass_languages::Engine::default().extract_source_universal_evidence(
                Path::new(file),
                file,
                source.as_bytes(),
            )?;
            assert_eq!(batch.typescript_merge_inventory_complete, Some(true));
            batches.push(batch);
        }
        let loads = &batches[0].typescript_merge_dependencies;
        assert_eq!(loads.len(), 2);
        let mut complete = Vec::new();
        for (dependency, target) in loads.iter().zip(["first.ts", "second.ts"]) {
            complete.push(EdgeRecord {
                source: "main".to_owned(),
                target: target.to_owned(),
                attributes: Map::from_iter([
                    (
                        "relation".to_owned(),
                        Value::String("imports_from".to_owned()),
                    ),
                    (
                        "source_file".to_owned(),
                        Value::String("main.ts".to_owned()),
                    ),
                    ("module".to_owned(), Value::String("augment".to_owned())),
                    ("target_file".to_owned(), Value::String(target.to_owned())),
                    (
                        TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE.to_owned(),
                        serde_json::to_value(TypeScriptSourceLoadProof {
                            version: 1,
                            dependency: dependency.clone(),
                        })?,
                    ),
                ]),
            });
        }
        let mut legacy = complete[0].clone();
        legacy
            .attributes
            .remove(TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE);
        let mut wrong_occurrence = complete[1].clone();
        wrong_occurrence.attributes.insert(
            TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE.to_owned(),
            complete[0].attributes[TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE].clone(),
        );
        for (mut edges, expected) in [
            (vec![legacy], false),
            (vec![complete[0].clone()], false),
            (complete.clone(), true),
            (vec![complete[0].clone(), wrong_occurrence], false),
        ] {
            for reverse in [false, true] {
                let mut inputs = batches.clone();
                if reverse {
                    inputs.reverse();
                    edges.reverse();
                }
                assert_eq!(
                    typescript_inventory_sources_are_present(
                        &inputs,
                        &[],
                        Path::new("."),
                        100,
                        256,
                        &edges
                    )?,
                    expected
                );
            }
        }
        let mut future = complete;
        future[0].attributes[TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE]["version"] = Value::from(2);
        assert!(
            matches!(typescript_inventory_sources_are_present(&batches, &[], Path::new("."), 100, 256, &future), Err(reason) if reason.contains("unsupported"))
        );
        Ok(())
    }

    #[test]
    fn ordinary_project_modules_do_not_consume_reference_types_proofs()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = b"/// <reference types=\"augment\" />\nimport 'augment'; export {};";
        let batch = compass_languages::Engine::default().extract_source_universal_evidence(
            Path::new("main.ts"),
            "main.ts",
            source,
        )?;
        let mut edges = Vec::new();
        for (kind, target) in [
            (
                compass_languages::TypeScriptMergeDependencyKind::Module,
                "module.ts",
            ),
            (
                compass_languages::TypeScriptMergeDependencyKind::ReferenceTypes,
                "types.d.ts",
            ),
        ] {
            let dependency = batch
                .typescript_merge_dependencies
                .iter()
                .find(|dependency| dependency.kind == kind)
                .ok_or("missing observed load")?;
            edges.push(EdgeRecord {
                source: "main".to_owned(),
                target: target.to_owned(),
                attributes: Map::from_iter([
                    (
                        "relation".to_owned(),
                        Value::String("imports_from".to_owned()),
                    ),
                    ("module".to_owned(), Value::String("augment".to_owned())),
                    (
                        "source_file".to_owned(),
                        Value::String("main.ts".to_owned()),
                    ),
                    ("target_file".to_owned(), Value::String(target.to_owned())),
                    (
                        TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE.to_owned(),
                        serde_json::to_value(TypeScriptSourceLoadProof {
                            version: 1,
                            dependency: dependency.clone(),
                        })?,
                    ),
                ]),
            });
        }
        for reverse in [false, true] {
            if reverse {
                edges.reverse();
            }
            let (modules, _) =
                typescript_project_module_index(&edges, &[], Path::new("."), 10, 10)?;
            assert_eq!(
                modules.get(&("main.ts".to_owned(), "augment".to_owned(), String::new())),
                Some(&vec!["module".to_owned()])
            );
        }
        Ok(())
    }

    #[test]
    fn source_load_proof_reader_rejects_unknown_versions_and_bounds_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut remaining = 1;
        charge_source_load_proof_work(&mut remaining)?;
        assert_eq!(remaining, 0);
        assert!(charge_source_load_proof_work(&mut remaining).is_err());
        let source = b"import './a';";
        let batch = compass_languages::Engine::default().extract_source_universal_evidence(
            Path::new("main.ts"),
            "main.ts",
            source,
        )?;
        let dependency = batch
            .typescript_merge_dependencies
            .first()
            .ok_or("missing source load")?;
        let proof = TypeScriptSourceLoadProof {
            version: 1,
            dependency: dependency.clone(),
        };
        let mut value = serde_json::to_value(proof)?;
        assert!(typescript_source_load_proof_matches(
            typescript_source_load_proof_payload(&value)?,
            dependency
        ));
        value["version"] = Value::from(2);
        assert!(
            matches!(typescript_source_load_proof_payload(&value), Err(reason) if reason.contains("unsupported"))
        );
        value["version"] = Value::from(1);
        value["dependency"]["specifier"] = Value::String("a".repeat(1025));
        assert!(typescript_source_load_proof_payload(&value).is_err());
        Ok(())
    }

    #[test]
    fn relative_load_extension_families_preserve_valid_source_substitution() {
        let families = [
            (
                &[".ts", ".tsx", ".d.ts", ".js", ".jsx"][..],
                &[".ts", ".tsx", ".d.ts"][..],
            ),
            (&[".mts", ".d.mts", ".mjs"][..], &[".mts", ".d.mts"][..]),
            (&[".cts", ".d.cts", ".cjs"][..], &[".cts", ".d.cts"][..]),
        ];
        for (specifiers, accepted) in families {
            for specifier in specifiers {
                for target in [".ts", ".tsx", ".d.ts", ".mts", ".d.mts", ".cts", ".d.cts"] {
                    let sources = BTreeSet::from([format!("provider{target}")]);
                    assert_eq!(
                        typescript_relative_load_is_covered(
                            "main.ts",
                            &format!("./provider{specifier}"),
                            &sources
                        ),
                        accepted.contains(&target),
                        "{specifier} -> {target}"
                    );
                }
            }
        }
        for (target, expected) in [
            ("provider.ts", true),
            ("provider/index.d.ts", true),
            ("provider.mts", false),
            ("provider.cts", false),
        ] {
            assert_eq!(
                typescript_relative_load_is_covered(
                    "main.ts",
                    "./provider",
                    &BTreeSet::from([target.to_owned()])
                ),
                expected
            );
        }
    }

    #[test]
    fn aggregate_typed_proof_caps_apply_to_borrowed_owned_and_prevalidated_inputs()
    -> Result<(), Box<dyn std::error::Error>> {
        let batch = compass_languages::Engine::default().extract_source_universal_evidence(
            Path::new("main.ts"), "main.ts",
            b"import type { Holder } from './provider'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }",
        )?;
        let usage = compass_languages::TypeScriptArgumentUsage::for_batch(&batch)?;
        assert!(usage.nodes > 0 && usage.bytes > 0);
        let exact = AuxiliaryLimits {
            type_nodes: usage.nodes,
            type_bytes: usage.bytes,
            ..AuxiliaryLimits::default()
        };
        AuxiliaryLimits {
            type_nodes: 0,
            type_bytes: 0,
            ..exact
        }
        .check(&[])?;
        exact.check(std::slice::from_ref(&batch))?;
        for quota in [
            exact,
            AuxiliaryLimits {
                type_nodes: usage.nodes - 1,
                ..exact
            },
            AuxiliaryLimits {
                type_bytes: usage.bytes - 1,
                ..exact
            },
        ] {
            let expected_ok = quota.type_nodes == usage.nodes && quota.type_bytes == usage.bytes;
            let borrowed = UniversalResolutionIndex::new_with_inventory_and_auxiliary_limits(
                std::slice::from_ref(&batch),
                &[],
                Path::new("."),
                UniversalResolutionLimits::default(),
                quota,
            );
            assert_eq!(
                borrowed.is_ok(),
                expected_ok,
                "borrowed: {:?}",
                borrowed.err()
            );
            for validate in [false, true] {
                let owned = IndexBuilder::new(
                    vec![batch.clone()],
                    &[],
                    &[],
                    Path::new("."),
                    UniversalResolutionLimits::default(),
                    validate,
                    ResolutionAdmission::Max,
                )
                .with_auxiliary_limits(quota)
                .build();
                assert_eq!(
                    owned.is_ok(),
                    expected_ok,
                    "owned validate={validate}: {:?}",
                    owned.err()
                );
            }
        }
        let repeated = exact.check(&[batch.clone(), batch.clone()]);
        assert!(matches!(
            repeated,
            Err(super::super::super::budget::AuxiliaryLimitError::Aggregate { .. })
        ));
        let mut invalid = batch.clone();
        let proof = invalid
            .candidates
            .iter_mut()
            .find_map(|candidate| candidate.constraints.typescript_type_arguments.as_mut())
            .ok_or("missing proof")?;
        proof.owner_binding_id = "x".repeat(1_025);
        let invalid_after_overflow = AuxiliaryLimits {
            type_nodes: 0,
            ..exact
        }
        .check(&[batch, invalid]);
        assert!(matches!(
            invalid_after_overflow,
            Err(super::super::super::budget::AuxiliaryLimitError::InvalidBatch(_))
        ));
        Ok(())
    }

    #[test]
    fn aggregate_merge_dependencies_check_exact_and_overflow_before_indexing()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = b"import './a'; import './b'; export {};";
        let batch = compass_languages::Engine::default().extract_source_universal_evidence(
            Path::new("main.ts"),
            "main.ts",
            source,
        )?;
        let usage = compass_languages::TypeScriptMergeDependencyUsage::for_dependencies(
            &batch.typescript_merge_dependencies,
        )?;
        assert_eq!(usage.count, 2);
        let limits = |count, bytes| AuxiliaryLimits {
            merge_dependencies: count,
            merge_bytes: bytes,
            ..AuxiliaryLimits::default()
        };
        limits(0, 0).check(&[])?;
        limits(usage.count, usage.text_bytes).check(std::slice::from_ref(&batch))?;
        assert!(
            limits(usage.count - 1, usage.text_bytes)
                .check(std::slice::from_ref(&batch))
                .is_err()
        );
        assert!(
            limits(usage.count, usage.text_bytes - 1)
                .check(std::slice::from_ref(&batch))
                .is_err()
        );
        assert!(
            limits(usage.count, usage.text_bytes)
                .check(&[batch.clone(), batch])
                .is_err()
        );
        Ok(())
    }
}
