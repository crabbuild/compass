//! Deterministic graph construction and graph algorithms for Trail.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde_json::{Map, Value};
use trail_languages::{Extraction, file_stem, make_id, normalize_id};
use trail_model::{EdgeRecord, GraphDocument, NodeRecord};

/// Build a NetworkX-compatible node-link document from extraction facts.
#[must_use]
pub fn build_from_extraction(
    extraction: &Extraction,
    directed: bool,
    root: Option<&Path>,
) -> GraphDocument {
    let rekey = semantic_id_remap(&extraction.nodes, root);
    let mut nodes = Vec::<NodeRecord>::new();
    let mut positions = HashMap::<String, usize>::new();
    for source in &extraction.nodes {
        let mut node = source.clone();
        if let Some(canonical) = rekey.get(&node.id) {
            node.id.clone_from(canonical);
        }
        canonicalize_node(&mut node, root);
        if let Some(&position) = positions.get(&node.id) {
            nodes[position].attributes.extend(node.attributes);
        } else {
            positions.insert(node.id.clone(), nodes.len());
            nodes.push(node);
        }
    }

    let mut normalized = HashMap::<String, String>::new();
    for node in &nodes {
        normalized.insert(normalize_id(&node.id), node.id.clone());
    }
    for (legacy, canonical) in &rekey {
        normalized
            .entry(normalize_id(legacy))
            .or_insert_with(|| canonical.clone());
    }

    let mut source_edges = extraction.edges.clone();
    for edge in &mut source_edges {
        if let Some(canonical) = rekey.get(&edge.source) {
            edge.source.clone_from(canonical);
        }
        if let Some(canonical) = rekey.get(&edge.target) {
            edge.target.clone_from(canonical);
        }
    }
    source_edges.sort_by(|left, right| {
        (left.source.as_str(), left.target.as_str(), relation(left)).cmp(&(
            right.source.as_str(),
            right.target.as_str(),
            relation(right),
        ))
    });
    let mut links = Vec::<EdgeRecord>::new();
    let mut edge_positions = HashMap::<(String, String), usize>::new();
    for mut edge in source_edges {
        let Some(source) = resolve_endpoint(&edge.source, &positions, &normalized) else {
            continue;
        };
        let Some(target) = resolve_endpoint(&edge.target, &positions, &normalized) else {
            continue;
        };
        edge.source = source;
        edge.target = target;
        edge.attributes.remove("target_file");
        sanitize_numeric(&mut edge.attributes, "weight");
        sanitize_numeric(&mut edge.attributes, "confidence_score");
        backfill_source_file(&mut edge, &nodes, &positions);
        normalize_attribute_path(&mut edge.attributes, "source_file", root);
        edge.attributes
            .insert("_src".to_owned(), Value::String(edge.source.clone()));
        edge.attributes
            .insert("_tgt".to_owned(), Value::String(edge.target.clone()));

        let key = edge_key(&edge.source, &edge.target, directed);
        if let Some(&position) = edge_positions.get(&key) {
            let existing = &links[position];
            let reverse_duplicate = !directed
                && relation(existing) == relation(&edge)
                && existing.attributes.get("_src").and_then(Value::as_str)
                    == Some(edge.target.as_str())
                && existing.attributes.get("_tgt").and_then(Value::as_str)
                    == Some(edge.source.as_str());
            if !reverse_duplicate {
                links[position].attributes.extend(edge.attributes);
            }
        } else {
            edge_positions.insert(key, links.len());
            links.push(edge);
        }
    }

    let mut graph = Map::new();
    let hyperedges = canonical_hyperedges(extraction, &positions, &normalized, &rekey, root);
    if !hyperedges.is_empty() {
        graph.insert("hyperedges".to_owned(), Value::Array(hyperedges));
    }
    let links = networkx_edge_order(&nodes, &links, directed);
    GraphDocument {
        directed,
        multigraph: false,
        graph,
        nodes,
        links,
        extras: BTreeMap::new(),
        used_legacy_edges_key: false,
    }
}

