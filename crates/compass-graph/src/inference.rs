use ahash::AHashSet as HashSet;
use compass_model::code_graph::{EdgeRecord, GraphDocument, NodeRecord};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, effective_confidence};

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
