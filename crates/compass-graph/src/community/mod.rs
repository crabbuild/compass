//! Versioned community detection identities and quality evidence.

mod artifact;
mod build;
mod hierarchy;
mod identity;
mod incremental;
mod leiden;
mod quality;
mod topology;

pub use artifact::{
    COMMUNITY_QUALITY_SCHEMA, CommunityQualityArtifact, CommunityQualityArtifactError,
};
pub use build::{
    CommunityError, CommunityExecution, CommunityIdentity, CommunityLimits, CommunityProfile,
    CommunityRequest, CommunityResult, FallbackReason, PreviousCommunities, ResolutionPolicy,
    build_communities,
};
pub use hierarchy::{
    BOUNDARY_KINDS, COMMUNITY_HIERARCHY_BUDGET, COMMUNITY_HIERARCHY_SCHEMA, CommunityHierarchy,
    CommunityHierarchyArtifactError, CommunityHierarchyDraft, DEFAULT_LEVEL_TARGET,
    DEFAULT_MAX_LEVELS, DEFAULT_ROOT_TARGET, GroupQuality, HierarchyBudget, HierarchyGroup,
    HierarchyLabel, HierarchyLabelRule, HierarchyLevel, HierarchyRequest, LABEL_COVERAGE_THRESHOLD,
    MIN_LEVEL_RESOLUTION, boundary_kind_names, build_community_hierarchy,
};
pub use identity::{
    COMPATIBILITY_CLUSTER_ALGORITHM, COMPATIBILITY_CLUSTER_LIMITS, COMPATIBILITY_CLUSTER_QUALITY,
    COMPATIBILITY_CLUSTER_SEED, COMPATIBILITY_CLUSTER_SEED_TEXT, COMPATIBILITY_CLUSTER_SELECTOR,
    COMPATIBILITY_CLUSTER_TOPOLOGY, QUALITY_CLUSTER_ALGORITHM, QUALITY_CLUSTER_LIMITS,
    QUALITY_CLUSTER_QUALITY, QUALITY_CLUSTER_SELECTOR, QUALITY_CLUSTER_TOPOLOGY,
};
pub use leiden::CommunityDetectorError;
pub(crate) use quality::compatibility_density_scores;
pub use quality::{
    CandidateAgreement, CommunityCandidateSummary, CommunityQuality, CommunityQualityError,
    PartitionQuality, adjusted_mutual_information, adjusted_rand_index, evaluate_partition_quality,
};
pub use topology::{CommunityTopologyError, TopologyEvidence};
