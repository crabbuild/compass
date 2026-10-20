//! Preserve typed export obligations before the legacy module view loses them.

use super::*;
use std::sync::Arc;

#[derive(Default)]
pub(in crate::evidence) struct TypedExportIndices {
    pub(in crate::evidence) complete_sources: BTreeSet<(String, String)>,
    pub(in crate::evidence) source_files: BTreeSet<(String, String)>,
    pub(in crate::evidence) binding_sources: AHashMap<String, Arc<TypeScriptBindingSources>>,
    pub(in crate::evidence) reexport_bindings: TypeScriptReexportBindings,
    pub(in crate::evidence) direct_exports: TypeScriptModuleIndex,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::evidence) fn typescript_typed_export_indices(
    declarations: &FactTable<DeclarationFact>,
    declaration_ids: &[String],
    bindings: &FactTable<BindingFact>,
    scopes: &FactTable<compass_languages::ScopeFact>,
    inventory_nodes: &[NodeRecord],
    project_edges: &[EdgeRecord],
    root: &Path,
    limits: UniversalResolutionLimits,
) -> Result<TypedExportIndices, String> {
    if inventory_nodes.len() > limits.candidates || project_edges.len() > limits.candidates {
        return Err("TypeScript typed export project inventory exceeds candidate limit".to_owned());
    }
    let mut result = TypedExportIndices::default();
    let mut sources = BTreeSet::new();
    let mut incomplete_sources = BTreeSet::new();
    for declaration in declarations.values().filter(|declaration| {
        matches!(declaration.language.as_str(), "typescript" | "javascript")
            && declaration.scope_id.is_none()
            && matches!(declaration.kind.as_str(), "file" | "module")
    }) {
        if let Some(source) = source_key(&declaration.range.source_file, root) {
            result
                .source_files
                .insert((declaration.language.clone(), source.clone()));
            let key = (declaration.language.clone(), source.clone());
            if declaration
                .ecmascript_export_inventory
                .is_some_and(|inventory| {
                    inventory.version == compass_languages::ECMASCRIPT_EXPORT_INVENTORY_VERSION
                        && inventory.complete
                })
            {
                result.complete_sources.insert(key);
            } else {
                incomplete_sources.insert(key);
            }
            sources.insert(source);
        }
    }

    result
        .complete_sources
        .retain(|key| !incomplete_sources.contains(key));

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
    }
    let mut seen = BTreeSet::new();
    let mut project = BTreeMap::<(String, String), BTreeSet<String>>::new();
    let max_keys = candidate_storage_limit(limits.candidates_per_lookup);
    let mut unavailable = BTreeSet::new();
    for edge in project_edges {
        if edge.attributes.get("relation").and_then(Value::as_str) != Some("imports_from") {
            continue;
        }
        if let Some(proof) = edge.attributes.get(TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE)
            && typescript_source_load_proof_payload(proof)?
                .get("kind")
                .and_then(Value::as_str)
                != Some("module")
        {
            continue;
        }
        let source = edge
            .attributes
            .get("universal_evidence_source_file")
            .and_then(Value::as_str)
            .filter(|source| !source.is_empty())
            .or_else(|| {
                edge.attributes
                    .get("source_file")
                    .and_then(Value::as_str)
                    .filter(|source| !source.is_empty())
            })
            .or_else(|| node_sources.get(edge.source.as_str()).copied().flatten());
        let Some(source) = source
            .filter(|source| source.len() <= 4_096)
            .and_then(|source| source_key(source, root))
            .filter(|source| sources.contains(source))
        else {
            continue;
        };
        let Some(module) = edge
            .attributes
            .get("module")
            .and_then(Value::as_str)
            .filter(|module| {
                !module.is_empty() && module.len() <= 4_096 && !module.contains(['\\', '\0'])
            })
        else {
            continue;
        };
        let key = (source, module.to_owned());
        seen.insert(key.clone());
        let target = match node_sources.get(edge.target.as_str()) {
            Some(target) => *target,
            None => edge.attributes.get("target_file").and_then(Value::as_str),
        };
        if let Some(target) = target
            .filter(|source| source.len() <= 4_096)
            .and_then(|source| source_key(source, root))
            .filter(|source| sources.contains(source))
        {
            let values = project.entry(key).or_default();
            if !values.contains(&target) && values.len() < max_keys {
                values.insert(target);
            }
        } else {
            unavailable.insert(key);
        }
    }
    // Preserve exact target source identity and share alternatives per owning
    // importer/specifier. A module stem cannot distinguish foo.ts from
    // foo/index.ts or a .mts source from a same-stem .ts declaration.
    let mut shared = BTreeMap::<(String, String), Arc<TypeScriptBindingSources>>::new();
    for binding in bindings.values().filter(|binding| {
        matches!(binding.language.as_str(), "typescript" | "javascript")
            && matches!(
                binding.kind,
                compass_languages::BindingKind::Import
                    | compass_languages::BindingKind::ImportAlias
                    | compass_languages::BindingKind::Reexport
            )
    }) {
        let owner = binding
            .scope_id
            .as_deref()
            .and_then(|id| scopes.get(id))
            .and_then(|scope| scope.owner_declaration_id.as_deref())
            .and_then(|id| declarations.get(id));
        let public_export = binding.kind == compass_languages::BindingKind::Reexport
            && owner.is_some_and(|owner| {
                owner.scope_id.is_none() && matches!(owner.kind.as_str(), "file" | "module")
            });
        if let Some(target) = binding.target_declaration_id.as_deref() {
            if public_export
                && let Some(slot) = declaration_slot(declaration_ids, target)
                && let Some(source) = source_key(&binding.range.source_file, root)
            {
                result
                    .direct_exports
                    .entry((binding.language.clone(), source, binding.spelling.clone()))
                    .or_default()
                    .push(slot);
            }

            continue;
        }
        // Keep the binding obligation even when its target cannot be indexed.
        // Otherwise a malformed/unavailable explicit export could vanish and
        // incorrectly expose a same-named star target as the unique answer.
        if public_export
            && let Some(owner) = owner
            && let Some(source) = source_key(&owner.range.source_file, root)
        {
            result
                .reexport_bindings
                .entry((owner.language.clone(), source, binding.spelling.clone()))
                .or_default()
                .push(binding.id.clone());
        }

        let Some((module, _exported)) = binding.qualified_target.rsplit_once("::") else {
            continue;
        };
        let Some(source) = source_key(&binding.range.source_file, root) else {
            continue;
        };
        let key = (source, module.to_owned());
        let targets = shared.entry(key.clone()).or_insert_with(|| {
            let keys = if seen.contains(&key) {
                project
                    .get(&key)
                    .map(|keys| keys.iter().cloned().collect())
                    .unwrap_or_default()
            } else {
                relative_binding_source(&binding.range.source_file, module, root, &sources)
                    .into_iter()
                    .collect()
            };
            Arc::new(TypeScriptBindingSources {
                sources: keys,
                unavailable: unavailable.contains(&key),
            })
        });
        result
            .binding_sources
            .insert(binding.id.clone(), Arc::clone(targets));
    }
    for bindings in result.reexport_bindings.values_mut() {
        bindings.sort_unstable();
        bindings.dedup();
    }
    sort_declaration_index(&mut result.direct_exports, declaration_ids, usize::MAX);
    Ok(result)
}

