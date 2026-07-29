//! Deterministic graph construction and graph algorithms for Compass.

mod analyze;
mod cluster;
mod dedup;
mod v1;

pub use analyze::{
    DiffEdge, DiffNode, GodNode, GraphDiff, ImportCycle, SuggestedQuestion, SurpriseConnection,
    find_import_cycles, god_nodes, graph_diff, graph_insights, suggest_questions,
    surprising_connections,
};
pub use cluster::{
    ClusterOptions, Communities, cluster, cohesion_score, community_member_signatures,
    label_communities_by_hub, remap_communities_to_previous, score_communities,
};
pub use compass_languages::{RawEdgeRecord, RawNodeRecord};
use dedup::deduplicate_owned;
pub use dedup::{
    AmbiguousPair, DedupError, DedupResult, DedupStats, EntityTiebreaker, deduplicate_entities,
    deduplicate_entities_with_tiebreaker,
};
pub use v1::{
    BuildEvidence, InventoryEvidence, extraction_from_v1, normalize_document_v1,
    normalize_document_v1_with_inventory, normalize_v1,
};

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use ahash::{AHashMap as HashMap, AHashSet as HashSet};
use compass_languages::{Extraction, file_stem, make_id, normalize_id};
use compass_model::{
    EdgeRecord as LegacyEdgeRecord, GraphDocument, NodeRecord as LegacyNodeRecord,
};
use rayon::prelude::*;
use serde_json::{Map, Value};

type EdgeRecord = RawEdgeRecord;
type NodeRecord = RawNodeRecord;

/// Merge resolved extraction chunks, apply native entity deduplication, and build
/// a node-link graph. This is the deterministic counterpart of `compass.build`.
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
            &std::collections::HashMap::new(),
            tiebreaker,
        )?;
        combined.nodes = result.nodes;
        combined.edges = result.edges;
    }
    Ok(build_from_extraction(&combined, directed, root))
}

/// Build a graph from an owned extraction without first cloning the complete
/// node and edge inventory into a temporary combined extraction.
pub fn build_owned_with_tiebreaker(
    mut extraction: Extraction,
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
) -> Result<GraphDocument, DedupError> {
    let mut profile_started = Instant::now();
    if dedup && !extraction.nodes.is_empty() {
        let result = deduplicate_owned(
            std::mem::take(&mut extraction.nodes),
            std::mem::take(&mut extraction.edges),
            &std::collections::HashMap::new(),
            tiebreaker,
        )?;
        extraction.nodes = result.nodes;
        extraction.edges = result.edges;
    }
    profile_internal("graph entity deduplication", &mut profile_started);
    let document = build_from_owned_extraction(extraction, directed, root);
    profile_internal("graph extraction conversion", &mut profile_started);
    Ok(document)
}

/// Build a NetworkX-compatible node-link document from extraction facts.
#[must_use]
pub fn build_from_extraction(
    extraction: &Extraction,
    directed: bool,
    root: Option<&Path>,
) -> GraphDocument {
    build_from_owned_extraction(extraction.clone(), directed, root)
}

