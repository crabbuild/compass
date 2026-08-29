//! Immutable repository and project-level resolution context.

use super::*;

pub(super) struct ProjectContext {
    pub(super) root: std::path::PathBuf,
    pub(super) typescript_project_modules: TypeScriptProjectModuleIndex,
    pub(super) typescript_project_metadata: TypeScriptProjectMetadataIndex,
    pub(super) go_module_path: Option<String>,
}

pub(in crate::evidence) fn python_module_name(source_file: &str, root: &Path) -> Option<String> {
    let path = Path::new(source_file);
    let relative = path.strip_prefix(root).unwrap_or(path);
    let source = relative.to_string_lossy().replace('\\', "/");
    let source = source.strip_suffix(".py")?;
    let source = source.strip_suffix("/__init__").unwrap_or(source);
    let module = source
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join(".");
    (!module.is_empty()).then_some(module)
}

pub(in crate::evidence) fn source_directory(source_file: &str, root: &Path) -> Option<String> {
    let path = Path::new(source_file);
    let relative = path.strip_prefix(root).unwrap_or(path);
    let directory = relative
        .parent()?
        .to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_owned();
    (!directory.is_empty()).then_some(directory)
}

pub(in crate::evidence) fn typescript_project_importer_key(
    source_file: &str,
    root: &Path,
) -> Option<String> {
    let relative = typescript_relative_path(source_file, root)?;
    let key = relative.to_str()?.to_owned();
    (!key.is_empty() && key.len() <= 4_096).then_some(key)
}

pub(in crate::evidence) fn typescript_project_module_keys(
    project_modules: &TypeScriptProjectModuleIndex,
    importer: &str,
    module: &str,
    root: &Path,
    context: Option<&str>,
) -> Option<Vec<String>> {
    let importer = typescript_project_importer_key(importer, root)?;
    if let Some(context) = context.filter(|context| !context.is_empty()) {
        return project_modules
            .get(&(importer, module.to_owned(), context.to_owned()))
            .filter(|keys| !keys.is_empty())
            .cloned();
    }
    let mut keys = project_modules
        .iter()
        .filter(|((candidate_importer, candidate_module, _), _)| {
            candidate_importer == &importer && candidate_module == module
        })
        .flat_map(|(_, keys)| keys.iter().cloned())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    (!keys.is_empty()).then_some(keys)
}

pub(in crate::evidence) fn typescript_project_module_index(
    edges: &[EdgeRecord],
    inventory_nodes: &[NodeRecord],
    root: &Path,
    entry_limit: usize,
    candidates_per_lookup: usize,
) -> Result<(TypeScriptProjectModuleIndex, TypeScriptProjectMetadataIndex), String> {
    if edges.is_empty() {
        return Ok((
            TypeScriptProjectModuleIndex::new(),
            TypeScriptProjectMetadataIndex::new(),
        ));
    }
    let mut source_by_node = BTreeMap::<String, BTreeSet<String>>::new();
    for node in inventory_nodes {
        let source = node.string("source_file");
        if !source.is_empty() {
            source_by_node.entry(node.id.clone()).or_default().insert(
                node.attributes
                    .get("universal_evidence_source_file")
                    .and_then(Value::as_str)
                    .filter(|source| !source.is_empty())
                    .unwrap_or(&source)
                    .to_owned(),
            );
        }
    }
    let max_targets = candidate_storage_limit(candidates_per_lookup);
    let mut targets = BTreeMap::<(String, String, String), BTreeSet<String>>::new();
    let mut metadata = TypeScriptProjectMetadataIndex::new();
    for edge in edges {
        if edge.attributes.get("relation").and_then(Value::as_str) != Some("imports_from") {
            continue;
        }
        let module = edge.string("module");
        if module.is_empty() || module.len() > 4_096 || module.contains(['\\', '\0']) {
            continue;
        }
        let importer = {
            let source = edge
                .attributes
                .get("universal_evidence_source_file")
                .and_then(Value::as_str)
                .filter(|source| !source.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| edge.string("source_file"));
            if source.is_empty() {
                unique_inventory_source(&source_by_node, &edge.source)
                    .unwrap_or_default()
                    .to_owned()
            } else {
                source
            }
        };
        let Some(importer) = typescript_project_importer_key(&importer, root) else {
            continue;
        };
        let target_source = unique_inventory_source(&source_by_node, &edge.target)
            .map(str::to_owned)
            .filter(|source| !source.is_empty())
            .unwrap_or_else(|| edge.string("target_file"));
        let normalized_target_source = if target_source.is_empty() {
            target_source.clone()
        } else {
            std::fs::canonicalize(&target_source)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|_| target_source.clone())
        };
        let edge_target_keys = if normalized_target_source.is_empty() {
            vec![String::new()]
        } else {
            let keys = typescript_source_module_keys(&normalized_target_source, root);
            if keys.is_empty() {
                continue;
            }
            keys
        };
        let context = edge.string("context");
        let key = (importer.clone(), module.clone(), context.clone());
        if !targets.contains_key(&key) && targets.len() >= entry_limit {
            return Err(format!(
                "TypeScript project module entry count exceeds limit {entry_limit}"
            ));
        }
        let values = targets.entry(key).or_default();
        for target_key in &edge_target_keys {
            if !target_key.is_empty() && (values.contains(target_key) || values.len() < max_targets)
            {
                values.insert(target_key.clone());
            }
        }
        let mut edge_metadata = edge
            .attributes
            .iter()
            .filter_map(|(name, value)| {
                matches!(
                    name.as_str(),
                    "resolution_rule"
                        | "package_condition"
                        | "resolution_config"
                        | "module_resolution"
                        | "module_kind"
                        | "resolution_project_references"
                )
                .then(|| {
                    value
                        .as_str()
                        .map(|value| (name.clone(), value.to_owned()))
                        .or_else(|| {
                            (name == "resolution_project_references")
                                .then(|| (name.clone(), value.to_string()))
                        })
                })
                .flatten()
            })
            .collect::<BTreeMap<_, _>>();
        edge_metadata.insert("project_module".to_owned(), module.clone());
        for target_key in edge_target_keys {
            let metadata_key = (
                importer.clone(),
                module.clone(),
                context.clone(),
                target_key,
            );
            match metadata.entry(metadata_key) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(edge_metadata.clone());
                }
                std::collections::hash_map::Entry::Occupied(entry)
                    if entry.get() != &edge_metadata =>
                {
                    entry.remove();
                }
                std::collections::hash_map::Entry::Occupied(_) => {}
            }
        }
    }
    Ok((
        targets
            .into_iter()
            .filter_map(|(key, values)| {
                (!values.is_empty()).then_some((key, values.into_iter().collect()))
            })
            .collect(),
        metadata,
    ))
}

