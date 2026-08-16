//! Deterministic graph construction and graph algorithms for Compass.

mod analyze;
mod cluster;
mod dedup;
mod inference;
mod quarantine;
mod snapshot;
mod v1;

pub use analyze::{
    BlindSpotEdge, BlindSpotLimits, BlindSpotNode, BlindSpotOmissions, BlindSpotReport,
    CommunityGap, DiffEdge, DiffNode, DisconnectedComponent, GRAPH_INSIGHTS_SCHEMA, GodNode,
    GraphDiff, GraphInsights, ImportCycle, SuggestedQuestion, SurpriseConnection,
    blind_spot_report, find_import_cycles, god_nodes, graph_diff, graph_insights,
    graph_insights_with_blind_spots, suggest_questions, surprising_connections,
};
pub use cluster::{
    ClusterOptions, Communities, IncrementalClusterLimits, IncrementalClusterResult, cluster,
    cluster_incremental, cohesion_score, community_member_signatures, label_communities_by_hub,
    remap_communities_to_previous, score_communities,
};
pub use compass_languages::{RawEdgeRecord, RawNodeRecord};
use dedup::deduplicate_owned;
pub use dedup::{
    AmbiguousPair, DedupError, DedupResult, DedupStats, EntityTiebreaker, deduplicate_entities,
    deduplicate_entities_with_tiebreaker,
};
pub use inference::{
    InferenceLevel, InferenceSelection, apply_inference_level, prefilter_extraction_inference,
};
use inference::{prefilter_raw_edges_inference, prune_orphan_external_placeholders};
pub use quarantine::{MAX_QUARANTINE_EXAMPLES, PublicationOmissions, PublicationOutcome};
pub use snapshot::{
    CanonicalGraphDocument, GRAPH_JSON_DELTA_MAX_SOURCE_BYTES, GRAPH_SNAPSHOT_LAYOUT_V1,
    GRAPH_SNAPSHOT_MAX_ITEMS, GRAPH_SNAPSHOT_MAX_OBJECTS, GRAPH_SNAPSHOT_SELECTOR_SCHEMA_V1,
    GRAPH_TERM_POSTING_CHUNK_ITEMS, GraphSnapshotBuilder, GraphSnapshotGcStats,
    GraphSnapshotManifest, GraphSnapshotMetadata, GraphSnapshotReader, IndexKind,
    PreparedGraphSnapshot, PreparedGraphSnapshotContent, SnapshotError, SnapshotReadLimits,
    SnapshotRoot, SnapshotSelector, TermPostingWork, activate_graph_snapshot,
    active_graph_snapshot, canonical_graph_document, canonical_graph_document_presorted,
    canonical_graph_json, encode_graph_index_key, garbage_collect_graph_snapshots,
    graph_snapshot_needs_gc, prepare_graph_snapshot, write_canonical_graph_json,
    write_fact_neutral_graph_json_delta, write_fact_neutral_graph_json_delta_prevalidated,
};
pub use v1::{
    BuildEvidence, InventoryEvidence, SourceDigest, V1_PUBLICATION_SEMANTICS_VERSION,
    canonical_edge_kind, canonical_raw_edge_sites, extraction_from_v1, normalize_document_v1,
    normalize_document_v1_with_evidence_best_effort_owned,
    normalize_document_v1_with_evidence_best_effort_owned_at_inference,
    normalize_document_v1_with_inventory,
    normalize_document_v1_with_inventory_and_source_digests_best_effort_owned,
    normalize_document_v1_with_inventory_and_source_digests_best_effort_owned_at_inference,
    normalize_document_v1_with_inventory_best_effort,
    normalize_document_v1_with_inventory_best_effort_at_inference,
    normalize_document_v1_with_inventory_best_effort_owned, normalize_v1, normalize_v1_best_effort,
    normalize_v1_best_effort_with_inference,
};

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use ahash::{AHashMap as HashMap, AHashSet as HashSet};
use compass_languages::{Extraction, file_stem, make_id, normalize_id};
use compass_model::provenance::{
    EndpointRewriteEvidence, EndpointRewriteRule, OCCURRENCE_RULE_ATTRIBUTE, SourceAnchor,
    append_endpoint_rewrite_evidence, preserve_occurrence_rule,
};
use compass_model::{
    EdgeRecord as LegacyEdgeRecord, GraphDocument, NodeRecord as LegacyNodeRecord,
};
use rayon::prelude::*;
use serde_json::{Map, Value};

