use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::build::{CommunityIdentity, CommunityLimits};
use super::quality::PartitionQuality;

pub const COMMUNITY_QUALITY_SCHEMA: &str = "compass.community-quality/1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityQualityArtifact {
    pub schema: String,
    pub graph_generation: String,
    pub graph_digest: String,
    pub identity: CommunityIdentity,
    pub limits: CommunityLimits,
    pub partition: PartitionQuality,
    pub result_digest: String,
}

#[derive(Debug, Error)]
pub enum CommunityQualityArtifactError {
    #[error("unsupported community quality schema `{0}`")]
    UnsupportedSchema(String),
    #[error("community quality artifact is missing graph identity")]
    MissingGraphIdentity,
    #[error("community quality profile identity does not match partition evidence")]
    IdentityMismatch,
    #[error("community quality result digest mismatch")]
    DigestMismatch,
    #[error("community quality graph identity does not match the selected graph")]
    GraphIdentityMismatch,
    #[error("invalid community quality evidence: {0}")]
    InvalidEvidence(String),
    #[error("could not encode community quality evidence: {0}")]
    Encode(#[from] serde_json::Error),
}

impl CommunityQualityArtifact {
    pub fn new(
        graph_generation: String,
        graph_digest: String,
        identity: CommunityIdentity,
        limits: CommunityLimits,
        partition: PartitionQuality,
    ) -> Result<Self, CommunityQualityArtifactError> {
        let mut artifact = Self {
            schema: COMMUNITY_QUALITY_SCHEMA.to_owned(),
            graph_generation,
            graph_digest,
            identity,
            limits,
            partition,
            result_digest: String::new(),
        };
        artifact.result_digest = artifact.calculate_digest()?;
        artifact.validate()?;
        Ok(artifact)
    }

    pub fn validate(&self) -> Result<(), CommunityQualityArtifactError> {
        if self.schema != COMMUNITY_QUALITY_SCHEMA {
            return Err(CommunityQualityArtifactError::UnsupportedSchema(
                self.schema.clone(),
            ));
        }
        if self.graph_generation.is_empty() || !is_sha256_identity(&self.graph_digest) {
            return Err(CommunityQualityArtifactError::MissingGraphIdentity);
        }
        if self.identity.algorithm != self.partition.algorithm
            || self.identity.topology != self.partition.topology
            || self.identity.quality != self.partition.quality
            || self.identity.selector != self.partition.selector
            || self.identity.seed != self.partition.seed
            || self.identity.limits != self.partition.limits
        {
            return Err(CommunityQualityArtifactError::IdentityMismatch);
        }
        self.validate_evidence()?;
        if self.result_digest != self.calculate_digest()? {
            return Err(CommunityQualityArtifactError::DigestMismatch);
        }
        Ok(())
    }

