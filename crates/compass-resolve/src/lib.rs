//! Deterministic cross-file resolution over immutable extraction facts.

mod members;

pub use members::resolve_language_calls;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use compass_languages::{Extraction, RawCall, make_id};
use compass_model::{EdgeRecord, NodeRecord};
use rayon::prelude::*;
use regex::Regex;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

const DECLARATION_SUFFIXES: &[&str] = &["h", "hpp", "hh", "hxx"];
const IMPLEMENTATION_SUFFIXES: &[&str] = &["m", "mm", "cpp", "cc", "cxx", "c"];

/// Collapse a clean sibling header/implementation declaration pair before
/// portable file-prefix remapping would split their shared symbol IDs.
///
/// This mirrors Graphify's collection-level C/C++/Objective-C pass. Only an
/// ID collision from one directory/base-stem family with exactly one header
/// is eligible; every other collision is left for conservative disambiguation.
pub fn merge_decl_def_classes(extractions: &mut [Extraction]) {
    let mut groups = HashMap::<String, Vec<(usize, usize, String)>>::new();
    for (extraction_index, extraction) in extractions.iter().enumerate() {
        for (node_index, node) in extraction.nodes.iter().enumerate() {
            let source = string_attribute(node, "source_file");
            if string_attribute(node, "file_type") == "code"
                && !node.id.is_empty()
                && !source.is_empty()
            {
                groups.entry(node.id.clone()).or_default().push((
                    extraction_index,
                    node_index,
                    source,
                ));
            }
        }
    }

    let mut dropped = HashSet::<(usize, usize)>::new();
    let mut definition_hashes = Vec::<((usize, usize), Vec<(String, Value)>)>::new();
    for entries in groups.values().filter(|entries| entries.len() > 1) {
        let mut sibling_keys = HashSet::new();
        let mut headers = Vec::new();
        let mut eligible = true;
        for &(extraction_index, node_index, ref source) in entries {
            let path = Path::new(source);
            let suffix = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !DECLARATION_SUFFIXES.contains(&suffix.as_str())
                && !IMPLEMENTATION_SUFFIXES.contains(&suffix.as_str())
            {
                eligible = false;
                break;
            }
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .split('+')
                .next()
                .unwrap_or_default();
            if stem.is_empty() {
                eligible = false;
                break;
            }
            sibling_keys.insert((
                path.parent().unwrap_or_else(|| Path::new("")).to_path_buf(),
                stem.to_owned(),
            ));
            if DECLARATION_SUFFIXES.contains(&suffix.as_str()) {
                headers.push((extraction_index, node_index));
            }
        }
        if eligible && sibling_keys.len() == 1 && headers.len() == 1 {
            let keeper = headers[0];
            if let Some((extraction_index, node_index, _)) = entries
                .iter()
                .filter(|(extraction_index, node_index, source)| {
                    let suffix = Path::new(source)
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    IMPLEMENTATION_SUFFIXES.contains(&suffix.as_str())
                        && extractions[*extraction_index].nodes[*node_index]
                            .attributes
                            .contains_key("implementation_hash")
                })
                .min_by_key(|(_, _, source)| source)
            {
                let definition = &extractions[*extraction_index].nodes[*node_index];
                let hashes = [
                    "_callable",
                    "signature_hash",
                    "implementation_hash",
                    "source_hash",
                ]
                .into_iter()
                .filter_map(|key| {
                    definition
                        .attributes
                        .get(key)
                        .cloned()
                        .map(|value| (key.to_owned(), value))
                })
                .collect::<Vec<_>>();
                definition_hashes.push((keeper, hashes));
            }
            dropped.extend(
                entries
                    .iter()
                    .map(|(extraction, node, _)| (*extraction, *node))
                    .filter(|coordinate| *coordinate != keeper),
            );
        }
    }
    if dropped.is_empty() {
        return;
    }

    for ((extraction_index, node_index), hashes) in definition_hashes {
        extractions[extraction_index].nodes[node_index]
            .attributes
            .extend(hashes);
    }

    for (extraction_index, extraction) in extractions.iter_mut().enumerate() {
        let mut node_index = 0_usize;
        extraction.nodes.retain(|_| {
            let keep = !dropped.contains(&(extraction_index, node_index));
            node_index += 1;
            keep
        });
    }
    let mut seen_edges = HashSet::new();
    for extraction in extractions {
        extraction.edges.retain(|edge| {
            edge.source != edge.target
                && seen_edges.insert((
                    edge.source.clone(),
                    edge.target.clone(),
                    relation(edge).to_owned(),
                    edge.string("context"),
                ))
        });
    }
}

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
    let language_facts = members::collect_language_call_facts(extractions);
    let mut merged = Extraction::default();
    for extraction in extractions {
        merged.nodes.extend(extraction.nodes.iter().cloned());
        merged.edges.extend(extraction.edges.iter().cloned());
        merged
            .hyperedges
            .extend(extraction.hyperedges.iter().cloned());
    }
    finish_resolution(merged, language_facts, sources, root)
}

/// Resolve a collection while transferring its node and edge buffers into the
/// merged graph. The build pipeline no longer needs the per-file facts after
/// this boundary, so ownership avoids a full corpus clone at peak RSS.
#[must_use]
pub fn resolve_owned_with_root(
    mut extractions: Vec<Extraction>,
    sources: &HashMap<String, String>,
    root: &Path,
) -> Extraction {
    let language_facts = members::collect_language_call_facts_owned(&mut extractions);
    let mut merged = Extraction::default();
    for extraction in &mut extractions {
        merged.nodes.append(&mut extraction.nodes);
        merged.edges.append(&mut extraction.edges);
        merged.hyperedges.append(&mut extraction.hyperedges);
    }
    extractions.into_par_iter().for_each(drop);
    finish_resolution(merged, language_facts, sources, root)
}

