//! Deterministic cross-file resolution over immutable extraction facts.

mod members;

pub use members::resolve_language_calls;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use trail_languages::{Extraction, RawCall, make_id};
use trail_model::{EdgeRecord, NodeRecord};

/// Merge per-file facts in source order, then resolve shared cross-file calls.
#[must_use]
pub fn resolve(extractions: &[Extraction], sources: &HashMap<String, String>) -> Extraction {
    resolve_with_root(extractions, sources, Path::new("."))
}

/// Merge and resolve facts with an explicit corpus root for portable collision salts.
#[must_use]
pub fn resolve_with_root(
    extractions: &[Extraction],
    sources: &HashMap<String, String>,
    root: &Path,
) -> Extraction {
    let mut merged = Extraction::default();
    for extraction in extractions {
        merged.nodes.extend(extraction.nodes.iter().cloned());
        merged.edges.extend(extraction.edges.iter().cloned());
        merged
            .hyperedges
            .extend(extraction.hyperedges.iter().cloned());
        if let Some(raw_calls) = &extraction.raw_calls {
            merged
                .raw_calls
                .get_or_insert_with(Vec::new)
                .extend(raw_calls.iter().cloned());
        }
        merged.extensions.extend(extraction.extensions.clone());
    }
    canonicalize_import_targets(&mut merged);
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    disambiguate_colliding_node_ids(&mut merged, &canonical_root);
    rewire_unique_stub_nodes(&mut merged);
    resolve_cross_file_calls(&mut merged, sources);
    members::resolve_language_calls(extractions, &mut merged);
    merged
}

fn canonicalize_import_targets(extraction: &mut Extraction) {
    let aliases = extraction
        .nodes
        .iter()
        .filter_map(|node| {
            let source = string_attribute(node, "source_file");
            is_file_node(node, &source).then_some((make_id(&[&source]), node.id.clone()))
        })
        .collect::<HashMap<_, _>>();
    for edge in &mut extraction.edges {
        if matches!(relation(edge), "imports" | "imports_from" | "re_exports")
            && let Some(target) = aliases.get(&edge.target)
        {
            edge.target.clone_from(target);
        }
    }
}

fn rewire_unique_stub_nodes(extraction: &mut Extraction) {
    let mut types = HashMap::<String, Vec<String>>::new();
    let mut functions = HashMap::<String, Vec<String>>::new();
    let mut stubs = Vec::<(String, String)>::new();
    for node in &extraction.nodes {
        let label = node
            .label()
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches('.')
            .to_owned();
        if label.is_empty() {
            continue;
        }
        let source = string_attribute(node, "source_file");
        if source.is_empty() {
            stubs.push((node.id.clone(), label));
        } else if is_type_like_definition(node) {
            types.entry(label).or_default().push(node.id.clone());
        } else if node.label().ends_with("()") && !node.label().starts_with('.') {
            functions.entry(label).or_default().push(node.id.clone());
        }
    }
    let supertype_stubs = extraction
        .edges
        .iter()
        .filter(|edge| matches!(relation(edge), "inherits" | "implements" | "extends"))
        .map(|edge| edge.target.as_str())
        .collect::<HashSet<_>>();
    let mut remap = HashMap::new();
    for (stub, label) in stubs {
        let candidates = types
            .get(&label)
            .filter(|items| items.len() == 1)
            .or_else(|| {
                (!supertype_stubs.contains(stub.as_str()))
                    .then(|| functions.get(&label).filter(|items| items.len() == 1))
                    .flatten()
            });
        if let Some(target) = candidates.and_then(|items| items.first())
            && target != &stub
        {
            remap.insert(stub, target.clone());
        }
    }
    if remap.is_empty() {
        return;
    }
    for edge in &mut extraction.edges {
        if let Some(target) = remap.get(&edge.source) {
            edge.source.clone_from(target);
        }
        if let Some(target) = remap.get(&edge.target) {
            edge.target.clone_from(target);
        }
    }
    let referenced = extraction
        .edges
        .iter()
        .flat_map(|edge| [&edge.source, &edge.target])
        .collect::<HashSet<_>>();
    extraction
        .nodes
        .retain(|node| !remap.contains_key(&node.id) || referenced.contains(&node.id));
}

fn is_type_like_definition(node: &NodeRecord) -> bool {
    if string_attribute(node, "type") == "namespace"
        || string_attribute(node, "file_type") != "code"
    {
        return false;
    }
    let label = node.label().trim();
    !label.is_empty() && !label.ends_with(')') && !label.starts_with('.') && !label.contains('.')
}

