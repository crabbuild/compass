use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use compass_model::code_graph::{DiagnosticSeverity, GraphDocument};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, effective_confidence};
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};

use crate::CoreError;

pub fn diagnose_graph_file(
    path: &Path,
    directed: Option<bool>,
    max_examples: usize,
    extract_path: Option<&Path>,
) -> Result<Value, CoreError> {
    enforce_graph_size_cap(path)?;
    let bytes = fs::read(path).map_err(|source| {
        CoreError::DiagnosticFile(format!(
            "Cannot parse {}: {}. The file may be corrupted — re-run 'compass extract'.",
            path.display(),
            python_io_error(path, &source)
        ))
    })?;
    let input: Value = serde_json::from_slice(&bytes).map_err(|source| {
        CoreError::DiagnosticFile(format!(
            "Cannot parse {}: {}. The file may be corrupted — re-run 'compass extract'.",
            path.display(),
            python_json_error(&bytes, &source)
        ))
    })?;
    let object = input.as_object().ok_or(CoreError::InvalidDiagnostic)?;
    let nodes = object
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let edges = object
        .get("edges")
        .filter(|value| !value.is_null())
        .or_else(|| object.get("links"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let effective_directed = directed.unwrap_or_else(|| {
        object
            .get("directed")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    });
    let node_ids = nodes
        .iter()
        .filter_map(|node| node.get("id").filter(|id| !id.is_null()).map(text))
        .collect::<HashSet<_>>();
    let unverified = nodes
        .iter()
        .filter(|node| node.get("verification").and_then(Value::as_str) == Some("unverified"))
        .count();
    let mut exact = HashMap::<String, usize>::new();
    let mut directed_pairs = HashMap::<(String, String), usize>::new();
    let mut undirected_pairs = HashMap::<(String, String), usize>::new();
    let mut order = Vec::new();
    let mut groups = HashMap::<(String, String), Vec<Edge>>::new();
    let (mut non_object, mut missing, mut dangling, mut loops, mut valid) = (0, 0, 0, 0, 0);
    for raw in &edges {
        *exact.entry(signature(raw)).or_default() += 1;
        let Some(edge) = Edge::new(raw) else {
            non_object += 1;
            continue;
        };
        if edge.source.is_empty() || edge.target.is_empty() {
            missing += 1;
            continue;
        }
        if !node_ids.contains(&edge.source) || !node_ids.contains(&edge.target) {
            dangling += 1;
            continue;
        }
        if edge.source == edge.target {
            loops += 1;
        }
        valid += 1;
        let pair = (edge.source.clone(), edge.target.clone());
        if !directed_pairs.contains_key(&pair) {
            order.push(pair.clone());
        }
        *directed_pairs.entry(pair.clone()).or_default() += 1;
        let undirected = if edge.source <= edge.target {
            pair.clone()
        } else {
            (edge.target.clone(), edge.source.clone())
        };
        *undirected_pairs.entry(undirected).or_default() += 1;
        groups.entry(pair).or_default().push(edge);
    }
    let examples = order
        .iter()
        .filter(|pair| directed_pairs[*pair] > 1)
        .take(max_examples)
        .map(|pair| {
            let edges = &groups[pair];
            json!({"source":pair.0,"target":pair.1,"edge_count":directed_pairs[pair],
            "relations":set(edges,|e|&e.relation),"source_files":set(edges,|e|&e.source_file),
            "source_locations":set(edges,|e|&e.location),"contexts":set(edges,|e|&e.context)})
        })
        .collect::<Vec<_>>();
    let producer_suppression = extract_path.map_or_else(
        default_producer_suppression,
        scan_producer_suppression_sites,
    );
    Ok(json!({
        "node_count":node_ids.len(),"unverified_node_count":unverified,"raw_edge_count":edges.len(),
        "non_object_edges":non_object,"missing_endpoint_edges":missing,"dangling_endpoint_edges":dangling,
        "self_loop_edges":loops,"valid_candidate_edges":valid,"exact_duplicate_edges":extra(&exact),
        "directed_unique_endpoint_pairs":directed_pairs.len(),"directed_same_endpoint_collapsed_edges":extra(&directed_pairs),
        "undirected_unique_endpoint_pairs":undirected_pairs.len(),"undirected_same_endpoint_collapsed_edges":extra(&undirected_pairs),
        "same_endpoint_group_count":directed_pairs.values().filter(|count|**count>1).count(),
        "relation_variant_groups":variants(&groups,|e|&e.relation,false),
        "source_file_variant_groups":variants(&groups,|e|&e.source_file,true),
        "source_location_variant_groups":variants(&groups,|e|&e.location,true),
        "context_variant_groups":variants(&groups,|e|&e.context,true),
        "post_build_graph_type":if effective_directed{"DiGraph"}else{"Graph"},
        "post_build_node_count":node_ids.len(),"post_build_edge_count":if effective_directed{directed_pairs.len()}else{undirected_pairs.len()},
        "post_build_error":"","producer_suppression":producer_suppression,
        "examples":examples,"input_path":path.to_string_lossy(),"effective_directed":effective_directed
    }))
}

/// Inspect the published typed graph and report the evidence and artifact
/// qualities that matter to agents and CI. This intentionally reads the
/// canonical `compass.graph/1` document rather than a legacy projection so
/// every count is tied to the artifact other commands consume.
pub fn diagnose_graph_quality(path: &Path) -> Result<Value, CoreError> {
    if let Some((size, cap)) = GraphDocument::size_cap_exceeded(path) {
        return diagnose_oversized_graph(path, size, cap);
    }
    let document = GraphDocument::load(path).map_err(|error| {
        CoreError::DiagnosticFile(format!(
            "Cannot load typed graph {}: {error}",
            path.display()
        ))
    })?;
    let file_size = fs::metadata(path).map_or(0, |metadata| metadata.len());
    let mut node_confidence = BTreeMap::<&str, usize>::new();
    let mut edge_confidence = BTreeMap::<&str, usize>::new();
    let mut node_kinds = BTreeMap::<&str, usize>::new();
    let mut edge_kinds = BTreeMap::<&str, usize>::new();
    let mut diagnostic_codes = BTreeMap::<String, usize>::new();
    let mut diagnostic_severity = BTreeMap::<&str, usize>::new();
    let mut ids = BTreeSet::new();
    let mut duplicate_node_ids = 0_usize;
    let mut source_backed_nodes = 0_usize;
    let mut valid_node_anchors = 0_usize;
    let mut external_placeholders = 0_usize;
    let mut heuristic_nodes = 0_usize;
    for node in &document.nodes {
        if !ids.insert(node.id.as_str()) {
            duplicate_node_ids = duplicate_node_ids.saturating_add(1);
        }
        *node_kinds.entry(node.kind.as_str()).or_default() += 1;
        let confidence = confidence_name(&node.evidence);
        *node_confidence.entry(confidence).or_default() += 1;
        if node.source.is_some() {
            source_backed_nodes = source_backed_nodes.saturating_add(1);
        }
        if node.source.as_ref().is_some_and(|anchor| anchor.is_valid()) {
            valid_node_anchors = valid_node_anchors.saturating_add(1);
        }
        if node
            .evidence
            .iter()
            .any(|evidence| evidence.rule.as_deref() == Some("external-symbol-placeholder"))
        {
            external_placeholders = external_placeholders.saturating_add(1);
        }
        if node
            .evidence
            .iter()
            .any(|evidence| evidence.origin == EvidenceOrigin::Heuristic)
        {
            heuristic_nodes = heuristic_nodes.saturating_add(1);
        }
        collect_diagnostic_counts(
            &node.diagnostics,
            &mut diagnostic_codes,
            &mut diagnostic_severity,
        );
    }

    let node_ids = document
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut source_backed_edges = 0_usize;
    let mut valid_edge_anchors = 0_usize;
    let mut heuristic_edges = 0_usize;
    let mut dangling_edges = 0_usize;
    for edge in &document.links {
        *edge_kinds.entry(edge.kind.as_str()).or_default() += 1;
        let confidence = confidence_name(&edge.evidence);
        *edge_confidence.entry(confidence).or_default() += 1;
        if edge.relationship_site.is_some() {
            source_backed_edges = source_backed_edges.saturating_add(1);
        }
        if edge
            .relationship_site
            .as_ref()
            .is_some_and(|anchor| anchor.is_valid())
        {
            valid_edge_anchors = valid_edge_anchors.saturating_add(1);
        }
        if edge
            .evidence
            .iter()
            .any(|evidence| evidence.origin == EvidenceOrigin::Heuristic)
        {
            heuristic_edges = heuristic_edges.saturating_add(1);
        }
        if !node_ids.contains(edge.source.as_str()) || !node_ids.contains(edge.target.as_str()) {
            dangling_edges = dangling_edges.saturating_add(1);
        }
        collect_diagnostic_counts(
            &edge.diagnostics,
            &mut diagnostic_codes,
            &mut diagnostic_severity,
        );
    }
    collect_diagnostic_counts(
        &document.graph.diagnostics,
        &mut diagnostic_codes,
        &mut diagnostic_severity,
    );

    let output_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let output_stats = read_json_object(&output_dir.join(".compass_output_stats.json"));
    let overview = read_json_object(&output_dir.join("graph-overview.json"));
    let stats_nodes = output_stats
        .as_ref()
        .and_then(|stats| stats.get("nodes"))
        .and_then(Value::as_u64);
    let stats_edges = output_stats
        .as_ref()
        .and_then(|stats| stats.get("edges"))
        .and_then(Value::as_u64);
    let stats_match = output_stats.as_ref().map(|_| {
        stats_nodes == Some(document.nodes.len() as u64)
            && stats_edges == Some(document.links.len() as u64)
    });
    let overview_nodes = overview_node_count(overview.as_ref()).unwrap_or_default();
    let publication_omitted_nodes = output_stats
        .as_ref()
        .and_then(|stats| stats.get("omitted_nodes"))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_else(|| {
            diagnostic_codes
                .get("publication_omitted_node")
                .copied()
                .unwrap_or_default()
        });
    let publication_omitted_edges = output_stats
        .as_ref()
        .and_then(|stats| stats.get("omitted_edges"))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_else(|| {
            diagnostic_codes
                .get("publication_omitted_edge")
                .copied()
                .unwrap_or_default()
        });
    let identity_collisions = output_stats
        .as_ref()
        .and_then(|stats| stats.get("identity_collisions"))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_else(|| {
            diagnostic_codes
                .get("publication_identity_collision")
                .copied()
                .unwrap_or_default()
        });
    let mut recommendations = Vec::new();
    if identity_collisions > 0 {
        recommendations.push(
            "Resolve publication_identity_collision diagnostics before using the graph for automation."
                .to_owned(),
        );
    }
    if publication_omitted_nodes > 0 || publication_omitted_edges > 0 {
        recommendations.push(
            "Re-run extraction with the reported source files and inspect publication omissions; this graph is partial."
                .to_owned(),
        );
    }
    if !document.links.is_empty()
        && edge_confidence.get("inferred").copied().unwrap_or_default() * 2 > document.links.len()
    {
        recommendations.push(
            "Most relationships are inferred; use exact-first query mode for agent decisions and inspect anchors before editing."
                .to_owned(),
        );
    }
    if external_placeholders > document.nodes.len() / 5 {
        recommendations.push(
            "External placeholders are a large share of nodes; enable Cargo/package metadata or constrain external resolution."
                .to_owned(),
        );
    }
    if !document.links.is_empty() && source_backed_edges * 2 < document.links.len() {
        recommendations.push(
            "Many relationships lack relationship-site anchors; improve extractor anchors before relying on line-level navigation."
                .to_owned(),
        );
    }
    if recommendations.is_empty() {
        recommendations.push("No blocking graph-quality findings detected.".to_owned());
    }

    Ok(json!({
        "schema": "compass.graph-quality/1",
        "quality_scope": "full",
        "input_path": path.to_string_lossy(),
        "graph_schema": document.graph.schema,
        "file_size_bytes": file_size,
        "node_count": document.nodes.len(),
        "edge_count": document.links.len(),
        "node_confidence": node_confidence,
        "edge_confidence": edge_confidence,
        "node_kinds": node_kinds,
        "edge_kinds": edge_kinds,
        "source_backed_nodes": source_backed_nodes,
        "source_backed_edges": source_backed_edges,
        "valid_node_anchors": valid_node_anchors,
        "valid_edge_anchors": valid_edge_anchors,
        "external_placeholder_nodes": external_placeholders,
        "heuristic_nodes": heuristic_nodes,
        "heuristic_edges": heuristic_edges,
        "dangling_edges": dangling_edges,
        "duplicate_node_ids": duplicate_node_ids,
        "graph_diagnostics": {
            "total": diagnostic_codes.values().sum::<usize>(),
            "by_code": diagnostic_codes,
            "by_severity": diagnostic_severity,
            "publication_omitted_nodes": publication_omitted_nodes,
            "publication_omitted_edges": publication_omitted_edges,
            "identity_collisions": identity_collisions,
        },
        "output_consistency": {
            "stats_file_present": output_stats.is_some(),
            "stats_match_graph": stats_match,
            "stats_bytes_match_graph": output_stats.as_ref().map(|stats| {
                stats
                    .get("graph_bytes")
                    .and_then(Value::as_u64)
                    .is_some_and(|recorded| recorded == file_size)
            }),
            "stats_nodes": stats_nodes,
            "stats_edges": stats_edges,
            "overview_file_present": overview.is_some(),
            "overview_node_count": overview_nodes,
        },
        "ratios": {
            "exact_edge_ratio": ratio(edge_confidence.get("exact").copied().unwrap_or_default(), document.links.len()),
            "inferred_edge_ratio": ratio(edge_confidence.get("inferred").copied().unwrap_or_default(), document.links.len()),
            "anchored_edge_ratio": ratio(source_backed_edges, document.links.len()),
            "anchored_node_ratio": ratio(source_backed_nodes, document.nodes.len()),
            "external_placeholder_ratio": ratio(external_placeholders, document.nodes.len()),
        },
        "recommendations": recommendations,
    }))
}

/// Produce a bounded quality report for a graph that is intentionally larger
/// than the normal in-memory query cap. The publisher stats are authoritative
/// for counts and omissions; the header scan validates the schema and collects
/// durable graph diagnostics without materializing hundreds of thousands of
/// records just to explain why the full graph was not opened.
fn diagnose_oversized_graph(path: &Path, size: u64, cap: u64) -> Result<Value, CoreError> {
    const MAX_STREAM_BYTES: u64 = 8 * 1024 * 1024 * 1024;
    if size > MAX_STREAM_BYTES {
        return Err(CoreError::DiagnosticFile(format!(
            "graph file {} is {} bytes, exceeds the {}-byte diagnostic stream cap",
            path.display(),
            grouped(u128::from(size)),
            grouped(u128::from(MAX_STREAM_BYTES)),
        )));
    }
    let header = read_graph_header(path).map_err(|error| {
        CoreError::DiagnosticFile(format!(
            "Cannot inspect oversized graph {}: {error}",
            path.display()
        ))
    })?;
    let output_stats = read_json_object(
        &path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".compass_output_stats.json"),
    )
    .ok_or_else(|| {
        CoreError::DiagnosticFile(format!(
            "graph file {} exceeds the {}-byte cap and has no .compass_output_stats.json; set COMPASS_MAX_GRAPH_BYTES to inspect it",
            path.display(),
            grouped(u128::from(cap)),
        ))
    })?;
    let overview = read_json_object(
        &path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("graph-overview.json"),
    );
    let node_count = output_stats
        .get("nodes")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let edge_count = output_stats
        .get("edges")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let omitted_nodes = output_stats
        .get("omitted_nodes")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let omitted_edges = output_stats
        .get("omitted_edges")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let identity_collisions = output_stats
        .get("identity_collisions")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let stats_bytes_match_graph = output_stats
        .get("graph_bytes")
        .and_then(Value::as_u64)
        .is_some_and(|recorded| recorded == size);
    let mut codes = BTreeMap::<String, usize>::new();
    let mut severities = BTreeMap::<&str, usize>::new();
    for diagnostic in &header.diagnostics {
        *codes.entry(diagnostic.code.clone()).or_default() += 1;
        let severity = match diagnostic.severity {
            DiagnosticSeverity::Info => "info",
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Error => "error",
        };
        *severities.entry(severity).or_default() += 1;
    }
    let mut recommendations = vec![format!(
        "Graph exceeds the default {}-byte in-memory cap; use the prepared store/overview or set COMPASS_MAX_GRAPH_BYTES only for bounded investigations.",
        grouped(u128::from(cap))
    )];
    if omitted_nodes > 0 || omitted_edges > 0 || identity_collisions > 0 {
        recommendations.push(
            "This publication is partial; fix omissions and identity collisions before automation."
                .to_owned(),
        );
    }
    Ok(json!({
        "schema": "compass.graph-quality/1",
        "quality_scope": "publisher-stats-only",
        "input_path": path.to_string_lossy(),
        "graph_schema": header.schema,
        "file_size_bytes": size,
        "node_count": node_count,
        "edge_count": edge_count,
        "node_confidence": {"unavailable": "graph exceeds the in-memory quality reader cap"},
        "edge_confidence": {"unavailable": "graph exceeds the in-memory quality reader cap"},
        "node_kinds": {},
        "edge_kinds": {},
        "source_backed_nodes": Value::Null,
        "source_backed_edges": Value::Null,
        "valid_node_anchors": Value::Null,
        "valid_edge_anchors": Value::Null,
        "external_placeholder_nodes": Value::Null,
        "heuristic_nodes": Value::Null,
        "heuristic_edges": Value::Null,
        "dangling_edges": Value::Null,
        "duplicate_node_ids": Value::Null,
        "graph_diagnostics": {
            "total": header.diagnostics.len(),
            "by_code": codes,
            "by_severity": severities,
            "publication_omitted_nodes": omitted_nodes,
            "publication_omitted_edges": omitted_edges,
            "identity_collisions": identity_collisions,
        },
        "output_consistency": {
            "stats_file_present": true,
            "stats_match_graph": Value::Null,
            "stats_bytes_match_graph": stats_bytes_match_graph,
            "stats_nodes": node_count,
            "stats_edges": edge_count,
            "overview_file_present": overview.is_some(),
            "overview_node_count": overview_node_count(overview.as_ref()),
        },
        "ratios": {
            "exact_edge_ratio": Value::Null,
            "inferred_edge_ratio": Value::Null,
            "anchored_edge_ratio": Value::Null,
            "anchored_node_ratio": Value::Null,
            "external_placeholder_ratio": Value::Null,
        },
        "recommendations": recommendations,
    }))
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GraphHeaderEnvelope {
    #[serde(default)]
    graph: Option<GraphHeader>,
    #[serde(default)]
    nodes: Option<IgnoredAny>,
    #[serde(default)]
    links: Option<IgnoredAny>,
    #[serde(default)]
    edges: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct GraphHeader {
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    diagnostics: Vec<compass_model::code_graph::GraphDiagnostic>,
}

struct ReadGraphHeaderError(String);

impl std::fmt::Display for ReadGraphHeaderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn read_graph_header(path: &Path) -> Result<GraphHeader, ReadGraphHeaderError> {
    let file = fs::File::open(path).map_err(|error| ReadGraphHeaderError(error.to_string()))?;
    let envelope = serde_json::from_reader::<_, GraphHeaderEnvelope>(std::io::BufReader::new(file))
        .map_err(|error| ReadGraphHeaderError(error.to_string()))?;
    envelope
        .graph
        .ok_or_else(|| ReadGraphHeaderError("graph metadata is missing".to_owned()))
}

fn confidence_name(evidence: &[compass_model::provenance::Provenance]) -> &'static str {
    match effective_confidence(evidence) {
        Some(EvidenceConfidence::Exact) => "exact",
        Some(EvidenceConfidence::Inferred) => "inferred",
        Some(EvidenceConfidence::Ambiguous) => "ambiguous",
        None => "none",
    }
}

fn collect_diagnostic_counts(
    diagnostics: &[compass_model::code_graph::GraphDiagnostic],
    codes: &mut BTreeMap<String, usize>,
    severities: &mut BTreeMap<&'static str, usize>,
) {
    for diagnostic in diagnostics {
        *codes.entry(diagnostic.code.clone()).or_default() += 1;
        let severity = match diagnostic.severity {
            DiagnosticSeverity::Info => "info",
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Error => "error",
        };
        *severities.entry(severity).or_default() += 1;
    }
}

fn read_json_object(path: &Path) -> Option<serde_json::Map<String, Value>> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.as_object().cloned())
}