fn finish_resolution(
    mut merged: Extraction,
    mut language_facts: members::LanguageCallFacts,
    sources: &HashMap<String, String>,
    root: &Path,
) -> Extraction {
    resolve_javascript_reexports(&mut merged);
    canonicalize_import_targets(&mut merged);
    let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    disambiguate_colliding_node_ids_with_calls(
        &mut merged,
        &canonical_root,
        &mut language_facts.calls,
    );
    canonicalize_csharp_namespace_nodes(&mut merged);
    resolve_php_type_references(&mut merged, sources);
    rewire_unique_family_stubs(&mut merged);
    rewire_unique_stub_nodes(&mut merged);
    resolve_cross_file_calls_with_root_calls(&mut merged, sources, root, &language_facts.calls);
    members::resolve_language_call_facts(language_facts, &mut merged);
    merged
}

/// Graphify's per-file JavaScript extractor emits only the explicit
/// `imports_from` module edge plus named symbol re-exports. Its collection pass
/// then adds the file-level `re_exports` edge used by cycle and facade analysis.
fn resolve_javascript_reexports(extraction: &mut Extraction) {
    let mut existing = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                relation(edge).to_owned(),
                edge.string("context"),
            )
        })
        .collect::<HashSet<_>>();
    let additions = extraction
        .edges
        .iter()
        .filter(|edge| relation(edge) == "imports_from" && edge.string("context") == "re-export")
        .filter_map(|edge| {
            let key = (
                edge.source.clone(),
                edge.target.clone(),
                "re_exports".to_owned(),
                "export".to_owned(),
            );
            if !existing.insert(key) {
                return None;
            }
            let mut resolved = edge.clone();
            resolved.attributes.insert(
                "relation".to_owned(),
                Value::String("re_exports".to_owned()),
            );
            resolved
                .attributes
                .insert("context".to_owned(), Value::String("export".to_owned()));
            Some(resolved)
        })
        .collect::<Vec<_>>();
    extraction.edges.extend(additions);
}