fn semantic_id_remap(nodes: &[NodeRecord], root: Option<&Path>) -> HashMap<String, String> {
    let mut remap = HashMap::new();
    for node in nodes {
        if node.attributes.get("_origin").and_then(Value::as_str) == Some("ast") {
            continue;
        }
        let Some(source) = node.attributes.get("source_file").and_then(Value::as_str) else {
            continue;
        };
        let portable = normalize_source_file(source, root);
        let relative = Path::new(&portable);
        if relative.is_absolute() || relative.file_name().is_none() {
            continue;
        }
        let canonical = make_id(&[&file_stem(relative)]);
        let normalized_id = normalize_id(&node.id);
        if normalized_id == canonical || normalized_id.starts_with(&format!("{canonical}_")) {
            continue;
        }
        for old in old_file_stems(relative) {
            let replacement = if normalized_id == old {
                Some(canonical.clone())
            } else {
                normalized_id
                    .strip_prefix(&format!("{old}_"))
                    .map(|suffix| make_id(&[&canonical, suffix]))
            };
            if let Some(replacement) = replacement {
                if replacement != node.id {
                    remap.insert(node.id.clone(), replacement);
                }
                break;
            }
        }
    }
    remap
}

fn old_file_stems(path: &Path) -> Vec<String> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let mut forms = Vec::new();
    if let Some(parent) = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
    {
        forms.push(make_id(&[&format!("{parent}.{stem}")]));
    }
    let bare = make_id(&[stem]);
    if !forms.contains(&bare) {
        forms.push(bare);
    }
    forms
}

fn networkx_edge_order(
    nodes: &[NodeRecord],
    links: &[EdgeRecord],
    directed: bool,
) -> Vec<EdgeRecord> {
    if directed {
        let mut output = Vec::with_capacity(links.len());
        for node in nodes {
            output.extend(links.iter().filter(|edge| edge.source == node.id).cloned());
        }
        return output;
    }
    let mut output = Vec::with_capacity(links.len());
    let mut visited = std::collections::HashSet::new();
    for node in nodes {
        for edge in links {
            let other = if edge.source == node.id {
                Some(edge.target.as_str())
            } else if edge.target == node.id {
                Some(edge.source.as_str())
            } else {
                None
            };
            let Some(other) = other else {
                continue;
            };
            if visited.contains(other) {
                continue;
            }
            let mut emitted = edge.clone();
            emitted.source = node.id.clone();
            emitted.target = other.to_owned();
            output.push(emitted);
        }
        visited.insert(node.id.clone());
    }
    output
}

/// Collapse nodes by ID using first-position, last-attribute semantics.
#[must_use]
pub fn dedupe_nodes(nodes: &[NodeRecord]) -> Vec<NodeRecord> {
    let mut output = Vec::<NodeRecord>::new();
    let mut positions = HashMap::<String, usize>::new();
    for node in nodes {
        if let Some(&position) = positions.get(&node.id) {
            output[position] = node.clone();
        } else {
            positions.insert(node.id.clone(), output.len());
            output.push(node.clone());
        }
    }
    output
}

/// Collapse exact connectivity relations, preserving the first edge.
#[must_use]
pub fn dedupe_edges(edges: &[EdgeRecord]) -> Vec<EdgeRecord> {
    let mut output = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for edge in edges {
        let key = (
            edge.source.clone(),
            edge.target.clone(),
            relation(edge).to_owned(),
        );
        if seen.insert(key) {
            output.push(edge.clone());
        }
    }
    output
}

fn canonicalize_node(node: &mut NodeRecord, root: Option<&Path>) {
    if !node.attributes.contains_key("source_file")
        && let Some(source) = node.attributes.remove("source")
    {
        node.attributes.insert("source_file".to_owned(), source);
    }
    let file_type = node
        .attributes
        .get("file_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let canonical = match file_type {
        "code" | "document" | "paper" | "image" | "rationale" | "concept" => None,
        "markdown" | "text" => Some("document"),
        "tool" | "library" => Some("code"),
        _ => Some("concept"),
    };
    if let Some(canonical) = canonical {
        node.attributes
            .insert("file_type".to_owned(), Value::String(canonical.to_owned()));
    }
    normalize_attribute_path(&mut node.attributes, "source_file", root);
}

