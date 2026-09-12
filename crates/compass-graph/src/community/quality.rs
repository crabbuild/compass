use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::identity::{
    COMPATIBILITY_CLUSTER_ALGORITHM, COMPATIBILITY_CLUSTER_LIMITS, COMPATIBILITY_CLUSTER_QUALITY,
    COMPATIBILITY_CLUSTER_SEED, COMPATIBILITY_CLUSTER_SELECTOR, COMPATIBILITY_CLUSTER_TOPOLOGY,
};
use super::topology::{ProjectedPairEvidence, TopologyEvidence};
use crate::cluster::{Communities, WeightedGraph};
use compass_model::GraphDocument;

pub const DEFAULT_MAX_QUALITY_VISITS: usize = 50_000_000;
pub const DEFAULT_QUALITY_WITNESS_LIMIT: usize = 8;

/// Quality evidence for one detected community.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityQuality {
    pub community: usize,
    pub member_count: usize,
    pub isolate: bool,
    pub connected_component_count: usize,
    pub internal_edge_count: usize,
    pub internal_weight: f64,
    pub boundary_edge_count: usize,
    pub boundary_weight: f64,
    pub volume: f64,
    pub density: f64,
    pub conductance: f64,
    pub modularity_contribution: f64,
    pub relationship_mix: BTreeMap<String, usize>,
    pub relation_strength_mix: BTreeMap<String, usize>,
    pub evidence_confidence_mix: BTreeMap<String, usize>,
    pub witness_node_ids: Vec<String>,
    pub witness_edge_ids: Vec<String>,
    pub omitted_witness_node_count: usize,
    pub omitted_witness_edge_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityCandidateSummary {
    pub resolution: f64,
    pub modularity: f64,
    pub weighted_mean_conductance: f64,
    pub disconnected_community_count: usize,
    pub size_violation_count: usize,
    pub non_isolate_singleton_count: usize,
    pub partition_digest: String,
    pub selected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateAgreement {
    pub left_resolution: f64,
    pub right_resolution: f64,
    pub adjusted_rand_index: f64,
    pub exact_membership: bool,
}

/// Complete evidence for a partition evaluated on one named topology.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PartitionQuality {
    pub assigned_node_count: usize,
    pub omitted_node_count: usize,
    pub community_count: usize,
    pub non_isolate_singleton_count: usize,
    pub disconnected_community_count: usize,
    pub modularity: f64,
    pub weighted_mean_conductance: f64,
    pub worst_conductance: f64,
    pub largest_community_fraction: f64,
    pub resolution: f64,
    pub algorithm: String,
    pub topology: String,
    pub quality: String,
    pub selector: String,
    pub seed: u32,
    pub limits: String,
    pub quality_visit_count: usize,
    pub quality_visit_limit: usize,
    pub witness_limit: usize,
    pub omitted_witness_node_count: usize,
    pub omitted_witness_edge_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_summaries: Vec<CommunityCandidateSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_agreement: Vec<CandidateAgreement>,
    pub selection_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology_evidence: Option<TopologyEvidence>,
    pub communities: BTreeMap<usize, CommunityQuality>,
}

#[derive(Clone, Copy)]
pub(crate) struct QualityIdentity<'a> {
    pub algorithm: &'a str,
    pub topology: &'a str,
    pub quality: &'a str,
    pub selector: &'a str,
    pub seed: u32,
    pub limits: &'a str,
}

/// A partition cannot be evaluated when it does not unambiguously assign the
/// detector topology.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum CommunityQualityError {
    #[error("community {community} contains unknown node `{node}`")]
    UnknownMember { community: usize, node: String },
    #[error("node `{node}` is assigned to communities {first} and {second}")]
    DuplicateMember {
        node: String,
        first: usize,
        second: usize,
    },
    #[error("community quality resolution must be finite and positive, got {resolution}")]
    InvalidResolution { resolution: f64 },
    #[error(
        "community quality {stage} requires {required} visits, exceeds limit {limit} after {processed}"
    )]
    QualityLimitExceeded {
        stage: &'static str,
        required: usize,
        limit: usize,
        processed: usize,
    },
}