    fn validate_evidence(&self) -> Result<(), CommunityQualityArtifactError> {
        let invalid =
            |message: &str| CommunityQualityArtifactError::InvalidEvidence(message.to_owned());
        if self.identity.algorithm.is_empty()
            || self.identity.topology.is_empty()
            || self.identity.quality.is_empty()
            || self.identity.selector.is_empty()
            || self.identity.limits.is_empty()
        {
            return Err(invalid("profile identity contains an empty field"));
        }
        if !self.limits.max_total_weight.is_finite() || self.limits.max_total_weight < 0.0 {
            return Err(invalid("maxTotalWeight must be finite and non-negative"));
        }
        if self.partition.quality_visit_limit != self.limits.max_quality_visits
            || self.partition.quality_visit_count > self.partition.quality_visit_limit
        {
            return Err(invalid("quality visit accounting does not match limits"));
        }
        if self.partition.witness_limit != self.limits.witness_limit {
            return Err(invalid("witness limit does not match profile limits"));
        }
        let candidates = &self.partition.candidate_summaries;
        if candidates.is_empty() || candidates.len() > self.limits.max_candidates {
            return Err(invalid("candidate count is outside profile limits"));
        }
        if candidates
            .iter()
            .filter(|candidate| candidate.selected)
            .count()
            != 1
        {
            return Err(invalid(
                "candidate evidence must select exactly one partition",
            ));
        }
        if !self.partition.resolution.is_finite() || self.partition.resolution <= 0.0 {
            return Err(invalid("partition resolution must be finite and positive"));
        }
        if candidates.iter().any(|candidate| {
            !candidate.resolution.is_finite()
                || candidate.resolution <= 0.0
                || !candidate.modularity.is_finite()
                || !candidate.weighted_mean_conductance.is_finite()
        }) {
            return Err(invalid(
                "candidate metrics must be finite with positive resolution",
            ));
        }
        if self.partition.communities.len() != self.partition.community_count
            || self
                .partition
                .communities
                .iter()
                .any(|(community, metric)| {
                    metric.community != *community
                        || metric.witness_node_ids.len() > self.limits.witness_limit
                        || metric.witness_edge_ids.len() > self.limits.witness_limit
                        || !metric.internal_weight.is_finite()
                        || !metric.boundary_weight.is_finite()
                        || !metric.volume.is_finite()
                        || !metric.density.is_finite()
                        || !metric.conductance.is_finite()
                        || !metric.modularity_contribution.is_finite()
                })
        {
            return Err(invalid("per-community evidence is inconsistent"));
        }
        let assigned = self
            .partition
            .communities
            .values()
            .map(|metric| metric.member_count)
            .fold(0usize, usize::saturating_add);
        let disconnected = self
            .partition
            .communities
            .values()
            .filter(|metric| !metric.isolate && metric.connected_component_count > 1)
            .count();
        let omitted_nodes = self
            .partition
            .communities
            .values()
            .map(|metric| metric.omitted_witness_node_count)
            .fold(0usize, usize::saturating_add);
        let omitted_edges = self
            .partition
            .communities
            .values()
            .map(|metric| metric.omitted_witness_edge_count)
            .fold(0usize, usize::saturating_add);
        if assigned != self.partition.assigned_node_count
            || disconnected != self.partition.disconnected_community_count
            || omitted_nodes != self.partition.omitted_witness_node_count
            || omitted_edges != self.partition.omitted_witness_edge_count
            || !self.partition.modularity.is_finite()
            || !self.partition.weighted_mean_conductance.is_finite()
            || !self.partition.worst_conductance.is_finite()
            || !self.partition.largest_community_fraction.is_finite()
        {
            return Err(invalid("partition aggregates are inconsistent"));
        }
        if let Some(topology) = &self.partition.topology_evidence
            && (topology.retained_occurrence_count > topology.input_edge_count
                || topology.projected_pair_count > self.limits.max_projected_pairs
                || !topology.input_weight_sum.is_finite()
                || !topology.retained_total_weight.is_finite()
                || topology.retained_total_weight > self.limits.max_total_weight)
        {
            return Err(invalid("topology evidence is inconsistent with limits"));
        }
        Ok(())
    }

    pub fn validate_for_graph(
        &self,
        generation: &str,
        graph_digest: &str,
    ) -> Result<(), CommunityQualityArtifactError> {
        self.validate()?;
        if self.graph_generation != generation || self.graph_digest != graph_digest {
            return Err(CommunityQualityArtifactError::GraphIdentityMismatch);
        }
        Ok(())
    }