pub(in crate::evidence) fn unique_inventory_source<'a>(
    source_by_node: &'a BTreeMap<String, BTreeSet<String>>,
    node_id: &str,
) -> Option<&'a str> {
    let sources = source_by_node.get(node_id)?;
    let mut values = sources.iter();
    let source = values.next()?;
    values.next().is_none().then_some(source.as_str())
}

pub(in crate::evidence) fn typescript_module_indices(
    declarations: &FactTable<DeclarationFact>,
    declaration_ids: &[String],
    bindings: &FactTable<BindingFact>,
    scopes: &FactTable<compass_languages::ScopeFact>,
    root: &Path,
    project_modules: &TypeScriptProjectModuleIndex,
) -> (
    TypeScriptModuleIndex,
    TypeScriptModuleIndex,
    TypeScriptReexportIndex,
) {
    let mut modules = TypeScriptModuleIndex::new();
    for declaration in declarations
        .values()
        .filter(|declaration| matches!(declaration.language.as_str(), "typescript" | "javascript"))
    {
        let Some(slot) = declaration_slot(declaration_ids, &declaration.id) else {
            continue;
        };
        for module in typescript_source_module_keys(&declaration.range.source_file, root) {
            modules
                .entry((
                    declaration.language.clone(),
                    module.clone(),
                    declaration.name.clone(),
                ))
                .or_default()
                .push(slot);
            if declaration.kind == "module" {
                modules
                    .entry((declaration.language.clone(), module, "module".to_owned()))
                    .or_default()
                    .push(slot);
            }
        }
    }

    let mut export_aliases = TypeScriptModuleIndex::new();
    for binding in bindings.values().filter(|binding| {
        binding.kind == compass_languages::BindingKind::Reexport
            && binding.target_declaration_id.is_some()
    }) {
        let Some(target) = binding.target_declaration_id.as_deref() else {
            continue;
        };
        let Some(slot) = declaration_slot(declaration_ids, target) else {
            continue;
        };
        let Some(owner) = binding
            .scope_id
            .as_deref()
            .and_then(|scope_id| scopes.get(scope_id))
            .and_then(|scope| scope.owner_declaration_id.as_deref())
            .and_then(|declaration_id| declarations.get(declaration_id))
        else {
            continue;
        };
        if !matches!(owner.language.as_str(), "typescript" | "javascript") {
            continue;
        }
        for module in typescript_source_module_keys(&owner.range.source_file, root) {
            export_aliases
                .entry((owner.language.clone(), module, binding.spelling.clone()))
                .or_default()
                .push(slot);
        }
    }
    let mut reexport_targets = TypeScriptReexportIndex::new();
    for binding in bindings.values().filter(|binding| {
        binding.kind == compass_languages::BindingKind::Reexport
            && binding.target_declaration_id.is_none()
    }) {
        let Some((target_module, target_export)) = binding.qualified_target.rsplit_once("::")
        else {
            continue;
        };
        if target_module.is_empty() || target_export.is_empty() {
            continue;
        }
        let Some(owner) = binding
            .scope_id
            .as_deref()
            .and_then(|scope_id| scopes.get(scope_id))
            .and_then(|scope| scope.owner_declaration_id.as_deref())
            .and_then(|declaration_id| declarations.get(declaration_id))
        else {
            continue;
        };
        if !matches!(owner.language.as_str(), "typescript" | "javascript") {
            continue;
        }
        let target_modules = typescript_project_module_keys(
            project_modules,
            &owner.range.source_file,
            target_module,
            root,
            None,
        )
        .unwrap_or_else(|| {
            typescript_import_module_keys(&owner.range.source_file, target_module, root)
        });
        if target_modules.is_empty() {
            continue;
        }
        for owner_module in typescript_source_module_keys(&owner.range.source_file, root) {
            let targets = reexport_targets
                .entry((
                    owner.language.clone(),
                    owner_module,
                    binding.spelling.clone(),
                ))
                .or_default();
            for target_module in &target_modules {
                targets.push(TypeScriptReexportTarget {
                    module: target_module.clone(),
                    exported: target_export.to_owned(),
                });
            }
        }
    }
    for targets in reexport_targets.values_mut() {
        targets.sort_unstable_by(|left, right| {
            left.module
                .cmp(&right.module)
                .then_with(|| left.exported.cmp(&right.exported))
        });
        targets.dedup();
    }
    for index in [&mut modules, &mut export_aliases] {
        sort_declaration_index(index, declaration_ids, usize::MAX);
    }
    (modules, export_aliases, reexport_targets)
}