fn disambiguate_colliding_node_ids(extraction: &mut Extraction, root: &Path) {
    let mut groups = HashMap::<String, Vec<usize>>::new();
    for (index, node) in extraction.nodes.iter().enumerate() {
        if matches!(
            string_attribute(node, "type").as_str(),
            "module" | "namespace"
        ) {
            continue;
        }
        if !node.id.is_empty() {
            groups.entry(node.id.clone()).or_default().push(index);
        }
    }
    let mut remap = HashMap::<(String, String), String>::new();
    let mut ambiguous = HashSet::new();
    for (old_id, indexes) in &groups {
        let source_keys = indexes
            .iter()
            .map(|index| node_source_key(&extraction.nodes[*index], root))
            .collect::<HashSet<_>>();
        if indexes.len() < 2 || source_keys.len() < 2 {
            continue;
        }
        ambiguous.insert(old_id.clone());
        let naive = source_keys
            .iter()
            .filter(|key| !key.is_empty())
            .map(|key| (key.clone(), make_id(&[key, old_id])))
            .collect::<HashMap<_, _>>();
        let mut counts = HashMap::<String, usize>::new();
        for value in naive.values() {
            *counts.entry(value.clone()).or_default() += 1;
        }
        for index in indexes {
            let source_key = node_source_key(&extraction.nodes[*index], root);
            if source_key.is_empty() {
                continue;
            }
            let naive_id = naive
                .get(&source_key)
                .cloned()
                .unwrap_or_else(|| make_id(&[&source_key, old_id]));
            let new_id = if counts.get(&naive_id).copied().unwrap_or_default() > 1 {
                let digest = Sha1::digest(source_key.as_bytes());
                let salt = format!("{digest:x}");
                make_id(&[&source_key, old_id, &salt[..6]])
            } else {
                naive_id
            };
            remap.insert((old_id.clone(), source_key), new_id.clone());
            extraction.nodes[*index].id = new_id;
        }
    }
    if remap.is_empty() {
        for edge in &mut extraction.edges {
            edge.attributes.remove("target_file");
        }
        return;
    }
    let mut header_remaps = HashMap::new();
    for old_id in &ambiguous {
        if let Some(indexes) = groups.get(old_id) {
            for index in indexes {
                let key = node_source_key(&extraction.nodes[*index], root);
                if matches!(
                    Path::new(&key)
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("h" | "hpp" | "hh" | "hxx")
                ) && let Some(new_id) = remap.get(&(old_id.clone(), key))
                {
                    header_remaps.insert(old_id.clone(), new_id.clone());
                    break;
                }
            }
        }
    }
    for edge in &mut extraction.edges {
        let edge_key = source_key(&edge.string("source_file"), root);
        if let Some(new_id) = remap.get(&(edge.source.clone(), edge_key.clone())) {
            edge.source.clone_from(new_id);
        }
        let target_file = edge
            .attributes
            .remove("target_file")
            .and_then(|value| value.as_str().map(str::to_owned));
        let relation = relation(edge);
        let target_key = if matches!(relation, "imports" | "imports_from" | "re_exports") {
            target_file
                .as_deref()
                .map_or(edge_key, |path| source_key(path, root))
        } else {
            edge_key
        };
        if matches!(relation, "imports" | "imports_from")
            && let Some(new_id) = header_remaps.get(&edge.target)
        {
            edge.target.clone_from(new_id);
        } else if let Some(new_id) = remap.get(&(edge.target.clone(), target_key)) {
            edge.target.clone_from(new_id);
        }
    }
    if let Some(raw_calls) = extraction.raw_calls.as_mut() {
        for raw in raw_calls {
            let key = source_key(&raw.source_file, root);
            if let Some(new_id) = remap.get(&(raw.caller_nid.clone(), key)) {
                raw.caller_nid.clone_from(new_id);
            }
        }
    }
}

fn node_source_key(node: &NodeRecord, root: &Path) -> String {
    let source = string_attribute(node, "source_file");
    if source.is_empty() {
        source_key(&string_attribute(node, "origin_file"), root)
    } else {
        source_key(&source, root)
    }
}

fn source_key(source: &str, root: &Path) -> String {
    if source.is_empty() {
        return String::new();
    }
    let path = Path::new(source);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    absolute.strip_prefix(root).map_or_else(
        |_| path.to_string_lossy().replace('\\', "/"),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    )
}