fn overview_node_count(value: Option<&serde_json::Map<String, Value>>) -> Option<usize> {
    value
        .and_then(|value| value.get("nodes"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .or_else(|| {
            value
                .and_then(|value| value.get("model"))
                .and_then(|model| model.get("stats"))
                .and_then(|stats| stats.get("nodes"))
                .and_then(Value::as_u64)
                .and_then(|nodes| usize::try_from(nodes).ok())
        })
}

fn ratio(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        count as f64 / total as f64
    }
}

#[must_use]
pub fn format_quality_json(summary: &Value) -> Value {
    summary.clone()
}

#[must_use]
pub fn format_quality_report(summary: &Value) -> String {
    let get_u64 = |key: &str| summary.get(key).and_then(Value::as_u64).unwrap_or_default();
    let diagnostics = summary.get("graph_diagnostics").unwrap_or(&Value::Null);
    let consistency = summary.get("output_consistency").unwrap_or(&Value::Null);
    let ratios = summary.get("ratios").unwrap_or(&Value::Null);
    let mut lines = vec![
        "[compass] Typed graph quality diagnostic".to_owned(),
        format!(
            "input: {}",
            text(summary.get("input_path").unwrap_or(&Value::Null))
        ),
        format!(
            "scope: {}",
            text(
                summary
                    .get("quality_scope")
                    .unwrap_or(&Value::String("full".to_owned()))
            )
        ),
        format!(
            "schema: {}",
            text(summary.get("graph_schema").unwrap_or(&Value::Null))
        ),
        format!("nodes: {}", get_u64("node_count")),
        format!("edges: {}", get_u64("edge_count")),
        format!(
            "exact_edge_ratio: {}",
            ratio_text(ratios, "exact_edge_ratio")
        ),
        format!(
            "inferred_edge_ratio: {}",
            ratio_text(ratios, "inferred_edge_ratio")
        ),
        format!(
            "anchored_edge_ratio: {}",
            ratio_text(ratios, "anchored_edge_ratio")
        ),
        format!(
            "external_placeholder_nodes: {}",
            get_u64("external_placeholder_nodes")
        ),
        format!(
            "publication_omitted_nodes: {}",
            value_u64(diagnostics, "publication_omitted_nodes")
        ),
        format!(
            "publication_omitted_edges: {}",
            value_u64(diagnostics, "publication_omitted_edges")
        ),
        format!(
            "identity_collisions: {}",
            value_u64(diagnostics, "identity_collisions")
        ),
        format!(
            "output_stats_match_graph: {}",
            consistency
                .get("stats_match_graph")
                .filter(|value| !value.is_null())
                .map(text)
                .unwrap_or_else(|| "publisher-recorded".to_owned())
        ),
    ];
    if let Some(recommendations) = summary.get("recommendations").and_then(Value::as_array) {
        lines.push("recommendations:".to_owned());
        lines.extend(
            recommendations
                .iter()
                .filter_map(Value::as_str)
                .map(|recommendation| format!("  - {recommendation}")),
        );
    }
    lines.join("\n")
}

fn value_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn ratio_text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_f64).map_or_else(
        || "unavailable".to_owned(),
        |ratio| format!("{:.1}%", ratio * 100.0),
    )
}