/// Evaluate a complete or partial partition on the exact topology consumed by
/// the compatibility detector.
pub fn evaluate_partition_quality(
    document: &GraphDocument,
    communities: &Communities,
    resolution: f64,
) -> Result<PartitionQuality, CommunityQualityError> {
    if !resolution.is_finite() || resolution <= 0.0 {
        return Err(CommunityQualityError::InvalidResolution { resolution });
    }
    let graph = WeightedGraph::from_document(document);
    evaluate_graph_partition(
        &graph,
        communities,
        resolution,
        QualityEvaluationOptions {
            identity: QualityIdentity {
                algorithm: COMPATIBILITY_CLUSTER_ALGORITHM,
                topology: COMPATIBILITY_CLUSTER_TOPOLOGY,
                quality: COMPATIBILITY_CLUSTER_QUALITY,
                selector: COMPATIBILITY_CLUSTER_SELECTOR,
                seed: COMPATIBILITY_CLUSTER_SEED,
                limits: COMPATIBILITY_CLUSTER_LIMITS,
            },
            topology_evidence: None,
            pair_evidence: None,
            max_quality_visits: DEFAULT_MAX_QUALITY_VISITS,
            witness_limit: DEFAULT_QUALITY_WITNESS_LIMIT,
        },
    )
}

/// Preserve the public cohesion projection while sourcing its edge inventory
/// from the same canonical topology as clustering and richer quality evidence.
pub(crate) fn compatibility_density_scores(
    document: &GraphDocument,
    communities: &Communities,
) -> BTreeMap<usize, f64> {
    let graph = WeightedGraph::from_document(document);
    density_scores(&graph, communities)
}

fn density_scores(graph: &WeightedGraph, communities: &Communities) -> BTreeMap<usize, f64> {
    let positions = graph.position_map();
    let mut assignments = vec![None; graph.len()];
    for (community, members) in communities {
        for member in members {
            if let Some(position) = positions.get(member) {
                assignments[*position] = Some(*community);
            }
        }
    }
    let mut internal_edges = BTreeMap::<usize, usize>::new();
    for (left, right, _) in graph.edges() {
        if let Some(community) = assignments[left]
            && assignments[right] == Some(community)
        {
            *internal_edges.entry(community).or_default() += 1;
        }
    }
    communities
        .iter()
        .map(|(community, members)| {
            let possible = members
                .len()
                .saturating_mul(members.len().saturating_sub(1))
                / 2;
            let density = if possible == 0 {
                1.0
            } else {
                internal_edges.get(community).copied().unwrap_or_default() as f64 / possible as f64
            };
            (*community, density)
        })
        .collect()
}

pub(crate) struct QualityEvaluationOptions<'a> {
    pub(crate) identity: QualityIdentity<'a>,
    pub(crate) topology_evidence: Option<TopologyEvidence>,
    pub(crate) pair_evidence: Option<&'a [ProjectedPairEvidence]>,
    pub(crate) max_quality_visits: usize,
    pub(crate) witness_limit: usize,
}

