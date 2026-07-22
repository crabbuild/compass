//! Deterministic graph construction and graph algorithms for Compass.

mod analyze;
mod cluster;
mod dedup;

pub use analyze::{
    DiffEdge, DiffNode, GodNode, GraphDiff, ImportCycle, SuggestedQuestion, SurpriseConnection,
    find_import_cycles, god_nodes, graph_diff, suggest_questions, surprising_connections,
};
pub use cluster::{
    ClusterOptions, Communities, cluster, cohesion_score, community_member_signatures,
    label_communities_by_hub, remap_communities_to_previous, score_communities,
};
pub use dedup::{
    AmbiguousPair, DedupError, DedupResult, DedupStats, EntityTiebreaker, deduplicate_entities,
    deduplicate_entities_with_tiebreaker,
};

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use compass_languages::{Extraction, file_stem, make_id, normalize_id};
use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
use serde_json::{Map, Value};

/// Merge resolved extraction chunks, apply native entity deduplication, and build
/// a node-link graph. This is the deterministic counterpart of `graphify.build`.
pub fn build(
    extractions: &[Extraction],
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
) -> Result<GraphDocument, DedupError> {
    build_with_tiebreaker(extractions, directed, dedup, root, None)
}

pub fn build_with_tiebreaker(
    extractions: &[Extraction],
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
) -> Result<GraphDocument, DedupError> {
    let mut combined = Extraction::default();
    for extraction in extractions {
        combined.nodes.extend(extraction.nodes.iter().cloned());
        combined.edges.extend(extraction.edges.iter().cloned());
        combined
            .hyperedges
            .extend(extraction.hyperedges.iter().cloned());
    }
    if dedup && !combined.nodes.is_empty() {
        let result = deduplicate_entities_with_tiebreaker(
            &combined.nodes,
            &combined.edges,
            &HashMap::new(),
            tiebreaker,
        )?;
        combined.nodes = result.nodes;
        combined.edges = result.edges;
    }
    Ok(build_from_extraction(&combined, directed, root))
}