/// Resolve non-member raw calls using unique definitions and import evidence.
pub fn resolve_cross_file_calls(extraction: &mut Extraction, sources: &HashMap<String, String>) {
    resolve_python_import_guided(extraction, sources);
    resolve_python_class_uses(extraction, sources);
    let mut exact = HashMap::<String, Vec<String>>::new();
    let mut folded = HashMap::<String, Vec<String>>::new();
    let mut source_by_id = HashMap::<String, String>::new();
    let mut file_by_source = HashMap::<String, String>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        source_by_id.insert(node.id.clone(), source.clone());
        if is_file_node(node, &source) {
            file_by_source
                .entry(source.clone())
                .or_insert_with(|| node.id.clone());
        }
        if string_attribute(node, "file_type") == "rationale"
            || string_attribute(node, "type") == "namespace"
        {
            continue;
        }
        let label = node
            .label()
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches('.')
            .to_owned();
        if label.is_empty() {
            continue;
        }
        exact
            .entry(label.clone())
            .or_default()
            .push(node.id.clone());
        if case_insensitive(&source) {
            folded
                .entry(label.to_lowercase())
                .or_default()
                .push(node.id.clone());
        }
    }

    let file_by_id = source_by_id
        .iter()
        .filter_map(|(id, source)| {
            file_by_source
                .get(source)
                .map(|file_id| (id.clone(), file_id.clone()))
        })
        .collect::<HashMap<_, _>>();
    let mut symbol_imports = HashMap::<String, HashSet<String>>::new();
    let mut module_imports = HashMap::<String, HashSet<String>>::new();
    let mut existing = HashSet::new();
    let mut call_like = HashSet::new();
    for edge in &extraction.edges {
        existing.insert((edge.source.clone(), edge.target.clone()));
        if matches!(relation(edge), "calls" | "indirect_call") {
            call_like.insert((edge.source.clone(), edge.target.clone()));
        }
        match relation(edge) {
            "imports" => {
                symbol_imports
                    .entry(edge.source.clone())
                    .or_default()
                    .insert(edge.target.clone());
            }
            "imports_from" => {
                module_imports
                    .entry(edge.source.clone())
                    .or_default()
                    .insert(edge.target.clone());
            }
            _ => {}
        }
    }

    let raw_calls = extraction.raw_calls.clone().unwrap_or_default();
    for raw in raw_calls {
        if raw.callee.is_empty()
            || raw.is_member_call == Some(true)
            || is_builtin(&raw.callee)
            || raw.extensions.get("is_mixin").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let candidates = candidate_calls(&raw, &exact, &folded, &source_by_id);
        if candidates.is_empty() {
            continue;
        }
        let caller_file = file_by_source
            .get(&raw.source_file)
            .or_else(|| file_by_id.get(&raw.caller_nid));
        let imported_symbols = caller_file.and_then(|id| symbol_imports.get(id));
        let imported_modules = caller_file.and_then(|id| module_imports.get(id));
        let selection = select_candidate(
            &candidates,
            imported_symbols,
            imported_modules,
            &file_by_id,
            &source_by_id,
            &raw.source_file,
        );
        let Some((target, import_evidence)) = selection else {
            continue;
        };
        if raw
            .extensions
            .get("symbol_import_use")
            .and_then(Value::as_bool)
            == Some(true)
            && !imported_symbols.is_some_and(|imports| imports.contains(&target))
        {
            continue;
        }
        let indirect = raw.extensions.get("indirect").and_then(Value::as_bool) == Some(true);
        if indirect {
            if target != raw.caller_nid
                && extraction.nodes.iter().any(|node| {
                    node.id == target
                        && node.attributes.get("_callable").and_then(Value::as_bool) == Some(true)
                })
                && call_like.insert((raw.caller_nid.clone(), target.clone()))
            {
                let mut edge = resolved_edge(&raw, &target, "INFERRED", 0.8);
                edge.attributes.insert(
                    "relation".to_owned(),
                    Value::String("indirect_call".to_owned()),
                );
                edge.attributes.insert(
                    "context".to_owned(),
                    raw.extensions
                        .get("context")
                        .cloned()
                        .unwrap_or_else(|| Value::String("argument".to_owned())),
                );
                extraction.edges.push(edge);
            }
            continue;
        }
        if target == raw.caller_nid || (!import_evidence && is_javascript(&raw.source_file)) {
            continue;
        }
        if existing.insert((raw.caller_nid.clone(), target.clone())) {
            let mut edge = resolved_edge(
                &raw,
                &target,
                if import_evidence {
                    "EXTRACTED"
                } else {
                    "INFERRED"
                },
                if import_evidence { 1.0 } else { 0.8 },
            );
            if raw
                .extensions
                .get("symbol_import_use")
                .and_then(Value::as_bool)
                == Some(true)
            {
                edge.attributes.remove("confidence_score");
            }
            extraction.edges.push(edge);
        }
    }
}

fn resolve_python_import_guided(extraction: &mut Extraction, sources: &HashMap<String, String>) {
    let mut definitions = HashMap::<String, Vec<(String, String)>>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if extension(&source) != "py" {
            continue;
        }
        let label = node
            .label()
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches('.')
            .to_owned();
        definitions
            .entry(label)
            .or_default()
            .push((source, node.id.clone()));
    }
    let mut known = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                relation(edge).to_owned(),
            )
        })
        .collect::<HashSet<_>>();
    let file_nodes = extraction
        .nodes
        .iter()
        .filter_map(|node| {
            let source = string_attribute(node, "source_file");
            is_file_node(node, &source).then_some((source, node.id.clone()))
        })
        .collect::<HashMap<_, _>>();
    for (source_file, source) in sources {
        if extension(source_file) != "py" {
            continue;
        }
        let Some(file_node) = file_nodes.get(source_file) else {
            continue;
        };
        for imported in python_symbol_imports(source) {
            let candidates = python_definition_candidates(
                Path::new(source_file),
                &imported.module,
                &imported.imported,
                &definitions,
                false,
            );
            if candidates.len() != 1 {
                continue;
            }
            let target = &candidates[0];
            if !known.insert((file_node.clone(), target.clone(), "imports".to_owned())) {
                continue;
            }
            let mut attributes = Map::new();
            attributes.insert("relation".to_owned(), Value::String("imports".to_owned()));
            attributes.insert("context".to_owned(), Value::String("import".to_owned()));
            attributes.insert(
                "confidence".to_owned(),
                Value::String("EXTRACTED".to_owned()),
            );
            attributes.insert("source_file".to_owned(), Value::String(source_file.clone()));
            attributes.insert(
                "source_location".to_owned(),
                Value::String(format!("L{}", imported.line)),
            );
            attributes.insert("weight".to_owned(), Value::from(1.0));
            extraction.edges.push(EdgeRecord {
                source: file_node.clone(),
                target: target.clone(),
                attributes,
            });
        }
    }
    let raw_calls = extraction.raw_calls.clone().unwrap_or_default();
    let aliases_by_source = sources
        .iter()
        .filter(|(source_file, _)| extension(source_file) == "py")
        .map(|(source_file, source)| (source_file.as_str(), python_import_aliases(source)))
        .collect::<HashMap<_, _>>();
    for raw in raw_calls {
        if raw.is_member_call == Some(true)
            || extension(&raw.source_file) != "py"
            || raw.extensions.get("indirect").and_then(Value::as_bool) == Some(true)
            || raw
                .extensions
                .get("symbol_import_use")
                .and_then(Value::as_bool)
                == Some(true)
        {
            continue;
        }
        let Some(aliases) = aliases_by_source.get(raw.source_file.as_str()) else {
            continue;
        };
        let Some((module, imported)) = aliases.get(&raw.callee) else {
            continue;
        };
        let candidates = python_definition_candidates(
            Path::new(&raw.source_file),
            module,
            imported,
            &definitions,
            false,
        );
        if candidates.len() != 1 {
            continue;
        }
        let target = &candidates[0];
        if target == &raw.caller_nid
            || !known.insert((raw.caller_nid.clone(), target.clone(), "calls".to_owned()))
        {
            continue;
        }
        let mut edge = resolved_edge(&raw, target, "EXTRACTED", 1.0);
        edge.attributes.remove("confidence_score");
        extraction.edges.push(edge);
    }
}

