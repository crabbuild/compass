//! Deterministic cross-file resolution over immutable extraction facts.

mod members;

pub use members::resolve_language_calls;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value};
use trail_languages::{Extraction, RawCall};
use trail_model::{EdgeRecord, NodeRecord};

/// Merge per-file facts in source order, then resolve shared cross-file calls.
#[must_use]
pub fn resolve(extractions: &[Extraction], sources: &HashMap<String, String>) -> Extraction {
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
    resolve_cross_file_calls(&mut merged, sources);
    members::resolve_language_calls(extractions, &mut merged);
    merged
}

/// Resolve non-member raw calls using unique definitions and import evidence.
pub fn resolve_cross_file_calls(extraction: &mut Extraction, sources: &HashMap<String, String>) {
    resolve_python_import_guided(extraction, sources);
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
            extraction.edges.push(resolved_edge(
                &raw,
                &target,
                if import_evidence {
                    "EXTRACTED"
                } else {
                    "INFERRED"
                },
                if import_evidence { 1.0 } else { 0.8 },
            ));
        }
    }
}

fn resolve_python_import_guided(extraction: &mut Extraction, sources: &HashMap<String, String>) {
    let mut definitions = HashMap::<(String, String), Vec<String>>::new();
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
            .entry((source, label))
            .or_default()
            .push(node.id.clone());
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
    let raw_calls = extraction.raw_calls.clone().unwrap_or_default();
    for raw in raw_calls {
        if raw.is_member_call == Some(true) || extension(&raw.source_file) != "py" {
            continue;
        }
        let Some(source) = sources.get(&raw.source_file) else {
            continue;
        };
        let aliases = python_import_aliases(source);
        let Some((module, imported)) = aliases.get(&raw.callee) else {
            continue;
        };
        let candidates = python_definition_candidates(
            Path::new(&raw.source_file),
            module,
            imported,
            &definitions,
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

fn python_import_aliases(source: &str) -> HashMap<String, (String, String)> {
    let mut aliases = HashMap::new();
    for line in source.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("from ") else {
            continue;
        };
        let Some((module, imports)) = rest.split_once(" import ") else {
            continue;
        };
        for item in imports
            .trim_matches(['(', ')'])
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty() && *item != "*")
        {
            let (imported, local) = item
                .split_once(" as ")
                .map_or((item, item), |(imported, local)| {
                    (imported.trim(), local.trim())
                });
            aliases.insert(local.to_owned(), (module.to_owned(), imported.to_owned()));
        }
    }
    aliases
}

fn python_definition_candidates(
    caller: &Path,
    module: &str,
    imported: &str,
    definitions: &HashMap<(String, String), Vec<String>>,
) -> Vec<String> {
    let bare_module = module.trim_start_matches('.');
    let module_tail = bare_module.rsplit('.').next().unwrap_or_default();
    let relative_candidate = caller
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{}.py", bare_module.replace('.', "/")));
    let mut output = Vec::new();
    for ((source, label), ids) in definitions {
        if label != imported {
            continue;
        }
        let source_path = Path::new(source);
        let exact_relative = source_path == relative_candidate;
        let matching_stem =
            source_path.file_stem().and_then(|value| value.to_str()) == Some(module_tail);
        if exact_relative || (!module.starts_with('.') && matching_stem) {
            output.extend(ids.iter().cloned());
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