fn normalize_attribute_path(attributes: &mut Map<String, Value>, key: &str, root: Option<&Path>) {
    let Some(value) = attributes.get(key).and_then(Value::as_str) else {
        return;
    };
    let normalized = normalize_source_file(value, root);
    attributes.insert(key.to_owned(), Value::String(normalized));
}

fn normalize_source_file(value: &str, root: Option<&Path>) -> String {
    let portable = value.replace('\\', "/");
    let path = Path::new(&portable);
    if path.is_absolute()
        && let Some(root) = root
        && let Ok(relative) = path.strip_prefix(root)
    {
        return path_text(relative);
    }
    portable
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn resolve_endpoint(
    value: &str,
    positions: &HashMap<String, usize>,
    normalized: &HashMap<String, String>,
) -> Option<String> {
    if positions.contains_key(value) {
        Some(value.to_owned())
    } else {
        normalized.get(&normalize_id(value)).cloned()
    }
}

fn relation(edge: &EdgeRecord) -> &str {
    edge.attributes
        .get("relation")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn sanitize_numeric(attributes: &mut Map<String, Value>, key: &str) {
    if !attributes.contains_key(key) {
        return;
    }
    let number = attributes
        .get(key)
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
        })
        .filter(|number| number.is_finite() && *number >= 0.0)
        .unwrap_or(1.0);
    attributes.insert(key.to_owned(), Value::from(number));
}

fn backfill_source_file(
    edge: &mut EdgeRecord,
    nodes: &[NodeRecord],
    positions: &HashMap<String, usize>,
) {
    if edge
        .attributes
        .get("source_file")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
    {
        return;
    }
    let source = positions
        .get(&edge.source)
        .and_then(|index| nodes.get(*index))
        .and_then(|node| node.attributes.get("source_file"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            positions
                .get(&edge.target)
                .and_then(|index| nodes.get(*index))
                .and_then(|node| node.attributes.get("source_file"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default();
    edge.attributes
        .insert("source_file".to_owned(), Value::String(source.to_owned()));
}

fn edge_key(source: &str, target: &str, directed: bool) -> (String, String) {
    if directed || source <= target {
        (source.to_owned(), target.to_owned())
    } else {
        (target.to_owned(), source.to_owned())
    }
}

fn canonical_hyperedges(
    extraction: &Extraction,
    positions: &HashMap<String, usize>,
    normalized: &HashMap<String, String>,
    rekey: &HashMap<String, String>,
    root: Option<&Path>,
) -> Vec<Value> {
    extraction
        .hyperedges
        .iter()
        .filter_map(|value| {
            let mut hyperedge = value.as_object()?.clone();
            if !hyperedge.get("nodes").is_some_and(Value::is_array) {
                for alias in ["members", "node_ids"] {
                    if let Some(members) = hyperedge.get(alias).and_then(Value::as_array) {
                        let mut deduped = Vec::new();
                        for member in members {
                            if !deduped.contains(member) {
                                deduped.push(member.clone());
                            }
                        }
                        hyperedge.insert("nodes".to_owned(), Value::Array(deduped));
                        break;
                    }
                }
            }
            hyperedge.remove("members");
            hyperedge.remove("node_ids");
            if let Some(source_file) = hyperedge.get("source_file").and_then(Value::as_str) {
                hyperedge.insert(
                    "source_file".to_owned(),
                    Value::String(normalize_source_file(source_file, root)),
                );
            }
            if let Some(members) = hyperedge.get("nodes").and_then(Value::as_array) {
                let valid = members
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|member| rekey.get(member).map_or(member, String::as_str))
                    .filter_map(|member| resolve_endpoint(member, positions, normalized))
                    .map(Value::String)
                    .collect::<Vec<_>>();
                if valid.is_empty() {
                    return None;
                }
                hyperedge.insert("nodes".to_owned(), Value::Array(valid));
            }
            Some(Value::Object(hyperedge))
        })
        .collect()
}