fn resolve_python_class_uses(extraction: &mut Extraction, sources: &HashMap<String, String>) {
    let mut definitions = HashMap::<String, Vec<(String, String)>>::new();
    let mut local_classes = HashMap::<String, Vec<String>>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        if extension(&source) != "py" {
            continue;
        }
        let label = node.label();
        if !label.is_empty()
            && !label.ends_with(')')
            && !label.ends_with(".py")
            && !label.starts_with('_')
            && string_attribute(node, "file_type") != "rationale"
        {
            definitions
                .entry(label.to_owned())
                .or_default()
                .push((source.clone(), node.id.clone()));
        }
        if !label.ends_with(')')
            && !label.ends_with(".py")
            && string_attribute(node, "file_type") != "rationale"
            && !is_file_node(node, &source)
        {
            local_classes
                .entry(source)
                .or_default()
                .push(node.id.clone());
        }
    }
    let mut known = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                relation(edge).to_owned(),
            )
        })
        .collect::<HashSet<_>>();
    for (source_file, source) in sources {
        if extension(source_file) != "py" {
            continue;
        }
        let Some(classes) = local_classes.get(source_file) else {
            continue;
        };
        for imported in python_symbol_imports(source) {
            let candidates = python_definition_candidates(
                Path::new(source_file),
                &imported.module,
                &imported.imported,
                &definitions,
                true,
            );
            if candidates.len() != 1 {
                continue;
            }
            for source_id in classes {
                let target = &candidates[0];
                if !known.insert((source_id.clone(), target.clone(), "uses".to_owned())) {
                    continue;
                }
                let mut attributes = Map::new();
                attributes.insert("relation".to_owned(), Value::String("uses".to_owned()));
                attributes.insert(
                    "confidence".to_owned(),
                    Value::String("INFERRED".to_owned()),
                );
                attributes.insert("source_file".to_owned(), Value::String(source_file.clone()));
                attributes.insert(
                    "source_location".to_owned(),
                    Value::String(format!("L{}", imported.line)),
                );
                attributes.insert("weight".to_owned(), Value::from(0.8));
                extraction.edges.push(EdgeRecord {
                    source: source_id.clone(),
                    target: target.clone(),
                    attributes,
                });
            }
        }
    }
}

fn python_import_aliases(source: &str) -> HashMap<String, (String, String)> {
    python_symbol_imports(source)
        .into_iter()
        .map(|import| (import.local, (import.module, import.imported)))
        .collect()
}

struct PythonImport {
    module: String,
    imported: String,
    local: String,
    line: usize,
}

fn python_symbol_imports(source: &str) -> Vec<PythonImport> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let Some(rest) = line.trim().strip_prefix("from ") else {
            index += 1;
            continue;
        };
        let Some((module, first_imports)) = rest.split_once(" import ") else {
            index += 1;
            continue;
        };
        let start_line = index + 1;
        let mut imports = first_imports.to_owned();
        while imports.contains('(') && !imports.contains(')') && index + 1 < lines.len() {
            index += 1;
            imports.push(' ');
            imports.push_str(lines[index].trim());
        }
        for item in imports
            .trim_matches(['(', ')'])
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty() && *item != "*")
        {
            let item = item.split('#').next().unwrap_or_default().trim();
            let (imported, local) = item
                .split_once(" as ")
                .map_or((item, item), |(imported, local)| {
                    (imported.trim(), local.trim())
                });
            if !imported.is_empty() && !local.is_empty() {
                output.push(PythonImport {
                    module: module.to_owned(),
                    imported: imported.to_owned(),
                    local: local.to_owned(),
                    line: start_line,
                });
            }
        }
        index += 1;
    }
    output
}

fn python_definition_candidates(
    caller: &Path,
    module: &str,
    imported: &str,
    definitions: &HashMap<String, Vec<(String, String)>>,
    allow_module_tail: bool,
) -> Vec<String> {
    let bare_module = module.trim_start_matches('.');
    let module_tail = bare_module.rsplit('.').next().unwrap_or_default();
    let relative_candidate = caller
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{}.py", bare_module.replace('.', "/")));
    let mut output = Vec::new();
    for (source, id) in definitions.get(imported).into_iter().flatten() {
        let source_path = Path::new(source);
        let exact_relative = source_path == relative_candidate;
        let matching_stem =
            source_path.file_stem().and_then(|value| value.to_str()) == Some(module_tail);
        if exact_relative
            || (!module.starts_with('.')
                && (!module.contains('.') || allow_module_tail)
                && matching_stem)
        {
            output.push(id.clone());
        }
    }
    output
}

