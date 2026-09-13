use std::collections::{BTreeMap, BTreeSet, HashMap};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::identity::{
    COMPATIBILITY_CLUSTER_ALGORITHM, COMPATIBILITY_CLUSTER_LIMITS, COMPATIBILITY_CLUSTER_QUALITY,
    COMPATIBILITY_CLUSTER_SEED, COMPATIBILITY_CLUSTER_SELECTOR, COMPATIBILITY_CLUSTER_TOPOLOGY,
    QUALITY_CLUSTER_ALGORITHM, QUALITY_CLUSTER_LIMITS, QUALITY_CLUSTER_QUALITY,
    QUALITY_CLUSTER_SELECTOR, QUALITY_CLUSTER_TOPOLOGY,
};
use super::incremental::{
    IncrementalPreparation, IncrementalPreparationFallback, baseline_partition,
    prepare_anchored_topology,
};
use super::leiden::{CommunityDetectorError, leiden};
use super::quality::{
    CandidateAgreement, CommunityCandidateSummary, CommunityQualityError, PartitionQuality,
    QualityEvaluationOptions, QualityIdentity, adjusted_rand_index, evaluate_graph_partition,
    evaluate_partition_quality,
};
use super::topology::{CommunityTopologyError, TopologyLimits, from_typed_document};
use crate::cluster::{
    ClusterOptions, Communities, IncrementalClusterLimits, cluster, cluster_incremental,
    community_member_signatures, excluded_hubs, label_communities_by_hub, reattach_hubs,
    remap_communities_to_previous,
};

pub type PreviousCommunities = HashMap<String, usize>;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ResolutionPolicy {
    Auto { base: f64 },
    Fixed(f64),
}