fn enforce_graph_size_cap(path: &Path) -> Result<(), CoreError> {
    let Ok(size) = path.metadata().map(|metadata| metadata.len()) else {
        return Ok(());
    };
    let cap = graph_size_cap();
    if u128::from(size) <= cap {
        return Ok(());
    }
    Err(CoreError::DiagnosticFile(format!(
        "graph file {} is {} bytes, exceeds {}-byte cap\n(set COMPASS_MAX_GRAPH_BYTES=<bytes> or COMPASS_MAX_GRAPH_BYTES=<N>GB to raise the limit)",
        path.display(),
        grouped(u128::from(size)),
        grouped(cap)
    )))
}

fn graph_size_cap() -> u128 {
    const DEFAULT: u128 = compass_model::DEFAULT_GRAPH_SIZE_CAP_BYTES as u128;
    let Ok(raw) = std::env::var("COMPASS_MAX_GRAPH_BYTES") else {
        return DEFAULT;
    };
    let upper = raw.trim().to_uppercase();
    let (number, multiplier) = if let Some(number) = upper.strip_suffix("GB") {
        (number.trim(), 1024_u128 * 1024 * 1024)
    } else if let Some(number) = upper.strip_suffix("MB") {
        (number.trim(), 1024_u128 * 1024)
    } else {
        (upper.as_str(), 1)
    };
    number
        .replace('_', "")
        .parse::<u128>()
        .ok()
        .filter(|value| *value > 0)
        .and_then(|value| value.checked_mul(multiplier))
        .unwrap_or(DEFAULT)
}