fn candidate_calls(
    raw: &RawCall,
    exact: &HashMap<String, Vec<String>>,
    folded: &HashMap<String, Vec<String>>,
    source_by_id: &HashMap<String, String>,
) -> Vec<String> {
    let mut candidates = exact.get(&raw.callee).cloned().unwrap_or_default();
    if candidates.is_empty() && case_insensitive(&raw.source_file) {
        candidates = folded
            .get(&raw.callee.to_lowercase())
            .cloned()
            .unwrap_or_default();
    }
    if let Some(family) = language_family(&raw.source_file) {
        candidates.retain(|candidate| {
            source_by_id
                .get(candidate)
                .and_then(|source| language_family(source))
                .is_none_or(|candidate_family| candidate_family == family)
        });
    }
    candidates
}

fn select_candidate(
    candidates: &[String],
    symbol_imports: Option<&HashSet<String>>,
    module_imports: Option<&HashSet<String>>,
    file_by_id: &HashMap<String, String>,
    source_by_id: &HashMap<String, String>,
    call_site_file: &str,
) -> Option<(String, bool)> {
    if candidates.len() == 1 {
        let candidate = candidates[0].clone();
        let evidence = symbol_imports.is_some_and(|imports| imports.contains(&candidate))
            || file_by_id
                .get(&candidate)
                .is_some_and(|file| module_imports.is_some_and(|imports| imports.contains(file)));
        return Some((candidate, evidence));
    }
    let symbol_matches = candidates
        .iter()
        .filter(|candidate| symbol_imports.is_some_and(|imports| imports.contains(*candidate)))
        .cloned()
        .collect::<Vec<_>>();
    if symbol_matches.len() == 1 {
        return Some((symbol_matches[0].clone(), true));
    }
    let module_matches = candidates
        .iter()
        .filter(|candidate| {
            file_by_id
                .get(*candidate)
                .is_some_and(|file| module_imports.is_some_and(|imports| imports.contains(file)))
        })
        .cloned()
        .collect::<Vec<_>>();
    if module_matches.len() == 1 {
        return Some((module_matches[0].clone(), true));
    }
    disambiguate_candidates(candidates, source_by_id, call_site_file)
        .map(|candidate| (candidate, false))
}

fn disambiguate_candidates(
    candidates: &[String],
    source_by_id: &HashMap<String, String>,
    call_site_file: &str,
) -> Option<String> {
    if candidates.len() == 1 {
        return candidates.first().cloned();
    }
    let call_is_test = is_test_path(call_site_file);
    let test_candidates = candidates
        .iter()
        .filter(|candidate| {
            source_by_id
                .get(*candidate)
                .is_some_and(|path| is_test_path(path))
        })
        .cloned()
        .collect::<Vec<_>>();
    let test_set = test_candidates.iter().collect::<HashSet<_>>();
    let non_test_candidates = candidates
        .iter()
        .filter(|candidate| !test_set.contains(candidate))
        .cloned()
        .collect::<Vec<_>>();
    let survivors = if call_is_test {
        let normalized_call = normalize_path(call_site_file);
        let same_file = test_candidates
            .iter()
            .filter(|candidate| {
                source_by_id
                    .get(*candidate)
                    .is_some_and(|path| normalize_path(path) == normalized_call)
            })
            .cloned()
            .collect::<Vec<_>>();
        if same_file.len() == 1 {
            return same_file.first().cloned();
        }
        if test_candidates.is_empty() {
            if non_test_candidates.is_empty() {
                candidates.to_vec()
            } else {
                non_test_candidates
            }
        } else {
            test_candidates
        }
    } else {
        non_test_candidates
    };
    if survivors.len() == 1 {
        return survivors.first().cloned();
    }
    path_proximity(&survivors, source_by_id, call_site_file)
}