/// Build a NetworkX-compatible node-link document from extraction facts.
#[must_use]
pub fn build_from_extraction(
    extraction: &Extraction,
    directed: bool,
    root: Option<&Path>,
) -> GraphDocument {
    let rekey = semantic_id_remap(&extraction.nodes, root);
    let mut prepared_nodes = extraction
        .nodes
        .iter()
        .cloned()
        .map(|mut node| {
            if let Some(canonical) = rekey.get(&node.id) {
                node.id.clone_from(canonical);
            }
            canonicalize_node(&mut node, root);
            node
        })
        .collect::<Vec<_>>();
    let doc_remap = doc_twin_remap(&prepared_nodes);
    prepared_nodes.retain(|node| !doc_remap.contains_key(&node.id));

    let mut nodes = Vec::<NodeRecord>::new();
    let mut positions = HashMap::<String, usize>::new();
    for node in prepared_nodes {
        if let Some(&position) = positions.get(&node.id) {
            nodes[position].attributes.extend(node.attributes);
        } else {
            positions.insert(node.id.clone(), nodes.len());
            nodes.push(node);
        }
    }

    let ghost_remap = ghost_duplicate_remap(&nodes);
    if !ghost_remap.is_empty() {
        nodes.retain(|node| !ghost_remap.contains_key(&node.id));
        positions.clear();
        for (index, node) in nodes.iter().enumerate() {
            positions.insert(node.id.clone(), index);
        }
    }

    let mut endpoint_remap = rekey.clone();
    endpoint_remap.extend(doc_remap.clone());
    endpoint_remap.extend(ghost_remap.clone());

    let mut normalized = HashMap::<String, String>::new();
    for node in &nodes {
        normalized.insert(normalize_id(&node.id), node.id.clone());
    }
    for (legacy, canonical) in &endpoint_remap {
        normalized
            .entry(normalize_id(legacy))
            .or_insert_with(|| canonical.clone());
    }
    add_unambiguous_legacy_aliases(&nodes, &mut normalized);

    let mut source_edges = extraction.edges.clone();
    for edge in &mut source_edges {
        let original_source = edge.source.clone();
        let original_target = edge.target.clone();
        edge.source = remap_endpoint(&edge.source, &endpoint_remap);
        edge.target = remap_endpoint(&edge.target, &endpoint_remap);
        if edge.source == edge.target
            && (doc_remap.contains_key(&remap_endpoint(&original_source, &rekey))
                || doc_remap.contains_key(&remap_endpoint(&original_target, &rekey)))
        {
            edge.attributes
                .insert("_drop".to_owned(), Value::Bool(true));
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
        if edge.attributes.remove("_drop") == Some(Value::Bool(true)) {
            continue;
        }
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
        if is_cross_language_phantom(&edge, &nodes, &positions) {
            continue;
        }
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
    let hyperedges =
        canonical_hyperedges(extraction, &positions, &normalized, &endpoint_remap, root);
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

fn doc_twin_remap(nodes: &[NodeRecord]) -> HashMap<String, String> {
    let by_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut remap = HashMap::new();
    for node in nodes {
        let Some(bare_id) = node.id.strip_suffix("_doc") else {
            continue;
        };
        let Some(bare) = by_id.get(bare_id) else {
            continue;
        };
        let source = node.string("source_file");
        if !source.is_empty()
            && bare.string("source_file") == source
            && node.string("file_type") == "document"
            && bare.string("file_type") == "document"
        {
            remap.insert(bare_id.to_owned(), node.id.clone());
        }
    }
    remap
}

fn ghost_duplicate_remap(nodes: &[NodeRecord]) -> HashMap<String, String> {
    let mut ordered = nodes.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.id.cmp(&right.id));
    let mut canonical = HashMap::<(String, String), String>::new();
    let mut collisions = std::collections::HashSet::new();
    for node in &ordered {
        let label = node.label().trim();
        let source = node.string("source_file");
        let basename = Path::new(&source)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if label.is_empty() || basename.is_empty() {
            continue;
        }
        let ast = node.attributes.get("_origin").and_then(Value::as_str) == Some("ast");
        let located = node
            .attributes
            .get("source_location")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty());
        if !ast && !located {
            continue;
        }
        let key = (basename.to_owned(), label.to_owned());
        if ast {
            if canonical.get(&key).is_some_and(|existing| {
                nodes.iter().any(|candidate| {
                    candidate.id == *existing
                        && candidate.attributes.get("_origin").and_then(Value::as_str)
                            == Some("ast")
                })
            }) {
                collisions.insert(key.clone());
            }
            canonical.insert(key, node.id.clone());
        } else if let Some(existing) = canonical.get(&key) {
            let different_source = nodes.iter().any(|candidate| {
                candidate.id == *existing
                    && candidate.attributes.get("_origin").and_then(Value::as_str) != Some("ast")
                    && candidate.string("source_file") != source
            });
            if different_source {
                collisions.insert(key);
            }
        } else {
            canonical.insert(key, node.id.clone());
        }
    }
    let mut remap = HashMap::new();
    for node in ordered {
        if node.attributes.get("_origin").and_then(Value::as_str) == Some("ast") {
            continue;
        }
        let source = node.string("source_file");
        let basename = Path::new(&source)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let key = (basename.to_owned(), node.label().trim().to_owned());
        if key.0.is_empty() || key.1.is_empty() || collisions.contains(&key) {
            continue;
        }
        if let Some(target) = canonical.get(&key).filter(|target| *target != &node.id) {
            remap.insert(node.id.clone(), target.clone());
        }
    }
    remap
}

fn add_unambiguous_legacy_aliases(nodes: &[NodeRecord], normalized: &mut HashMap<String, String>) {
    let mut candidates = HashMap::<String, std::collections::HashSet<String>>::new();
    for node in nodes {
        let source = node.string("source_file");
        let path = Path::new(&source);
        if source.is_empty() || path.is_absolute() || path.file_name().is_none() {
            continue;
        }
        let canonical_stem = make_id(&[&file_stem(path)]);
        let normalized_id = normalize_id(&node.id);
        let is_file = path.file_name().and_then(|value| value.to_str()) == Some(node.label());
        let suffix = if is_file {
            ""
        } else {
            normalized_id
                .strip_prefix(&canonical_stem)
                .unwrap_or_default()
        };
        for old_stem in old_file_stems(path) {
            if old_stem == canonical_stem {
                continue;
            }
            let alias = format!("{old_stem}{suffix}");
            candidates
                .entry(normalize_id(&alias))
                .or_default()
                .insert(node.id.clone());
            candidates.entry(alias).or_default().insert(node.id.clone());
        }
    }
    for (alias, ids) in candidates {
        if ids.len() == 1
            && let Some(id) = ids.into_iter().next()
        {
            normalized.entry(alias).or_insert(id);
        }
    }
}

fn remap_endpoint(value: &str, remap: &HashMap<String, String>) -> String {
    let mut current = value;
    let mut remaining = remap.len() + 1;
    while remaining > 0 {
        let Some(next) = remap.get(current) else {
            break;
        };
        if next == current {
            break;
        }
        current = next;
        remaining -= 1;
    }
    current.to_owned()
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

fn is_cross_language_phantom(
    edge: &EdgeRecord,
    nodes: &[NodeRecord],
    positions: &HashMap<String, usize>,
) -> bool {
    let relation = relation(edge);
    if !matches!(
        relation,
        "calls" | "imports" | "imports_from" | "references"
    ) {
        return false;
    }
    let source_file = positions
        .get(&edge.source)
        .and_then(|index| nodes.get(*index))
        .map(|node| node.string("source_file"))
        .unwrap_or_default();
    let target_file = positions
        .get(&edge.target)
        .and_then(|index| nodes.get(*index))
        .map(|node| node.string("source_file"))
        .unwrap_or_default();
    let source_ext = extension(&source_file);
    let target_ext = extension(&target_file);
    let source_family = edge_language_family(&source_ext);
    let target_family = edge_language_family(&target_ext);
    if relation == "calls" {
        return edge.attributes.get("confidence").and_then(Value::as_str) == Some("INFERRED")
            && !source_ext.is_empty()
            && !target_ext.is_empty()
            && source_family != target_family;
    }
    source_family.is_some() && target_family.is_some() && source_family != target_family
}

fn extension(source: &str) -> String {
    Path::new(source)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn edge_language_family(extension: &str) -> Option<&'static str> {
    match extension {
        "py" | "pyi" => Some("py"),
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "mts" | "cts" => Some("js"),
        "go" => Some("go"),
        "rs" => Some("rs"),
        "java" | "kt" | "scala" | "groovy" => Some("jvm"),
        "c" | "h" | "cc" | "cpp" | "hpp" | "cxx" | "hh" | "hxx" | "cu" | "cuh" | "metal" | "m"
        | "mm" => Some("c"),
        "rb" | "rake" => Some("rb"),
        "php" => Some("php"),
        "cs" => Some("cs"),
        "swift" => Some("swift"),
        "lua" => Some("lua"),
        _ => None,
    }
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
                    .map(|member| remap_endpoint(member, rekey))
                    .filter_map(|member| resolve_endpoint(&member, positions, normalized))
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
