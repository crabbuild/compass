use ahash::AHashSet as HashSet;
use compass_languages::{Extraction, RawEdgeRecord, RawNodeRecord};
use compass_model::code_graph::{EdgeRecord, GraphDocument, NodeRecord};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, effective_confidence};
use serde_json::{Map, Value};

/// Maximum inference admitted to a published structural graph.
///
/// Levels are nested: every relationship admitted by a lower level is also
/// admitted by each higher level. `Max` preserves the complete publication
/// behavior used before inference controls were introduced.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InferenceLevel {
    /// Publish exact relationships only.
    Low,
    /// Also publish inferred relationships between source-backed nodes.
    Medium,
    /// Also publish explicitly qualified external relationships.
    High,
    /// Also publish deferred-receiver and all other inferred relationships.
    #[default]
    Max,
}

impl InferenceLevel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

/// Records intentionally suppressed by an inference-level publication policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InferenceSelection {
    pub suppressed_nodes: usize,
    pub suppressed_edges: usize,
}

/// Discard relationships that are provably outside an inference level before
/// graph assembly and v1 normalization allocate their materialized records.
///
/// This is deliberately conservative. Unknown or incomplete raw provenance is
/// retained for the validated v1 admission pass, which remains authoritative.
/// Raw nodes are deliberately retained because inventory accounting and v1
/// normalization can consume them even when the final policy removes them.
/// Edges removed here are a strict subset of those removed by
/// [`apply_inference_level`], so moving the cheap decision earlier reduces
/// peak memory without changing the published graph.
pub fn prefilter_extraction_inference(
    extraction: &mut Extraction,
    level: InferenceLevel,
) -> InferenceSelection {
    if level == InferenceLevel::Max {
        return InferenceSelection::default();
    }
    let collision_keys = raw_edge_collision_keys(&extraction.edges);
    prefilter_raw_edges_inference_protected(
        &extraction.nodes,
        &mut extraction.edges,
        level,
        &collision_keys,
    )
}

pub(crate) fn prefilter_raw_edges_inference(
    nodes: &[RawNodeRecord],
    edges: &mut Vec<RawEdgeRecord>,
    level: InferenceLevel,
) -> InferenceSelection {
    prefilter_raw_edges_inference_protected(nodes, edges, level, &HashSet::new())
}

pub(crate) fn prune_orphan_external_placeholders<'a>(
    nodes: &mut Vec<RawNodeRecord>,
    edges: &'a [RawEdgeRecord],
    hyperedges: &'a [Value],
    level: InferenceLevel,
) -> usize {
    if level == InferenceLevel::Max {
        return 0;
    }
    let mut referenced = edges
        .iter()
        .flat_map(|edge| [edge.source.as_str(), edge.target.as_str()])
        .collect::<HashSet<_>>();
    for hyperedge in hyperedges {
        collect_json_strings(hyperedge, &mut referenced);
    }
    let original_nodes = nodes.len();
    nodes.retain(|node| {
        referenced.contains(node.id.as_str()) || !raw_node_is_external_placeholder(node)
    });
    original_nodes.saturating_sub(nodes.len())
}