type EdgeRecord = RawEdgeRecord;
type NodeRecord = RawNodeRecord;
type EndpointAliases = HashMap<String, BTreeSet<String>>;

#[derive(Clone, Copy)]
struct NodeSourceFacts<'a> {
    source_file: &'a str,
    has_extension: bool,
    language_family: Option<&'static str>,
}

const COALESCED_EDGE_EVIDENCE: &str = "_coalesced_edge_evidence";
pub const GRAPH_DIAGNOSTICS_EXTENSION: &str = "_compass_v1_graph_diagnostics";

enum EndpointResolution<'a> {
    Exact(&'a str),
    Rewritten {
        endpoint: String,
        evidence: EndpointRewriteEvidence,
    },
    Ambiguous {
        alias: String,
        candidates: Vec<String>,
    },
    Missing,
}

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
    extraction: Extraction,
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
) -> Result<GraphDocument, DedupError> {
    build_owned_with_tiebreaker_at_inference(
        extraction,
        directed,
        dedup,
        root,
        tiebreaker,
        InferenceLevel::Max,
    )
}

/// Build an owned graph while admitting inferred relationships at the final
/// raw-record boundary, after endpoint rewrites and duplicate evidence have
/// been resolved but before the node-link document is materialized.
pub fn build_owned_with_tiebreaker_at_inference(
    mut extraction: Extraction,
    directed: bool,
    dedup: bool,
    root: Option<&Path>,
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
    inference_level: InferenceLevel,
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
    let document = build_from_owned_extraction(extraction, directed, root, inference_level);
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
    build_from_owned_extraction(extraction.clone(), directed, root, InferenceLevel::Max)
}