impl ResolutionPolicy {
    fn base(self) -> f64 {
        match self {
            Self::Auto { base } | Self::Fixed(base) => base,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunityProfile {
    CompatibilityV1,
    QualityV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityLimits {
    pub incremental: IncrementalClusterLimits,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_projected_pairs: usize,
    pub max_total_weight: f64,
    pub max_candidates: usize,
    pub max_levels: usize,
    pub max_moves: usize,
    pub max_quality_visits: usize,
    pub witness_limit: usize,
}

impl Default for CommunityLimits {
    fn default() -> Self {
        Self {
            incremental: IncrementalClusterLimits::default(),
            max_nodes: 2_000_000,
            max_edges: 8_000_000,
            max_projected_pairs: 8_000_000,
            max_total_weight: 512_000_000.0,
            max_candidates: 3,
            max_levels: 10,
            max_moves: 100_000_000,
            max_quality_visits: super::quality::DEFAULT_MAX_QUALITY_VISITS,
            witness_limit: super::quality::DEFAULT_QUALITY_WITNESS_LIMIT,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CommunityRequest<'a> {
    pub profile: CommunityProfile,
    pub resolution: ResolutionPolicy,
    pub exclude_hubs_percentile: Option<f64>,
    pub previous: Option<&'a PreviousCommunities>,
    pub incremental: bool,
    pub changed_sources: &'a BTreeSet<String>,
    pub limits: CommunityLimits,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FallbackReason {
    CompatibilityGuard,
    InvalidIncrementalLimits,
    RemovedNode,
    AffectedRegionLimit,
    HubPolicy,
    FrozenAnchorsWouldMerge,
    QualityRegression,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunityExecution {
    Full,
    Incremental {
        affected_nodes: usize,
    },
    FullFallback {
        affected_nodes: usize,
        reason: FallbackReason,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityIdentity {
    pub algorithm: String,
    pub topology: String,
    pub quality: String,
    pub selector: String,
    pub seed: u32,
    pub limits: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityResult {
    pub communities: Communities,
    pub base_labels: BTreeMap<usize, String>,
    pub signatures: BTreeMap<usize, String>,
    pub quality: PartitionQuality,
    pub execution: CommunityExecution,
    pub identity: CommunityIdentity,
}

#[derive(Debug, Error)]
pub enum CommunityError {
    #[error("invalid community profile: {reason}")]
    InvalidProfile { reason: &'static str },
    #[error("invalid community resolution {resolution}; expected a finite positive value")]
    InvalidResolution { resolution: f64 },
    #[error("could not adapt the typed graph for community detection: {0}")]
    GraphAdapter(#[from] compass_model::GraphError),
    #[error(transparent)]
    Quality(#[from] CommunityQualityError),
    #[error(transparent)]
    Topology(#[from] CommunityTopologyError),
    #[error(transparent)]
    Detector(#[from] CommunityDetectorError),
    #[error(
        "community selector candidates requires {required} candidates, exceeds limit {limit} after {processed} candidates"
    )]
    CandidateLimitExceeded {
        required: usize,
        limit: usize,
        processed: usize,
    },
}

/// Build the complete compatibility community result from the typed Base Graph
/// authority. This is the migration seam for all production callers.
pub fn build_communities(
    document: &compass_model::code_graph::GraphDocument,
    request: &CommunityRequest<'_>,
) -> Result<CommunityResult, CommunityError> {
    let resolution = request.resolution.base();
    if !resolution.is_finite() || resolution <= 0.0 {
        return Err(CommunityError::InvalidResolution { resolution });
    }
    if !request.limits.max_total_weight.is_finite() || request.limits.max_total_weight < 0.0 {
        return Err(CommunityError::InvalidProfile {
            reason: "max_total_weight must be finite and non-negative",
        });
    }
    let legacy = document.to_legacy_document()?;
    if request.profile == CommunityProfile::QualityV1 {
        return build_quality_communities(document, &legacy, request, resolution);
    }
    let options = ClusterOptions {
        resolution,
        exclude_hubs_percentile: request.exclude_hubs_percentile,
    };
    let (communities, execution) = if request.incremental
        && let Some(previous) = request.previous
    {
        let incremental = cluster_incremental(
            &legacy,
            previous,
            request.changed_sources,
            options,
            request.limits.incremental,
        );
        let execution = if incremental.used_incremental {
            CommunityExecution::Incremental {
                affected_nodes: incremental.affected_nodes,
            }
        } else {
            CommunityExecution::FullFallback {
                affected_nodes: incremental.affected_nodes,
                reason: FallbackReason::CompatibilityGuard,
            }
        };
        (incremental.communities, execution)
    } else {
        (cluster(&legacy, options), CommunityExecution::Full)
    };
    let communities = match request.previous {
        Some(previous) => remap_communities_to_previous(&communities, previous),
        None => communities,
    };
    let base_labels = label_communities_by_hub(&legacy, &communities);
    let signatures = community_member_signatures(&communities);
    let quality = evaluate_partition_quality(&legacy, &communities, resolution)?;
    let identity = CommunityIdentity {
        algorithm: COMPATIBILITY_CLUSTER_ALGORITHM.to_owned(),
        topology: COMPATIBILITY_CLUSTER_TOPOLOGY.to_owned(),
        quality: COMPATIBILITY_CLUSTER_QUALITY.to_owned(),
        selector: COMPATIBILITY_CLUSTER_SELECTOR.to_owned(),
        seed: COMPATIBILITY_CLUSTER_SEED,
        limits: COMPATIBILITY_CLUSTER_LIMITS.to_owned(),
    };
    Ok(CommunityResult {
        communities,
        base_labels,
        signatures,
        quality,
        execution,
        identity,
    })
}

fn build_quality_communities(
    document: &compass_model::code_graph::GraphDocument,
    legacy: &compass_model::GraphDocument,
    request: &CommunityRequest<'_>,
    base_resolution: f64,
) -> Result<CommunityResult, CommunityError> {
    let topology = from_typed_document(
        document,
        TopologyLimits {
            max_nodes: request.limits.max_nodes,
            max_edges: request.limits.max_edges,
            max_projected_pairs: request.limits.max_projected_pairs,
            max_total_weight: request.limits.max_total_weight,
        },
    )?;
    let resolutions = match request.resolution {
        ResolutionPolicy::Fixed(value) => vec![value],
        ResolutionPolicy::Auto { base } => vec![base * 0.75, base, base * (4.0 / 3.0)],
    };
    if let Some(resolution) = resolutions
        .iter()
        .copied()
        .find(|resolution| !resolution.is_finite() || *resolution <= 0.0)
    {
        return Err(CommunityError::InvalidResolution { resolution });
    }
    if resolutions.len() > request.limits.max_candidates {
        return Err(CommunityError::CandidateLimitExceeded {
            required: resolutions.len(),
            limit: request.limits.max_candidates,
            processed: 0,
        });
    }
    let selector_identity = if resolutions.len() == 1 {
        COMPATIBILITY_CLUSTER_SELECTOR
    } else {
        QUALITY_CLUSTER_SELECTOR
    };
    let identity = QualityIdentity {
        algorithm: QUALITY_CLUSTER_ALGORITHM,
        topology: QUALITY_CLUSTER_TOPOLOGY,
        quality: QUALITY_CLUSTER_QUALITY,
        selector: selector_identity,
        seed: COMPATIBILITY_CLUSTER_SEED,
        limits: QUALITY_CLUSTER_LIMITS,
    };
    let preparation = request
        .incremental
        .then_some(request.previous)
        .flatten()
        .map(|previous| {
            prepare_anchored_topology(
                document,
                &topology.graph,
                previous,
                request.changed_sources,
                request.limits.incremental,
                request.exclude_hubs_percentile,
            )
        });
    let mut execution = match &preparation {
        Some(IncrementalPreparation::Ready(anchored)) => CommunityExecution::Incremental {
            affected_nodes: anchored.affected_nodes,
        },
        Some(IncrementalPreparation::Unchanged(_)) => {
            CommunityExecution::Incremental { affected_nodes: 0 }
        }
        Some(IncrementalPreparation::Fallback {
            affected_nodes,
            reason,
        }) => CommunityExecution::FullFallback {
            affected_nodes: *affected_nodes,
            reason: map_preparation_fallback(*reason),
        },
        None => CommunityExecution::Full,
    };
    let mut candidates = Vec::<(f64, Communities, PartitionQuality, String)>::new();
    let mut quality_guards = Vec::new();
    if matches!(
        preparation,
        None | Some(IncrementalPreparation::Fallback { .. })
    ) {
        candidates = resolutions
            .par_iter()
            .map(|resolution| {
                full_candidate(&topology, *resolution, base_resolution, request, identity)
            })
            .collect::<Result<Vec<_>, CommunityError>>()?;
        quality_guards.resize(candidates.len(), true);
    } else {
        for resolution in &resolutions {
            let incremental = match &preparation {
                Some(IncrementalPreparation::Ready(anchored)) => anchored.cluster(
                    *resolution,
                    request.limits.max_levels,
                    request.limits.max_moves,
                )?,
                Some(IncrementalPreparation::Unchanged(communities)) => Some(communities.clone()),
                Some(IncrementalPreparation::Fallback { .. }) | None => None,
            };
            if matches!(&preparation, Some(IncrementalPreparation::Ready(_)))
                && incremental.is_none()
            {
                execution = CommunityExecution::FullFallback {
                    affected_nodes: match &preparation {
                        Some(IncrementalPreparation::Ready(anchored)) => anchored.affected_nodes,
                        _ => 0,
                    },
                    reason: FallbackReason::FrozenAnchorsWouldMerge,
                };
            }
            let communities = incremental.unwrap_or_default();
            let quality = evaluate_graph_partition(
                &topology.graph,
                &communities,
                base_resolution,
                QualityEvaluationOptions {
                    identity: QualityIdentity { ..identity },
                    topology_evidence: Some(topology.evidence.clone()),
                    pair_evidence: Some(&topology.pair_evidence),
                    max_quality_visits: request.limits.max_quality_visits,
                    witness_limit: request.limits.witness_limit,
                },
            )?;
            let digest = partition_digest(&communities);
            let quality_guard = if let Some(previous) = request.previous
                && matches!(execution, CommunityExecution::Incremental { .. })
            {
                let baseline = baseline_partition(&topology.graph, previous);
                let baseline_quality = evaluate_graph_partition(
                    &topology.graph,
                    &baseline,
                    base_resolution,
                    QualityEvaluationOptions {
                        identity,
                        topology_evidence: None,
                        pair_evidence: Some(&topology.pair_evidence),
                        max_quality_visits: request.limits.max_quality_visits,
                        witness_limit: request.limits.witness_limit,
                    },
                )?;
                quality.modularity + 1e-4 >= baseline_quality.modularity
                    && (quality.largest_community_fraction <= 0.25
                        || baseline_quality.largest_community_fraction > 0.25)
                    && quality.weighted_mean_conductance
                        <= baseline_quality.weighted_mean_conductance + 1e-4
            } else {
                true
            };
            quality_guards.push(quality_guard);
            candidates.push((*resolution, communities, quality, digest));
        }
    }
    let anchors_would_merge = matches!(
        execution,
        CommunityExecution::FullFallback {
            reason: FallbackReason::FrozenAnchorsWouldMerge,
            ..
        }
    );
    if anchors_would_merge
        || (matches!(execution, CommunityExecution::Incremental { .. })
            && !quality_guards.iter().any(|guard| *guard))
    {
        let affected_nodes = match &execution {
            CommunityExecution::Incremental { affected_nodes } => *affected_nodes,
            _ => 0,
        };
        if !anchors_would_merge {
            execution = CommunityExecution::FullFallback {
                affected_nodes,
                reason: FallbackReason::QualityRegression,
            };
        }
        candidates.clear();
        quality_guards.clear();
        candidates = resolutions
            .par_iter()
            .map(|resolution| {
                full_candidate(&topology, *resolution, base_resolution, request, identity)
            })
            .collect::<Result<Vec<_>, CommunityError>>()?;
        quality_guards.resize(candidates.len(), true);
    }
    let best_modularity = candidates
        .iter()
        .filter(|(_, _, quality, _)| quality.disconnected_community_count == 0)
        .map(|(_, _, quality, _)| quality.modularity)
        .fold(f64::NEG_INFINITY, f64::max);
    let tolerance = 1e-4;
    let mut eligible = candidates
        .iter()
        .enumerate()
        .filter(|(index, (_, _, quality, _))| {
            quality_guards[*index]
                && quality.assigned_node_count == topology.graph.len()
                && quality.disconnected_community_count == 0
                && quality.modularity + tolerance >= best_modularity
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| {
        let left_candidate = &candidates[*left];
        let right_candidate = &candidates[*right];
        candidate_size_violations(&left_candidate.2)
            .cmp(&candidate_size_violations(&right_candidate.2))
            .then_with(|| {
                left_candidate
                    .2
                    .weighted_mean_conductance
                    .total_cmp(&right_candidate.2.weighted_mean_conductance)
            })
            .then_with(|| {
                left_candidate
                    .2
                    .non_isolate_singleton_count
                    .cmp(&right_candidate.2.non_isolate_singleton_count)
            })
            .then_with(|| left_candidate.3.cmp(&right_candidate.3))
    });
    let Some(selected_index) = eligible.first().copied() else {
        if candidates
            .iter()
            .any(|(_, _, quality, _)| quality.assigned_node_count != topology.graph.len())
        {
            return Err(CommunityDetectorError::IncompletePartition.into());
        }
        return Err(CommunityDetectorError::DisconnectedPartition.into());
    };
    let candidate_agreement = candidate_agreement(&candidates);
    let summaries = candidates
        .iter()
        .enumerate()
        .map(|(index, (resolution, _, quality, digest))| {
            let rejection_reason = if !quality_guards[index] {
                Some("incremental quality regression".to_owned())
            } else if quality.assigned_node_count != topology.graph.len() {
                Some("partition incomplete".to_owned())
            } else if quality.disconnected_community_count != 0 {
                Some("partition disconnected".to_owned())
            } else if quality.modularity + tolerance < best_modularity {
                Some("outside modularity tolerance".to_owned())
            } else if index != selected_index {
                Some("lost deterministic quality tie-break".to_owned())
            } else {
                None
            };
            CommunityCandidateSummary {
                resolution: *resolution,
                modularity: quality.modularity,
                weighted_mean_conductance: quality.weighted_mean_conductance,
                disconnected_community_count: quality.disconnected_community_count,
                size_violation_count: candidate_size_violations(quality),
                non_isolate_singleton_count: quality.non_isolate_singleton_count,
                partition_digest: digest.clone(),
                selected: index == selected_index,
                rejection_reason,
            }
        })
        .collect();
    let (_, selected_communities, selected_quality, _) = candidates.swap_remove(selected_index);
    let communities = match request.previous {
        Some(previous) => remap_communities_to_previous(&selected_communities, previous),
        None => selected_communities,
    };
    let mut quality = if request.previous.is_none() {
        selected_quality
    } else {
        evaluate_graph_partition(
            &topology.graph,
            &communities,
            base_resolution,
            QualityEvaluationOptions {
                identity,
                topology_evidence: Some(topology.evidence.clone()),
                pair_evidence: Some(&topology.pair_evidence),
                max_quality_visits: request.limits.max_quality_visits,
                witness_limit: request.limits.witness_limit,
            },
        )?
    };
    quality.candidate_summaries = summaries;
    quality.candidate_agreement = candidate_agreement;
    quality.selection_reason = if quality.candidate_summaries.len() == 1 {
        "fixed resolution".to_owned()
    } else {
        "modularity plateau, size, conductance, fragmentation, partition digest".to_owned()
    };
    let base_labels = label_communities_by_hub(legacy, &communities);
    let signatures = community_member_signatures(&communities);
    let identity = CommunityIdentity {
        algorithm: QUALITY_CLUSTER_ALGORITHM.to_owned(),
        topology: QUALITY_CLUSTER_TOPOLOGY.to_owned(),
        quality: QUALITY_CLUSTER_QUALITY.to_owned(),
        selector: selector_identity.to_owned(),
        seed: COMPATIBILITY_CLUSTER_SEED,
        limits: QUALITY_CLUSTER_LIMITS.to_owned(),
    };
    Ok(CommunityResult {
        communities,
        base_labels,
        signatures,
        quality,
        execution,
        identity,
    })
}

fn full_candidate(
    topology: &super::topology::CommunityTopology,
    resolution: f64,
    base_resolution: f64,
    request: &CommunityRequest<'_>,
    identity: QualityIdentity<'_>,
) -> Result<(f64, Communities, PartitionQuality, String), CommunityError> {
    let communities = quality_partition(
        &topology.graph,
        resolution,
        request.exclude_hubs_percentile,
        request.limits.max_levels,
        request.limits.max_moves,
    )?
    .into_iter()
    .enumerate()
    .collect::<Communities>();
    let quality = evaluate_graph_partition(
        &topology.graph,
        &communities,
        base_resolution,
        QualityEvaluationOptions {
            identity,
            topology_evidence: Some(topology.evidence.clone()),
            pair_evidence: Some(&topology.pair_evidence),
            max_quality_visits: request.limits.max_quality_visits,
            witness_limit: request.limits.witness_limit,
        },
    )?;
    let digest = partition_digest(&communities);
    Ok((resolution, communities, quality, digest))
}

fn map_preparation_fallback(reason: IncrementalPreparationFallback) -> FallbackReason {
    match reason {
        IncrementalPreparationFallback::InvalidLimits => FallbackReason::InvalidIncrementalLimits,
        IncrementalPreparationFallback::RemovedNode => FallbackReason::RemovedNode,
        IncrementalPreparationFallback::AffectedLimit => FallbackReason::AffectedRegionLimit,
        IncrementalPreparationFallback::HubPolicy => FallbackReason::HubPolicy,
    }
}

fn quality_partition(
    graph: &crate::cluster::WeightedGraph,
    resolution: f64,
    exclude_hubs_percentile: Option<f64>,
    max_levels: usize,
    max_moves: usize,
) -> Result<Vec<Vec<String>>, CommunityDetectorError> {
    let hubs = excluded_hubs(graph, exclude_hubs_percentile);
    let isolates = (0..graph.len())
        .filter(|node| graph.degree_unweighted(*node) == 0 && !hubs.contains(node))
        .collect::<Vec<_>>();
    let selected = (0..graph.len())
        .filter(|node| graph.degree_unweighted(*node) > 0 && !hubs.contains(node))
        .collect::<Vec<_>>();
    let mut communities = if selected.len() == graph.len() {
        leiden(graph, resolution, max_levels, max_moves)?
    } else {
        let connected = graph.subgraph(&selected);
        leiden(&connected, resolution, max_levels, max_moves)?
    };
    communities.extend(
        isolates
            .into_iter()
            .map(|node| vec![graph.ids[node].clone()]),
    );
    reattach_hubs(graph, &hubs, &mut communities);
    for members in &mut communities {
        members.sort();
    }
    communities.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    Ok(communities)
}

fn candidate_agreement(
    candidates: &[(f64, Communities, PartitionQuality, String)],
) -> Vec<CandidateAgreement> {
    let mut agreement = Vec::new();
    for left in 0..candidates.len() {
        for right in left + 1..candidates.len() {
            agreement.push(CandidateAgreement {
                left_resolution: candidates[left].0,
                right_resolution: candidates[right].0,
                adjusted_rand_index: adjusted_rand_index(&candidates[left].1, &candidates[right].1),
                exact_membership: candidates[left].3 == candidates[right].3,
            });
        }
    }
    agreement
}

fn candidate_size_violations(quality: &PartitionQuality) -> usize {
    quality
        .communities
        .values()
        .filter(|community| {
            community.member_count >= 10
                && (community.member_count as f64 / quality.assigned_node_count.max(1) as f64)
                    > 0.25
        })
        .count()
}

fn partition_digest(communities: &Communities) -> String {
    let mut hasher = Sha256::new();
    for members in communities.values() {
        let mut members = members.clone();
        members.sort();
        for member in members {
            hasher.update(member.as_bytes());
            hasher.update([0]);
        }
        hasher.update([0xff]);
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::code_graph::{BuildMetadata, EdgeKind, EdgeRecord, NodeKind, NodeRecord};
    use compass_model::provenance::SourceAnchor;

    fn node(id: &str) -> NodeRecord {
        NodeRecord {
            id: id.to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: id.to_owned(),
            qualified_name: format!("crate::{id}"),
            language: Some("rust".to_owned()),
            framework: None,
            source: Some(SourceAnchor {
                file: format!("src/{id}.rs"),
                start_byte: 0,
                end_byte: 1,
                start_line: 1,
                start_column: 0,
                end_line: 1,
                end_column: 1,
            }),
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        }
    }

    fn typed_graph() -> compass_model::code_graph::GraphDocument {
        let mut document = compass_model::code_graph::GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "test".to_owned(),
            source_tree_digest: "test".to_owned(),
            configuration_digest: "test".to_owned(),
            generation_id: "test".to_owned(),
            source_commit: None,
        });
        document.nodes = ["a", "b", "c", "d"].into_iter().map(node).collect();
        document.links = [("a", "b"), ("b", "c"), ("c", "d")]
            .into_iter()
            .enumerate()
            .map(|(index, (source, target))| EdgeRecord {
                id: format!("edge-{index}"),
                key: format!("edge-{index}"),
                source: source.to_owned(),
                target: target.to_owned(),
                kind: EdgeKind::Calls,
                occurrence_rule: None,
                relationship_site: None,
                details: None,
                evidence: Vec::new(),
                weight: Some(1.0),
                context: None,
                deferred: false,
                diagnostics: Vec::new(),
            })
            .collect();
        document
    }

    fn planted_graph() -> compass_model::code_graph::GraphDocument {
        let mut document = compass_model::code_graph::GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "test".to_owned(),
            source_tree_digest: "test".to_owned(),
            configuration_digest: "test".to_owned(),
            generation_id: "test".to_owned(),
            source_commit: None,
        });
        document.nodes = ["a", "b", "c", "d", "w", "x", "y", "z"]
            .into_iter()
            .map(node)
            .collect();
        let pairs = [
            ("a", "b"),
            ("a", "c"),
            ("a", "d"),
            ("b", "c"),
            ("b", "d"),
            ("c", "d"),
            ("w", "x"),
            ("w", "y"),
            ("w", "z"),
            ("x", "y"),
            ("x", "z"),
            ("y", "z"),
            ("d", "w"),
        ];
        document.links = pairs
            .into_iter()
            .enumerate()
            .map(|(index, (source, target))| EdgeRecord {
                id: format!("edge-{index:02}"),
                key: format!("edge-{index:02}"),
                source: source.to_owned(),
                target: target.to_owned(),
                kind: EdgeKind::Calls,
                occurrence_rule: None,
                relationship_site: None,
                details: None,
                evidence: Vec::new(),
                weight: Some(1.0),
                context: None,
                deferred: false,
                diagnostics: Vec::new(),
            })
            .collect();
        document
    }

    fn previous_from(result: &CommunityResult) -> PreviousCommunities {
        result
            .communities
            .iter()
            .flat_map(|(community, members)| {
                members
                    .iter()
                    .map(move |member| (member.clone(), *community))
            })
            .collect()
    }

    fn canonical_memberships(communities: &Communities) -> Vec<Vec<String>> {
        let mut memberships = communities.values().cloned().collect::<Vec<_>>();
        for members in &mut memberships {
            members.sort();
        }
        memberships.sort();
        memberships
    }

    #[test]
    fn facade_preserves_compatibility_membership_and_adds_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let document = typed_graph();
        let legacy = document.to_legacy_document()?;
        let expected = cluster(&legacy, ClusterOptions::default());
        let changed_sources = BTreeSet::new();
        let result = build_communities(
            &document,
            &CommunityRequest {
                profile: CommunityProfile::CompatibilityV1,
                resolution: ResolutionPolicy::Fixed(1.0),
                exclude_hubs_percentile: None,
                previous: None,
                incremental: false,
                changed_sources: &changed_sources,
                limits: CommunityLimits::default(),
            },
        )?;

        assert_eq!(result.communities, expected);
        assert_eq!(result.execution, CommunityExecution::Full);
        assert_eq!(result.quality.assigned_node_count, document.nodes.len());
        assert_eq!(result.identity.algorithm, COMPATIBILITY_CLUSTER_ALGORITHM);
        assert_eq!(result.signatures.len(), result.communities.len());
        assert_eq!(result.base_labels.len(), result.communities.len());
        Ok(())
    }

    #[test]
    fn quality_profile_uses_typed_leiden_and_bounded_selection()
    -> Result<(), Box<dyn std::error::Error>> {
        let document = typed_graph();
        let changed_sources = BTreeSet::new();
        let result = build_communities(
            &document,
            &CommunityRequest {
                profile: CommunityProfile::QualityV1,
                resolution: ResolutionPolicy::Auto { base: 1.0 },
                exclude_hubs_percentile: None,
                previous: None,
                incremental: false,
                changed_sources: &changed_sources,
                limits: CommunityLimits::default(),
            },
        );

        let result = result?;
        assert_eq!(result.identity.algorithm, QUALITY_CLUSTER_ALGORITHM);
        assert_eq!(result.identity.topology, QUALITY_CLUSTER_TOPOLOGY);
        assert_eq!(result.identity.selector, QUALITY_CLUSTER_SELECTOR);
        assert_eq!(result.quality.candidate_summaries.len(), 3);
        assert_eq!(result.quality.candidate_agreement.len(), 3);
        assert_eq!(
            result
                .quality
                .candidate_summaries
                .iter()
                .filter(|candidate| candidate.selected)
                .count(),
            1
        );
        assert!(
            result
                .quality
                .candidate_summaries
                .iter()
                .filter(|candidate| !candidate.selected)
                .all(|candidate| candidate.rejection_reason.is_some())
        );
        assert_eq!(result.quality.disconnected_community_count, 0);
        assert!(result.quality.quality_visit_count > 0);
        assert!(
            result
                .quality
                .topology_evidence
                .as_ref()
                .is_some_and(|evidence| !evidence.retained_relationship_counts.is_empty())
        );
        Ok(())
    }

    #[test]
    fn quality_profile_fails_closed_at_the_quality_visit_limit() {
        let document = typed_graph();
        let changed_sources = BTreeSet::new();
        let limits = CommunityLimits {
            max_quality_visits: 0,
            ..CommunityLimits::default()
        };
        assert!(matches!(
            build_communities(
                &document,
                &CommunityRequest {
                    profile: CommunityProfile::QualityV1,
                    resolution: ResolutionPolicy::Fixed(1.0),
                    exclude_hubs_percentile: None,
                    previous: None,
                    incremental: false,
                    changed_sources: &changed_sources,
                    limits,
                },
            ),
            Err(CommunityError::Quality(
                CommunityQualityError::QualityLimitExceeded { limit: 0, .. }
            ))
        ));
    }

    #[test]
    fn quality_profile_rejects_invalid_limits_and_overflowed_auto_candidates() {
        let document = typed_graph();
        let changed_sources = BTreeSet::new();
        let limits = CommunityLimits {
            max_total_weight: f64::NAN,
            ..CommunityLimits::default()
        };
        assert!(matches!(
            build_communities(
                &document,
                &CommunityRequest {
                    profile: CommunityProfile::QualityV1,
                    resolution: ResolutionPolicy::Fixed(1.0),
                    exclude_hubs_percentile: None,
                    previous: None,
                    incremental: false,
                    changed_sources: &changed_sources,
                    limits,
                },
            ),
            Err(CommunityError::InvalidProfile { .. })
        ));
        assert!(matches!(
            build_communities(
                &document,
                &CommunityRequest {
                    profile: CommunityProfile::QualityV1,
                    resolution: ResolutionPolicy::Auto { base: f64::MAX },
                    exclude_hubs_percentile: None,
                    previous: None,
                    incremental: false,
                    changed_sources: &changed_sources,
                    limits: CommunityLimits::default(),
                },
            ),
            Err(CommunityError::InvalidResolution { .. })
        ));
    }

    #[test]
    fn quality_incremental_run_preserves_frozen_assignments()
    -> Result<(), Box<dyn std::error::Error>> {
        let document = typed_graph();
        let no_changes = BTreeSet::new();
        let initial = build_communities(
            &document,
            &CommunityRequest {
                profile: CommunityProfile::QualityV1,
                resolution: ResolutionPolicy::Fixed(1.0),
                exclude_hubs_percentile: None,
                previous: None,
                incremental: false,
                changed_sources: &no_changes,
                limits: CommunityLimits::default(),
            },
        )?;
        let previous = initial
            .communities
            .iter()
            .flat_map(|(community, members)| {
                members
                    .iter()
                    .map(move |member| (member.clone(), *community))
            })
            .collect::<PreviousCommunities>();
        let changed_sources = BTreeSet::from(["src/a.rs".to_owned()]);
        let mut limits = CommunityLimits::default();
        limits.incremental.max_affected_fraction = 1.0;
        let incremental = build_communities(
            &document,
            &CommunityRequest {
                profile: CommunityProfile::QualityV1,
                resolution: ResolutionPolicy::Fixed(1.0),
                exclude_hubs_percentile: None,
                previous: Some(&previous),
                incremental: true,
                changed_sources: &changed_sources,
                limits,
            },
        )?;

        assert!(matches!(
            incremental.execution,
            CommunityExecution::Incremental { affected_nodes } if affected_nodes > 0
        ));
        for node in ["c", "d"] {
            let retained = incremental
                .communities
                .iter()
                .find_map(|(community, members)| {
                    members.contains(&node.to_owned()).then_some(*community)
                });
            assert_eq!(retained, previous.get(node).copied());
        }
        assert_eq!(incremental.quality.disconnected_community_count, 0);
        Ok(())
    }

    #[test]
    fn quality_profile_recovers_planted_groups_and_is_permutation_invariant()
    -> Result<(), Box<dyn std::error::Error>> {
        let document = planted_graph();
        let changes = BTreeSet::new();
        let request = CommunityRequest {
            profile: CommunityProfile::QualityV1,
            resolution: ResolutionPolicy::Auto { base: 1.0 },
            exclude_hubs_percentile: None,
            previous: None,
            incremental: false,
            changed_sources: &changes,
            limits: CommunityLimits::default(),
        };
        let expected = build_communities(&document, &request)?;
        assert_eq!(
            expected.communities.values().cloned().collect::<Vec<_>>(),
            vec![
                vec![
                    "a".to_owned(),
                    "b".to_owned(),
                    "c".to_owned(),
                    "d".to_owned()
                ],
                vec![
                    "w".to_owned(),
                    "x".to_owned(),
                    "y".to_owned(),
                    "z".to_owned()
                ],
            ]
        );

        let mut permuted = document;
        permuted.nodes.reverse();
        permuted.links.reverse();
        let actual = build_communities(&permuted, &request)?;
        assert_eq!(actual.communities, expected.communities);
        assert_eq!(
            actual
                .quality
                .candidate_summaries
                .iter()
                .map(|candidate| (&candidate.partition_digest, candidate.selected))
                .collect::<Vec<_>>(),
            expected
                .quality
                .candidate_summaries
                .iter()
                .map(|candidate| (&candidate.partition_digest, candidate.selected))
                .collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn quality_incremental_edit_revert_rename_delete_sequence_is_stable()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = planted_graph();
        let no_changes = BTreeSet::new();
        let limits = CommunityLimits {
            incremental: IncrementalClusterLimits {
                max_affected_nodes: 4_096,
                max_affected_fraction: 1.0,
            },
            ..CommunityLimits::default()
        };
        let build = |document: &compass_model::code_graph::GraphDocument,
                     previous: Option<&PreviousCommunities>,
                     changed_sources: &BTreeSet<String>|
         -> Result<CommunityResult, CommunityError> {
            build_communities(
                document,
                &CommunityRequest {
                    profile: CommunityProfile::QualityV1,
                    resolution: ResolutionPolicy::Fixed(1.0),
                    exclude_hubs_percentile: None,
                    previous,
                    incremental: previous.is_some(),
                    changed_sources,
                    limits,
                },
            )
        };

        let initial = build(&original, None, &no_changes)?;
        let initial_memberships = canonical_memberships(&initial.communities);

        let mut edited_graph = original.clone();
        let edited_edge = edited_graph
            .links
            .iter_mut()
            .find(|edge| edge.source == "a" && edge.target == "b")
            .ok_or_else(|| std::io::Error::other("missing editable edge"))?;
        edited_edge.kind = EdgeKind::References;
        let edited_sources = BTreeSet::from(["src/a.rs".to_owned(), "src/b.rs".to_owned()]);
        let edited = build(
            &edited_graph,
            Some(&previous_from(&initial)),
            &edited_sources,
        )?;
        assert_eq!(edited.quality.disconnected_community_count, 0);

        let reverted = build(&original, Some(&previous_from(&edited)), &edited_sources)?;
        assert_eq!(
            canonical_memberships(&reverted.communities),
            initial_memberships
        );

        let mut renamed_graph = original.clone();
        let renamed_node = renamed_graph
            .nodes
            .iter_mut()
            .find(|node| node.id == "a")
            .ok_or_else(|| std::io::Error::other("missing rename node"))?;
        renamed_node.id = "a2".to_owned();
        renamed_node.name = "a2".to_owned();
        renamed_node.qualified_name = "crate::a2".to_owned();
        if let Some(source) = renamed_node.source.as_mut() {
            source.file = "src/a2.rs".to_owned();
        }
        for edge in &mut renamed_graph.links {
            if edge.source == "a" {
                edge.source = "a2".to_owned();
            }
            if edge.target == "a" {
                edge.target = "a2".to_owned();
            }
        }
        let renamed_sources = BTreeSet::from(["src/a.rs".to_owned(), "src/a2.rs".to_owned()]);
        let renamed = build(
            &renamed_graph,
            Some(&previous_from(&reverted)),
            &renamed_sources,
        )?;
        assert!(matches!(
            renamed.execution,
            CommunityExecution::FullFallback {
                reason: FallbackReason::RemovedNode,
                ..
            }
        ));
        assert_eq!(renamed.quality.disconnected_community_count, 0);

        let mut deleted_graph = renamed_graph;
        deleted_graph.nodes.retain(|node| node.id != "a2");
        deleted_graph
            .links
            .retain(|edge| edge.source != "a2" && edge.target != "a2");
        let deleted = build(
            &deleted_graph,
            Some(&previous_from(&renamed)),
            &BTreeSet::from(["src/a2.rs".to_owned()]),
        )?;
        assert!(matches!(
            deleted.execution,
            CommunityExecution::FullFallback {
                reason: FallbackReason::RemovedNode,
                ..
            }
        ));
        assert_eq!(
            deleted.quality.assigned_node_count,
            deleted_graph.nodes.len()
        );
        assert_eq!(deleted.quality.disconnected_community_count, 0);
        Ok(())
    }
}