fn grouped(value: u128) -> String {
    let digits = value.to_string();
    digits
        .chars()
        .enumerate()
        .flat_map(|(index, character)| {
            let separator = (index > 0 && (digits.len() - index).is_multiple_of(3)).then_some('_');
            separator.into_iter().chain(std::iter::once(character))
        })
        .collect()
}

fn python_io_error(path: &Path, source: &std::io::Error) -> String {
    let Some(errno) = source.raw_os_error() else {
        return source.to_string();
    };
    let suffix = format!(" (os error {errno})");
    let reason = source
        .to_string()
        .strip_suffix(&suffix)
        .map_or_else(|| source.to_string(), str::to_owned);
    format!("[Errno {errno}] {reason}: '{}'", path.display())
}

fn python_json_error(bytes: &[u8], source: &serde_json::Error) -> String {
    let raw = source.to_string();
    let description = raw
        .split_once(" at line ")
        .map_or(raw.as_str(), |(description, _)| description);
    let text = String::from_utf8_lossy(bytes);
    let (message, line, column) = if matches!(description, "expected ident" | "expected value") {
        let offset = text
            .char_indices()
            .find(|(_, character)| !character.is_whitespace())
            .map_or(text.len(), |(offset, _)| offset);
        let prefix = &text[..offset];
        (
            "Expecting value".to_owned(),
            prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
            prefix
                .rsplit('\n')
                .next()
                .map_or(1, |line| line.chars().count() + 1),
        )
    } else {
        let message = match description {
            "key must be a string" => "Expecting property name enclosed in double quotes",
            "trailing characters" => "Extra data",
            value if value.starts_with("expected `,` or") => "Expecting ',' delimiter",
            value => value,
        };
        (message.to_owned(), source.line(), source.column())
    };
    let character = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::chars)
        .map(Iterator::count)
        .sum::<usize>()
        + column.saturating_sub(1);
    format!("{message}: line {line} column {column} (char {character})")
}