fn build_from_owned_extraction(
    mut extraction: Extraction,
    directed: bool,
    root: Option<&Path>,
    inference_level: InferenceLevel,
) -> GraphDocument {
    let mut profile_started = Instant::now();
    let mut graph_diagnostics = extraction
        .extensions
        .remove(GRAPH_DIAGNOSTICS_EXTENSION)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
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

    let mut endpoint_rewrite_evidence = HashMap::new();
    for old in rekey.keys() {
        endpoint_rewrite_evidence.insert(
            old.clone(),
            EndpointRewriteEvidence {
                rule: EndpointRewriteRule::GraphSemanticIdRemap,
                score: 1.0,
            },
        );
    }
    // The semantic remap is the first stage in the document-twin rule. Keep
    // only the raw endpoint IDs that can reach a document-twin remap before
    // transferring all three maps into the single lookup table below. This
    // preserves the self-loop quarantine check without cloning the complete
    // remap maps at the graph-assembly peak.
    let mut doc_twin_rewrite_sources =
        HashSet::with_capacity(doc_remap.len().saturating_add(rekey.len()));
    doc_twin_rewrite_sources.extend(doc_remap.keys().cloned());
    for old in rekey.keys() {
        if doc_remap.contains_key(&remap_endpoint(old, &rekey)) {
            doc_twin_rewrite_sources.insert(old.clone());
        }
    }
    for old in doc_remap.keys() {
        endpoint_rewrite_evidence.insert(
            old.clone(),
            EndpointRewriteEvidence {
                rule: EndpointRewriteRule::GraphDocumentTwinRemap,
                score: 1.0,
            },
        );
    }
    for old in ghost_remap.keys() {
        endpoint_rewrite_evidence.insert(
            old.clone(),
            EndpointRewriteEvidence {
                rule: EndpointRewriteRule::GraphGhostEndpointRemap,
                score: 0.95,
            },
        );
    }
    let mut endpoint_remap = rekey;
    endpoint_remap.extend(doc_remap);
    endpoint_remap.extend(ghost_remap);

    let mut normalized = EndpointAliases::new();
    for node in &nodes {
        normalized
            .entry(normalize_id(&node.id))
            .or_default()
            .insert(node.id.clone());
    }
    for (legacy, canonical) in &endpoint_remap {
        let canonical = remap_endpoint(canonical, &endpoint_remap);
        if positions.contains_key(&canonical) {
            normalized
                .entry(normalize_id(legacy))
                .or_default()
                .insert(canonical);
        }
    }
    let needed_aliases = referenced_legacy_aliases(&extraction, &endpoint_remap, &positions);
    add_legacy_alias_candidates(&nodes, &needed_aliases, &mut normalized);
    profile_internal("graph alias preparation", &mut profile_started);

    // Edge normalization asks the same two endpoint questions for every
    // relation. Cache the immutable node facts once so the hot loop does not
    // allocate a source string and parse its extension for every edge.
    let node_source_facts = nodes
        .iter()
        .map(|node| {
            let source_file = node
                .attributes
                .get("source_file")
                .and_then(Value::as_str)
                .unwrap_or_default();
            NodeSourceFacts {
                source_file,
                has_extension: Path::new(source_file)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| !extension.is_empty()),
                language_family: source_language_family(source_file),
            }
        })
        .collect::<Vec<_>>();

    let mut source_edges = std::mem::take(&mut extraction.edges);
    for edge in &mut source_edges {
        preserve_occurrence_rule(&mut edge.attributes);
        let original_source_was_doc_twin = doc_twin_rewrite_sources.contains(edge.source.as_str());
        let original_target_was_doc_twin = doc_twin_rewrite_sources.contains(edge.target.as_str());
        let (source, mut rewrites) =
            remap_endpoint_with_evidence(&edge.source, &endpoint_remap, &endpoint_rewrite_evidence);
        let (target, target_rewrites) =
            remap_endpoint_with_evidence(&edge.target, &endpoint_remap, &endpoint_rewrite_evidence);
        if let Cow::Owned(source) = source {
            edge.source = source;
        }
        if let Cow::Owned(target) = target {
            edge.target = target;
        }
        rewrites.extend(target_rewrites);
        rewrites.sort_by_key(|rewrite| rewrite.rule);
        rewrites.dedup_by_key(|rewrite| rewrite.rule);
        if !rewrites.is_empty() {
            stamp_graph_endpoint_rewrites(edge, &rewrites);
        }
        if edge.source == edge.target
            && (original_source_was_doc_twin || original_target_was_doc_twin)
        {
            edge.attributes
                .insert("_drop".to_owned(), Value::Bool(true));
        }
    }
    if source_edges.len() < 100_000 {
        source_edges.sort_by_cached_key(edge_occurrence_key);
    } else {
        source_edges.par_sort_by_cached_key(edge_occurrence_key);
    }
    profile_internal("graph edge cloning and sort", &mut profile_started);
    let normalize_edge = |mut edge: EdgeRecord| {
        let mut diagnostics = Vec::new();
        if edge.attributes.remove("_drop") == Some(Value::Bool(true)) {
            return (None, diagnostics);
        }
        let source = match resolve_edge_endpoint(
            &edge.source,
            "source",
            &positions,
            &normalized,
            &mut diagnostics,
        ) {
            Some(EndpointResolution::Exact(_)) => None,
            Some(EndpointResolution::Rewritten { endpoint, evidence }) => {
                stamp_graph_endpoint_rewrites(&mut edge, &[evidence]);
                Some(endpoint)
            }
            Some(EndpointResolution::Ambiguous { .. })
            | Some(EndpointResolution::Missing)
            | None => return (None, diagnostics),
        };
        let target = match resolve_edge_endpoint(
            &edge.target,
            "target",
            &positions,
            &normalized,
            &mut diagnostics,
        ) {
            Some(EndpointResolution::Exact(_)) => None,
            Some(EndpointResolution::Rewritten { endpoint, evidence }) => {
                stamp_graph_endpoint_rewrites(&mut edge, &[evidence]);
                Some(endpoint)
            }
            Some(EndpointResolution::Ambiguous { .. })
            | Some(EndpointResolution::Missing)
            | None => return (None, diagnostics),
        };
        if let Some(source) = source {
            edge.source = source;
        }
        if let Some(target) = target {
            edge.target = target;
        }
        edge.attributes.remove("target_file");
        sanitize_numeric(&mut edge.attributes, "weight");
        sanitize_numeric(&mut edge.attributes, "confidence_score");
        backfill_source_file(&mut edge, &positions, &node_source_facts);
        normalize_attribute_path(&mut edge.attributes, "source_file", root);
        normalize_source_anchor_path(&mut edge.attributes, root);
        if is_cross_language_phantom(&edge, &positions, &node_source_facts) {
            return (None, diagnostics);
        }
        // NetworkX-style undirected documents may reorder endpoints, so keep
        // the pre-order direction as private metadata in that mode. Directed
        // documents already retain the authoritative endpoints directly on
        // the edge and do not need a second pair of ID strings per link.
        if !directed {
            edge.attributes
                .insert("_src".to_owned(), Value::String(edge.source.clone()));
            edge.attributes
                .insert("_tgt".to_owned(), Value::String(edge.target.clone()));
        }
        (Some(edge), diagnostics)
    };
    let normalized_results = if source_edges.len() < 100_000 {
        source_edges
            .into_iter()
            .map(normalize_edge)
            .collect::<Vec<_>>()
    } else {
        source_edges
            .into_par_iter()
            .map(normalize_edge)
            .collect::<Vec<_>>()
    };
    let mut normalized_edges = Vec::new();
    for (edge, mut diagnostics) in normalized_results {
        graph_diagnostics.append(&mut diagnostics);
        if let Some(edge) = edge {
            normalized_edges.push(edge);
        }
    }
    let mut links = Vec::<EdgeRecord>::new();
    let mut edge_positions = HashMap::<(String, String, String, String, String), usize>::new();
    for edge in normalized_edges {
        let key = edge_occurrence_key(&edge);
        if let Some(&position) = edge_positions.get(&key) {
            merge_edge_attributes(&mut links[position].attributes, edge.attributes);
        } else {
            edge_positions.insert(key, links.len());
            links.push(edge);
        }
    }
    prefilter_raw_edges_inference(&nodes, &mut links, inference_level);
    profile_internal("graph inference admission", &mut profile_started);
    profile_internal("graph edge normalization", &mut profile_started);

    let mut graph = Map::new();
    let (hyperedges, mut hyperedge_diagnostics) = canonical_hyperedges(
        &extraction,
        &positions,
        &normalized,
        &endpoint_remap,
        &endpoint_rewrite_evidence,
        root,
    );
    graph_diagnostics.append(&mut hyperedge_diagnostics);
    prune_orphan_external_placeholders(&mut nodes, &links, &hyperedges, inference_level);
    profile_internal("graph placeholder admission", &mut profile_started);
    if !hyperedges.is_empty() {
        graph.insert("hyperedges".to_owned(), Value::Array(hyperedges));
    }
    if !graph_diagnostics.is_empty() {
        graph_diagnostics.sort_by_cached_key(Value::to_string);
        graph_diagnostics.dedup();
        graph.insert(
            GRAPH_DIAGNOSTICS_EXTENSION.to_owned(),
            Value::Array(graph_diagnostics),
        );
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
    let mut canonical = HashMap::<(String, String, String, String), Vec<String>>::new();
    for node in nodes
        .iter()
        .filter(|node| node.attributes.get("_origin").and_then(Value::as_str) == Some("ast"))
    {
        if let Some(identity) = ghost_semantic_identity(node) {
            canonical.entry(identity).or_default().push(node.id.clone());
        }
    }
    let mut remap = HashMap::new();
    for node in nodes {
        if node.attributes.get("_origin").and_then(Value::as_str) == Some("ast") {
            continue;
        }
        let Some(identity) = ghost_semantic_identity(node) else {
            continue;
        };
        if let Some([target]) = canonical.get(&identity).map(Vec::as_slice)
            && target != &node.id
        {
            remap.insert(node.id.clone(), target.clone());
        }
    }
    remap
}