fn path_proximity(
    candidates: &[String],
    source_by_id: &HashMap<String, String>,
    call_site_file: &str,
) -> Option<String> {
    if call_site_file.is_empty() {
        return None;
    }
    let call = normalize_path(call_site_file);
    let call_dir = parent_segments(&call);
    let same_file = candidates
        .iter()
        .filter(|candidate| {
            source_by_id
                .get(*candidate)
                .is_some_and(|path| normalize_path(path) == call)
        })
        .cloned()
        .collect::<Vec<_>>();
    if same_file.len() == 1 {
        return same_file.first().cloned();
    }
    if same_file.len() > 1 {
        return None;
    }
    let same_dir = candidates
        .iter()
        .filter(|candidate| {
            source_by_id
                .get(*candidate)
                .is_some_and(|path| parent_segments(&normalize_path(path)) == call_dir)
        })
        .cloned()
        .collect::<Vec<_>>();
    if same_dir.len() == 1 {
        return same_dir.first().cloned();
    }
    if same_dir.len() > 1 {
        return None;
    }
    let scores = candidates
        .iter()
        .map(|candidate| {
            let parts = source_by_id
                .get(candidate)
                .map(|path| parent_segments(&normalize_path(path)))
                .unwrap_or_default();
            let score = call_dir
                .iter()
                .zip(parts.iter())
                .take_while(|(left, right)| left == right)
                .count();
            (candidate, score)
        })
        .collect::<Vec<_>>();
    let best = scores.iter().map(|(_, score)| *score).max()?;
    let winners = scores
        .iter()
        .filter(|(_, score)| *score == best)
        .collect::<Vec<_>>();
    (best > 0 && winners.len() == 1).then(|| (*winners[0].0).clone())
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn parent_segments(path: &str) -> Vec<String> {
    path.rsplit_once('/').map_or_else(Vec::new, |(parent, _)| {
        parent.split('/').map(str::to_owned).collect()
    })
}

fn is_test_path(path: &str) -> bool {
    let normalized = normalize_path(path);
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts.iter().any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "tests" | "test" | "spec" | "specs" | "__tests__"
        )
    }) {
        return true;
    }
    let filename = parts.last().copied().unwrap_or_default();
    let folded = filename.to_ascii_lowercase();
    folded.starts_with("test_")
        || folded.contains("_test.")
        || folded.contains(".test.")
        || folded.contains(".spec.")
        || folded.contains("_spec.")
        || folded.ends_with(".tests.ps1")
        || filename.ends_with("Test.java")
        || filename.ends_with("Tests.java")
        || filename.ends_with("Tests.cs")
}

fn resolved_edge(raw: &RawCall, target: &str, confidence: &str, score: f64) -> EdgeRecord {
    let mut attributes = Map::new();
    attributes.insert("relation".to_owned(), Value::String("calls".to_owned()));
    attributes.insert("context".to_owned(), Value::String("call".to_owned()));
    attributes.insert(
        "confidence".to_owned(),
        Value::String(confidence.to_owned()),
    );
    attributes.insert("confidence_score".to_owned(), Value::from(score));
    attributes.insert(
        "source_file".to_owned(),
        Value::String(raw.source_file.clone()),
    );
    attributes.insert(
        "source_location".to_owned(),
        Value::String(raw.source_location.clone()),
    );
    attributes.insert("weight".to_owned(), Value::from(1.0));
    EdgeRecord {
        source: raw.caller_nid.clone(),
        target: target.to_owned(),
        attributes,
    }
}

fn string_attribute(node: &NodeRecord, key: &str) -> String {
    node.attributes
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn is_file_node(node: &NodeRecord, source: &str) -> bool {
    !source.is_empty()
        && Path::new(source)
            .file_name()
            .and_then(|value| value.to_str())
            == Some(node.label())
}

fn relation(edge: &EdgeRecord) -> &str {
    edge.attributes
        .get("relation")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn case_insensitive(source: &str) -> bool {
    matches!(
        extension(source).as_str(),
        "php" | "phtml" | "php3" | "php4" | "php5" | "php7" | "phps" | "sql"
    )
}

fn is_javascript(source: &str) -> bool {
    matches!(
        extension(source).as_str(),
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs"
    )
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Number"
            | "Boolean"
            | "Object"
            | "Array"
            | "Symbol"
            | "BigInt"
            | "Date"
            | "RegExp"
            | "Error"
            | "TypeError"
            | "RangeError"
            | "SyntaxError"
            | "ReferenceError"
            | "EvalError"
            | "URIError"
            | "Promise"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "JSON"
            | "Math"
            | "Reflect"
            | "Proxy"
            | "Intl"
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "isFinite"
            | "encodeURIComponent"
            | "decodeURIComponent"
            | "encodeURI"
            | "decodeURI"
            | "URL"
            | "URLSearchParams"
            | "FormData"
            | "Blob"
            | "File"
            | "Headers"
            | "Request"
            | "Response"
            | "AbortController"
            | "AbortSignal"
            | "TextEncoder"
            | "TextDecoder"
            | "console"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "bytes"
            | "len"
            | "range"
            | "enumerate"
            | "zip"
            | "map"
            | "filter"
            | "sum"
            | "min"
            | "max"
            | "print"
            | "open"
            | "isinstance"
            | "type"
            | "super"
            | "sorted"
            | "reversed"
            | "any"
            | "all"
            | "abs"
            | "round"
            | "next"
            | "iter"
            | "hash"
            | "id"
            | "repr"
            | "callable"
            | "getattr"
            | "setattr"
            | "hasattr"
            | "delattr"
            | "vars"
            | "dir"
    )
}

fn language_family(source: &str) -> Option<&'static str> {
    match extension(source).as_str() {
        "py" | "pyi" => Some("py"),
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte"
        | "astro" => Some("js"),
        "java" | "kt" | "kts" | "scala" | "groovy" | "gradle" => Some("jvm"),
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "cu" | "cuh" | "metal" | "m" | "mm"
        | "swift" => Some("native"),
        "go" => Some("go"),
        "rs" => Some("rs"),
        "rb" | "rake" => Some("rb"),
        "php" => Some("php"),
        "cs" => Some("cs"),
        "lua" | "luau" => Some("lua"),
        "razor" | "cshtml" | "xaml" => Some("cs"),
        "zig" => Some("zig"),
        "ex" | "exs" => Some("elixir"),
        "jl" => Some("julia"),
        "dart" => Some("dart"),
        "sh" | "bash" => Some("shell"),
        "ps1" | "psm1" | "psd1" => Some("powershell"),
        _ => None,
    }
}