pub(crate) fn evaluate_graph_partition(
    graph: &WeightedGraph,
    communities: &Communities,
    resolution: f64,
    options: QualityEvaluationOptions<'_>,
) -> Result<PartitionQuality, CommunityQualityError> {
    let QualityEvaluationOptions {
        identity,
        topology_evidence,
        pair_evidence,
        max_quality_visits,
        witness_limit,
    } = options;
    let assigned_members = communities.values().map(Vec::len).sum::<usize>();
    let quality_visit_count = graph
        .len()
        .saturating_add(assigned_members.saturating_mul(2))
        .saturating_add(graph.edge_count().saturating_mul(4));
    if quality_visit_count > max_quality_visits {
        return Err(CommunityQualityError::QualityLimitExceeded {
            stage: "partition_metrics",
            required: quality_visit_count,
            limit: max_quality_visits,
            processed: 0,
        });
    }
    let positions = graph.position_map();
    let mut assignments = vec![None; graph.len()];
    for (community, members) in communities {
        for member in members {
            let Some(position) = positions.get(member) else {
                return Err(CommunityQualityError::UnknownMember {
                    community: *community,
                    node: member.clone(),
                });
            };
            if let Some(first) = assignments[*position] {
                return Err(CommunityQualityError::DuplicateMember {
                    node: member.clone(),
                    first,
                    second: *community,
                });
            }
            assignments[*position] = Some(*community);
        }
    }

    let total_weight = graph.total_weight();
    let total_volume = 2.0 * total_weight;
    let densities = density_scores(graph, communities);
    let mut metrics = communities
        .iter()
        .map(|(community, members)| {
            (
                *community,
                CommunityQuality {
                    community: *community,
                    member_count: members.len(),
                    isolate: false,
                    connected_component_count: 0,
                    internal_edge_count: 0,
                    internal_weight: 0.0,
                    boundary_edge_count: 0,
                    boundary_weight: 0.0,
                    volume: 0.0,
                    density: 1.0,
                    conductance: 0.0,
                    modularity_contribution: 0.0,
                    relationship_mix: BTreeMap::new(),
                    relation_strength_mix: BTreeMap::new(),
                    evidence_confidence_mix: BTreeMap::new(),
                    witness_node_ids: Vec::new(),
                    witness_edge_ids: Vec::new(),
                    omitted_witness_node_count: 0,
                    omitted_witness_edge_count: 0,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    for (node, assignment) in assignments.iter().enumerate() {
        if let Some(community) = assignment
            && let Some(metric) = metrics.get_mut(community)
        {
            metric.volume += graph.degree_weighted(node);
        }
    }
    for (left, right, weight) in graph.edges() {
        let left_community = assignments[left];
        let right_community = assignments[right];
        if let Some(community) = left_community
            && right_community == Some(community)
            && let Some(metric) = metrics.get_mut(&community)
        {
            metric.internal_edge_count += 1;
            metric.internal_weight += weight;
        } else {
            if let Some(community) = left_community
                && let Some(metric) = metrics.get_mut(&community)
            {
                metric.boundary_edge_count += 1;
                metric.boundary_weight += weight;
            }
            if right != left
                && let Some(community) = right_community
                && let Some(metric) = metrics.get_mut(&community)
            {
                metric.boundary_edge_count += 1;
                metric.boundary_weight += weight;
            }
        }
    }
    let mut witness_edges = BTreeMap::<usize, BTreeSet<String>>::new();
    if let Some(pair_evidence) = pair_evidence {
        for pair in pair_evidence {
            let left_community = assignments[pair.left];
            let right_community = assignments[pair.right];
            let mut admitted = BTreeSet::new();
            if let Some(community) = left_community {
                admitted.insert(community);
            }
            if let Some(community) = right_community {
                admitted.insert(community);
            }
            for community in admitted {
                if let Some(metric) = metrics.get_mut(&community) {
                    merge_counts(&mut metric.relationship_mix, &pair.relationship_counts);
                    merge_counts(&mut metric.relation_strength_mix, &pair.strength_counts);
                    merge_counts(&mut metric.evidence_confidence_mix, &pair.confidence_counts);
                    if left_community != right_community {
                        witness_edges
                            .entry(community)
                            .or_default()
                            .extend(pair.edge_ids.iter().cloned());
                    }
                }
            }
        }
    }

    for (community, members) in communities {
        let member_positions = members
            .iter()
            .filter_map(|member| positions.get(member).copied())
            .collect::<BTreeSet<_>>();
        let component_count = connected_component_count(graph, &member_positions);
        if let Some(metric) = metrics.get_mut(community) {
            metric.connected_component_count = component_count;
            metric.isolate = metric.member_count == 1 && metric.volume == 0.0;
            metric.density = densities.get(community).copied().unwrap_or(1.0);
            let conductance_denominator = metric.volume.min(total_volume - metric.volume);
            metric.conductance = if conductance_denominator > 0.0 {
                metric.boundary_weight / conductance_denominator
            } else {
                0.0
            };
            metric.modularity_contribution = if total_weight > 0.0 {
                metric.internal_weight / total_weight
                    - resolution * (metric.volume / total_volume).powi(2)
            } else {
                0.0
            };
            let fraction = metric.member_count as f64 / graph.len().max(1) as f64;
            let degraded = component_count > 1
                || (metric.member_count == 1 && !metric.isolate)
                || metric.conductance >= 0.5
                || fraction > 0.25;
            if degraded {
                let mut node_ids = members.clone();
                node_ids.sort();
                metric.omitted_witness_node_count = node_ids.len().saturating_sub(witness_limit);
                node_ids.truncate(witness_limit);
                metric.witness_node_ids = node_ids;
                let mut edge_ids = witness_edges
                    .remove(community)
                    .unwrap_or_default()
                    .into_iter()
                    .collect::<Vec<_>>();
                metric.omitted_witness_edge_count = edge_ids.len().saturating_sub(witness_limit);
                edge_ids.truncate(witness_limit);
                metric.witness_edge_ids = edge_ids;
            }
        }
    }

    let assigned_node_count = assignments.iter().filter(|value| value.is_some()).count();
    let disconnected_community_count = metrics
        .values()
        .filter(|metric| !metric.isolate && metric.connected_component_count > 1)
        .count();
    let non_isolate_singleton_count = metrics
        .values()
        .filter(|metric| metric.member_count == 1 && !metric.isolate)
        .count();
    let modularity = metrics
        .values()
        .map(|metric| metric.modularity_contribution)
        .sum();
    let retained_volume = metrics.values().map(|metric| metric.volume).sum::<f64>();
    let weighted_mean_conductance = if retained_volume > 0.0 {
        metrics
            .values()
            .map(|metric| metric.conductance * metric.volume)
            .sum::<f64>()
            / retained_volume
    } else {
        0.0
    };
    let worst_conductance = metrics
        .values()
        .map(|metric| metric.conductance)
        .fold(0.0, f64::max);
    let largest_community_fraction = if graph.is_empty() {
        0.0
    } else {
        metrics
            .values()
            .map(|metric| metric.member_count)
            .max()
            .unwrap_or_default() as f64
            / graph.len() as f64
    };
    let omitted_witness_node_count = metrics
        .values()
        .map(|metric| metric.omitted_witness_node_count)
        .sum();
    let omitted_witness_edge_count = metrics
        .values()
        .map(|metric| metric.omitted_witness_edge_count)
        .sum();

    Ok(PartitionQuality {
        assigned_node_count,
        omitted_node_count: graph.len().saturating_sub(assigned_node_count),
        community_count: metrics.len(),
        non_isolate_singleton_count,
        disconnected_community_count,
        modularity,
        weighted_mean_conductance,
        worst_conductance,
        largest_community_fraction,
        resolution,
        algorithm: identity.algorithm.to_owned(),
        topology: identity.topology.to_owned(),
        quality: identity.quality.to_owned(),
        selector: identity.selector.to_owned(),
        seed: identity.seed,
        limits: identity.limits.to_owned(),
        quality_visit_count,
        quality_visit_limit: max_quality_visits,
        witness_limit,
        omitted_witness_node_count,
        omitted_witness_edge_count,
        candidate_summaries: Vec::new(),
        candidate_agreement: Vec::new(),
        selection_reason: "fixed resolution".to_owned(),
        topology_evidence,
        communities: metrics,
    })
}

fn merge_counts(target: &mut BTreeMap<String, usize>, source: &BTreeMap<String, usize>) {
    for (key, count) in source {
        *target.entry(key.clone()).or_default() += count;
    }
}

/// Adjusted Rand agreement for two complete partitions of the same node set.
#[must_use]
pub fn adjusted_rand_index(left: &Communities, right: &Communities) -> f64 {
    let left_assignments = assignments_by_id(left);
    let right_assignments = assignments_by_id(right);
    if left_assignments.keys().ne(right_assignments.keys()) {
        return 0.0;
    }
    let mut contingency = BTreeMap::<(usize, usize), usize>::new();
    let mut left_counts = BTreeMap::<usize, usize>::new();
    let mut right_counts = BTreeMap::<usize, usize>::new();
    for (node, left_community) in &left_assignments {
        let Some(right_community) = right_assignments.get(node) else {
            return 0.0;
        };
        *contingency
            .entry((*left_community, *right_community))
            .or_default() += 1;
        *left_counts.entry(*left_community).or_default() += 1;
        *right_counts.entry(*right_community).or_default() += 1;
    }
    let pairs = combination_two(left_assignments.len());
    if pairs == 0.0 {
        return 1.0;
    }
    let index = contingency
        .values()
        .map(|count| combination_two(*count))
        .sum::<f64>();
    let left_index = left_counts
        .values()
        .map(|count| combination_two(*count))
        .sum::<f64>();
    let right_index = right_counts
        .values()
        .map(|count| combination_two(*count))
        .sum::<f64>();
    let expected = left_index * right_index / pairs;
    let maximum = 0.5 * (left_index + right_index);
    if (maximum - expected).abs() <= f64::EPSILON {
        return if left_assignments == right_assignments {
            1.0
        } else {
            0.0
        };
    }
    (index - expected) / (maximum - expected)
}

/// Adjusted mutual information using the arithmetic-mean entropy normalizer.
/// Returns zero when the partitions do not cover the same node IDs.
pub fn adjusted_mutual_information(
    left: &Communities,
    right: &Communities,
) -> Result<f64, CommunityQualityError> {
    let left_assignments = assignments_by_id(left);
    let right_assignments = assignments_by_id(right);
    if left_assignments.keys().ne(right_assignments.keys()) {
        return Ok(0.0);
    }
    let node_count = left_assignments.len();
    if node_count <= 1 {
        return Ok(1.0);
    }
    let mut contingency = BTreeMap::<(usize, usize), usize>::new();
    let mut left_counts = BTreeMap::<usize, usize>::new();
    let mut right_counts = BTreeMap::<usize, usize>::new();
    for (node, left_community) in &left_assignments {
        let Some(right_community) = right_assignments.get(node) else {
            return Ok(0.0);
        };
        *contingency
            .entry((*left_community, *right_community))
            .or_default() += 1;
        *left_counts.entry(*left_community).or_default() += 1;
        *right_counts.entry(*right_community).or_default() += 1;
    }
    let total = node_count as f64;
    let mutual_information = contingency
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|((left_community, right_community), count)| {
            let count = *count as f64;
            let left_count = left_counts[left_community] as f64;
            let right_count = right_counts[right_community] as f64;
            count / total * (total * count / (left_count * right_count)).ln()
        })
        .sum::<f64>();
    let expected = expected_mutual_information(
        node_count,
        left_counts.values().copied(),
        right_counts.values().copied(),
        DEFAULT_MAX_QUALITY_VISITS,
    )?;
    let left_entropy = entropy(total, left_counts.values().copied());
    let right_entropy = entropy(total, right_counts.values().copied());
    let normalizer = 0.5 * (left_entropy + right_entropy);
    if (normalizer - expected).abs() <= 1e-12 {
        return Ok(if left_assignments == right_assignments {
            1.0
        } else {
            0.0
        });
    }
    Ok((mutual_information - expected) / (normalizer - expected))
}

fn entropy(total: f64, counts: impl Iterator<Item = usize>) -> f64 {
    counts
        .filter(|count| *count > 0)
        .map(|count| {
            let probability = count as f64 / total;
            -probability * probability.ln()
        })
        .sum()
}

fn expected_mutual_information(
    node_count: usize,
    left_counts: impl Iterator<Item = usize> + Clone,
    right_counts: impl Iterator<Item = usize> + Clone,
    max_visits: usize,
) -> Result<f64, CommunityQualityError> {
    let total = node_count as f64;
    let log_factorials = (0..=node_count)
        .scan(0.0, |sum, value| {
            if value > 1 {
                *sum += (value as f64).ln();
            }
            Some(*sum)
        })
        .collect::<Vec<_>>();
    let log_choose = |whole: usize, selected: usize| {
        log_factorials[whole]
            - log_factorials[selected]
            - log_factorials[whole.saturating_sub(selected)]
    };
    let mut expected = 0.0;
    let mut visits = 0usize;
    for left_count in left_counts {
        for right_count in right_counts.clone() {
            let lower = left_count
                .saturating_add(right_count)
                .saturating_sub(node_count)
                .max(1);
            let upper = left_count.min(right_count);
            for overlap in lower..=upper {
                visits = visits.saturating_add(1);
                if visits > max_visits {
                    return Err(CommunityQualityError::QualityLimitExceeded {
                        stage: "adjusted_mutual_information",
                        required: visits,
                        limit: max_visits,
                        processed: visits.saturating_sub(1),
                    });
                }
                let probability = (log_choose(left_count, overlap)
                    + log_choose(node_count - left_count, right_count - overlap)
                    - log_choose(node_count, right_count))
                .exp();
                let overlap = overlap as f64;
                expected += probability * overlap / total
                    * (total * overlap / (left_count as f64 * right_count as f64)).ln();
            }
        }
    }
    Ok(expected)
}

fn assignments_by_id(communities: &Communities) -> BTreeMap<&str, usize> {
    communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect()
}

fn combination_two(count: usize) -> f64 {
    count.saturating_mul(count.saturating_sub(1)) as f64 / 2.0
}

fn connected_component_count(graph: &WeightedGraph, members: &BTreeSet<usize>) -> usize {
    let mut remaining = members.clone();
    let mut components = 0usize;
    while let Some(start) = remaining.pop_first() {
        components += 1;
        let mut queue = VecDeque::from([start]);
        while let Some(node) = queue.pop_front() {
            for (neighbor, _) in graph.neighbors(node) {
                if remaining.remove(neighbor) {
                    queue.push_back(*neighbor);
                }
            }
        }
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::{EdgeRecord, GraphDocument, NodeRecord};
    use serde_json::json;

    fn node(id: &str) -> NodeRecord {
        NodeRecord {
            id: id.to_owned(),
            attributes: serde_json::Map::from_iter([("label".to_owned(), json!(id))]),
        }
    }

    fn edge(source: &str, target: &str, weight: f64) -> EdgeRecord {
        EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes: serde_json::Map::from_iter([("weight".to_owned(), json!(weight))]),
        }
    }

    #[test]
    fn evaluates_reference_partition() -> Result<(), CommunityQualityError> {
        let document = GraphDocument {
            directed: true,
            multigraph: true,
            graph: serde_json::Map::new(),
            nodes: ["a", "b", "c", "d"].into_iter().map(node).collect(),
            links: vec![
                edge("a", "b", 2.0),
                edge("b", "c", 1.0),
                edge("c", "d", 2.0),
            ],
            extras: BTreeMap::new(),
        };
        let communities = BTreeMap::from([
            (0, vec!["a".to_owned(), "b".to_owned()]),
            (1, vec!["c".to_owned(), "d".to_owned()]),
        ]);

        let quality = evaluate_partition_quality(&document, &communities, 1.0)?;

        assert_eq!(quality.assigned_node_count, 4);
        assert_eq!(quality.omitted_node_count, 0);
        assert_eq!(quality.disconnected_community_count, 0);
        assert!((quality.modularity - 0.3).abs() < 1e-12);
        assert!((quality.weighted_mean_conductance - 0.2).abs() < 1e-12);
        assert_eq!(quality.communities[&0].internal_edge_count, 1);
        assert_eq!(quality.communities[&0].internal_weight, 2.0);
        assert_eq!(quality.communities[&0].boundary_edge_count, 1);
        assert_eq!(quality.communities[&0].boundary_weight, 1.0);
        assert_eq!(quality.communities[&0].density, 1.0);
        assert_eq!(quality.communities[&0].connected_component_count, 1);
        Ok(())
    }

    #[test]
    fn reports_disconnected_and_omitted_nodes() -> Result<(), CommunityQualityError> {
        let document = GraphDocument {
            directed: false,
            multigraph: false,
            graph: serde_json::Map::new(),
            nodes: ["a", "b", "c"].into_iter().map(node).collect(),
            links: vec![edge("a", "c", 1.0)],
            extras: BTreeMap::new(),
        };
        let communities = BTreeMap::from([(0, vec!["a".to_owned(), "b".to_owned()])]);

        let quality = evaluate_partition_quality(&document, &communities, 1.0)?;

        assert_eq!(quality.assigned_node_count, 2);
        assert_eq!(quality.omitted_node_count, 1);
        assert_eq!(quality.disconnected_community_count, 1);
        assert_eq!(quality.communities[&0].connected_component_count, 2);
        assert_eq!(quality.communities[&0].conductance, 1.0);
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_partition_membership() {
        let document = GraphDocument {
            directed: false,
            multigraph: false,
            graph: serde_json::Map::new(),
            nodes: vec![node("a")],
            links: Vec::new(),
            extras: BTreeMap::new(),
        };
        let communities = BTreeMap::from([(2, vec!["a".to_owned()]), (3, vec!["a".to_owned()])]);

        assert_eq!(
            evaluate_partition_quality(&document, &communities, 1.0),
            Err(CommunityQualityError::DuplicateMember {
                node: "a".to_owned(),
                first: 2,
                second: 3,
            })
        );
    }

    #[test]
    fn adjusted_partition_metrics_match_identity_and_disagreement()
    -> Result<(), CommunityQualityError> {
        let planted = BTreeMap::from([
            (0, vec!["a".to_owned(), "b".to_owned()]),
            (1, vec!["c".to_owned(), "d".to_owned()]),
        ]);
        let crossed = BTreeMap::from([
            (0, vec!["a".to_owned(), "c".to_owned()]),
            (1, vec!["b".to_owned(), "d".to_owned()]),
        ]);
        assert!((adjusted_rand_index(&planted, &planted) - 1.0).abs() < 1e-12);
        assert!((adjusted_mutual_information(&planted, &planted)? - 1.0).abs() < 1e-12);
        assert!(adjusted_rand_index(&planted, &crossed) < 0.0);
        assert!(adjusted_mutual_information(&planted, &crossed)? < 0.0);
        Ok(())
    }
}