    fn calculate_digest(&self) -> Result<String, serde_json::Error> {
        let mut canonical = self.clone();
        canonical.result_digest.clear();
        let bytes = serde_json::to_vec(&canonical)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

fn is_sha256_identity(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        COMPATIBILITY_CLUSTER_ALGORITHM, COMPATIBILITY_CLUSTER_LIMITS,
        COMPATIBILITY_CLUSTER_QUALITY, COMPATIBILITY_CLUSTER_SEED, COMPATIBILITY_CLUSTER_SELECTOR,
        COMPATIBILITY_CLUSTER_TOPOLOGY,
    };
    use std::collections::BTreeMap;

    fn identity() -> CommunityIdentity {
        CommunityIdentity {
            algorithm: COMPATIBILITY_CLUSTER_ALGORITHM.to_owned(),
            topology: COMPATIBILITY_CLUSTER_TOPOLOGY.to_owned(),
            quality: COMPATIBILITY_CLUSTER_QUALITY.to_owned(),
            selector: COMPATIBILITY_CLUSTER_SELECTOR.to_owned(),
            seed: COMPATIBILITY_CLUSTER_SEED,
            limits: COMPATIBILITY_CLUSTER_LIMITS.to_owned(),
        }
    }

    fn partition() -> PartitionQuality {
        PartitionQuality {
            assigned_node_count: 0,
            omitted_node_count: 0,
            community_count: 0,
            non_isolate_singleton_count: 0,
            disconnected_community_count: 0,
            modularity: 0.0,
            weighted_mean_conductance: 0.0,
            worst_conductance: 0.0,
            largest_community_fraction: 0.0,
            resolution: 1.0,
            algorithm: COMPATIBILITY_CLUSTER_ALGORITHM.to_owned(),
            topology: COMPATIBILITY_CLUSTER_TOPOLOGY.to_owned(),
            quality: COMPATIBILITY_CLUSTER_QUALITY.to_owned(),
            selector: COMPATIBILITY_CLUSTER_SELECTOR.to_owned(),
            seed: COMPATIBILITY_CLUSTER_SEED,
            limits: COMPATIBILITY_CLUSTER_LIMITS.to_owned(),
            quality_visit_count: 0,
            quality_visit_limit: super::super::quality::DEFAULT_MAX_QUALITY_VISITS,
            witness_limit: 8,
            omitted_witness_node_count: 0,
            omitted_witness_edge_count: 0,
            candidate_summaries: vec![super::super::quality::CommunityCandidateSummary {
                resolution: 1.0,
                modularity: 0.0,
                weighted_mean_conductance: 0.0,
                disconnected_community_count: 0,
                size_violation_count: 0,
                non_isolate_singleton_count: 0,
                partition_digest: "fixture".to_owned(),
                selected: true,
                rejection_reason: None,
            }],
            candidate_agreement: Vec::new(),
            selection_reason: "test".to_owned(),
            topology_evidence: None,
            communities: BTreeMap::new(),
        }
    }

    #[test]
    fn artifact_digest_detects_mutation() -> Result<(), CommunityQualityArtifactError> {
        let mut artifact = CommunityQualityArtifact::new(
            "generation".to_owned(),
            format!("sha256:{}", "0".repeat(64)),
            identity(),
            CommunityLimits::default(),
            partition(),
        )?;
        artifact.partition.selection_reason = "mutated".to_owned();
        assert!(matches!(
            artifact.validate(),
            Err(CommunityQualityArtifactError::DigestMismatch)
        ));
        Ok(())
    }

    #[test]
    fn artifact_rejects_unknown_major_and_wrong_graph() -> Result<(), CommunityQualityArtifactError>
    {
        let artifact = CommunityQualityArtifact::new(
            "generation".to_owned(),
            format!("sha256:{}", "0".repeat(64)),
            identity(),
            CommunityLimits::default(),
            partition(),
        )?;
        assert!(matches!(
            artifact.validate_for_graph("other", &format!("sha256:{}", "0".repeat(64))),
            Err(CommunityQualityArtifactError::GraphIdentityMismatch)
        ));
        let mut unknown = artifact;
        unknown.schema = "compass.community-quality/2".to_owned();
        assert!(matches!(
            unknown.validate(),
            Err(CommunityQualityArtifactError::UnsupportedSchema(_))
        ));
        Ok(())
    }

    #[test]
    fn artifact_rejects_unknown_fields() -> Result<(), Box<dyn std::error::Error>> {
        let artifact = CommunityQualityArtifact::new(
            "generation".to_owned(),
            format!("sha256:{}", "0".repeat(64)),
            identity(),
            CommunityLimits::default(),
            partition(),
        )?;
        let mut value = serde_json::to_value(artifact)?;
        value
            .as_object_mut()
            .ok_or_else(|| std::io::Error::other("artifact must be an object"))?
            .insert("futureField".to_owned(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<CommunityQualityArtifact>(value).is_err());
        Ok(())
    }
}