/// Match Python's last-writer graph semantics without making the retained C#
/// namespace depend on filesystem traversal order. Namespace IDs are label
/// based, so declarations from multiple files intentionally collide; the
/// lexicographically earliest source/location is the canonical representative.
fn canonicalize_csharp_namespace_nodes(extraction: &mut Extraction) {
    let mut by_label = HashMap::<String, Vec<usize>>::new();
    for (index, node) in extraction.nodes.iter().enumerate() {
        if string_attribute(node, "type") == "namespace" {
            by_label
                .entry(node.label().to_owned())
                .or_default()
                .push(index);
        }
    }

    let mut dropped = HashSet::new();
    let mut remap = HashMap::new();
    for indexes in by_label.values().filter(|indexes| indexes.len() > 1) {
        let canonical = indexes
            .iter()
            .copied()
            .min_by_key(|index| {
                let node = &extraction.nodes[*index];
                (
                    string_attribute(node, "source_file"),
                    string_attribute(node, "source_location"),
                    node.id.clone(),
                )
            })
            .unwrap_or(indexes[0]);
        let canonical_id = extraction.nodes[canonical].id.clone();
        for &index in indexes {
            if index != canonical {
                dropped.insert(index);
                remap.insert(extraction.nodes[index].id.clone(), canonical_id.clone());
            }
        }
    }
    if dropped.is_empty() {
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
    let mut index = 0_usize;
    extraction.nodes.retain(|_| {
        let keep = !dropped.contains(&index);
        index += 1;
        keep
    });
}

/// Resolve a sourceless type stub inside the language family of the edge that
/// references it. A globally common name such as `Processor` is ambiguous, but
/// a JVM edge can still have exactly one JVM definition. This is the same
/// conservative boundary used by Graphify's Java/Groovy resolver.
fn rewire_unique_family_stubs(extraction: &mut Extraction) {
    let mut definitions = HashMap::<(String, &'static str), Vec<String>>::new();
    let mut stubs = HashMap::<String, String>::new();
    for node in &extraction.nodes {
        let source = string_attribute(node, "source_file");
        let label = node.label().trim().to_owned();
        if label.is_empty() {
            continue;
        }
        if source.is_empty() {
            stubs.insert(node.id.clone(), label);
        } else if is_type_like_definition(node)
            && let Some(family @ "jvm") = language_family(&source)
        {
            definitions
                .entry((label, family))
                .or_default()
                .push(node.id.clone());
        }
    }

    let repoint_relations = ["implements", "inherits", "extends", "imports", "references"];
    let mut repointed = HashSet::new();
    for edge in &mut extraction.edges {
        if !repoint_relations.contains(&relation(edge)) {
            continue;
        }
        let Some(label) = stubs.get(&edge.target) else {
            continue;
        };
        let source_file = edge.string("source_file");
        let Some(family @ "jvm") = language_family(&source_file) else {
            continue;
        };
        let Some(candidates) = definitions.get(&(label.clone(), family)) else {
            continue;
        };
        if let [target] = candidates.as_slice()
            && target != &edge.target
        {
            repointed.insert(edge.target.clone());
            edge.target.clone_from(target);
        }
    }
    drop_unreferenced_nodes(extraction, &repointed);
}

fn resolve_php_type_references(extraction: &mut Extraction, sources: &HashMap<String, String>) {
    let namespace_re = Regex::new(r"(?im)^\s*namespace\s+([^;{]+)\s*[;{]")
        .unwrap_or_else(|_| unreachable!("static PHP namespace regex is valid"));
    let use_re = Regex::new(r"(?im)^\s*use\s+([^;]+);")
        .unwrap_or_else(|_| unreachable!("static PHP use regex is valid"));

    let stub_labels = extraction
        .nodes
        .iter()
        .filter(|node| string_attribute(node, "source_file").is_empty())
        .map(|node| (node.id.clone(), node.label().to_owned()))
        .collect::<HashMap<_, _>>();
    if stub_labels.is_empty() {
        return;
    }

    let mut facts = HashMap::<String, (String, HashMap<String, String>)>::new();
    for (source_file, source) in sources {
        if extension(source_file) != "php" {
            continue;
        }
        let namespace = namespace_re
            .captures(source)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().trim().trim_matches('\\').to_owned())
            .unwrap_or_default();
        let mut uses = HashMap::new();
        for captures in use_re.captures_iter(source) {
            let Some(body) = captures.get(1).map(|value| value.as_str().trim()) else {
                continue;
            };
            if body.starts_with("function ") || body.starts_with("const ") {
                continue;
            }
            for (alias, fqn) in php_use_entries(body) {
                uses.entry(alias.to_ascii_lowercase()).or_insert(fqn);
            }
        }
        facts.insert(source_file.clone(), (namespace, uses));
    }

    let mut internal_types = HashMap::new();
    for node in &extraction.nodes {
        let source_file = string_attribute(node, "source_file");
        let Some((namespace, _)) = facts.get(&source_file) else {
            continue;
        };
        let label = node.label().trim();
        if label.is_empty()
            || label.ends_with(')')
            || label.contains('.')
            || is_file_node(node, &source_file)
        {
            continue;
        }
        let fqn = if namespace.is_empty() {
            label.to_owned()
        } else {
            format!("{namespace}\\{label}")
        };
        internal_types
            .entry(fqn.to_ascii_lowercase())
            .or_insert_with(|| node.id.clone());
    }

    let mut created = HashSet::new();
    let mut new_nodes = Vec::new();
    let mut repointed = HashSet::new();
    for edge in &mut extraction.edges {
        if !matches!(
            relation(edge),
            "inherits" | "implements" | "mixes_in" | "imports" | "references"
        ) {
            continue;
        }
        let Some((namespace, uses)) = facts.get(&edge.string("source_file")) else {
            continue;
        };
        let Some(label) = stub_labels.get(&edge.target) else {
            continue;
        };
        let key = label.trim().to_ascii_lowercase();
        let explicit = uses.contains_key(&key);
        let fqn = uses
            .get(&key)
            .cloned()
            .or_else(|| (!namespace.is_empty()).then(|| format!("{namespace}\\{}", label.trim())));
        let Some(fqn) = fqn else {
            continue;
        };
        let target = if let Some(target) = internal_types.get(&fqn.to_ascii_lowercase()) {
            target.clone()
        } else if explicit {
            make_id(&[&fqn])
        } else {
            continue;
        };
        if target == edge.target {
            continue;
        }
        if explicit
            && created.insert(target.clone())
            && !extraction.nodes.iter().any(|node| node.id == target)
        {
            let mut attributes = Map::new();
            attributes.insert("label".to_owned(), Value::String(fqn));
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            attributes.insert("source_file".to_owned(), Value::String(String::new()));
            attributes.insert("source_location".to_owned(), Value::String(String::new()));
            new_nodes.push(NodeRecord {
                id: target.clone(),
                attributes,
            });
        }
        repointed.insert(edge.target.clone());
        edge.target = target;
    }
    extraction.nodes.extend(new_nodes);
    drop_unreferenced_nodes(extraction, &repointed);
}

fn php_use_entries(body: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let (prefix, members) = body
        .find('{')
        .and_then(|start| {
            body.rfind('}').map(|end| {
                (
                    body[..start].trim().trim_end_matches('\\'),
                    &body[start + 1..end],
                )
            })
        })
        .unwrap_or(("", body));
    for member in members
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (target, alias) = member
            .rsplit_once(" as ")
            .map_or((member, None), |(target, alias)| {
                (target.trim(), Some(alias.trim()))
            });
        let fqn = if prefix.is_empty() {
            target.trim_start_matches('\\').to_owned()
        } else {
            format!("{prefix}\\{}", target.trim_start_matches('\\'))
        };
        let local = alias.unwrap_or_else(|| fqn.rsplit('\\').next().unwrap_or_default());
        if !local.is_empty() && !fqn.is_empty() {
            entries.push((local.to_owned(), fqn));
        }
    }
    entries
}