fn source_key(source: &str, root: &Path) -> Option<String> {
    if source.is_empty() || source.len() > 4_096 {
        return None;
    }
    typescript_project_importer_key(source, root)
}

/// Bounded native source profile: exact owning metadata wins above. Without it,
/// only supported relative source/declaration substitutions are considered,
/// in file-before-directory and TypeScript-before-JavaScript order.
fn relative_binding_source(
    importer: &str,
    module: &str,
    root: &Path,
    sources: &BTreeSet<String>,
) -> Option<String> {
    if !(module.starts_with("./") || module.starts_with("../"))
        || module.len() > 4_096
        || importer.len() > 4_096
        || module.contains(['\\', '\0'])
    {
        return None;
    }
    let importer = typescript_relative_path(importer, root)?;
    let base = importer.parent()?;
    let joined = base.join(module);
    let path = typescript_project_importer_key(joined.to_str()?, Path::new("."))?;
    let mut stem = path.as_str();
    let mut explicit = false;
    let mut extensions: &[&str] = &[".ts", ".tsx", ".d.ts", ".js", ".jsx"];
    for suffix in [
        ".d.mts", ".d.cts", ".d.ts", ".mts", ".cts", ".tsx", ".jsx", ".mjs", ".cjs", ".ts", ".js",
    ] {
        if let Some(value) = path.strip_suffix(suffix) {
            explicit = true;
            stem = value;
            extensions = match suffix {
                ".d.mts" | ".mts" | ".mjs" => &[".mts", ".d.mts", ".mjs"],
                ".d.cts" | ".cts" | ".cjs" => &[".cts", ".d.cts", ".cjs"],
                ".tsx" | ".jsx" => &[".tsx", ".ts", ".d.ts", ".jsx", ".js"],
                _ => &[".ts", ".tsx", ".d.ts", ".js", ".jsx"],
            };
            break;
        }
    }
    if !explicit && Path::new(stem).extension().is_some() {
        return None;
    }
    for suffix in extensions {
        let candidate = format!("{stem}{suffix}");
        if sources.contains(&candidate) {
            return Some(candidate);
        }
    }
    if !explicit {
        for suffix in extensions {
            let candidate = format!("{stem}/index{suffix}");
            if sources.contains(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_source_keys_and_relative_targets_preserve_portable_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let native_importer = Path::new("src").join("app").join("main.ts");
        let native_provider = Path::new("src").join("providers").join("index.ts");
        for source in [native_importer.as_path(), native_provider.as_path()] {
            let expected = if source == native_importer {
                "src/app/main.ts"
            } else {
                "src/providers/index.ts"
            };
            assert_eq!(
                source_key(source.to_str().ok_or("non-UTF-8 fixture")?, root).as_deref(),
                Some(expected)
            );
            assert_eq!(
                source_key(
                    root.join(source).to_str().ok_or("non-UTF-8 fixture root")?,
                    root
                )
                .as_deref(),
                Some(expected)
            );
        }
        let sources = BTreeSet::from([
            "src/providers/index.ts".to_owned(),
            "src/providers/client.ts".to_owned(),
            "src/providers/Client.ts".to_owned(),
            "src/shared.ts".to_owned(),
        ]);
        let absolute_importer = root.join(&native_importer);
        for importer in [native_importer.as_path(), absolute_importer.as_path()] {
            let importer = importer.to_str().ok_or("non-UTF-8 fixture importer")?;
            for (module, expected) in [
                ("../providers", "src/providers/index.ts"),
                ("../providers/index", "src/providers/index.ts"),
                ("../providers/client.js", "src/providers/client.ts"),
                ("../providers/Client.js", "src/providers/Client.ts"),
                ("../providers/../shared", "src/shared.ts"),
            ] {
                assert_eq!(
                    relative_binding_source(importer, module, root, &sources).as_deref(),
                    Some(expected),
                    "{module}"
                );
            }
            for module in [
                "../../../outside",
                "../missing",
                "../providers/client.custom",
                "../providers/CLIENT.js",
            ] {
                assert_eq!(
                    relative_binding_source(importer, module, root, &sources),
                    None,
                    "{module}"
                );
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unix_literal_backslashes_do_not_alias_source_directories() {
        assert_eq!(
            source_key(r"src/literal\name.ts", Path::new(".")).as_deref(),
            Some(r"src/literal\name.ts")
        );
        assert_ne!(
            source_key(r"src/literal\name.ts", Path::new(".")),
            source_key("src/literal/name.ts", Path::new("."))
        );
    }

    #[test]
    fn relative_sources_preserve_extension_priority_and_file_identity() {
        let sources = [
            "provider.ts",
            "provider.tsx",
            "provider.d.ts",
            "provider/index.ts",
            "provider.mts",
            "provider.cts",
            "provider.js",
            "provider.jsx",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        for (specifier, expected) in [
            ("./provider", "provider.ts"),
            ("./provider.js", "provider.ts"),
            ("./provider.ts", "provider.ts"),
            ("./provider.d.ts", "provider.ts"),
            ("./provider.jsx", "provider.tsx"),
            ("./provider.tsx", "provider.tsx"),
            ("./provider.mjs", "provider.mts"),
            ("./provider.cjs", "provider.cts"),
            ("./provider/index", "provider/index.ts"),
        ] {
            assert_eq!(
                relative_binding_source("main.ts", specifier, Path::new("."), &sources).as_deref(),
                Some(expected),
                "{specifier}"
            );
        }
        for specifier in [
            "provider",
            "./provider.custom",
            "../../provider",
            "./provider\\bad",
        ] {
            assert_eq!(
                relative_binding_source("main.ts", specifier, Path::new("."), &sources),
                None,
                "{specifier}"
            );
        }
        let directory = BTreeSet::from(["provider/index.ts".to_owned()]);
        assert_eq!(
            relative_binding_source("main.ts", "./provider", Path::new("."), &directory).as_deref(),
            Some("provider/index.ts")
        );
        assert_eq!(
            relative_binding_source("main.ts", "./provider.ts", Path::new("."), &directory),
            None
        );
    }
}