fn extension(source: &str) -> String {
    Path::new(source)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, label: &str, source_file: &str, kind: &str) -> NodeRecord {
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(source_file.to_owned()),
        );
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        attributes.insert("type".to_owned(), Value::String(kind.to_owned()));
        NodeRecord {
            id: id.to_owned(),
            attributes,
        }
    }

    fn edge(source: &str, target: &str, relation: &str, source_file: &str) -> EdgeRecord {
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(source_file.to_owned()),
        );
        EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        }
    }

    fn raw(caller: &str, callee: &str, source_file: &str) -> RawCall {
        RawCall {
            caller_nid: caller.to_owned(),
            callee: callee.to_owned(),
            is_member_call: Some(false),
            source_file: source_file.to_owned(),
            source_location: "L7".to_owned(),
            receiver: None,
            receiver_type: None,
            lang: None,
            extensions: Map::new(),
        }
    }

    #[test]
    fn python_import_parser_handles_aliases_multiline_comments_and_wildcards() {
        let imports = python_symbol_imports(
            "from pkg.api import (\n  Widget as LocalWidget,\n  helper, # kept\n  *,\n)\nfrom invalid\nimport os\n",
        );
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].module, "pkg.api");
        assert_eq!(imports[0].imported, "Widget");
        assert_eq!(imports[0].local, "LocalWidget");
        assert_eq!(imports[0].line, 1);
        assert_eq!(imports[1].imported, "helper");
        let aliases = python_import_aliases("from lib import run as execute");
        assert_eq!(
            aliases.get("execute"),
            Some(&("lib".to_owned(), "run".to_owned()))
        );
    }

    #[test]
    fn python_definition_matching_respects_relative_and_module_tail_rules() {
        let definitions = HashMap::from([(
            "Widget".to_owned(),
            vec![
                ("app/models.py".to_owned(), "models-widget".to_owned()),
                ("other/models.py".to_owned(), "other-widget".to_owned()),
            ],
        )]);
        assert_eq!(
            python_definition_candidates(
                Path::new("app/use.py"),
                ".models",
                "Widget",
                &definitions,
                false,
            ),
            vec!["models-widget"]
        );
        assert_eq!(
            python_definition_candidates(
                Path::new("app/use.py"),
                "pkg.models",
                "Widget",
                &definitions,
                false,
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            python_definition_candidates(
                Path::new("app/use.py"),
                "pkg.models",
                "Widget",
                &definitions,
                true,
            )
            .len(),
            2
        );
    }

    #[test]
    fn candidate_disambiguation_prefers_imports_tests_and_nearby_paths() {
        let candidates = vec!["prod".to_owned(), "test".to_owned()];
        let sources = HashMap::from([
            ("prod".to_owned(), "src/service.py".to_owned()),
            ("test".to_owned(), "tests/test_service.py".to_owned()),
        ]);
        assert_eq!(
            disambiguate_candidates(&candidates, &sources, "tests/test_service.py"),
            Some("test".to_owned())
        );
        assert_eq!(
            disambiguate_candidates(&candidates, &sources, "src/caller.py"),
            Some("prod".to_owned())
        );

        let nearby = vec!["same-dir".to_owned(), "far".to_owned()];
        let nearby_sources = HashMap::from([
            ("same-dir".to_owned(), "src/api/helper.py".to_owned()),
            ("far".to_owned(), "vendor/helper.py".to_owned()),
        ]);
        assert_eq!(
            path_proximity(&nearby, &nearby_sources, "src/api/caller.py"),
            Some("same-dir".to_owned())
        );
        assert_eq!(path_proximity(&nearby, &nearby_sources, ""), None);

        let symbols = HashSet::from(["far".to_owned()]);
        assert_eq!(
            select_candidate(
                &nearby,
                Some(&symbols),
                None,
                &HashMap::new(),
                &nearby_sources,
                "src/api/caller.py",
            ),
            Some(("far".to_owned(), true))
        );
    }

    #[test]
    fn path_and_language_classifiers_cover_supported_families() {
        for path in [
            "tests/a.py",
            "src/test_a.py",
            "src/a_test.go",
            "src/a.spec.ts",
            "src/WidgetTests.cs",
            "spec/unit.rb",
        ] {
            assert!(is_test_path(path), "{path}");
        }
        assert!(!is_test_path("src/widget.rs"));
        assert_eq!(normalize_path(r"src\api\x.py"), "src/api/x.py");
        assert_eq!(parent_segments("src/api/x.py"), vec!["src", "api"]);
        assert_eq!(language_family("x.tsx"), Some("js"));
        assert_eq!(language_family("x.hpp"), Some("native"));
        assert_eq!(language_family("x.psm1"), Some("powershell"));
        assert_eq!(language_family("README"), None);
        assert!(case_insensitive("query.SQL"));
        assert!(is_javascript("view.mjs"));
        assert!(is_builtin("Promise"));
        assert!(!is_builtin("project_function"));
    }

    #[test]
    fn resolve_adds_import_guided_calls_and_class_uses() {
        let file_a = make_id(&["app/a.py"]);
        let file_b = make_id(&["app/b.py"]);
        let extraction = Extraction {
            nodes: vec![
                node(&file_a, "a.py", "app/a.py", "file"),
                node(&file_b, "b.py", "app/b.py", "file"),
                node("caller", "caller()", "app/a.py", "function"),
                node("local-class", "Local", "app/a.py", "class"),
                node("helper", "helper()", "app/b.py", "function"),
                node("widget", "Widget", "app/b.py", "class"),
            ],
            raw_calls: Some(vec![raw("caller", "run", "app/a.py")]),
            ..Extraction::default()
        };
        let sources = HashMap::from([(
            "app/a.py".to_owned(),
            "from .b import helper as run\nfrom .b import Widget\nrun()\n".to_owned(),
        )]);
        let resolved = resolve(&[extraction], &sources);
        assert!(resolved.edges.iter().any(|candidate| {
            candidate.source == "caller"
                && candidate.target == "helper"
                && relation(candidate) == "calls"
                && candidate.string("confidence") == "EXTRACTED"
        }));
        assert!(resolved.edges.iter().any(|candidate| {
            candidate.source == "local-class"
                && candidate.target == "widget"
                && relation(candidate) == "uses"
        }));
        assert!(resolved.edges.iter().any(|candidate| {
            candidate.source == file_a
                && candidate.target == "helper"
                && relation(candidate) == "imports"
        }));
    }

    #[test]
    fn cross_file_resolution_filters_builtins_members_mixins_and_javascript_guesses() {
        let mut callable = node("target", "work()", "src/b.py", "function");
        callable
            .attributes
            .insert("_callable".to_owned(), Value::Bool(true));
        let mut indirect = raw("caller", "work", "src/a.py");
        indirect
            .extensions
            .insert("indirect".to_owned(), Value::Bool(true));
        indirect
            .extensions
            .insert("context".to_owned(), Value::String("callback".to_owned()));
        let mut mixin = raw("caller", "work", "src/a.py");
        mixin
            .extensions
            .insert("is_mixin".to_owned(), Value::Bool(true));
        let mut extraction = Extraction {
            nodes: vec![
                node("caller", "caller()", "src/a.py", "function"),
                callable,
                node("js-caller", "caller()", "web/a.ts", "function"),
                node("js-target", "work()", "web/b.ts", "function"),
            ],
            raw_calls: Some(vec![
                indirect,
                mixin,
                raw("caller", "len", "src/a.py"),
                RawCall {
                    is_member_call: Some(true),
                    ..raw("caller", "work", "src/a.py")
                },
                raw("js-caller", "work", "web/a.ts"),
            ]),
            ..Extraction::default()
        };
        resolve_cross_file_calls(&mut extraction, &HashMap::new());
        assert!(extraction.edges.iter().any(|candidate| {
            candidate.source == "caller"
                && candidate.target == "target"
                && relation(candidate) == "indirect_call"
                && candidate.string("context") == "callback"
        }));
        assert!(!extraction.edges.iter().any(|candidate| {
            candidate.source == "js-caller"
                && candidate.target == "js-target"
                && relation(candidate) == "calls"
        }));
    }

    #[test]
    fn collision_disambiguation_rewrites_nodes_edges_and_raw_callers() {
        let mut first = node("duplicate", "Thing", "include/thing.h", "class");
        first.attributes.insert(
            "origin_file".to_owned(),
            Value::String("include/thing.h".to_owned()),
        );
        let second = node("duplicate", "Thing", "src/thing.cpp", "class");
        let mut import = edge("source", "duplicate", "imports", "src/use.cpp");
        import.attributes.insert(
            "target_file".to_owned(),
            Value::String("include/thing.h".to_owned()),
        );
        let mut extraction = Extraction {
            nodes: vec![first, second],
            edges: vec![import],
            raw_calls: Some(vec![raw("duplicate", "work", "src/thing.cpp")]),
            extensions: Map::from_iter([("fixture".to_owned(), json!(true))]),
            ..Extraction::default()
        };
        disambiguate_colliding_node_ids(&mut extraction, Path::new("."));
        assert_ne!(extraction.nodes[0].id, extraction.nodes[1].id);
        assert_eq!(extraction.edges[0].target, extraction.nodes[0].id);
        assert_eq!(
            extraction
                .raw_calls
                .as_ref()
                .and_then(|calls| calls.first())
                .map(|call| &call.caller_nid),
            Some(&extraction.nodes[1].id)
        );
        assert!(!extraction.edges[0].attributes.contains_key("target_file"));
    }

    #[test]
    fn unique_stub_rewiring_retargets_edges_and_removes_unreferenced_stubs() {
        let mut extraction = Extraction {
            nodes: vec![
                node("type", "Widget", "src/widget.py", "class"),
                node("stub", "Widget", "", "stub"),
                node("func", "run()", "src/run.py", "function"),
                node("func-stub", "run()", "", "stub"),
            ],
            edges: vec![
                edge("stub", "func-stub", "uses", "src/use.py"),
                edge("type", "stub", "inherits", "src/widget.py"),
            ],
            ..Extraction::default()
        };
        rewire_unique_stub_nodes(&mut extraction);
        assert!(
            extraction
                .edges
                .iter()
                .any(|candidate| { candidate.source == "type" && candidate.target == "func" })
        );
        assert!(
            extraction
                .nodes
                .iter()
                .all(|candidate| candidate.id != "stub")
        );
        assert!(
            extraction
                .nodes
                .iter()
                .all(|candidate| candidate.id != "func-stub")
        );
    }
}