fn drop_unreferenced_nodes(extraction: &mut Extraction, candidates: &HashSet<String>) {
    if candidates.is_empty() {
        return;
    }
    let referenced = extraction
        .edges
        .iter()
        .flat_map(|edge| [&edge.source, &edge.target])
        .collect::<HashSet<_>>();
    extraction
        .nodes
        .retain(|node| !candidates.contains(&node.id) || referenced.contains(&node.id));
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
    let mut types_ci = HashMap::<String, Vec<String>>::new();
    let mut functions = HashMap::<String, Vec<String>>::new();
    let mut source_by_id = HashMap::<String, String>::new();
    let mut stubs = Vec::<(String, String)>::new();
    for node in &extraction.nodes {
        let normalized_label = node
            .label()
            .trim()
            .trim_matches(['(', ')'])
            .trim_start_matches('.')
            .to_owned();
        let label = normalized_label
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        if label.is_empty() {
            continue;
        }
        let source = string_attribute(node, "source_file");
        source_by_id.insert(node.id.clone(), source.clone());
        if source.is_empty() {
            stubs.push((node.id.clone(), label));
        } else if is_type_like_definition(node) {
            types
                .entry(label.clone())
                .or_default()
                .push(node.id.clone());
            if case_insensitive(&source) {
                types_ci
                    .entry(label.to_ascii_lowercase())
                    .or_default()
                    .push(node.id.clone());
            }
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
    let stub_ids = stubs
        .iter()
        .map(|(id, _)| id.as_str())
        .collect::<HashSet<_>>();
    let mut stub_families = HashMap::<String, HashSet<&'static str>>::new();
    for edge in &extraction.edges {
        let Some(family) = language_family(&edge.string("source_file")) else {
            continue;
        };
        for endpoint in [&edge.source, &edge.target] {
            if stub_ids.contains(endpoint.as_str()) {
                stub_families
                    .entry(endpoint.clone())
                    .or_default()
                    .insert(family);
            }
        }
    }
    let mut remap = HashMap::new();
    for (stub, label) in stubs {
        let candidates = types
            .get(&label)
            .filter(|items| items.len() == 1)
            .or_else(|| {
                types_ci
                    .get(&label.to_ascii_lowercase())
                    .filter(|items| items.len() == 1)
            })
            .or_else(|| {
                if supertype_stubs.contains(stub.as_str()) {
                    return None;
                }
                let items = functions.get(&label).filter(|items| items.len() == 1)?;
                let target = items.first()?;
                let families = stub_families.get(&stub);
                let candidate_family = source_by_id
                    .get(target)
                    .and_then(|source| language_family(source));
                (families.is_none_or(HashSet::is_empty)
                    || candidate_family.is_none()
                    || candidate_family.is_some_and(|family| {
                        families.is_some_and(|families| families.contains(family))
                    }))
                .then_some(items)
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

#[cfg(test)]
fn disambiguate_colliding_node_ids(extraction: &mut Extraction, root: &Path) {
    let mut raw_calls = extraction.raw_calls.take();
    if let Some(calls) = raw_calls.as_mut() {
        disambiguate_colliding_node_ids_with_calls(extraction, root, calls);
    } else {
        disambiguate_colliding_node_ids_with_calls(extraction, root, &mut []);
    }
    extraction.raw_calls = raw_calls;
}

fn disambiguate_colliding_node_ids_with_calls(
    extraction: &mut Extraction,
    root: &Path,
    raw_calls: &mut [RawCall],
) {
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
    for raw in raw_calls {
        let key = source_key(&raw.source_file, root);
        if let Some(new_id) = remap.get(&(raw.caller_nid.clone(), key)) {
            raw.caller_nid.clone_from(new_id);
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
    if let Ok(relative) = absolute.strip_prefix(root) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    if let Ok(canonical) = std::fs::canonicalize(&absolute)
        && let Ok(relative) = canonical.strip_prefix(root)
    {
        return relative.to_string_lossy().replace('\\', "/");
    }
    path.to_string_lossy().replace('\\', "/")
}

/// Resolve non-member raw calls using unique definitions and import evidence.
pub fn resolve_cross_file_calls(extraction: &mut Extraction, sources: &HashMap<String, String>) {
    resolve_cross_file_calls_with_root(extraction, sources, Path::new("."));
}

fn resolve_cross_file_calls_with_root(
    extraction: &mut Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
) {
    let raw_calls = extraction.raw_calls.clone().unwrap_or_default();
    resolve_cross_file_calls_with_root_calls(extraction, sources, root, &raw_calls);
}

fn resolve_cross_file_calls_with_root_calls(
    extraction: &mut Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
    raw_calls: &[RawCall],
) {
    resolve_python_import_guided_with_calls(extraction, sources, root, raw_calls);
    resolve_python_class_uses(extraction, sources, root);
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

    for raw in raw_calls {
        if raw.callee.is_empty()
            || raw.is_member_call == Some(true)
            || is_builtin(&raw.callee)
            || raw.extensions.get("is_mixin").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let candidates = candidate_calls(raw, &exact, &folded, &source_by_id);
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
        if raw
            .extensions
            .get("module_import_use")
            .and_then(Value::as_bool)
            == Some(true)
            && !file_by_id.get(&target).is_some_and(|target_file| {
                imported_modules.is_some_and(|imports| imports.contains(target_file))
            })
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
                let mut edge = resolved_edge(raw, &target, "INFERRED", 0.8);
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
                raw,
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

#[cfg(test)]
fn resolve_python_import_guided(
    extraction: &mut Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
) {
    let raw_calls = extraction.raw_calls.clone().unwrap_or_default();
    resolve_python_import_guided_with_calls(extraction, sources, root, &raw_calls);
}

fn resolve_python_import_guided_with_calls(
    extraction: &mut Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
    raw_calls: &[RawCall],
) {
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
    let normalized_sources = sources
        .iter()
        .map(|(source_file, source)| {
            (
                normalize_path(source_file),
                (source_file.as_str(), source.as_str()),
            )
        })
        .collect::<HashMap<_, _>>();
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
    let normalized_file_nodes = file_nodes
        .iter()
        .map(|(source, id)| (normalize_path(source), id.clone()))
        .collect::<HashMap<_, _>>();
    for (source_file, source) in sources {
        if extension(source_file) != "py" {
            continue;
        }
        let Some(file_node) = file_nodes.get(source_file) else {
            continue;
        };
        for imported in python_symbol_imports(source) {
            let module_file = python_module_file(
                Path::new(source_file),
                root,
                &imported.module,
                None,
                &normalized_file_nodes,
            );
            let candidates = python_resolved_definition_candidates(
                Path::new(source_file),
                root,
                &imported.module,
                &imported.imported,
                &definitions,
                &normalized_sources,
                false,
            );
            if candidates.len() == 1 {
                let target = &candidates[0];
                if known.insert((file_node.clone(), target.clone(), "imports".to_owned())) {
                    extraction.edges.push(python_import_edge(
                        file_node,
                        target,
                        "imports",
                        "import",
                        source_file,
                        imported.line,
                    ));
                }
            } else if let Some(target) = python_module_file(
                Path::new(source_file),
                root,
                &imported.module,
                Some(&imported.imported),
                &normalized_file_nodes,
            ) && known.insert((
                file_node.clone(),
                target.clone(),
                "imports_from".to_owned(),
            )) {
                extraction.edges.push(python_import_edge(
                    file_node,
                    &target,
                    "imports_from",
                    "submodule_import",
                    source_file,
                    imported.line,
                ));
            }
            if Path::new(source_file)
                .file_name()
                .and_then(|name| name.to_str())
                == Some("__init__.py")
                && let Some(target) = module_file
                && known.insert((file_node.clone(), target.clone(), "re_exports".to_owned()))
            {
                extraction.edges.push(python_import_edge(
                    file_node,
                    &target,
                    "re_exports",
                    "export",
                    source_file,
                    imported.line,
                ));
            }
        }
    }
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
                != Some(true)
        {
            continue;
        }
        let Some(aliases) = aliases_by_source.get(raw.source_file.as_str()) else {
            continue;
        };
        let Some((module, imported)) = aliases.get(&raw.callee) else {
            continue;
        };
        let candidates = python_resolved_definition_candidates(
            Path::new(&raw.source_file),
            root,
            module,
            imported,
            &definitions,
            &normalized_sources,
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
        let mut edge = resolved_edge(raw, target, "EXTRACTED", 1.0);
        edge.attributes.remove("confidence_score");
        extraction.edges.push(edge);
    }
}

fn python_import_edge(
    source: &str,
    target: &str,
    relation: &str,
    context: &str,
    source_file: &str,
    line: usize,
) -> EdgeRecord {
    EdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes: Map::from_iter([
            ("relation".to_owned(), Value::String(relation.to_owned())),
            ("context".to_owned(), Value::String(context.to_owned())),
            (
                "confidence".to_owned(),
                Value::String("EXTRACTED".to_owned()),
            ),
            (
                "source_file".to_owned(),
                Value::String(source_file.to_owned()),
            ),
            (
                "source_location".to_owned(),
                Value::String(format!("L{line}")),
            ),
            ("weight".to_owned(), Value::from(1.0)),
        ]),
    }
}

fn python_module_file(
    caller: &Path,
    root: &Path,
    module: &str,
    submodule: Option<&str>,
    file_nodes: &HashMap<String, String>,
) -> Option<String> {
    let depth = module
        .len()
        .saturating_sub(module.trim_start_matches('.').len());
    let bare = module.trim_start_matches('.');
    let mut base = if depth == 0 {
        root.to_path_buf()
    } else {
        let mut base = caller
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        for _ in 1..depth {
            base = base.parent().unwrap_or(&base).to_path_buf();
        }
        base
    };
    if !bare.is_empty() {
        base.push(bare.replace('.', "/"));
    }
    if let Some(submodule) = submodule.filter(|value| !value.is_empty()) {
        base.push(submodule.replace('.', "/"));
    }
    let candidates = [base.with_extension("py"), base.join("__init__.py")];
    candidates.iter().find_map(|candidate| {
        file_nodes
            .get(&normalize_path(&candidate.to_string_lossy()))
            .cloned()
    })
}

fn resolve_python_class_uses(
    extraction: &mut Extraction,
    sources: &HashMap<String, String>,
    root: &Path,
) {
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
    let normalized_sources = sources
        .iter()
        .map(|(source_file, source)| {
            (
                normalize_path(source_file),
                (source_file.as_str(), source.as_str()),
            )
        })
        .collect::<HashMap<_, _>>();
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
            let candidates = python_resolved_definition_candidates(
                Path::new(source_file),
                root,
                &imported.module,
                &imported.imported,
                &definitions,
                &normalized_sources,
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
    let masked = mask_python_non_code(source);
    let lines = masked.lines().collect::<Vec<_>>();
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

fn mask_python_non_code(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    let original = source.as_bytes();
    let mut index = 0_usize;
    let mut quote = None::<(u8, bool)>;
    let mut escaped = false;
    while index < original.len() {
        if let Some((delimiter, triple)) = quote {
            if original[index] == b'\n' || original[index] == b'\r' {
                escaped = false;
                index += 1;
                continue;
            }
            if escaped {
                bytes[index] = b' ';
                escaped = false;
                index += 1;
                continue;
            }
            if original[index] == b'\\' {
                bytes[index] = b' ';
                escaped = true;
                index += 1;
                continue;
            }
            let closes = if triple {
                original.get(index..index + 3) == Some(&[delimiter, delimiter, delimiter])
            } else {
                original[index] == delimiter
            };
            if closes {
                let width = if triple { 3 } else { 1 };
                bytes[index..index + width].fill(b' ');
                index += width;
                quote = None;
                continue;
            }
            bytes[index] = b' ';
            index += 1;
            continue;
        }
        if original[index] == b'#' {
            while index < original.len() && !matches!(original[index], b'\n' | b'\r') {
                bytes[index] = b' ';
                index += 1;
            }
            continue;
        }
        if matches!(original[index], b'\'' | b'"') {
            let delimiter = original[index];
            let triple = original.get(index..index + 3) == Some(&[delimiter, delimiter, delimiter]);
            let width = if triple { 3 } else { 1 };
            bytes[index..index + width].fill(b' ');
            index += width;
            quote = Some((delimiter, triple));
            continue;
        }
        index += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| source.to_owned())
}

fn python_definition_candidates(
    caller: &Path,
    root: &Path,
    module: &str,
    imported: &str,
    definitions: &HashMap<String, Vec<(String, String)>>,
    allow_module_tail: bool,
) -> Vec<String> {
    let bare_module = module.trim_start_matches('.');
    let module_tail = bare_module.rsplit('.').next().unwrap_or_default();
    let relative_candidate = if module.starts_with('.') {
        let depth = module
            .len()
            .saturating_sub(module.trim_start_matches('.').len());
        let mut base = caller.parent().unwrap_or_else(|| Path::new("."));
        for _ in 1..depth {
            base = base.parent().unwrap_or(base);
        }
        base.join(format!("{}.py", bare_module.replace('.', "/")))
    } else {
        root.join(format!("{}.py", bare_module.replace('.', "/")))
    };
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

fn python_resolved_definition_candidates(
    caller: &Path,
    root: &Path,
    module: &str,
    imported: &str,
    definitions: &HashMap<String, Vec<(String, String)>>,
    sources: &HashMap<String, (&str, &str)>,
    allow_module_tail: bool,
) -> Vec<String> {
    let mut caller = caller.to_path_buf();
    let mut module = module.to_owned();
    let mut imported = imported.to_owned();
    let mut seen = HashSet::new();
    for _ in 0..16 {
        let direct = python_definition_candidates(
            &caller,
            root,
            &module,
            &imported,
            definitions,
            allow_module_tail,
        );
        if !direct.is_empty() {
            return direct;
        }
        let Some((target_source, target_text)) =
            python_module_source(&caller, root, &module, sources)
        else {
            break;
        };
        let target_key = normalize_path(target_source);
        let in_module = definitions
            .get(&imported)
            .into_iter()
            .flatten()
            .filter(|(source_file, _)| normalize_path(source_file) == target_key)
            .map(|(_, id)| id.clone())
            .collect::<Vec<_>>();
        if !in_module.is_empty() {
            return in_module;
        }
        if Path::new(target_source)
            .file_name()
            .and_then(|name| name.to_str())
            != Some("__init__.py")
            || !seen.insert((target_source.to_owned(), imported.clone()))
        {
            break;
        }
        let Some(reexport) = python_symbol_imports(target_text)
            .into_iter()
            .find(|candidate| candidate.local == imported)
        else {
            break;
        };
        caller = Path::new(target_source).to_path_buf();
        module = reexport.module;
        imported = reexport.imported;
    }
    Vec::new()
}

fn python_module_source<'a>(
    caller: &Path,
    root: &Path,
    module: &str,
    sources: &'a HashMap<String, (&'a str, &'a str)>,
) -> Option<(&'a str, &'a str)> {
    let depth = module
        .len()
        .saturating_sub(module.trim_start_matches('.').len());
    let bare = module.trim_start_matches('.');
    let mut base = if depth == 0 {
        root.to_path_buf()
    } else {
        let mut base = caller
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        for _ in 1..depth {
            base = base.parent().unwrap_or(&base).to_path_buf();
        }
        base
    };
    if !bare.is_empty() {
        base.push(bare.replace('.', "/"));
    }
    [base.with_extension("py"), base.join("__init__.py")]
        .iter()
        .find_map(|candidate| {
            sources
                .get(&normalize_path(&candidate.to_string_lossy()))
                .copied()
        })
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
    fn javascript_collection_pass_adds_module_reexport_once() {
        let mut import = edge("barrel", "module", "imports_from", "barrel.ts");
        import
            .attributes
            .insert("context".to_owned(), Value::String("re-export".to_owned()));
        import.attributes.insert(
            "target_file".to_owned(),
            Value::String("module.ts".to_owned()),
        );
        let mut extraction = Extraction {
            edges: vec![import],
            ..Extraction::default()
        };
        resolve_javascript_reexports(&mut extraction);
        resolve_javascript_reexports(&mut extraction);
        let reexports = extraction
            .edges
            .iter()
            .filter(|edge| relation(edge) == "re_exports")
            .collect::<Vec<_>>();
        assert_eq!(reexports.len(), 1);
        assert_eq!(reexports[0].string("context"), "export");
        assert_eq!(reexports[0].string("target_file"), "module.ts");
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
                Path::new("."),
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
                Path::new("."),
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
                Path::new("."),
                "pkg.models",
                "Widget",
                &definitions,
                true,
            )
            .len(),
            2
        );
        let qualified = HashMap::from([(
            "edge_data".to_owned(),
            vec![(
                "/repo/graphify/build.py".to_owned(),
                "graphify_build_edge_data".to_owned(),
            )],
        )]);
        assert_eq!(
            python_definition_candidates(
                Path::new("/repo/graphify/analyze.py"),
                Path::new("/repo"),
                "graphify.build",
                "edge_data",
                &qualified,
                false,
            ),
            vec!["graphify_build_edge_data"]
        );
    }

    #[test]
    fn python_package_initializers_reexport_imported_symbols() {
        let root = Path::new("/repo");
        let caller = "/repo/caller.py";
        let init = "/repo/pkg/__init__.py";
        let module = "/repo/pkg/mod.py";
        let mut extraction = Extraction {
            nodes: vec![
                node("caller", "caller.py", caller, "module"),
                node("pkg_init", "__init__.py", init, "module"),
                node("pkg_mod", "mod.py", module, "module"),
                node("pkg_mod_fn", "fn()", module, "function"),
            ],
            ..Extraction::default()
        };
        let sources = HashMap::from([
            (caller.to_owned(), "from pkg import fn\n".to_owned()),
            (init.to_owned(), "from .mod import fn\n".to_owned()),
            (module.to_owned(), "def fn():\n    return 1\n".to_owned()),
        ]);

        resolve_python_import_guided(&mut extraction, &sources, root);

        assert!(extraction.edges.iter().any(|edge| {
            edge.source == "pkg_init"
                && edge.target == "pkg_mod"
                && relation(edge) == "re_exports"
                && edge.string("context") == "export"
        }));
        assert!(extraction.edges.iter().any(|edge| {
            edge.source == "caller"
                && edge.target == "pkg_mod_fn"
                && relation(edge) == "imports"
                && edge.string("context") == "import"
        }));
    }

    #[test]
    fn python_package_form_submodule_imports_target_the_submodule_file() {
        let root = Path::new("/repo");
        let caller = "/repo/caller.py";
        let init = "/repo/pkg/__init__.py";
        let module = "/repo/pkg/mod.py";
        let mut extraction = Extraction {
            nodes: vec![
                node("caller", "caller.py", caller, "module"),
                node("pkg_init", "__init__.py", init, "module"),
                node("pkg_mod", "mod.py", module, "module"),
            ],
            ..Extraction::default()
        };
        let sources = HashMap::from([
            (caller.to_owned(), "from pkg import mod\n".to_owned()),
            (init.to_owned(), String::new()),
            (module.to_owned(), "VALUE = 1\n".to_owned()),
        ]);

        resolve_python_import_guided(&mut extraction, &sources, root);

        assert!(extraction.edges.iter().any(|edge| {
            edge.source == "caller"
                && edge.target == "pkg_mod"
                && relation(edge) == "imports_from"
                && edge.string("context") == "submodule_import"
        }));
    }

    #[test]
    fn python_imports_inside_multiline_strings_are_ignored() {
        let root = Path::new("/repo");
        let caller = "/repo/hooks.py";
        let module = "/repo/pkg/mod.py";
        let mut extraction = Extraction {
            nodes: vec![
                node("hooks", "hooks.py", caller, "module"),
                node("pkg_mod", "mod.py", module, "module"),
                node("pkg_mod_fn", "fn()", module, "function"),
            ],
            ..Extraction::default()
        };
        let sources = HashMap::from([
            (
                caller.to_owned(),
                "SCRIPT = \"\"\"\\\nfrom pkg.mod import fn\nfn()\n\"\"\"\n".to_owned(),
            ),
            (module.to_owned(), "def fn():\n    return 1\n".to_owned()),
        ]);

        resolve_python_import_guided(&mut extraction, &sources, root);

        assert!(!extraction.edges.iter().any(|edge| {
            edge.source == "hooks" && edge.target == "pkg_mod_fn" && relation(edge) == "imports"
        }));
    }

    #[test]
    fn python_module_member_calls_require_a_matching_module_import() {
        let caller = "/repo/cli.py";
        let unrelated = "/repo/unrelated.py";
        let mut module_call = raw("dispatch", "log_query", caller);
        module_call
            .extensions
            .insert("module_import_use".to_owned(), Value::Bool(true));
        let mut extraction = Extraction {
            nodes: vec![
                node("cli", "cli.py", caller, "module"),
                node("dispatch", "dispatch()", caller, "function"),
                node("unrelated", "unrelated.py", unrelated, "module"),
                node("unrelated_log_query", "log_query()", unrelated, "function"),
            ],
            raw_calls: Some(vec![module_call]),
            ..Extraction::default()
        };

        resolve_cross_file_calls_with_root(&mut extraction, &HashMap::new(), Path::new("/repo"));

        assert!(!extraction.edges.iter().any(|edge| {
            edge.source == "dispatch"
                && edge.target == "unrelated_log_query"
                && relation(edge) == "calls"
        }));
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
        let mut import_call = raw("caller", "run", "app/a.py");
        import_call
            .extensions
            .insert("symbol_import_use".to_owned(), Value::Bool(true));
        let extraction = Extraction {
            nodes: vec![
                node(&file_a, "a.py", "app/a.py", "file"),
                node(&file_b, "b.py", "app/b.py", "file"),
                node("caller", "caller()", "app/a.py", "function"),
                node("local-class", "Local", "app/a.py", "class"),
                node("helper", "helper()", "app/b.py", "function"),
                node("widget", "Widget", "app/b.py", "class"),
            ],
            raw_calls: Some(vec![import_call]),
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
    fn declaration_definition_merge_keeps_the_unique_sibling_header() {
        let header = Extraction {
            nodes: vec![node("widget_draw", "draw", "native/Widget.h", "method")],
            edges: vec![edge("widget", "widget_draw", "method", "native/Widget.h")],
            ..Extraction::default()
        };
        let mut implementation_node = node(
            "widget_draw",
            "Widget::draw()",
            "native/Widget.cpp",
            "method",
        );
        implementation_node.attributes.insert(
            "implementation_hash".to_owned(),
            Value::String("body-digest".to_owned()),
        );
        implementation_node.attributes.insert(
            "signature_hash".to_owned(),
            Value::String("signature-digest".to_owned()),
        );
        implementation_node.attributes.insert(
            "source_hash".to_owned(),
            Value::String("source-digest".to_owned()),
        );
        let implementation = Extraction {
            nodes: vec![implementation_node],
            edges: vec![
                edge("widget", "widget_draw", "method", "native/Widget.cpp"),
                edge("widget_draw", "widget_draw", "calls", "native/Widget.cpp"),
            ],
            ..Extraction::default()
        };
        let unrelated = Extraction {
            nodes: vec![
                node("logger", "Logger", "a/Logger.h", "class"),
                node("logger", "Logger", "b/Logger.cpp", "class"),
            ],
            ..Extraction::default()
        };
        let mut extractions = vec![header, implementation, unrelated];

        merge_decl_def_classes(&mut extractions);

        let merged = extractions
            .iter()
            .flat_map(|extraction| &extraction.nodes)
            .filter(|candidate| candidate.id == "widget_draw")
            .collect::<Vec<_>>();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].label(), "draw");
        assert_eq!(merged[0].string("source_file"), "native/Widget.h");
        assert_eq!(merged[0].string("implementation_hash"), "body-digest");
        assert_eq!(merged[0].string("signature_hash"), "signature-digest");
        assert_eq!(merged[0].string("source_hash"), "source-digest");
        assert_eq!(
            extractions
                .iter()
                .flat_map(|extraction| &extraction.nodes)
                .filter(|candidate| candidate.id == "logger")
                .count(),
            2
        );
        assert_eq!(
            extractions
                .iter()
                .flat_map(|extraction| &extraction.edges)
                .filter(|candidate| candidate.source == "widget")
                .count(),
            1
        );
        assert!(
            extractions
                .iter()
                .flat_map(|extraction| &extraction.edges)
                .all(|candidate| candidate.source != candidate.target)
        );
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

    #[test]
    fn generic_stub_rewiring_matches_python_ascii_and_case_insensitive_fallbacks() {
        let mut extraction = Extraction {
            nodes: vec![
                node("php-result", "Result", "fixture.php", "class"),
                node("rust-result", "Result", "fixture.rs", "struct"),
                node("result", "Result", "", "stub"),
                node("python-image", "_ImageRef", "llm.py", "class"),
                node("rust-image", "ImageRef", "image.rs", "struct"),
                node("imageref", "ImageRef", "", "stub"),
            ],
            edges: vec![
                edge("caller", "result", "references", "caller.rs"),
                edge("caller", "imageref", "references", "caller.rs"),
            ],
            ..Extraction::default()
        };

        rewire_unique_stub_nodes(&mut extraction);

        assert_eq!(extraction.edges[0].target, "php-result");
        assert_eq!(extraction.edges[1].target, "imageref");
    }

    #[test]
    fn csharp_namespace_canonicalization_keeps_lexicographically_earliest_source() {
        let mut later = node(
            "namespace-id",
            "Demo.ViewModels",
            "views/ToolkitViewModel.cs",
            "namespace",
        );
        later
            .attributes
            .insert("source_location".to_owned(), Value::String("L4".to_owned()));
        let mut earlier = node(
            "namespace-id",
            "Demo.ViewModels",
            "views/DesignViewModel.cs",
            "namespace",
        );
        earlier
            .attributes
            .insert("source_location".to_owned(), Value::String("L1".to_owned()));
        let mut extraction = Extraction {
            nodes: vec![later, earlier],
            edges: vec![edge("consumer", "namespace-id", "imports", "views/App.cs")],
            ..Extraction::default()
        };

        canonicalize_csharp_namespace_nodes(&mut extraction);

        assert_eq!(extraction.nodes.len(), 1);
        assert_eq!(
            extraction.nodes[0].string("source_file"),
            "views/DesignViewModel.cs"
        );
        assert_eq!(extraction.edges[0].target, "namespace-id");
    }

    #[test]
    fn family_stub_rewiring_does_not_conflate_same_named_cross_language_types() {
        let mut extraction = Extraction {
            nodes: vec![
                node(
                    "java-processor",
                    "Processor",
                    "src/Processor.java",
                    "interface",
                ),
                node("rust-processor", "Processor", "src/processor.rs", "trait"),
                node("processor", "Processor", "", "stub"),
                node(
                    "java-data",
                    "DataProcessor",
                    "src/DataProcessor.java",
                    "class",
                ),
            ],
            edges: vec![edge(
                "java-data",
                "processor",
                "implements",
                "src/DataProcessor.java",
            )],
            ..Extraction::default()
        };

        rewire_unique_family_stubs(&mut extraction);

        assert_eq!(extraction.edges[0].target, "java-processor");
        assert!(
            extraction
                .nodes
                .iter()
                .all(|candidate| candidate.id != "processor")
        );
    }

    #[test]
    fn php_use_aliases_retarget_external_type_stubs_to_qualified_nodes() {
        let mut extraction = Extraction {
            nodes: vec![
                node("file", "Client.php", "src/Client.php", "file"),
                node("client", "Client", "src/Client.php", "class"),
                node("authenticator", "Authenticator", "", "stub"),
            ],
            edges: vec![
                edge("file", "authenticator", "imports", "src/Client.php"),
                edge("client", "authenticator", "references", "src/Client.php"),
            ],
            ..Extraction::default()
        };
        let sources = HashMap::from([(
            "src/Client.php".to_owned(),
            "<?php\nnamespace App\\Http;\nuse App\\Auth\\Authenticator;\nclass Client {}\n"
                .to_owned(),
        )]);

        resolve_php_type_references(&mut extraction, &sources);

        let qualified = make_id(&["App\\Auth\\Authenticator"]);
        assert!(
            extraction
                .edges
                .iter()
                .all(|candidate| candidate.target == qualified)
        );
        assert!(extraction.nodes.iter().any(|candidate| {
            candidate.id == qualified && candidate.label() == "App\\Auth\\Authenticator"
        }));
        assert!(
            extraction
                .nodes
                .iter()
                .all(|candidate| candidate.id != "authenticator")
        );
        assert_eq!(
            php_use_entries("Vendor\\Package\\{Service, Contract as API}"),
            vec![
                ("Service".to_owned(), "Vendor\\Package\\Service".to_owned()),
                ("API".to_owned(), "Vendor\\Package\\Contract".to_owned()),
            ]
        );
    }
}