fn ghost_semantic_identity(node: &NodeRecord) -> Option<(String, String, String, String)> {
    let source = node.string("source_file");
    let kind = node
        .attributes
        .get("symbol_kind")
        .or_else(|| node.attributes.get("type"))
        .or_else(|| node.attributes.get("file_type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let qualified_name = node
        .attributes
        .get("qualified_name")
        .or_else(|| node.attributes.get("qualifiedName"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| node.label())
        .trim();
    if source.is_empty() || kind.is_empty() || qualified_name.is_empty() {
        return None;
    }
    let mut details = Map::new();
    for key in [
        "signature",
        "declaring_type",
        "overload_discriminator",
        "resource_kind",
        "config_path",
        "package_name",
        "database",
        "database_schema",
        "logical_database",
        "transport",
        "subject",
    ] {
        if let Some(value) = node.attributes.get(key) {
            details.insert(key.to_owned(), value.clone());
        }
    }
    Some((
        source,
        kind.to_owned(),
        qualified_name.to_owned(),
        Value::Object(details).to_string(),
    ))
}

fn referenced_legacy_aliases(
    extraction: &Extraction,
    endpoint_remap: &HashMap<String, String>,
    positions: &HashMap<String, usize>,
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
        needed.insert(normalize_id(endpoint));
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

fn add_legacy_alias_candidates(
    nodes: &[NodeRecord],
    needed: &HashSet<String>,
    normalized: &mut EndpointAliases,
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
        normalized.entry(alias).or_default().extend(ids);
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

fn remap_endpoint_with_evidence<'a>(
    value: &'a str,
    remap: &HashMap<String, String>,
    evidence: &HashMap<String, EndpointRewriteEvidence>,
) -> (Cow<'a, str>, Vec<EndpointRewriteEvidence>) {
    let mut current = value;
    let mut rewrites = Vec::new();
    let mut remaining = remap.len() + 1;
    while remaining > 0 {
        let Some(next) = remap.get(current) else {
            break;
        };
        if next == current {
            break;
        }
        if let Some(item) = evidence.get(current) {
            rewrites.push(*item);
        }
        current = next;
        remaining -= 1;
    }
    let endpoint = if current == value {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(current.to_owned())
    };
    (endpoint, rewrites)
}

fn stamp_graph_endpoint_rewrites(edge: &mut EdgeRecord, rewrites: &[EndpointRewriteEvidence]) {
    stamp_endpoint_rewrite_attributes(&mut edge.attributes, rewrites);
}

fn stamp_endpoint_rewrite_attributes(
    attributes: &mut Map<String, Value>,
    rewrites: &[EndpointRewriteEvidence],
) {
    for rewrite in rewrites {
        append_endpoint_rewrite_evidence(attributes, *rewrite);
    }
}

fn merge_edge_attributes(existing: &mut Map<String, Value>, incoming: Map<String, Value>) {
    let mut snapshots = edge_evidence_snapshots(existing);
    snapshots.extend(edge_evidence_snapshots(&incoming));
    snapshots.sort_by_cached_key(Value::to_string);
    snapshots.dedup();

    let mut merged = Map::new();
    for attributes in [existing.clone(), incoming] {
        for (key, value) in attributes {
            if key == COALESCED_EDGE_EVIDENCE {
                continue;
            }
            match merged.get(&key) {
                Some(current) if json_value_is_at_most(current, &value) => {}
                _ => {
                    merged.insert(key, value);
                }
            }
        }
    }

    let primary_heuristic = snapshots
        .iter()
        .filter_map(Value::as_object)
        .filter(|attributes| edge_evidence_is_heuristic(attributes))
        .min_by_key(|attributes| Value::Object((*attributes).clone()).to_string())
        .cloned();
    if let Some(primary) = primary_heuristic {
        for key in [
            "_origin",
            "origin",
            "confidence",
            "rule",
            "confidence_score",
            "score",
            "extractor",
            "source_file",
            "source_location",
            "source_anchor",
            "line_start",
            "line_end",
            "column_start",
            "column_end",
            "start_byte",
            "end_byte",
            "candidates",
        ] {
            merged.remove(key);
            if let Some(value) = primary.get(key) {
                merged.insert(key.to_owned(), value.clone());
            }
        }
        if let Some(rewrite_entries) = primary
            .get("_endpoint_rewrite_rules")
            .and_then(Value::as_array)
        {
            merged.insert(
                "_endpoint_rewrite_rules".to_owned(),
                Value::Array(rewrite_entries.clone()),
            );
        } else {
            merged.remove("_endpoint_rewrite_rules");
        }
    }
    merged.insert(COALESCED_EDGE_EVIDENCE.to_owned(), Value::Array(snapshots));
    merged.sort_keys();
    *existing = merged;
}

fn json_value_is_at_most(left: &Value, right: &Value) -> bool {
    let left = left.to_string();
    let right = right.to_string();
    left <= right
}

fn edge_evidence_snapshots(attributes: &Map<String, Value>) -> Vec<Value> {
    if let Some(snapshots) = attributes
        .get(COALESCED_EDGE_EVIDENCE)
        .and_then(Value::as_array)
    {
        return snapshots.clone();
    }
    let mut snapshot = attributes.clone();
    snapshot.remove(COALESCED_EDGE_EVIDENCE);
    vec![Value::Object(snapshot)]
}

fn edge_evidence_is_heuristic(attributes: &Map<String, Value>) -> bool {
    attributes
        .get("_endpoint_rewrite_rules")
        .and_then(Value::as_array)
        .is_some_and(|rules| !rules.is_empty())
        || attributes
            .get("_origin")
            .or_else(|| attributes.get("origin"))
            .and_then(Value::as_str)
            == Some("heuristic")
        || matches!(
            attributes.get("confidence").and_then(Value::as_str),
            Some("INFERRED" | "inferred" | "AMBIGUOUS" | "ambiguous")
        )
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

/// Count the nodes that remain after ID-based deduplication without cloning
/// their attributes. This is useful for guards that only need the resulting
/// cardinality, not the deduplicated records themselves.
#[must_use]
pub fn deduped_node_count(nodes: &[NodeRecord]) -> usize {
    let mut ids = HashSet::with_capacity(nodes.len());
    nodes
        .iter()
        .filter(|node| ids.insert(node.id.as_str()))
        .count()
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

fn normalize_source_anchor_path(attributes: &mut Map<String, Value>, root: Option<&Path>) {
    for key in ["source_anchor", "sourceAnchor", "anchor"] {
        let Some(anchor) = attributes.get_mut(key).and_then(Value::as_object_mut) else {
            continue;
        };
        let Some(file) = anchor.get("file").and_then(Value::as_str) else {
            continue;
        };
        anchor.insert(
            "file".to_owned(),
            Value::String(normalize_source_file(file, root)),
        );
    }
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

fn resolve_endpoint<'a>(
    value: &'a str,
    positions: &HashMap<String, usize>,
    normalized: &EndpointAliases,
) -> EndpointResolution<'a> {
    if positions.contains_key(value) {
        return EndpointResolution::Exact(value);
    }
    let alias = normalize_id(value);
    let Some(candidates) = normalized.get(&alias) else {
        return EndpointResolution::Missing;
    };
    if candidates.len() == 1 {
        return EndpointResolution::Rewritten {
            endpoint: candidates.iter().next().cloned().unwrap_or_default(),
            evidence: EndpointRewriteEvidence {
                rule: EndpointRewriteRule::GraphNormalizedIdRemap,
                score: 0.8,
            },
        };
    }
    EndpointResolution::Ambiguous {
        alias,
        candidates: candidates.iter().cloned().collect(),
    }
}

fn resolve_edge_endpoint<'a>(
    value: &'a str,
    role: &str,
    positions: &HashMap<String, usize>,
    normalized: &EndpointAliases,
    diagnostics: &mut Vec<Value>,
) -> Option<EndpointResolution<'a>> {
    match resolve_endpoint(value, positions, normalized) {
        exact @ EndpointResolution::Exact(_) => Some(exact),
        rewritten @ EndpointResolution::Rewritten { .. } => Some(rewritten),
        EndpointResolution::Ambiguous { alias, candidates } => {
            diagnostics.push(ambiguous_endpoint_diagnostic(
                value, &alias, role, candidates,
            ));
            None
        }
        EndpointResolution::Missing => None,
    }
}

fn ambiguous_endpoint_diagnostic(
    value: &str,
    alias: &str,
    role: &str,
    candidates: Vec<String>,
) -> Value {
    serde_json::json!({
        "severity": "warning",
        "code": "ambiguous_normalized_endpoint",
        "message": format!(
            "{role} endpoint {value:?} has ambiguous normalized alias {alias:?}; topology omitted"
        ),
        "relatedIds": candidates,
    })
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
    positions: &HashMap<String, usize>,
    node_source_facts: &[NodeSourceFacts<'_>],
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
        .and_then(|index| node_source_facts.get(*index))
        .map(|facts| facts.source_file)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            positions
                .get(&edge.target)
                .and_then(|index| node_source_facts.get(*index))
                .map(|facts| facts.source_file)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default();
    edge.attributes
        .insert("source_file".to_owned(), Value::String(source.to_owned()));
}

fn is_cross_language_phantom(
    edge: &EdgeRecord,
    positions: &HashMap<String, usize>,
    node_source_facts: &[NodeSourceFacts<'_>],
) -> bool {
    let relation = relation(edge);
    if !matches!(
        relation,
        "calls" | "imports" | "imports_from" | "references"
    ) {
        return false;
    }
    let source_facts = positions
        .get(&edge.source)
        .and_then(|index| node_source_facts.get(*index));
    let target_facts = positions
        .get(&edge.target)
        .and_then(|index| node_source_facts.get(*index));
    let source_family = source_facts.and_then(|facts| facts.language_family);
    let target_family = target_facts.and_then(|facts| facts.language_family);
    if relation == "calls" {
        return edge.attributes.get("confidence").and_then(Value::as_str) == Some("INFERRED")
            && source_facts.is_some_and(|facts| facts.has_extension)
            && target_facts.is_some_and(|facts| facts.has_extension)
            && source_family != target_family;
    }
    source_family.is_some() && target_family.is_some() && source_family != target_family
}

fn source_language_family(source: &str) -> Option<&'static str> {
    let extension = Path::new(source).extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("py") || extension.eq_ignore_ascii_case("pyi") {
        Some("py")
    } else if ["js", "mjs", "cjs", "jsx", "ts", "tsx", "mts", "cts"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    {
        Some("js")
    } else if extension.eq_ignore_ascii_case("go") {
        Some("go")
    } else if extension.eq_ignore_ascii_case("rs") {
        Some("rs")
    } else if ["java", "kt", "scala", "groovy"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    {
        Some("jvm")
    } else if [
        "c", "h", "cc", "cpp", "hpp", "cxx", "hh", "hxx", "cu", "cuh", "metal", "m", "mm",
    ]
    .iter()
    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    {
        Some("c")
    } else if extension.eq_ignore_ascii_case("rb") || extension.eq_ignore_ascii_case("rake") {
        Some("rb")
    } else if extension.eq_ignore_ascii_case("php") {
        Some("php")
    } else if extension.eq_ignore_ascii_case("cs") {
        Some("cs")
    } else if extension.eq_ignore_ascii_case("swift") {
        Some("swift")
    } else if extension.eq_ignore_ascii_case("lua") {
        Some("lua")
    } else {
        None
    }
}

fn edge_occurrence_key(edge: &EdgeRecord) -> (String, String, String, String, String) {
    (
        edge.source.clone(),
        edge.target.clone(),
        relation(edge).to_owned(),
        edge_anchor_key(&edge.attributes),
        edge_rule_key(&edge.attributes),
    )
}

fn edge_anchor_key(attributes: &Map<String, Value>) -> String {
    if let Some(anchor) = canonical_edge_anchor(attributes) {
        return serde_json::to_string(&[
            Value::String(anchor.file),
            Value::from(anchor.start_byte),
            Value::from(anchor.end_byte),
            Value::from(anchor.start_line),
            Value::from(anchor.start_column),
            Value::from(anchor.end_line),
            Value::from(anchor.end_column),
        ])
        .unwrap_or_default();
    }
    serde_json::to_string(&[
        attributes
            .get("source_file")
            .and_then(Value::as_str)
            .map(|file| Value::String(file.replace('\\', "/")))
            .unwrap_or(Value::Null),
        attributes
            .get("source_location")
            .cloned()
            .unwrap_or(Value::Null),
    ])
    .unwrap_or_default()
}

fn canonical_edge_anchor(attributes: &Map<String, Value>) -> Option<SourceAnchor> {
    if let Some(mut anchor) = attributes
        .get("source_anchor")
        .or_else(|| attributes.get("sourceAnchor"))
        .or_else(|| attributes.get("anchor"))
        .and_then(|value| serde_json::from_value::<SourceAnchor>(value.clone()).ok())
    {
        anchor.file = anchor.file.replace('\\', "/");
        return Some(anchor);
    }
    Some(SourceAnchor {
        file: attributes.get("source_file")?.as_str()?.replace('\\', "/"),
        start_byte: attributes.get("start_byte")?.as_u64()?,
        end_byte: attributes.get("end_byte")?.as_u64()?,
        start_line: u32::try_from(attributes.get("line_start")?.as_u64()?).ok()?,
        start_column: u32::try_from(attributes.get("column_start")?.as_u64()?).ok()?,
        end_line: u32::try_from(attributes.get("line_end")?.as_u64()?).ok()?,
        end_column: u32::try_from(attributes.get("column_end")?.as_u64()?).ok()?,
    })
}

fn edge_rule_key(attributes: &Map<String, Value>) -> String {
    attributes
        .get(OCCURRENCE_RULE_ATTRIBUTE)
        .or_else(|| attributes.get("rule"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
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
    normalized: &EndpointAliases,
    rekey: &HashMap<String, String>,
    rewrite_evidence: &HashMap<String, EndpointRewriteEvidence>,
    root: Option<&Path>,
) -> (Vec<Value>, Vec<Value>) {
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    for value in &extraction.hyperedges {
        let Some(mut hyperedge) = value.as_object().cloned() else {
            continue;
        };
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
            let mut valid = Vec::new();
            let mut rewrites = Vec::new();
            let mut ambiguous = false;
            for member in members.iter().filter_map(Value::as_str) {
                let (remapped, mut member_rewrites) =
                    remap_endpoint_with_evidence(member, rekey, rewrite_evidence);
                rewrites.append(&mut member_rewrites);
                match resolve_endpoint(remapped.as_ref(), positions, normalized) {
                    EndpointResolution::Exact(endpoint) => {
                        valid.push(Value::String(endpoint.to_owned()));
                    }
                    EndpointResolution::Rewritten { endpoint, evidence } => {
                        valid.push(Value::String(endpoint));
                        rewrites.push(evidence);
                    }
                    EndpointResolution::Ambiguous { alias, candidates } => {
                        ambiguous = true;
                        diagnostics.push(ambiguous_endpoint_diagnostic(
                            member,
                            &alias,
                            "hyperedge member",
                            candidates,
                        ));
                    }
                    EndpointResolution::Missing => {}
                }
            }
            if ambiguous || valid.is_empty() {
                continue;
            }
            hyperedge.insert("nodes".to_owned(), Value::Array(valid));
            stamp_endpoint_rewrite_attributes(&mut hyperedge, &rewrites);
        }
        output.push(Value::Object(hyperedge));
    }
    (output, diagnostics)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{dedupe_nodes, deduped_node_count, remap_endpoint_with_evidence};
    use ahash::AHashMap;
    use compass_languages::RawNodeRecord;
    use serde_json::Map;

    #[test]
    fn deduped_node_count_matches_record_deduplication_without_cloning_records() {
        let nodes = vec![
            RawNodeRecord {
                id: "duplicate".to_owned(),
                attributes: Map::new(),
            },
            RawNodeRecord {
                id: "unique".to_owned(),
                attributes: Map::new(),
            },
            RawNodeRecord {
                id: "duplicate".to_owned(),
                attributes: Map::new(),
            },
        ];

        assert_eq!(deduped_node_count(&nodes), dedupe_nodes(&nodes).len());
    }

    #[test]
    fn endpoint_remap_borrows_unchanged_ids_and_owns_rewritten_ids() {
        let remap = AHashMap::new();
        let evidence = AHashMap::new();
        let unchanged = String::from("stable");
        let (unchanged_endpoint, unchanged_rewrites) =
            remap_endpoint_with_evidence(&unchanged, &remap, &evidence);
        assert!(matches!(unchanged_endpoint, Cow::Borrowed("stable")));
        assert!(unchanged_rewrites.is_empty());

        let remap = AHashMap::from([(String::from("legacy"), String::from("canonical"))]);
        let rewritten = String::from("legacy");
        let (rewritten_endpoint, rewritten_rewrites) =
            remap_endpoint_with_evidence(&rewritten, &remap, &evidence);
        assert!(matches!(rewritten_endpoint, Cow::Owned(endpoint) if endpoint == "canonical"));
        assert!(rewritten_rewrites.is_empty());
    }
}