fn build_from_owned_extraction(
    mut extraction: Extraction,
    directed: bool,
    root: Option<&Path>,
) -> GraphDocument {
    let mut profile_started = Instant::now();
    let rekey = semantic_id_remap(&extraction.nodes, root);
    let mut prepared_nodes = std::mem::take(&mut extraction.nodes)
        .into_iter()
        .map(|mut node| {
            if let Some(canonical) = rekey.get(&node.id) {
                node.id.clone_from(canonical);
            }
            canonicalize_node(&mut node, root);
            node
        })
        .collect::<Vec<_>>();
    profile_internal("graph node preparation", &mut profile_started);
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
    profile_internal("graph node deduplication", &mut profile_started);

    let ghost_remap = ghost_duplicate_remap(&nodes);
    if !ghost_remap.is_empty() {
        nodes.retain(|node| !ghost_remap.contains_key(&node.id));
        positions.clear();
        for (index, node) in nodes.iter().enumerate() {
            positions.insert(node.id.clone(), index);
        }
    }
    profile_internal("graph ghost remapping", &mut profile_started);

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
    let needed_aliases =
        needed_legacy_aliases(&extraction, &endpoint_remap, &positions, &normalized);
    add_unambiguous_legacy_aliases(&nodes, &needed_aliases, &mut normalized);
    profile_internal("graph alias preparation", &mut profile_started);

    let mut source_edges = std::mem::take(&mut extraction.edges);
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
    source_edges.par_sort_by(|left, right| {
        (left.source.as_str(), left.target.as_str(), relation(left)).cmp(&(
            right.source.as_str(),
            right.target.as_str(),
            relation(right),
        ))
    });
    profile_internal("graph edge cloning and sort", &mut profile_started);
    let normalized_edges = source_edges
        .into_par_iter()
        .filter_map(|mut edge| {
            if edge.attributes.remove("_drop") == Some(Value::Bool(true)) {
                return None;
            }
            let source = resolve_endpoint(&edge.source, &positions, &normalized)?;
            let target = resolve_endpoint(&edge.target, &positions, &normalized)?;
            edge.source = source;
            edge.target = target;
            edge.attributes.remove("target_file");
            sanitize_numeric(&mut edge.attributes, "weight");
            sanitize_numeric(&mut edge.attributes, "confidence_score");
            backfill_source_file(&mut edge, &nodes, &positions);
            normalize_attribute_path(&mut edge.attributes, "source_file", root);
            if is_cross_language_phantom(&edge, &nodes, &positions) {
                return None;
            }
            edge.attributes
                .insert("_src".to_owned(), Value::String(edge.source.clone()));
            edge.attributes
                .insert("_tgt".to_owned(), Value::String(edge.target.clone()));
            Some(edge)
        })
        .collect::<Vec<_>>();
    let mut links = Vec::<EdgeRecord>::new();
    let mut edge_positions = HashMap::<(String, String, String), usize>::new();
    for edge in normalized_edges {
        let key = edge_key(&edge.source, &edge.target, relation(&edge));
        if let Some(&position) = edge_positions.get(&key) {
            links[position].attributes.extend(edge.attributes);
        } else {
            edge_positions.insert(key, links.len());
            links.push(edge);
        }
    }
    profile_internal("graph edge normalization", &mut profile_started);

    let mut graph = Map::new();
    let hyperedges =
        canonical_hyperedges(&extraction, &positions, &normalized, &endpoint_remap, root);
    if !hyperedges.is_empty() {
        graph.insert("hyperedges".to_owned(), Value::Array(hyperedges));
    }
    let links = networkx_edge_order(&nodes, links, directed);
    profile_internal("graph NetworkX edge ordering", &mut profile_started);
    let multigraph = has_parallel_edges(&links, directed);
    GraphDocument {
        directed,
        multigraph,
        graph,
        nodes: nodes.into_iter().map(publish_legacy_node).collect(),
        links: links.into_iter().map(publish_legacy_edge).collect(),
        extras: BTreeMap::new(),
    }
}

fn publish_legacy_node(record: RawNodeRecord) -> LegacyNodeRecord {
    LegacyNodeRecord {
        id: record.id,
        attributes: record.attributes,
    }
}

fn publish_legacy_edge(record: RawEdgeRecord) -> LegacyEdgeRecord {
    LegacyEdgeRecord {
        source: record.source,
        target: record.target,
        attributes: record.attributes,
    }
}

