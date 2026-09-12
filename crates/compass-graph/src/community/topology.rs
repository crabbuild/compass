use std::collections::{BTreeMap, BTreeSet};

use compass_model::code_graph::{EdgeKind, GraphDocument};
use compass_model::provenance::{EvidenceConfidence, effective_confidence};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cluster::WeightedGraph;

pub(crate) const MAX_OCCURRENCES_PER_PAIR_KIND: usize = 4;
pub(crate) const MAX_PAIR_WEIGHT: f64 = 64.0;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Strength {
    Weak,
    Medium,
    Strong,
}

impl Strength {
    const fn weight(self) -> f64 {
        match self {
            Self::Weak => 1.0,
            Self::Medium => 2.0,
            Self::Strong => 4.0,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Weak => "weak",
            Self::Medium => "medium",
            Self::Strong => "strong",
        }
    }
}

const fn relationship_strength(kind: EdgeKind) -> Strength {
    match kind {
        EdgeKind::Calls
        | EdgeKind::RoutesTo
        | EdgeKind::Reads
        | EdgeKind::Writes
        | EdgeKind::Handles
        | EdgeKind::Publishes
        | EdgeKind::Subscribes
        | EdgeKind::Produces
        | EdgeKind::Consumes
        | EdgeKind::Schedules
        | EdgeKind::Triggers
        | EdgeKind::Renders => Strength::Strong,
        EdgeKind::Imports
        | EdgeKind::DependsOn
        | EdgeKind::Instantiates
        | EdgeKind::Registers
        | EdgeKind::Extends
        | EdgeKind::Implements
        | EdgeKind::MixesIn
        | EdgeKind::Overrides
        | EdgeKind::Decorates
        | EdgeKind::TypeOf
        | EdgeKind::Returns => Strength::Medium,
        EdgeKind::Contains
        | EdgeKind::Embeds
        | EdgeKind::Exports
        | EdgeKind::References
        | EdgeKind::Aliases
        | EdgeKind::Tests
        | EdgeKind::Documents
        | EdgeKind::MapsTo => Strength::Weak,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopologyEvidence {
    pub input_edge_count: usize,
    pub input_weight_sum: f64,
    pub retained_occurrence_count: usize,
    pub retained_total_weight: f64,
    pub projected_pair_count: usize,
    pub omitted_ambiguous_count: usize,
    pub omitted_capped_count: usize,
    pub omitted_duplicate_occurrence_count: usize,
    pub omitted_pair_weight_cap_count: usize,
    pub forward_occurrence_count: usize,
    pub reverse_occurrence_count: usize,
    pub relationship_counts: BTreeMap<String, usize>,
    pub strength_counts: BTreeMap<String, usize>,
    pub confidence_counts: BTreeMap<String, usize>,
    pub retained_relationship_counts: BTreeMap<String, usize>,
    pub retained_strength_counts: BTreeMap<String, usize>,
    pub retained_confidence_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TopologyLimits {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_projected_pairs: usize,
    pub max_total_weight: f64,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum CommunityTopologyError {
    #[error("typed community topology contains duplicate node id `{node}`")]
    DuplicateNode { node: String },
    #[error("typed community topology edge `{edge}` references missing endpoint `{endpoint}`")]
    DanglingEndpoint { edge: String, endpoint: String },
    #[error("typed community topology edge `{edge}` has invalid weight {weight}")]
    InvalidEdgeWeight { edge: String, weight: f64 },
    #[error(
        "community topology {stage} requires {required} items, exceeds limit {limit} after {processed}"
    )]
    LimitExceeded {
        stage: &'static str,
        required: usize,
        limit: usize,
        processed: usize,
    },
    #[error(
        "community topology total_weight requires {required}, exceeds limit {limit} after {processed} projected pairs"
    )]
    TotalWeightLimitExceeded {
        required: f64,
        limit: f64,
        processed: usize,
    },
}