fn collect_json_strings<'a>(value: &'a Value, output: &mut HashSet<&'a str>) {
    match value {
        Value::String(value) => {
            output.insert(value);
        }
        Value::Array(values) => {
            for value in values {
                collect_json_strings(value, output);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_json_strings(value, output);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn raw_node_is_external_placeholder(node: &RawNodeRecord) -> bool {
    !raw_node_is_source_backed(node)
        && raw_confidence(&node.attributes) == Some(EvidenceConfidence::Inferred)
        && (node.attributes.get("extractor").and_then(Value::as_str)
            == Some("compass.graph.external-placeholder")
            || node
                .attributes
                .get("rule")
                .and_then(Value::as_str)
                .is_some_and(|rule| {
                    matches!(rule, "external-symbol-placeholder" | "deferred-receiver")
                }))
}

fn prefilter_raw_edges_inference_protected(
    nodes: &[RawNodeRecord],
    edges: &mut Vec<RawEdgeRecord>,
    level: InferenceLevel,
    collision_keys: &HashSet<String>,
) -> InferenceSelection {
    if level == InferenceLevel::Max {
        return InferenceSelection::default();
    }

    let source_backed = nodes
        .iter()
        .filter(|node| raw_node_is_source_backed(node))
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let known_nodes = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let constructible_nodes = nodes
        .iter()
        .filter(|node| raw_node_is_constructible(node))
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let original_edges = edges.len();
    edges.retain(|edge| {
        collision_keys.contains(&raw_edge_collision_key(edge))
            || raw_edge_may_be_admitted(
                edge,
                level,
                &source_backed,
                &known_nodes,
                &constructible_nodes,
            )
    });

    InferenceSelection {
        suppressed_nodes: 0,
        suppressed_edges: original_edges.saturating_sub(edges.len()),
    }
}

fn raw_edge_collision_keys(edges: &[RawEdgeRecord]) -> HashSet<String> {
    let mut seen = HashSet::with_capacity(edges.len());
    let mut collisions = HashSet::new();
    for edge in edges {
        let key = raw_edge_collision_key(edge);
        if !seen.insert(key.clone()) {
            collisions.insert(key);
        }
    }
    collisions
}

fn raw_edge_collision_key(edge: &RawEdgeRecord) -> String {
    let attributes = &edge.attributes;
    let values = [
        Value::String(edge.source.clone()),
        Value::String(edge.target.clone()),
        attributes.get("relation").cloned().unwrap_or(Value::Null),
        attributes
            .get("source_anchor")
            .or_else(|| attributes.get("sourceAnchor"))
            .or_else(|| attributes.get("anchor"))
            .cloned()
            .unwrap_or(Value::Null),
        attributes
            .get("source_file")
            .cloned()
            .unwrap_or(Value::Null),
        attributes
            .get("source_location")
            .cloned()
            .unwrap_or(Value::Null),
        attributes
            .get("_occurrence_rule")
            .or_else(|| attributes.get("rule"))
            .cloned()
            .unwrap_or(Value::Null),
    ];
    serde_json::to_string(&values).unwrap_or_default()
}

fn raw_edge_may_be_admitted(
    edge: &RawEdgeRecord,
    level: InferenceLevel,
    source_backed: &HashSet<&str>,
    known_nodes: &HashSet<&str>,
    constructible_nodes: &HashSet<&str>,
) -> bool {
    let admitted = match raw_confidence(&edge.attributes) {
        Some(EvidenceConfidence::Exact) | None => true,
        Some(EvidenceConfidence::Ambiguous) => false,
        Some(EvidenceConfidence::Inferred) => match level {
            InferenceLevel::Low => raw_edge_has_unresolved_endpoint(edge, known_nodes),
            InferenceLevel::Medium => {
                raw_edge_has_unresolved_endpoint(edge, known_nodes)
                    || raw_edge_is_source_backed(edge, source_backed)
            }
            InferenceLevel::High => {
                raw_edge_has_unresolved_endpoint(edge, known_nodes)
                    || raw_edge_is_source_backed(edge, source_backed)
                    || raw_edge_is_qualified_external(edge)
            }
            InferenceLevel::Max => true,
        },
    };
    admitted || !raw_edge_is_safe_for_early_suppression(edge, constructible_nodes)
}

fn raw_edge_is_safe_for_early_suppression(
    edge: &RawEdgeRecord,
    constructible_nodes: &HashSet<&str>,
) -> bool {
    // Some relations are consumed during v1 normalization even when their
    // final edge is policy-suppressed. In particular, `tests` assigns the test
    // role and route relations enrich route metadata. Calls do not mutate node
    // metadata, so they can be removed before materialization.
    edge.attributes.get("relation").and_then(Value::as_str) == Some("calls")
        && !constructible_nodes.contains(edge.target.as_str())
}

fn raw_edge_has_unresolved_endpoint(edge: &RawEdgeRecord, known_nodes: &HashSet<&str>) -> bool {
    !known_nodes.contains(edge.source.as_str()) || !known_nodes.contains(edge.target.as_str())
}

fn raw_edge_is_source_backed(edge: &RawEdgeRecord, source_backed: &HashSet<&str>) -> bool {
    source_backed.contains(edge.source.as_str()) && source_backed.contains(edge.target.as_str())
}

fn raw_edge_is_qualified_external(edge: &RawEdgeRecord) -> bool {
    edge.attributes
        .get("resolution_rule")
        .or_else(|| edge.attributes.get("rule"))
        .and_then(Value::as_str)
        .is_some_and(|rule| rule.ends_with("qualified-external"))
}

fn raw_node_is_source_backed(node: &RawNodeRecord) -> bool {
    nonempty_string(&node.attributes, "source_file")
        || nonempty_string(&node.attributes, "source")
        || ["source_anchor", "sourceAnchor", "anchor"]
            .iter()
            .any(|key| {
                node.attributes
                    .get(*key)
                    .and_then(Value::as_object)
                    .is_some_and(|anchor| nonempty_string(anchor, "file"))
            })
}

fn raw_node_is_constructible(node: &RawNodeRecord) -> bool {
    let kind = node
        .attributes
        .get("symbol_kind")
        .or_else(|| node.attributes.get("type"))
        .and_then(Value::as_str)
        .or_else(|| {
            node.attributes
                .get("_compass_v1_node")
                .and_then(Value::as_object)
                .and_then(|record| record.get("kind"))
                .and_then(Value::as_str)
        });
    matches!(
        kind,
        Some("class" | "struct" | "enum" | "enum_member" | "component" | "database_procedure")
    )
}

fn nonempty_string(attributes: &Map<String, Value>, key: &str) -> bool {
    attributes
        .get(key)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn raw_confidence(attributes: &Map<String, Value>) -> Option<EvidenceConfidence> {
    if attributes
        .get("_origin")
        .or_else(|| attributes.get("origin"))
        .and_then(Value::as_str)
        == Some("heuristic")
    {
        return Some(EvidenceConfidence::Inferred);
    }
    match attributes.get("confidence").and_then(Value::as_str) {
        None | Some("EXTRACTED" | "exact") => Some(EvidenceConfidence::Exact),
        Some("INFERRED" | "inferred") => Some(EvidenceConfidence::Inferred),
        Some("AMBIGUOUS" | "ambiguous") => Some(EvidenceConfidence::Ambiguous),
        Some(_) => None,
    }
}

/// Apply a deterministic inference policy to an already validated graph.
///
/// Filtering occurs after normalization so producer evidence stays complete in
/// extraction caches. Removing edges first and then unreferenced non-exact
/// nodes preserves endpoint coherence without treating policy suppression as a
/// publication omission.
pub fn apply_inference_level(
    document: &mut GraphDocument,
    level: InferenceLevel,
) -> InferenceSelection {
    if level == InferenceLevel::Max {
        return InferenceSelection::default();
    }

    let source_backed = document
        .nodes
        .iter()
        .filter(|node| node.source.is_some())
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let original_edges = document.links.len();
    document
        .links
        .retain(|edge| admits_edge(edge, level, &source_backed));

    let retained_endpoints = document
        .links
        .iter()
        .flat_map(|edge| [edge.source.as_str(), edge.target.as_str()])
        .collect::<HashSet<_>>();
    let original_nodes = document.nodes.len();
    document.nodes.retain(|node| {
        retained_endpoints.contains(node.id.as_str()) || admits_unreferenced_node(node, level)
    });

    InferenceSelection {
        suppressed_nodes: original_nodes.saturating_sub(document.nodes.len()),
        suppressed_edges: original_edges.saturating_sub(document.links.len()),
    }
}

fn admits_edge(edge: &EdgeRecord, level: InferenceLevel, source_backed: &HashSet<&str>) -> bool {
    match effective_confidence(&edge.evidence) {
        Some(EvidenceConfidence::Exact) => true,
        Some(EvidenceConfidence::Inferred) => match level {
            InferenceLevel::Low => false,
            InferenceLevel::Medium => edge_is_source_backed(edge, source_backed),
            InferenceLevel::High => {
                edge_is_source_backed(edge, source_backed) || explicitly_qualified_external(edge)
            }
            InferenceLevel::Max => true,
        },
        Some(EvidenceConfidence::Ambiguous) | None => level == InferenceLevel::Max,
    }
}

fn edge_is_source_backed(edge: &EdgeRecord, source_backed: &HashSet<&str>) -> bool {
    source_backed.contains(edge.source.as_str()) && source_backed.contains(edge.target.as_str())
}

fn explicitly_qualified_external(edge: &EdgeRecord) -> bool {
    edge.evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Ast
            && evidence.confidence == EvidenceConfidence::Inferred
            && evidence
                .rule
                .as_deref()
                .is_some_and(|rule| rule.ends_with("qualified-external"))
    })
}

fn admits_unreferenced_node(node: &NodeRecord, level: InferenceLevel) -> bool {
    match effective_confidence(&node.evidence) {
        Some(EvidenceConfidence::Exact) => true,
        Some(EvidenceConfidence::Inferred) => {
            level >= InferenceLevel::Medium && node.source.is_some()
        }
        Some(EvidenceConfidence::Ambiguous) | None => false,
    }
}

#[cfg(test)]
mod tests {
    use compass_languages::{Extraction, RawEdgeRecord, RawNodeRecord};
    use compass_model::code_graph::{BuildMetadata, EdgeKind, GraphMetadata, NodeKind};
    use compass_model::provenance::{Provenance, SourceAnchor};

    use super::*;

    #[test]
    fn inference_levels_are_nested_and_prune_unreferenced_placeholders() {
        let source = anchor("src/lib.rs", 1);
        let mut document = GraphDocument {
            directed: true,
            multigraph: true,
            graph: GraphMetadata::v1(BuildMetadata {
                builder_version: "test".to_owned(),
                schema_fingerprint: "sha256:test".to_owned(),
                source_tree_digest: "sha256:test".to_owned(),
                configuration_digest: "sha256:test".to_owned(),
                generation_id: "sha256:test".to_owned(),
                source_commit: None,
            }),
            nodes: vec![
                node("source", Some(source.clone()), exact(&source)),
                node("exact", Some(source.clone()), exact(&source)),
                node(
                    "source-inferred",
                    Some(source.clone()),
                    inferred(&source, "unique-stub"),
                ),
                node("qualified-external", None, placeholder(&source)),
                node("deferred-external", None, placeholder(&source)),
            ],
            links: vec![
                edge("exact-edge", "source", "exact", exact(&source)),
                edge(
                    "source-inferred-edge",
                    "source",
                    "source-inferred",
                    inferred(&source, "unique-stub-endpoint-resolution"),
                ),
                edge_with_placeholder(
                    "qualified-edge",
                    "source",
                    "qualified-external",
                    &source,
                    "universal-call-qualified-external",
                ),
                edge_with_placeholder(
                    "deferred-edge",
                    "source",
                    "deferred-external",
                    &source,
                    "universal-call-deferred-receiver",
                ),
            ],
        };

        let expectations = [
            (InferenceLevel::Low, 2, 1),
            (InferenceLevel::Medium, 3, 2),
            (InferenceLevel::High, 4, 3),
            (InferenceLevel::Max, 5, 4),
        ];
        for (level, nodes, edges) in expectations {
            let mut filtered = document.clone();
            let selection = apply_inference_level(&mut filtered, level);
            assert_eq!((filtered.nodes.len(), filtered.links.len()), (nodes, edges));
            assert_eq!(
                selection,
                InferenceSelection {
                    suppressed_nodes: document.nodes.len() - nodes,
                    suppressed_edges: document.links.len() - edges,
                }
            );
            let retained = filtered
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<HashSet<_>>();
            assert!(filtered.links.iter().all(|edge| {
                retained.contains(edge.source.as_str()) && retained.contains(edge.target.as_str())
            }));
        }

        let selection = apply_inference_level(&mut document, InferenceLevel::Max);
        assert_eq!(selection, InferenceSelection::default());
    }

    #[test]
    fn inference_level_names_are_stable() {
        assert_eq!(InferenceLevel::default(), InferenceLevel::Max);
        assert_eq!(InferenceLevel::Low.as_str(), "low");
        assert_eq!(InferenceLevel::Medium.as_str(), "medium");
        assert_eq!(InferenceLevel::High.as_str(), "high");
        assert_eq!(InferenceLevel::Max.as_str(), "max");
    }

    #[test]
    fn raw_prefilter_removes_only_provably_disallowed_low_records() {
        let mut extraction = Extraction {
            nodes: vec![
                raw_node("source", "EXTRACTED", "src/lib.rs"),
                raw_node("exact", "EXTRACTED", "src/lib.rs"),
                raw_node("placeholder", "INFERRED", ""),
                raw_node("unknown", "future-confidence", ""),
            ],
            edges: vec![
                raw_edge("source", "exact", "EXTRACTED", None),
                raw_edge("source", "placeholder", "INFERRED", None),
                raw_edge("source", "unknown", "future-confidence", None),
            ],
            ..Extraction::default()
        };

        let selection = prefilter_extraction_inference(&mut extraction, InferenceLevel::Low);

        assert_eq!(selection.suppressed_edges, 1);
        assert_eq!(selection.suppressed_nodes, 0);
        assert_eq!(
            extraction
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["source", "exact", "placeholder", "unknown"]
        );
        assert_eq!(extraction.edges.len(), 2);
    }

    #[test]
    fn raw_prefilter_levels_are_nested_for_source_and_external_inference() {
        let base = Extraction {
            nodes: vec![
                raw_node("source", "EXTRACTED", "src/lib.rs"),
                raw_node("source-inferred", "INFERRED", "src/other.rs"),
                raw_node("qualified", "INFERRED", ""),
                raw_node("deferred", "INFERRED", ""),
            ],
            edges: vec![
                raw_edge("source", "source-inferred", "INFERRED", None),
                raw_edge(
                    "source",
                    "qualified",
                    "INFERRED",
                    Some("qualified-external"),
                ),
                raw_edge("source", "deferred", "INFERRED", Some("deferred-receiver")),
            ],
            ..Extraction::default()
        };

        for (level, nodes, edges) in [
            (InferenceLevel::Low, 4, 0),
            (InferenceLevel::Medium, 4, 1),
            (InferenceLevel::High, 4, 2),
            (InferenceLevel::Max, 4, 3),
        ] {
            let mut extraction = base.clone();
            prefilter_extraction_inference(&mut extraction, level);
            assert_eq!(
                (extraction.nodes.len(), extraction.edges.len()),
                (nodes, edges)
            );
        }
    }

    #[test]
    fn raw_prefilter_preserves_every_record_in_an_admitted_duplicate_group() {
        let mut inferred_duplicate = raw_node("shared", "INFERRED", "");
        inferred_duplicate
            .attributes
            .insert("roles".to_owned(), serde_json::json!(["test"]));
        let mut extraction = Extraction {
            nodes: vec![
                raw_node("shared", "EXTRACTED", "src/lib.rs"),
                inferred_duplicate,
                raw_node("placeholder", "INFERRED", ""),
            ],
            edges: vec![raw_edge("shared", "placeholder", "INFERRED", None)],
            ..Extraction::default()
        };

        let selection = prefilter_extraction_inference(&mut extraction, InferenceLevel::Low);

        assert_eq!(selection.suppressed_edges, 1);
        assert_eq!(selection.suppressed_nodes, 0);
        assert_eq!(
            extraction
                .nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["shared", "shared", "placeholder"]
        );
    }

    #[test]
    fn raw_prefilter_defers_edges_that_may_coalesce() {
        let mut exact = raw_edge("source", "target", "EXTRACTED", Some("direct-call"));
        exact.attributes.insert(
            "source_anchor".to_owned(),
            serde_json::json!({"file": "src/lib.rs", "startLine": 10}),
        );
        let mut inferred = exact.clone();
        inferred.attributes.insert(
            "confidence".to_owned(),
            Value::String("INFERRED".to_owned()),
        );
        let mut extraction = Extraction {
            nodes: vec![
                raw_node("source", "EXTRACTED", "src/lib.rs"),
                raw_node("target", "EXTRACTED", "src/lib.rs"),
            ],
            edges: vec![exact, inferred],
            ..Extraction::default()
        };

        let selection = prefilter_extraction_inference(&mut extraction, InferenceLevel::Low);

        assert_eq!(selection, InferenceSelection::default());
        assert_eq!(extraction.edges.len(), 2);
    }

    #[test]
    fn raw_prefilter_leaves_unresolved_calls_for_authoritative_validation() {
        let mut extraction = Extraction {
            nodes: vec![raw_node("source", "EXTRACTED", "src/lib.rs")],
            edges: vec![raw_edge("source", "missing", "INFERRED", None)],
            ..Extraction::default()
        };

        let selection = prefilter_extraction_inference(&mut extraction, InferenceLevel::Low);

        assert_eq!(selection, InferenceSelection::default());
        assert_eq!(extraction.edges.len(), 1);
    }

    #[test]
    fn raw_prefilter_retains_constructor_calls_for_normalization_diagnostics() {
        let mut target = raw_node("target", "EXTRACTED", "src/lib.rs");
        target
            .attributes
            .insert("symbol_kind".to_owned(), Value::String("struct".to_owned()));
        let mut extraction = Extraction {
            nodes: vec![raw_node("source", "EXTRACTED", "src/lib.rs"), target],
            edges: vec![raw_edge("source", "target", "INFERRED", None)],
            ..Extraction::default()
        };

        let selection = prefilter_extraction_inference(&mut extraction, InferenceLevel::Low);

        assert_eq!(selection, InferenceSelection::default());
        assert_eq!(extraction.edges.len(), 1);
    }

    #[test]
    fn orphan_external_placeholder_pruning_is_reference_safe() {
        let mut orphan = raw_node("orphan", "INFERRED", "");
        orphan.attributes.insert(
            "extractor".to_owned(),
            Value::String("compass.graph.external-placeholder".to_owned()),
        );
        let mut retained = orphan.clone();
        retained.id = "retained".to_owned();
        let mut nodes = vec![
            raw_node("source", "EXTRACTED", "src/lib.rs"),
            orphan,
            retained,
        ];
        let edges = vec![raw_edge("source", "retained", "EXTRACTED", None)];

        let removed =
            prune_orphan_external_placeholders(&mut nodes, &edges, &[], InferenceLevel::Low);

        assert_eq!(removed, 1);
        assert_eq!(
            nodes
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            ["source", "retained"]
        );
    }

    fn raw_node(id: &str, confidence: &str, source_file: &str) -> RawNodeRecord {
        RawNodeRecord {
            id: id.to_owned(),
            attributes: Map::from_iter([
                (
                    "confidence".to_owned(),
                    Value::String(confidence.to_owned()),
                ),
                (
                    "source_file".to_owned(),
                    Value::String(source_file.to_owned()),
                ),
            ]),
        }
    }

    fn raw_edge(
        source: &str,
        target: &str,
        confidence: &str,
        resolution_rule: Option<&str>,
    ) -> RawEdgeRecord {
        let mut attributes = Map::from_iter([
            (
                "confidence".to_owned(),
                Value::String(confidence.to_owned()),
            ),
            ("relation".to_owned(), Value::String("calls".to_owned())),
        ]);
        if let Some(rule) = resolution_rule {
            attributes.insert("resolution_rule".to_owned(), Value::String(rule.to_owned()));
        }
        RawEdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        }
    }

    fn anchor(file: &str, line: u32) -> SourceAnchor {
        SourceAnchor {
            file: file.to_owned(),
            start_byte: u64::from(line),
            end_byte: u64::from(line) + 1,
            start_line: line,
            start_column: 0,
            end_line: line,
            end_column: 1,
        }
    }

    fn exact(anchor: &SourceAnchor) -> Vec<Provenance> {
        vec![Provenance {
            origin: EvidenceOrigin::Ast,
            extractor: "test".to_owned(),
            confidence: EvidenceConfidence::Exact,
            rule: None,
            anchors: vec![anchor.clone()],
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        }]
    }

    fn inferred(anchor: &SourceAnchor, rule: &str) -> Vec<Provenance> {
        vec![Provenance {
            origin: EvidenceOrigin::Ast,
            extractor: "test".to_owned(),
            confidence: EvidenceConfidence::Inferred,
            rule: Some(rule.to_owned()),
            anchors: vec![anchor.clone()],
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        }]
    }

    fn placeholder(anchor: &SourceAnchor) -> Vec<Provenance> {
        vec![Provenance {
            origin: EvidenceOrigin::Heuristic,
            extractor: "test".to_owned(),
            confidence: EvidenceConfidence::Inferred,
            rule: Some("external-symbol-placeholder".to_owned()),
            anchors: Vec::new(),
            wiring_site: Some(anchor.clone()),
            score: None,
            candidates: Vec::new(),
        }]
    }

    fn node(id: &str, source: Option<SourceAnchor>, evidence: Vec<Provenance>) -> NodeRecord {
        NodeRecord {
            id: id.to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: id.to_owned(),
            qualified_name: id.to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            source,
            details: None,
            evidence,
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        }
    }

    fn edge(id: &str, source: &str, target: &str, evidence: Vec<Provenance>) -> EdgeRecord {
        EdgeRecord {
            id: id.to_owned(),
            key: id.to_owned(),
            source: source.to_owned(),
            target: target.to_owned(),
            kind: EdgeKind::Calls,
            occurrence_rule: None,
            relationship_site: evidence
                .iter()
                .flat_map(|item| item.anchors.iter())
                .next()
                .cloned(),
            details: None,
            evidence,
            weight: Some(1.0),
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        }
    }

    fn edge_with_placeholder(
        id: &str,
        source: &str,
        target: &str,
        anchor: &SourceAnchor,
        rule: &str,
    ) -> EdgeRecord {
        let mut evidence = inferred(anchor, rule);
        evidence.extend(placeholder(anchor));
        edge(id, source, target, evidence)
    }
}