#[must_use]
pub fn format_diagnostic_json(summary: &Value) -> Value {
    let mut body = summary.as_object().cloned().unwrap_or_default();
    let examples = body.remove("examples").unwrap_or_else(|| json!([]));
    let producer = body
        .remove("producer_suppression")
        .unwrap_or_else(|| json!({}));
    json!({"schema_version":1,"summary":body,"examples":examples,"producer_suppression":producer,"notes":["Diagnostics are read-only.","A normal graph.json is already post-build and cannot recover raw producer edges.","Producer suppression sites are heuristic source-code evidence."]})
}

#[must_use]
pub fn format_diagnostic_report(s: &Value) -> String {
    let get = |k: &str| s.get(k).map(text).unwrap_or_default();
    let mut lines = vec![
        "[compass] MultiDiGraph edge-collapse diagnostic".to_owned(),
        format!("input: {}", get("input_path")),
        "input_stage: provided JSON (normal graph.json is post-build)".to_owned(),
        format!("effective_directed: {}", get("effective_directed")),
    ];
    for (label, key) in [
        ("nodes", "node_count"),
        ("unverified_code_nodes", "unverified_node_count"),
        ("raw_edges", "raw_edge_count"),
        ("valid_candidate_edges", "valid_candidate_edges"),
        ("missing_endpoint_edges", "missing_endpoint_edges"),
        ("dangling_endpoint_edges", "dangling_endpoint_edges"),
        ("self_loop_edges", "self_loop_edges"),
        ("exact_duplicate_edges", "exact_duplicate_edges"),
        (
            "directed_unique_endpoint_pairs",
            "directed_unique_endpoint_pairs",
        ),
        (
            "directed_same_endpoint_collapsed_edges",
            "directed_same_endpoint_collapsed_edges",
        ),
        (
            "undirected_unique_endpoint_pairs",
            "undirected_unique_endpoint_pairs",
        ),
        (
            "undirected_same_endpoint_collapsed_edges",
            "undirected_same_endpoint_collapsed_edges",
        ),
        ("same_endpoint_group_count", "same_endpoint_group_count"),
        ("relation_variant_groups", "relation_variant_groups"),
        ("source_file_variant_groups", "source_file_variant_groups"),
        (
            "source_location_variant_groups",
            "source_location_variant_groups",
        ),
        ("context_variant_groups", "context_variant_groups"),
        ("post_build_graph_type", "post_build_graph_type"),
        ("post_build_edges", "post_build_edge_count"),
    ] {
        lines.push(format!("{label}: {}", get(key)));
    }
    let suppression = s.get("producer_suppression").unwrap_or(&Value::Null);
    lines.push(format!(
        "producer_suppression_sites: {}",
        suppression
            .get("total_sites")
            .map(text)
            .unwrap_or_else(|| "0".to_owned())
    ));
    if let Some(error) = suppression
        .get("error")
        .and_then(Value::as_str)
        .filter(|error| !error.is_empty())
    {
        lines.push(format!("producer_suppression_error: {error}"));
    }
    if let Some(sites) = suppression
        .get("sites")
        .and_then(Value::as_array)
        .filter(|sites| !sites.is_empty())
    {
        lines.push("producer_suppression_examples:".to_owned());
        for site in sites.iter().take(8) {
            let arity = site
                .get("tuple_arity")
                .and_then(Value::as_u64)
                .filter(|arity| *arity > 0)
                .map_or_else(|| "unknown".to_owned(), |arity| arity.to_string());
            lines.push(format!(
                "  - L{} {} arity={arity}",
                site.get("line").map(text).unwrap_or_default(),
                site.get("name").map(text).unwrap_or_default(),
            ));
        }
    }
    if let Some(examples) = s
        .get("examples")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    {
        lines.push("examples:".to_owned());
        for e in examples {
            lines.push(format!(
                "  - {} -> {} edges={} relations={} locations={} contexts={}",
                text(&e["source"]),
                text(&e["target"]),
                text(&e["edge_count"]),
                list(&e["relations"]),
                list(&e["source_locations"]),
                list(&e["contexts"])
            ));
        }
    }
    lines.push(
        "note: normal graph.json is post-build; raw producer loss must be measured earlier."
            .to_owned(),
    );
    lines.join("\n")
}