pub(crate) struct CommunityTopology {
    pub graph: WeightedGraph,
    pub evidence: TopologyEvidence,
    pub pair_evidence: Vec<ProjectedPairEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectedPairEvidence {
    pub left: usize,
    pub right: usize,
    pub edge_ids: Vec<String>,
    pub relationship_counts: BTreeMap<String, usize>,
    pub strength_counts: BTreeMap<String, usize>,
    pub confidence_counts: BTreeMap<String, usize>,
}

#[derive(Clone)]
struct Contribution {
    edge_id: String,
    occurrence_key: String,
    forward: bool,
    weight: f64,
    strength: &'static str,
    confidence: &'static str,
}

pub(crate) fn from_typed_document(
    document: &GraphDocument,
    limits: TopologyLimits,
) -> Result<CommunityTopology, CommunityTopologyError> {
    if document.nodes.len() > limits.max_nodes {
        return Err(CommunityTopologyError::LimitExceeded {
            stage: "nodes",
            required: document.nodes.len(),
            limit: limits.max_nodes,
            processed: 0,
        });
    }
    if document.links.len() > limits.max_edges {
        return Err(CommunityTopologyError::LimitExceeded {
            stage: "edges",
            required: document.links.len(),
            limit: limits.max_edges,
            processed: 0,
        });
    }
    let mut ids = document
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    for duplicate in ids.windows(2) {
        if duplicate[0] == duplicate[1] {
            return Err(CommunityTopologyError::DuplicateNode {
                node: duplicate[0].clone(),
            });
        }
    }
    let positions = ids
        .iter()
        .enumerate()
        .map(|(position, id)| (id.as_str(), position))
        .collect::<BTreeMap<_, _>>();
    let mut grouped = BTreeMap::<(usize, usize, String), Vec<Contribution>>::new();
    let mut projected_pairs = BTreeSet::<(usize, usize)>::new();
    let mut evidence = TopologyEvidence {
        input_edge_count: document.links.len(),
        ..TopologyEvidence::default()
    };
    for (processed, edge) in document.links.iter().enumerate() {
        let Some(&source) = positions.get(edge.source.as_str()) else {
            return Err(CommunityTopologyError::DanglingEndpoint {
                edge: edge.id.clone(),
                endpoint: edge.source.clone(),
            });
        };
        let Some(&target) = positions.get(edge.target.as_str()) else {
            return Err(CommunityTopologyError::DanglingEndpoint {
                edge: edge.id.clone(),
                endpoint: edge.target.clone(),
            });
        };
        let input_weight = edge.weight.unwrap_or(1.0);
        if !input_weight.is_finite() || input_weight <= 0.0 {
            return Err(CommunityTopologyError::InvalidEdgeWeight {
                edge: edge.id.clone(),
                weight: input_weight,
            });
        }
        evidence.input_weight_sum += input_weight;
        if !evidence.input_weight_sum.is_finite() {
            return Err(CommunityTopologyError::InvalidEdgeWeight {
                edge: edge.id.clone(),
                weight: evidence.input_weight_sum,
            });
        }
        let confidence =
            effective_confidence(&edge.evidence).unwrap_or(EvidenceConfidence::Inferred);
        *evidence
            .relationship_counts
            .entry(edge.kind.as_str().to_owned())
            .or_default() += 1;
        let strength = relationship_strength(edge.kind);
        *evidence
            .strength_counts
            .entry(strength.name().to_owned())
            .or_default() += 1;
        *evidence
            .confidence_counts
            .entry(confidence.as_str().to_owned())
            .or_default() += 1;
        if confidence == EvidenceConfidence::Ambiguous {
            evidence.omitted_ambiguous_count += 1;
            continue;
        }
        let (left, right, forward) = if source <= target {
            (source, target, true)
        } else {
            (target, source, false)
        };
        let relation = edge.kind.as_str().to_owned();
        let pair_is_new = !projected_pairs.contains(&(left, right));
        if pair_is_new && projected_pairs.len() == limits.max_projected_pairs {
            return Err(CommunityTopologyError::LimitExceeded {
                stage: "projected_pairs",
                required: projected_pairs.len().saturating_add(1),
                limit: limits.max_projected_pairs,
                processed,
            });
        }
        projected_pairs.insert((left, right));
        let confidence_factor = match confidence {
            EvidenceConfidence::Exact => 1.0,
            EvidenceConfidence::Inferred => 0.5,
            EvidenceConfidence::Ambiguous => 0.0,
        };
        grouped
            .entry((left, right, relation))
            .or_default()
            .push(Contribution {
                edge_id: edge.id.clone(),
                occurrence_key: occurrence_key(edge),
                forward,
                weight: strength.weight() * confidence_factor * input_weight,
                strength: strength.name(),
                confidence: confidence.as_str(),
            });
    }

    let members = ids.iter().cloned().map(|id| BTreeSet::from([id])).collect();
    let mut graph = WeightedGraph::new(ids, members);
    let mut pair_weights = BTreeMap::<(usize, usize), f64>::new();
    let mut pair_evidence = BTreeMap::<(usize, usize), ProjectedPairEvidence>::new();
    for ((left, right, relation), contributions) in &mut grouped {
        let before_deduplication = contributions.len();
        contributions.sort_by(|left, right| {
            left.occurrence_key
                .cmp(&right.occurrence_key)
                .then_with(|| left.forward.cmp(&right.forward))
                .then_with(|| left.edge_id.cmp(&right.edge_id))
        });
        contributions.dedup_by(|left, right| {
            left.occurrence_key == right.occurrence_key && left.forward == right.forward
        });
        evidence.omitted_duplicate_occurrence_count +=
            before_deduplication.saturating_sub(contributions.len());
        evidence.omitted_capped_count += contributions
            .len()
            .saturating_sub(MAX_OCCURRENCES_PER_PAIR_KIND);
        for contribution in contributions.iter().take(MAX_OCCURRENCES_PER_PAIR_KIND) {
            evidence.retained_occurrence_count += 1;
            if contribution.forward {
                evidence.forward_occurrence_count += 1;
            } else {
                evidence.reverse_occurrence_count += 1;
            }
            *evidence
                .retained_relationship_counts
                .entry(relation.clone())
                .or_default() += 1;
            *evidence
                .retained_strength_counts
                .entry(contribution.strength.to_owned())
                .or_default() += 1;
            *evidence
                .retained_confidence_counts
                .entry(contribution.confidence.to_owned())
                .or_default() += 1;
            let pair =
                pair_evidence
                    .entry((*left, *right))
                    .or_insert_with(|| ProjectedPairEvidence {
                        left: *left,
                        right: *right,
                        edge_ids: Vec::new(),
                        relationship_counts: BTreeMap::new(),
                        strength_counts: BTreeMap::new(),
                        confidence_counts: BTreeMap::new(),
                    });
            pair.edge_ids.push(contribution.edge_id.clone());
            *pair
                .relationship_counts
                .entry(relation.clone())
                .or_default() += 1;
            *pair
                .strength_counts
                .entry(contribution.strength.to_owned())
                .or_default() += 1;
            *pair
                .confidence_counts
                .entry(contribution.confidence.to_owned())
                .or_default() += 1;
            *pair_weights.entry((*left, *right)).or_default() += contribution.weight;
        }
    }
    evidence.projected_pair_count = pair_weights.len();
    for (processed, ((left, right), weight)) in pair_weights.into_iter().enumerate() {
        if weight > MAX_PAIR_WEIGHT {
            evidence.omitted_pair_weight_cap_count += 1;
        }
        let retained_weight = weight.min(MAX_PAIR_WEIGHT);
        let required = evidence.retained_total_weight + retained_weight;
        if !required.is_finite() || required > limits.max_total_weight {
            return Err(CommunityTopologyError::TotalWeightLimitExceeded {
                required,
                limit: limits.max_total_weight,
                processed,
            });
        }
        evidence.retained_total_weight = required;
        graph.add_edge(left, right, retained_weight);
    }
    for pair in pair_evidence.values_mut() {
        pair.edge_ids.sort();
        pair.edge_ids.dedup();
    }
    Ok(CommunityTopology {
        graph,
        evidence,
        pair_evidence: pair_evidence.into_values().collect(),
    })
}

fn occurrence_key(edge: &compass_model::code_graph::EdgeRecord) -> String {
    let mut anchors = edge
        .relationship_site
        .iter()
        .chain(edge.evidence.iter().flat_map(|item| item.anchors.iter()))
        .map(|anchor| {
            format!(
                "{}:{}:{}:{}",
                anchor.file, anchor.start_byte, anchor.end_byte, anchor.start_line
            )
        })
        .collect::<Vec<_>>();
    anchors.sort();
    anchors.dedup();
    if anchors.is_empty() {
        edge.id.clone()
    } else {
        anchors.join("|")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::code_graph::{BuildMetadata, EdgeRecord, NodeKind, NodeRecord};
    use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance};

    fn node(id: &str) -> NodeRecord {
        NodeRecord {
            id: id.to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: id.to_owned(),
            qualified_name: id.to_owned(),
            language: None,
            framework: None,
            source: None,
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        }
    }

    fn edge(
        id: &str,
        source: &str,
        target: &str,
        kind: EdgeKind,
        confidence: EvidenceConfidence,
    ) -> EdgeRecord {
        EdgeRecord {
            id: id.to_owned(),
            key: id.to_owned(),
            source: source.to_owned(),
            target: target.to_owned(),
            kind,
            occurrence_rule: None,
            relationship_site: None,
            details: None,
            evidence: vec![Provenance {
                origin: EvidenceOrigin::Ast,
                extractor: "test".to_owned(),
                confidence,
                rule: None,
                anchors: Vec::new(),
                wiring_site: None,
                score: None,
                candidates: Vec::new(),
            }],
            weight: Some(1.0),
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        }
    }

    fn document() -> GraphDocument {
        let mut document = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "test".to_owned(),
            source_tree_digest: "test".to_owned(),
            configuration_digest: "test".to_owned(),
            generation_id: "test".to_owned(),
            source_commit: None,
        });
        document.nodes = ["a", "b"].into_iter().map(node).collect();
        document
    }

    #[test]
    fn typed_projection_weights_direction_confidence_and_relation()
    -> Result<(), CommunityTopologyError> {
        let mut document = document();
        document.links = vec![
            edge("1", "a", "b", EdgeKind::Calls, EvidenceConfidence::Exact),
            edge(
                "2",
                "b",
                "a",
                EdgeKind::Imports,
                EvidenceConfidence::Inferred,
            ),
            edge(
                "3",
                "a",
                "b",
                EdgeKind::Calls,
                EvidenceConfidence::Ambiguous,
            ),
        ];
        let topology = from_typed_document(
            &document,
            TopologyLimits {
                max_nodes: 10,
                max_edges: 10,
                max_projected_pairs: 10,
                max_total_weight: 1_000.0,
            },
        )?;
        assert_eq!(topology.graph.edge_count(), 1);
        assert_eq!(topology.graph.total_weight(), 5.0);
        assert_eq!(topology.evidence.forward_occurrence_count, 1);
        assert_eq!(topology.evidence.reverse_occurrence_count, 1);
        assert_eq!(topology.evidence.omitted_ambiguous_count, 1);
        assert_eq!(topology.evidence.retained_relationship_counts["calls"], 1);
        assert_eq!(topology.evidence.retained_confidence_counts["exact"], 1);
        assert_eq!(topology.evidence.retained_total_weight, 5.0);
        Ok(())
    }

    #[test]
    fn projection_caps_occurrences_and_is_order_invariant() -> Result<(), CommunityTopologyError> {
        let mut document = document();
        document.links = (0..6)
            .map(|index| {
                edge(
                    &format!("edge-{index}"),
                    "a",
                    "b",
                    EdgeKind::Calls,
                    EvidenceConfidence::Exact,
                )
            })
            .collect();
        document.links.push(document.links[0].clone());
        let limits = TopologyLimits {
            max_nodes: 10,
            max_edges: 10,
            max_projected_pairs: 10,
            max_total_weight: 1_000.0,
        };
        let expected = from_typed_document(&document, limits)?;
        document.nodes.reverse();
        document.links.reverse();
        let actual = from_typed_document(&document, limits)?;

        assert_eq!(expected.graph.total_weight(), 16.0);
        assert_eq!(expected.evidence.omitted_duplicate_occurrence_count, 1);
        assert_eq!(expected.evidence.omitted_capped_count, 2);
        assert_eq!(actual.evidence, expected.evidence);
        assert_eq!(actual.pair_evidence, expected.pair_evidence);
        assert_eq!(actual.graph.ids, expected.graph.ids);
        assert_eq!(
            actual.graph.edges().collect::<Vec<_>>(),
            expected.graph.edges().collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn projection_fails_closed_at_the_total_weight_limit() {
        let mut document = document();
        document.links = vec![edge(
            "edge",
            "a",
            "b",
            EdgeKind::Calls,
            EvidenceConfidence::Exact,
        )];
        assert!(matches!(
            from_typed_document(
                &document,
                TopologyLimits {
                    max_nodes: 10,
                    max_edges: 10,
                    max_projected_pairs: 10,
                    max_total_weight: 1.0,
                }
            ),
            Err(CommunityTopologyError::TotalWeightLimitExceeded {
                required: 4.0,
                limit: 1.0,
                processed: 0,
            })
        ));
    }
}