pub(in crate::evidence) fn typescript_source_module_keys(
    source_file: &str,
    root: &Path,
) -> Vec<String> {
    let Some(relative) = typescript_relative_path(source_file, root) else {
        return Vec::new();
    };
    let Some(module) = typescript_module_stem(&relative) else {
        return Vec::new();
    };
    let mut keys = vec![module.clone()];
    if relative
        .file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("index"))
        && let Some(parent) = module.rsplit_once('/').map(|(parent, _)| parent)
    {
        if !parent.is_empty() {
            keys.push(parent.to_owned());
        }
    } else if relative
        .file_stem()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("index"))
    {
        keys.push(String::new());
    }
    keys.sort();
    keys.dedup();
    keys.retain(|key| !key.is_empty());
    keys
}

pub(in crate::evidence) fn typescript_import_module_keys(
    importer: &str,
    module: &str,
    root: &Path,
) -> Vec<String> {
    let mut keys = Vec::new();
    if module.starts_with('.') {
        if let Some(importer_path) = typescript_relative_path(importer, root) {
            let base = importer_path.parent().unwrap_or_else(|| Path::new(""));
            if let Some(joined) = normalize_relative_module_path(&base.join(module))
                && let Some(key) = typescript_module_stem(&joined)
            {
                keys.push(key);
                if joined
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("index"))
                    && let Some(parent) = keys[0].rsplit_once('/').map(|(parent, _)| parent)
                    && !parent.is_empty()
                {
                    keys.push(parent.to_owned());
                }
            }
        }
    } else if !module.is_empty() && !module.starts_with('#') {
        let path = Path::new(module);
        if let Some(key) = typescript_module_stem(path) {
            keys.push(key);
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

pub(in crate::evidence) fn typescript_relative_path(
    source_file: &str,
    root: &Path,
) -> Option<std::path::PathBuf> {
    let path = Path::new(source_file);
    let relative = if path.is_absolute() {
        path.strip_prefix(root).ok()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    normalize_relative_module_path(&relative)
}

pub(in crate::evidence) fn normalize_relative_module_path(
    path: &Path,
) -> Option<std::path::PathBuf> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                components.pop()?;
            }
            std::path::Component::Normal(value) => components.push(value.to_owned()),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }
    let mut normalized = std::path::PathBuf::new();
    for component in components {
        normalized.push(component);
    }
    Some(normalized)
}

pub(in crate::evidence) fn typescript_module_stem(path: &Path) -> Option<String> {
    let mut value = path.to_string_lossy().replace('\\', "/");
    value = value.trim_start_matches("./").to_owned();
    for extension in [
        ".d.mts", ".d.cts", ".d.ts", ".mts", ".cts", ".tsx", ".jsx", ".mjs", ".cjs", ".ts", ".js",
    ] {
        if let Some(stem) = value.strip_suffix(extension) {
            value = stem.to_owned();
            break;
        }
    }
    while value.ends_with('/') {
        value.pop();
    }
    (!value.is_empty()
        && !value.starts_with('/')
        && !value
            .split('/')
            .any(|component| component.is_empty() || component == ".."))
    .then_some(value)
}

pub(in crate::evidence) fn read_go_module_path(root: &Path) -> Option<String> {
    const MAX_GO_MOD_BYTES: u64 = 1024 * 1024;
    let source = compass_files::read_source_lossy(&root.join("go.mod"), MAX_GO_MOD_BYTES).ok()?;
    source.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()? != "module" {
            return None;
        }
        let module = fields.next()?;
        if fields.next().is_some()
            || module.len() > 4096
            || module.starts_with('.')
            || module.contains(['\\', '\0'])
            || module
                .split('/')
                .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return None;
        }
        Some(module.to_owned())
    })
}