fn default_producer_suppression() -> Value {
    let source = std::env::current_dir()
        .ok()
        .map(|directory| directory.join("compass").join("extract.py"));
    if let Some(path) = source.filter(|path| path.is_file()) {
        return scan_producer_suppression_sites(&path);
    }

    // Binary releases contain no Python runtime or source tree. This versioned snapshot keeps
    // the producer-risk diagnostic useful while explicit --extract-path scans remain live.
    json!({
        "path": "compass/extract.py",
        "total_sites": 10,
        "sites": [
            {"line":967,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids = {n[\"id\"] for n in nodes}"},
            {"line":1117,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids = {n[\"id\"] for n in nodes}"},
            {"line":1119,"name":"seen_doc_refs","tuple_arity":0,"sample":"seen_doc_refs: set[str] = set()"},
            {"line":1542,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = {n[\"id\"] for n in nodes}"},
            {"line":2011,"name":"seen_keys","tuple_arity":0,"sample":"seen_keys: set[tuple] = set()"},
            {"line":2934,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = set()"},
            {"line":3042,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = set()"},
            {"line":3121,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = set()"},
            {"line":3616,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = set()"},
            {"line":3617,"name":"seen_edges","tuple_arity":4,"sample":"seen_edges: set[tuple[str, str, str, str | None]] = set()"}
        ],
        "error": ""
    })
}

fn scan_producer_suppression_sites(path: &Path) -> Value {
    let path_text = path.to_string_lossy();
    let Ok(source) = fs::read_to_string(path) else {
        return json!({"path":path_text,"total_sites":0,"sites":[],"error":"file not found"});
    };
    let sites = source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();
            let name_len = trimmed
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .map(char::len_utf8)
                .sum::<usize>();
            let name = &trimmed[..name_len];
            let declaration = trimmed[name_len..].trim_start();
            if !name.starts_with("seen_")
                || name.len() == "seen_".len()
                || !matches!(declaration.chars().next(), Some(':' | '='))
            {
                return None;
            }
            let tuple_arity = tuple_arity_from_annotation(line);
            Some(json!({
                "line":index + 1,
                "name":name,
                "tuple_arity":tuple_arity,
                "sample":line.trim().chars().take(120).collect::<String>()
            }))
        })
        .collect::<Vec<_>>();
    json!({"path":path_text,"total_sites":sites.len(),"sites":sites,"error":""})
}

fn tuple_arity_from_annotation(line: &str) -> usize {
    let Some(after) = line.split_once("set[tuple[").map(|(_, after)| after) else {
        return 0;
    };
    let Some(inside) = after.split_once("]]").map(|(inside, _)| inside.trim()) else {
        return 0;
    };
    if inside.is_empty() {
        0
    } else {
        inside.matches(',').count() + 1
    }
}

#[derive(Clone)]
struct Edge {
    source: String,
    target: String,
    relation: String,
    source_file: String,
    location: String,
    context: String,
}
impl Edge {
    fn new(v: &Value) -> Option<Self> {
        let o = v.as_object()?;
        Some(Self {
            source: text(
                o.get("source")
                    .or_else(|| o.get("from"))
                    .unwrap_or(&Value::Null),
            ),
            target: text(
                o.get("target")
                    .or_else(|| o.get("to"))
                    .unwrap_or(&Value::Null),
            ),
            relation: text(o.get("relation").unwrap_or(&Value::Null)),
            source_file: text(o.get("source_file").unwrap_or(&Value::Null)),
            location: text(o.get("source_location").unwrap_or(&Value::Null)),
            context: text(o.get("context").unwrap_or(&Value::Null)),
        })
    }
}
fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(v) => {
            if *v {
                "True".into()
            } else {
                "False".into()
            }
        }
        Value::String(v) => v.clone(),
        Value::Number(v) => v.to_string(),
        v => serde_json::to_string(v).unwrap_or_default(),
    }
}
fn signature(v: &Value) -> String {
    let Some(o) = v.as_object() else {
        return "<non-object>".into();
    };
    let mut b = BTreeMap::new();
    for (k, v) in o {
        if k != "from" && k != "to" {
            b.insert(k.clone(), v.clone());
        }
    }
    if !b.contains_key("source")
        && let Some(v) = o.get("from")
    {
        b.insert("source".to_owned(), v.clone());
    }
    if !b.contains_key("target")
        && let Some(v) = o.get("to")
    {
        b.insert("target".to_owned(), v.clone());
    }
    serde_json::to_string(&b).unwrap_or_default()
}
fn extra<K: Eq + std::hash::Hash>(m: &HashMap<K, usize>) -> usize {
    m.values().map(|v| v.saturating_sub(1)).sum()
}
fn set(edges: &[Edge], f: impl Fn(&Edge) -> &String) -> Vec<String> {
    edges
        .iter()
        .map(f)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn variants<'a>(
    groups: &'a HashMap<(String, String), Vec<Edge>>,
    f: impl Fn(&'a Edge) -> &'a String,
    relation_sensitive: bool,
) -> usize {
    groups
        .values()
        .map(|edges| {
            if relation_sensitive {
                let mut r = HashMap::<&str, HashSet<&str>>::new();
                for e in edges {
                    r.entry(&e.relation).or_default().insert(f(e));
                }
                r.values().filter(|v| v.len() > 1).count()
            } else {
                usize::from(edges.iter().map(&f).collect::<HashSet<_>>().len() > 1)
            }
        })
        .sum()
}
fn list(v: &Value) -> String {
    format!(
        "[{}]",
        v.as_array()
            .into_iter()
            .flatten()
            .map(|v| format!("'{}'", text(v)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::{diagnose_graph_quality, format_quality_report};
    use compass_model::code_graph::{
        BuildMetadata, ExtractionStatus, FileRecord, GraphDocument, NodeKind, NodeRecord,
    };
    use compass_model::identity::file_id;
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
    use serde_json::Value;

    #[test]
    fn quality_diagnostic_reads_the_typed_contract() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("graph.json");
        let mut document = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "sha256:schema".to_owned(),
            source_tree_digest: "sha256:tree".to_owned(),
            configuration_digest: "sha256:config".to_owned(),
            generation_id: "sha256:generation".to_owned(),
            source_commit: None,
        });
        let anchor = SourceAnchor {
            file: "src/lib.rs".to_owned(),
            start_byte: 0,
            end_byte: 1,
            start_line: 1,
            start_column: 1,
            end_line: 1,
            end_column: 2,
        };
        document.graph.files.push(FileRecord {
            id: file_id("src/lib.rs"),
            path: "src/lib.rs".to_owned(),
            language: Some("rust".to_owned()),
            content_digest: "sha256:test".to_owned(),
            byte_size: 1,
            generated: false,
            extraction_status: ExtractionStatus::Extracted,
            extractor_versions: vec!["test".to_owned()],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        });
        document.nodes.push(NodeRecord {
            id: "node:test".to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: "test".to_owned(),
            qualified_name: "crate::test".to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            source: Some(anchor.clone()),
            details: None,
            evidence: vec![Provenance {
                origin: EvidenceOrigin::Ast,
                extractor: "test".to_owned(),
                confidence: EvidenceConfidence::Exact,
                rule: None,
                anchors: vec![anchor],
                wiring_site: None,
                score: None,
                candidates: Vec::new(),
            }],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        });
        std::fs::write(&path, serde_json::to_vec(&document)?)?;
        let summary = diagnose_graph_quality(&path)?;
        assert_eq!(summary["schema"], "compass.graph-quality/1");
        assert_eq!(summary["node_count"], 1);
        assert_eq!(summary["node_confidence"]["exact"], 1);
        assert_eq!(summary["output_consistency"]["stats_file_present"], false);
        assert_eq!(
            summary["output_consistency"]["stats_match_graph"],
            Value::Null
        );
        assert!(format_quality_report(&summary).contains("Typed graph quality diagnostic"));
        Ok(())
    }
}
