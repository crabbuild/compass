use std::collections::{BTreeMap, BTreeSet};

use ahash::{AHashMap as HashMap, AHashSet as HashSet};
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
    pub relationship_counts: Vec<(&'static str, usize)>,
    pub strength_counts: Vec<(&'static str, usize)>,
    pub confidence_counts: Vec<(&'static str, usize)>,
}

struct PairAccumulator {
    evidence: ProjectedPairEvidence,
    weight: f64,
}

#[derive(Clone)]
struct Contribution {
    edge_index: usize,
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
        .collect::<HashMap<_, _>>();
    let mut grouped =
        Vec::<((usize, usize, &'static str), Contribution)>::with_capacity(document.links.len());
    let mut projected_pairs = HashSet::<(usize, usize)>::new();
    let mut evidence = TopologyEvidence {
        input_edge_count: document.links.len(),
        ..TopologyEvidence::default()
    };
    let mut relationship_counts = HashMap::<&'static str, usize>::with_capacity(32);
    let mut strength_counts = HashMap::<&'static str, usize>::with_capacity(3);
    let mut confidence_counts = HashMap::<&'static str, usize>::with_capacity(3);
    let mut retained_relationship_counts = HashMap::<&'static str, usize>::with_capacity(32);
    let mut retained_strength_counts = HashMap::<&'static str, usize>::with_capacity(3);
    let mut retained_confidence_counts = HashMap::<&'static str, usize>::with_capacity(3);
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
        increment_internal_count(&mut relationship_counts, edge.kind.as_str());
        let strength = relationship_strength(edge.kind);
        increment_internal_count(&mut strength_counts, strength.name());
        increment_internal_count(&mut confidence_counts, confidence.as_str());
        if confidence == EvidenceConfidence::Ambiguous {
            evidence.omitted_ambiguous_count += 1;
            continue;
        }
        let (left, right, forward) = if source <= target {
            (source, target, true)
        } else {
            (target, source, false)
        };
        let relation = edge.kind.as_str();
        if projected_pairs.insert((left, right))
            && projected_pairs.len() > limits.max_projected_pairs
        {
            return Err(CommunityTopologyError::LimitExceeded {
                stage: "projected_pairs",
                required: projected_pairs.len(),
                limit: limits.max_projected_pairs,
                processed,
            });
        }
        let confidence_factor = match confidence {
            EvidenceConfidence::Exact => 1.0,
            EvidenceConfidence::Inferred => 0.5,
            EvidenceConfidence::Ambiguous => 0.0,
        };
        grouped.push((
            (left, right, relation),
            Contribution {
                edge_index: processed,
                occurrence_key: String::new(),
                forward,
                weight: strength.weight() * confidence_factor * input_weight,
                strength: strength.name(),
                confidence: confidence.as_str(),
            },
        ));
    }

    let members = ids.iter().cloned().map(|id| BTreeSet::from([id])).collect();
    let mut graph = WeightedGraph::new(ids, members);
    let mut pairs =
        HashMap::<(usize, usize), PairAccumulator>::with_capacity(projected_pairs.len());
    grouped.sort_by(|left, right| left.0.cmp(&right.0));
    let mut group_start = 0usize;
    while group_start < grouped.len() {
        let (left, right, relation) = grouped[group_start].0;
        let mut group_end = group_start + 1;
        while group_end < grouped.len() && grouped[group_end].0 == grouped[group_start].0 {
            group_end += 1;
        }
        let contributions = &mut grouped[group_start..group_end];
        if contributions.len() > 1 {
            for (_, contribution) in contributions.iter_mut() {
                contribution.occurrence_key =
                    occurrence_key(&document.links[contribution.edge_index]);
            }
            contributions.sort_by(|left, right| {
                left.1
                    .occurrence_key
                    .cmp(&right.1.occurrence_key)
                    .then_with(|| left.1.forward.cmp(&right.1.forward))
                    .then_with(|| {
                        document.links[left.1.edge_index]
                            .id
                            .cmp(&document.links[right.1.edge_index].id)
                    })
            });
        }
        let mut unique_count = 0usize;
        for index in 0..contributions.len() {
            let duplicate = index > 0
                && contributions[index].1.occurrence_key
                    == contributions[index - 1].1.occurrence_key
                && contributions[index].1.forward == contributions[index - 1].1.forward;
            if duplicate {
                continue;
            }
            unique_count += 1;
            if unique_count > MAX_OCCURRENCES_PER_PAIR_KIND {
                continue;
            }
            let contribution = &contributions[index].1;
            evidence.retained_occurrence_count += 1;
            if contribution.forward {
                evidence.forward_occurrence_count += 1;
            } else {
                evidence.reverse_occurrence_count += 1;
            }
            increment_internal_count(&mut retained_relationship_counts, relation);
            increment_internal_count(&mut retained_strength_counts, contribution.strength);
            increment_internal_count(&mut retained_confidence_counts, contribution.confidence);
            let pair = pairs
                .entry((left, right))
                .or_insert_with(|| PairAccumulator {
                    evidence: ProjectedPairEvidence {
                        left,
                        right,
                        edge_ids: Vec::new(),
                        relationship_counts: Vec::new(),
                        strength_counts: Vec::new(),
                        confidence_counts: Vec::new(),
                    },
                    weight: 0.0,
                });
            pair.evidence
                .edge_ids
                .push(document.links[contribution.edge_index].id.clone());
            increment_count(&mut pair.evidence.relationship_counts, relation);
            increment_count(&mut pair.evidence.strength_counts, contribution.strength);
            increment_count(
                &mut pair.evidence.confidence_counts,
                contribution.confidence,
            );
            pair.weight += contribution.weight;
        }
        evidence.omitted_duplicate_occurrence_count +=
            contributions.len().saturating_sub(unique_count);
        evidence.omitted_capped_count += unique_count.saturating_sub(MAX_OCCURRENCES_PER_PAIR_KIND);
        group_start = group_end;
    }
    evidence.projected_pair_count = pairs.len();
    let mut pairs = pairs.into_values().collect::<Vec<_>>();
    pairs.sort_by_key(|pair| (pair.evidence.left, pair.evidence.right));
    for (processed, pair) in pairs.iter_mut().enumerate() {
        if pair.weight > MAX_PAIR_WEIGHT {
            evidence.omitted_pair_weight_cap_count += 1;
        }
        let retained_weight = pair.weight.min(MAX_PAIR_WEIGHT);
        let required = evidence.retained_total_weight + retained_weight;
        if !required.is_finite() || required > limits.max_total_weight {
            return Err(CommunityTopologyError::TotalWeightLimitExceeded {
                required,
                limit: limits.max_total_weight,
                processed,
            });
        }
        evidence.retained_total_weight = required;
        graph.add_unique_edge(pair.evidence.left, pair.evidence.right, retained_weight);
        pair.evidence.edge_ids.sort();
        pair.evidence.edge_ids.dedup();
        pair.evidence
            .relationship_counts
            .sort_by_key(|(key, _)| *key);
        pair.evidence.strength_counts.sort_by_key(|(key, _)| *key);
        pair.evidence.confidence_counts.sort_by_key(|(key, _)| *key);
    }
    let pair_evidence = pairs.into_iter().map(|pair| pair.evidence).collect();
    evidence.relationship_counts = published_counts(relationship_counts);
    evidence.strength_counts = published_counts(strength_counts);
    evidence.confidence_counts = published_counts(confidence_counts);
    evidence.retained_relationship_counts = published_counts(retained_relationship_counts);
    evidence.retained_strength_counts = published_counts(retained_strength_counts);
    evidence.retained_confidence_counts = published_counts(retained_confidence_counts);
    Ok(CommunityTopology {
        graph,
        evidence,
        pair_evidence,
    })
}

fn increment_count(counts: &mut Vec<(&'static str, usize)>, key: &'static str) {
    if let Some((_, count)) = counts.iter_mut().find(|(candidate, _)| *candidate == key) {
        *count += 1;
    } else {
        counts.push((key, 1));
    }
}

fn increment_internal_count(counts: &mut HashMap<&'static str, usize>, key: &'static str) {
    *counts.entry(key).or_default() += 1;
}

fn published_counts(counts: HashMap<&'static str, usize>) -> BTreeMap<String, usize> {
    counts
        .into_iter()
        .map(|(key, count)| (key.to_owned(), count))
        .collect()
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
    } else if anchors.len() == 1 {
        match anchors.pop() {
            Some(anchor) => anchor,
            None => edge.id.clone(),
        }
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