fn profile_internal(label: &str, started: &mut Instant) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!(
            "[compass internal] {label}: {:.3}s",
            started.elapsed().as_secs_f64()
        );
    }
    *started = Instant::now();
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
    let ast_ids = nodes
        .iter()
        .filter(|node| node.attributes.get("_origin").and_then(Value::as_str) == Some("ast"))
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut non_ast_sources = HashMap::<&str, HashSet<String>>::new();
    for node in nodes
        .iter()
        .filter(|node| node.attributes.get("_origin").and_then(Value::as_str) != Some("ast"))
    {
        non_ast_sources
            .entry(node.id.as_str())
            .or_default()
            .insert(node.string("source_file"));
    }
    let mut canonical = HashMap::<(String, String), String>::new();
    let mut collisions = HashSet::new();
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
            if canonical
                .get(&key)
                .is_some_and(|existing| ast_ids.contains(existing.as_str()))
            {
                collisions.insert(key.clone());
            }
            canonical.insert(key, node.id.clone());
        } else if let Some(existing) = canonical.get(&key) {
            let different_source = non_ast_sources
                .get(existing.as_str())
                .is_some_and(|sources| sources.iter().any(|candidate| candidate != &source));
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

fn needed_legacy_aliases(
    extraction: &Extraction,
    endpoint_remap: &HashMap<String, String>,
    positions: &HashMap<String, usize>,
    normalized: &HashMap<String, String>,
) -> HashSet<String> {
    let mut needed = HashSet::new();
    let mut inspect = |endpoint: &str| {
        if positions.contains_key(endpoint) {
            return;
        }
        let remapped;
        let endpoint = if endpoint_remap.contains_key(endpoint) {
            remapped = remap_endpoint(endpoint, endpoint_remap);
            if positions.contains_key(&remapped) {
                return;
            }
            remapped.as_str()
        } else {
            endpoint
        };
        let alias = normalize_id(endpoint);
        if !normalized.contains_key(&alias) {
            needed.insert(alias);
        }
    };
    for edge in &extraction.edges {
        inspect(&edge.source);
        inspect(&edge.target);
    }
    for hyperedge in &extraction.hyperedges {
        let Some(object) = hyperedge.as_object() else {
            continue;
        };
        for key in ["nodes", "members", "node_ids"] {
            if let Some(members) = object.get(key).and_then(Value::as_array) {
                for member in members.iter().filter_map(Value::as_str) {
                    inspect(member);
                }
            }
        }
    }
    needed
}

fn add_unambiguous_legacy_aliases(
    nodes: &[NodeRecord],
    needed: &HashSet<String>,
    normalized: &mut HashMap<String, String>,
) {
    if needed.is_empty() {
        return;
    }
    let mut nodes_by_source = std::collections::HashMap::<String, Vec<&NodeRecord>>::new();
    for node in nodes {
        let source = node.string("source_file");
        let path = Path::new(&source);
        if source.is_empty() || path.is_absolute() || path.file_name().is_none() {
            continue;
        }
        nodes_by_source.entry(source).or_default().push(node);
    }
    let candidates = nodes_by_source
        .into_par_iter()
        .fold(
            HashMap::<String, HashSet<String>>::new,
            |mut candidates, (source, source_nodes)| {
                let path = Path::new(&source);
                let canonical_stem = make_id(&[&file_stem(path)]);
                let old_stems = old_file_stems(path)
                    .into_iter()
                    .filter(|stem| stem != &canonical_stem)
                    .collect::<Vec<_>>();
                for node in source_nodes {
                    let normalized_id = normalize_id(&node.id);
                    let is_file =
                        path.file_name().and_then(|value| value.to_str()) == Some(node.label());
                    let suffix = if is_file {
                        ""
                    } else {
                        normalized_id
                            .strip_prefix(canonical_stem.as_str())
                            .unwrap_or_default()
                    };
                    for old_stem in &old_stems {
                        let normalized_alias = normalize_id(&format!("{old_stem}{suffix}"));
                        if needed.contains(&normalized_alias) {
                            candidates
                                .entry(normalized_alias)
                                .or_default()
                                .insert(node.id.clone());
                        }
                    }
                }
                candidates
            },
        )
        .reduce(HashMap::new, |mut combined, candidates| {
            for (alias, ids) in candidates {
                combined.entry(alias).or_default().extend(ids);
            }
            combined
        });
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
    links: Vec<EdgeRecord>,
    directed: bool,
) -> Vec<EdgeRecord> {
    let positions = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut incident = vec![Vec::new(); nodes.len()];
    for (edge_index, edge) in links.iter().enumerate() {
        let Some(&source) = positions.get(edge.source.as_str()) else {
            continue;
        };
        incident[source].push(edge_index);
        if !directed
            && let Some(&target) = positions.get(edge.target.as_str())
            && target != source
        {
            incident[target].push(edge_index);
        }
    }

    let mut links = links.into_iter().map(Some).collect::<Vec<_>>();
    if directed {
        let mut output = Vec::with_capacity(links.len());
        for edge_indices in incident {
            output.extend(
                edge_indices
                    .into_iter()
                    .filter_map(|index| links[index].take()),
            );
        }
        return output;
    }

    let mut output = Vec::with_capacity(links.len());
    let mut visited = vec![false; nodes.len()];
    for (node_index, edge_indices) in incident.into_iter().enumerate() {
        for edge_index in edge_indices {
            let Some(edge) = links[edge_index].as_ref() else {
                continue;
            };
            let Some(&source) = positions.get(edge.source.as_str()) else {
                continue;
            };
            let Some(&target) = positions.get(edge.target.as_str()) else {
                continue;
            };
            let other = if source == node_index { target } else { source };
            if visited[other] {
                continue;
            }
            let Some(mut emitted) = links[edge_index].take() else {
                continue;
            };
            emitted.source.clone_from(&nodes[node_index].id);
            emitted.target.clone_from(&nodes[other].id);
            output.push(emitted);
        }
        visited[node_index] = true;
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
    let mut seen = HashSet::new();
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

fn edge_key(source: &str, target: &str, relation: &str) -> (String, String, String) {
    (source.to_owned(), target.to_owned(), relation.to_owned())
}

fn has_parallel_edges(links: &[EdgeRecord], directed: bool) -> bool {
    let mut endpoints = HashSet::new();
    links.iter().any(|edge| {
        let source = edge.source.as_str();
        let target = edge.target.as_str();
        let key = if directed || source <= target {
            (source, target)
        } else {
            (target, source)
        };
        !endpoints.insert(key)
    })
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
