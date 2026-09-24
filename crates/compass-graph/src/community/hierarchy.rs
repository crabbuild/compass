//! Budgeted community hierarchy: recursive grouping under an explicit level
//! budget, evidence-derived labels with provenance, and an exact coverage
//! accounting trail.
//!
//! Community detection publishes a flat partition whose size scales with the
//! repository, so a committed reader cannot scan it on a large codebase. This
//! module derives a bounded number of named, nested units from that partition
//! and the same typed topology projection clustering uses.
//!
//! Levels are defined over the level below, never by repeating node ids:
//! `levels[0]` is the coarsest level, the last level is the published partition
//! itself, and [`HierarchyGroup::child_indices`] index the next finer level.
//! Determinism is the contract: an identical document plus an identical budget
//! must produce a byte-identical artifact digest.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use compass_model::code_graph::{GraphDocument, NodeKind, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::build::{CommunityError, CommunityIdentity, CommunityLimits};
use super::leiden::leiden;
use super::topology::{TopologyLimits, from_typed_document};
use crate::cluster::{
    Communities, WeightedGraph, community_member_signatures, label_communities_by_hub,
};

/// Version of the published hierarchy artifact.
pub const COMMUNITY_HIERARCHY_SCHEMA: &str = "compass.community-hierarchy/1";

/// Identity of the budget rule that produced a hierarchy.
pub const COMMUNITY_HIERARCHY_BUDGET: &str = "community-hierarchy-budget/v1";

/// Identity of the merge rule that produced a hierarchy.
///
/// Relationship evidence decides a level first. When a repository has
/// communities that share no relationship at all, the remaining levels group
/// them by shared source location instead, and every such level records that
/// rule so a reader can tell the two apart.
pub const COMMUNITY_HIERARCHY_MERGE_POLICY: &str = "relationship-then-location-affinity/v1";

/// Identity of the rule that derives group signatures and durable ids.
pub const COMMUNITY_HIERARCHY_SIGNATURE_ALGORITHM: &str = "hierarchy-signature/v1";

/// Length of a group signature in hex characters.
const GROUP_SIGNATURE_LENGTH: usize = 16;

/// Root groups a hierarchy aims for.
pub const DEFAULT_ROOT_TARGET: usize = 24;

/// Groups a non-root level aims for.
pub const DEFAULT_LEVEL_TARGET: usize = 300;

/// Levels a hierarchy may publish, including the published partition.
pub const DEFAULT_MAX_LEVELS: usize = 4;

/// Lower bound of the deterministic coarsening schedule.
pub const MIN_LEVEL_RESOLUTION: f64 = 0.05;

/// Share of members a directory or module label rule must cover to win.
pub const LABEL_COVERAGE_THRESHOLD: f64 = 0.6;

/// Coarsening attempts per level, bounding the resolution schedule.
const COARSEN_ATTEMPT_LIMIT: usize = 16;

/// Share of a level a relationship pass must remove to be worth publishing.
/// A pass that merges 2% of a partition produces a level a reader cannot tell
/// from the one below it, and pays for that in artifact size on every level.
const MIN_LEVEL_REDUCTION: f64 = 0.9;

/// Deepest directory prefix a label may cite.
const MAX_LABEL_PREFIX_DEPTH: usize = 6;

/// Node kinds a committed reader navigates by. The set is recorded inside the
/// artifact so a reader can audit boundary accounting without reading code.
pub const BOUNDARY_KINDS: [NodeKind; 20] = [
    NodeKind::Route,
    NodeKind::Event,
    NodeKind::Message,
    NodeKind::Topic,
    NodeKind::Queue,
    NodeKind::Job,
    NodeKind::Resource,
    NodeKind::Schema,
    NodeKind::Query,
    NodeKind::Migration,
    NodeKind::ConfigKey,
    NodeKind::Database,
    NodeKind::DatabaseSchema,
    NodeKind::DatabaseTable,
    NodeKind::DatabaseView,
    NodeKind::DatabaseColumn,
    NodeKind::DatabaseIndex,
    NodeKind::DatabaseConstraint,
    NodeKind::DatabaseProcedure,
    NodeKind::DatabaseTrigger,
];

/// The bounded shape of a hierarchy: how many groups each level may hold and
/// how far coarsening may go.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HierarchyBudget {
    pub root_target: usize,
    pub level_target: usize,
    pub max_levels: usize,
    pub min_level_resolution: f64,
}

impl Default for HierarchyBudget {
    fn default() -> Self {
        Self {
            root_target: DEFAULT_ROOT_TARGET,
            level_target: DEFAULT_LEVEL_TARGET,
            max_levels: DEFAULT_MAX_LEVELS,
            min_level_resolution: MIN_LEVEL_RESOLUTION,
        }
    }
}

impl HierarchyBudget {
    fn validate(self) -> Result<Self, CommunityError> {
        if self.max_levels == 0 {
            return Err(CommunityError::InvalidHierarchyBudget {
                reason: "maxLevels must be at least 1",
            });
        }
        if self.root_target == 0 {
            return Err(CommunityError::InvalidHierarchyBudget {
                reason: "rootTarget must be at least 1",
            });
        }
        if self.level_target == 0 {
            return Err(CommunityError::InvalidHierarchyBudget {
                reason: "levelTarget must be at least 1",
            });
        }
        if !self.min_level_resolution.is_finite() || self.min_level_resolution <= 0.0 {
            return Err(CommunityError::InvalidHierarchyBudget {
                reason: "minLevelResolution must be finite and positive",
            });
        }
        Ok(self)
    }
}

/// The rule that produced a group label, in the order the builder tries them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HierarchyLabelRule {
    DominantDirectory,
    ModulePrefix,
    HubMember,
    CommunityId,
}

/// A group label plus the exact evidence that produced it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HierarchyLabel {
    pub text: String,
    pub rule: HierarchyLabelRule,
    pub generic: bool,
    pub evidence: BTreeMap<String, Value>,
}

impl HierarchyLabel {
    /// True when no evidence-derived rule named this group.
    #[must_use]
    pub const fn is_generic(&self) -> bool {
        self.generic
    }
}

/// Group evidence computed on the graph the group's level partitions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupQuality {
    pub cohesion: f64,
    pub conductance: f64,
    pub boundary_kinds: BTreeMap<String, usize>,
}

/// One group of one level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HierarchyGroup {
    pub index: usize,
    /// Durable identity: `h<level>-<signature>` over the member evidence.
    ///
    /// Reconciliation rewrites this to the previous build's id when the same
    /// group survives, so a level keeps its vocabulary across rebuilds. `index`
    /// stays presentation order.
    pub id: String,
    /// First 16 hex characters of the group's member-signature digest.
    pub signature: String,
    /// The published community this group is, on the finest level only.
    ///
    /// Community ids are not always dense: the incremental path remaps
    /// surviving communities and drops retired ones. Pairing each finest group
    /// with its published id keeps the hierarchy joinable to `graph.json`,
    /// community details, and labels without renumbering the partition.
    pub community: Option<usize>,
    pub label: HierarchyLabel,
    pub member_count: usize,
    /// Indices into the next finer level; empty on the published partition.
    pub child_indices: Vec<usize>,
    pub quality: GroupQuality,
}

/// One level of the hierarchy. Level `0` is the coarsest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HierarchyLevel {
    pub level: usize,
    /// Digest over the level's sorted group signatures.
    pub signature: String,
    /// Rule that grouped this level's children into its groups.
    pub merge: LevelMerge,
    /// Resolution that produced this level, for relationship levels only.
    pub resolution: Option<f64>,
    /// The counts that justify an affinity level, empty for relationship levels.
    pub merge_evidence: BTreeMap<String, Value>,
    pub group_count: usize,
    pub groups: Vec<HierarchyGroup>,
}

/// How a level's groups were derived from the level below.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LevelMerge {
    /// Groups merged on the projected relationship evidence clustering uses.
    Relationship,
    /// Groups merged on shared source location, after relationship evidence
    /// stopped reducing the level.
    LocationAffinity,
}

/// Everything a hierarchy knows before it is bound to a graph generation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityHierarchyDraft {
    pub identity: CommunityIdentity,
    pub limits: CommunityLimits,
    pub budget_identity: String,
    pub merge_policy: String,
    pub signature_algorithm: String,
    pub budget: HierarchyBudget,
    /// Exact boundary kind set used for group accounting, sorted.
    pub boundary_kinds: Vec<String>,
    pub finest_community_count: usize,
    /// Digest over the published partition's member signatures.
    pub finest_signature: String,
    pub budget_satisfied: bool,
    pub levels: Vec<HierarchyLevel>,
}

/// The level data a renderer or reader consumes, borrowed from either the
/// draft a build is still finishing or the published artifact.
#[derive(Clone, Copy, Debug)]
pub struct CommunityHierarchyLevels<'a> {
    pub budget_identity: &'a str,
    pub merge_policy: &'a str,
    pub budget: HierarchyBudget,
    pub boundary_kinds: &'a [String],
    pub finest_community_count: usize,
    pub finest_signature: &'a str,
    pub budget_satisfied: bool,
    pub levels: &'a [HierarchyLevel],
}

impl CommunityHierarchyDraft {
    #[must_use]
    pub fn levels_view(&self) -> CommunityHierarchyLevels<'_> {
        CommunityHierarchyLevels {
            budget_identity: &self.budget_identity,
            merge_policy: &self.merge_policy,
            budget: self.budget,
            boundary_kinds: &self.boundary_kinds,
            finest_community_count: self.finest_community_count,
            finest_signature: &self.finest_signature,
            budget_satisfied: self.budget_satisfied,
            levels: &self.levels,
        }
    }
}

impl CommunityHierarchy {
    #[must_use]
    pub fn levels_view(&self) -> CommunityHierarchyLevels<'_> {
        CommunityHierarchyLevels {
            budget_identity: &self.budget_identity,
            merge_policy: &self.merge_policy,
            budget: self.budget,
            boundary_kinds: &self.boundary_kinds,
            finest_community_count: self.finest_community_count,
            finest_signature: &self.finest_signature,
            budget_satisfied: self.budget_satisfied,
            levels: &self.levels,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// The published, versioned community hierarchy artifact.
pub struct CommunityHierarchy {
    pub schema: String,
    pub graph_generation: String,
    pub graph_digest: String,
    pub identity: CommunityIdentity,
    pub limits: CommunityLimits,
    pub budget_identity: String,
    pub merge_policy: String,
    pub signature_algorithm: String,
    pub budget: HierarchyBudget,
    pub boundary_kinds: Vec<String>,
    pub finest_community_count: usize,
    pub finest_signature: String,
    pub budget_satisfied: bool,
    pub levels: Vec<HierarchyLevel>,
    pub result_digest: String,
}

#[derive(Debug, Error)]
pub enum CommunityHierarchyArtifactError {
    #[error("unsupported community hierarchy schema `{0}`")]
    UnsupportedSchema(String),
    #[error("community hierarchy artifact is missing graph identity")]
    MissingGraphIdentity,
    #[error("community hierarchy profile identity does not match the published partition")]
    IdentityMismatch,
    #[error("community hierarchy result digest mismatch")]
    DigestMismatch,
    #[error("community hierarchy graph identity does not match the selected graph")]
    GraphIdentityMismatch,
    #[error("invalid community hierarchy evidence: {0}")]
    InvalidEvidence(String),
    #[error("could not encode community hierarchy evidence: {0}")]
    Encode(#[from] serde_json::Error),
}

impl CommunityHierarchy {
    pub fn new(
        graph_generation: String,
        graph_digest: String,
        draft: CommunityHierarchyDraft,
    ) -> Result<Self, CommunityHierarchyArtifactError> {
        let mut artifact = Self {
            schema: COMMUNITY_HIERARCHY_SCHEMA.to_owned(),
            graph_generation,
            graph_digest,
            identity: draft.identity,
            limits: draft.limits,
            budget_identity: draft.budget_identity,
            merge_policy: draft.merge_policy,
            signature_algorithm: draft.signature_algorithm,
            budget: draft.budget,
            boundary_kinds: draft.boundary_kinds,
            finest_community_count: draft.finest_community_count,
            finest_signature: draft.finest_signature,
            budget_satisfied: draft.budget_satisfied,
            levels: draft.levels,
            result_digest: String::new(),
        };
        artifact.result_digest = artifact.calculate_digest()?;
        artifact.validate()?;
        Ok(artifact)
    }

    /// The coarsest level, which a reader opens first.
    #[must_use]
    pub fn root(&self) -> Option<&HierarchyLevel> {
        self.levels.first()
    }

    /// The published community partition as one group per community.
    #[must_use]
    pub fn finest(&self) -> Option<&HierarchyLevel> {
        self.levels.last()
    }

    #[must_use]
    pub fn level(&self, level: usize) -> Option<&HierarchyLevel> {
        self.levels.get(level)
    }

    pub fn validate(&self) -> Result<(), CommunityHierarchyArtifactError> {
        let invalid =
            |message: &str| CommunityHierarchyArtifactError::InvalidEvidence(message.to_owned());
        if self.schema != COMMUNITY_HIERARCHY_SCHEMA {
            return Err(CommunityHierarchyArtifactError::UnsupportedSchema(
                self.schema.clone(),
            ));
        }
        if self.graph_generation.is_empty() || !is_sha256_identity(&self.graph_digest) {
            return Err(CommunityHierarchyArtifactError::MissingGraphIdentity);
        }
        if self.identity.algorithm.is_empty()
            || self.identity.topology.is_empty()
            || self.identity.quality.is_empty()
            || self.identity.selector.is_empty()
            || self.identity.limits.is_empty()
        {
            return Err(CommunityHierarchyArtifactError::IdentityMismatch);
        }
        if self.limits.max_nodes == 0
            || self.limits.max_edges == 0
            || self.limits.max_levels == 0
            || !self.limits.max_total_weight.is_finite()
            || self.limits.max_total_weight < 0.0
        {
            return Err(invalid("clustering limits are outside their domain"));
        }
        if self.budget_identity != COMMUNITY_HIERARCHY_BUDGET {
            return Err(invalid("unexpected budget identity"));
        }
        if self.merge_policy != COMMUNITY_HIERARCHY_MERGE_POLICY {
            return Err(invalid("unexpected merge policy"));
        }
        if self.signature_algorithm != COMMUNITY_HIERARCHY_SIGNATURE_ALGORITHM {
            return Err(invalid("unexpected signature algorithm"));
        }
        if self.budget.max_levels == 0
            || self.budget.root_target == 0
            || self.budget.level_target == 0
            || !self.budget.min_level_resolution.is_finite()
            || self.budget.min_level_resolution <= 0.0
        {
            return Err(invalid("budget fields are outside their domain"));
        }
        if self.boundary_kinds.is_empty() || !is_sorted_unique(&self.boundary_kinds) {
            return Err(invalid("boundary kinds must be a non-empty sorted set"));
        }
        if self.levels.is_empty() || self.levels.len() > self.budget.max_levels {
            return Err(invalid("level count is outside the published budget"));
        }
        if !is_sha256_identity(&self.finest_signature) {
            return Err(invalid("finest signature must be a sha256 identity"));
        }
        for (position, level) in self.levels.iter().enumerate() {
            if level.level != position {
                return Err(invalid("levels must be ordered coarsest first"));
            }
            if level.group_count != level.groups.len() || level.groups.is_empty() {
                return Err(invalid("level group count does not match its groups"));
            }
            match (level.merge, level.resolution) {
                (LevelMerge::Relationship, Some(resolution))
                    if resolution.is_finite() && resolution > 0.0 => {}
                (LevelMerge::Relationship, _) => {
                    return Err(invalid(
                        "relationship levels must record the resolution that produced them",
                    ));
                }
                (LevelMerge::LocationAffinity, None) if !level.merge_evidence.is_empty() => {}
                (LevelMerge::LocationAffinity, _) => {
                    return Err(invalid(
                        "affinity levels record their evidence and no resolution",
                    ));
                }
            }
            for (index, group) in level.groups.iter().enumerate() {
                if group.index != index {
                    return Err(invalid("group indices must be dense and ordered"));
                }
                if !is_group_signature(&group.signature) {
                    return Err(invalid("group signatures must be digests"));
                }
                // Reconciliation rewrites an id to the previous build's, so an
                // id has to be a well-formed identity of its level rather than
                // the digest of this build's membership.
                if !is_group_id(level.level, &group.id) {
                    return Err(invalid(
                        "group ids must name their level and carry a signature",
                    ));
                }
                if group.member_count == 0 {
                    return Err(invalid("groups must cover at least one member"));
                }
                if group.label.text.trim().is_empty() {
                    return Err(invalid("group labels must not be empty"));
                }
                if group.label.evidence.is_empty() {
                    return Err(invalid("group labels must carry provenance"));
                }
                if group.label.generic
                    != matches!(group.label.rule, HierarchyLabelRule::CommunityId)
                {
                    return Err(invalid("only community-id labels may be marked generic"));
                }
                if !group.quality.cohesion.is_finite()
                    || !(0.0..=1.0).contains(&group.quality.cohesion)
                    || !group.quality.conductance.is_finite()
                    || group.quality.conductance < 0.0
                {
                    return Err(invalid("group quality must be finite and bounded"));
                }
            }
            let mut ids = BTreeSet::new();
            if level
                .groups
                .iter()
                .any(|group| !ids.insert(group.id.as_str()))
            {
                return Err(invalid("group ids must be unique within a level"));
            }
            if level.signature
                != signature_digest(level.groups.iter().map(|group| group.signature.as_str()))
            {
                return Err(invalid("level signature must digest its groups"));
            }
            if position + 1 == self.levels.len()
                && level.groups.len() != self.finest_community_count
            {
                return Err(invalid(
                    "the finest level must publish one group per community",
                ));
            }
            if position + 1 == self.levels.len()
                && level
                    .groups
                    .iter()
                    .any(|group| !group.child_indices.is_empty())
            {
                return Err(invalid("the finest level has no finer level to index"));
            }
            if position + 1 == self.levels.len()
                && !level.groups.windows(2).all(|pair| {
                    let (Some(left), Some(right)) = (pair.first(), pair.get(1)) else {
                        return false;
                    };
                    matches!(
                        (left.community, right.community),
                        (Some(left), Some(right)) if left < right
                    )
                })
            {
                return Err(invalid(
                    "the finest level must pair every group with its published community",
                ));
            }
            if position + 1 < self.levels.len()
                && level.groups.iter().any(|group| group.community.is_some())
            {
                return Err(invalid(
                    "only the finest level belongs to a published community",
                ));
            }
        }
        for (position, level) in self.levels.iter().enumerate() {
            let Some(finer) = self.levels.get(position + 1) else {
                continue;
            };
            verify_level_coverage(position, level, finer)
                .map_err(|error| invalid(&error.to_string()))?;
        }
        if self.result_digest != self.calculate_digest()? {
            return Err(CommunityHierarchyArtifactError::DigestMismatch);
        }
        Ok(())
    }

    pub fn validate_for_graph(
        &self,
        generation: &str,
        graph_digest: &str,
    ) -> Result<(), CommunityHierarchyArtifactError> {
        self.validate()?;
        if self.graph_generation != generation || self.graph_digest != graph_digest {
            return Err(CommunityHierarchyArtifactError::GraphIdentityMismatch);
        }
        Ok(())
    }

    /// Recompute the artifact digest after reconciliation rewrote group ids.
    ///
    /// Reconciliation changes identity, never membership, so the tree still
    /// has to satisfy the same validation before the digest is republished.
    pub fn reseal(&mut self) -> Result<(), CommunityHierarchyArtifactError> {
        self.result_digest = self.calculate_digest()?;
        self.validate()
    }

    fn calculate_digest(&self) -> Result<String, serde_json::Error> {
        let mut canonical = self.clone();
        canonical.result_digest.clear();
        let bytes = serde_json::to_vec(&canonical)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

/// How much of a previous group's membership a successor must cover to inherit
/// its identity, and how close two candidates may be before the match is
/// reported as ambiguous instead of guessed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReconcilePolicy {
    pub keep_threshold: f64,
    pub ambiguity_margin: f64,
    pub max_events: usize,
}

impl Default for ReconcilePolicy {
    fn default() -> Self {
        Self {
            keep_threshold: 0.5,
            ambiguity_margin: 0.05,
            max_events: 256,
        }
    }
}

/// What happened to a group between two builds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HierarchyEventKind {
    Stable,
    Split,
    Merged,
    Appeared,
    Disappeared,
    /// Two candidates were close enough that choosing one would invent a fact.
    Ambiguous,
}

/// One bounded, evidence-carrying entry in a reconciliation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HierarchyEvent {
    pub kind: HierarchyEventKind,
    pub level: usize,
    /// Previous-build group ids involved, sorted.
    pub previous_ids: Vec<String>,
    /// New-build group ids involved, sorted.
    pub next_ids: Vec<String>,
    /// Best overlap ratio that justifies this entry.
    pub overlap: f64,
    /// Members involved on the new side, or the previous side for a disappearance.
    pub member_count: usize,
}

/// The counts and the bounded event list for one reconciliation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HierarchyReconciliation {
    pub policy: ReconcilePolicy,
    pub matched: usize,
    pub stable: usize,
    pub split: usize,
    pub merged: usize,
    pub appeared: usize,
    pub disappeared: usize,
    pub ambiguous: usize,
    pub events: Vec<HierarchyEvent>,
    /// Events beyond the policy bound, counted exactly rather than dropped.
    pub omitted_events: usize,
}

impl HierarchyReconciliation {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matched == 0
            && self.split == 0
            && self.merged == 0
            && self.appeared == 0
            && self.disappeared == 0
            && self.ambiguous == 0
    }
}

/// Everything the builder needs beyond the published partition itself.
pub struct HierarchyRequest<'a> {
    pub identity: &'a CommunityIdentity,
    pub limits: &'a CommunityLimits,
    /// Resolution that produced the published partition; coarsening halves it.
    pub resolution: f64,
    pub budget: HierarchyBudget,
}

/// Build the hierarchy over an already published community partition.
///
/// The published partition is never recomputed: group `i` of the finest level
/// is community `i`, and `child_indices` alone describe every coarser level.
pub fn build_community_hierarchy(
    document: &GraphDocument,
    communities: &Communities,
    request: &HierarchyRequest<'_>,
) -> Result<CommunityHierarchyDraft, CommunityError> {
    let budget = request.budget.validate()?;
    if !request.resolution.is_finite() || request.resolution <= 0.0 {
        return Err(CommunityError::InvalidResolution {
            resolution: request.resolution,
        });
    }
    if communities.is_empty() {
        return Err(CommunityError::InvalidHierarchyBudget {
            reason: "the published partition is empty",
        });
    }

    let topology = from_typed_document(
        document,
        TopologyLimits {
            max_nodes: request.limits.max_nodes,
            max_edges: request.limits.max_edges,
            max_projected_pairs: request.limits.max_projected_pairs,
            max_total_weight: request.limits.max_total_weight,
        },
    )?;
    let legacy = document.to_legacy_document()?;
    let nodes = document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let graph = topology.graph;
    let assignments = community_assignments(&graph, communities);
    let boundary_by_node = graph
        .ids
        .iter()
        .map(|id| node_boundary_kinds(nodes.get(id.as_str()).copied()))
        .collect::<Vec<_>>();
    let community_signatures = community_member_signatures(communities);

    let context = BuildContext {
        communities,
        community_signatures: &community_signatures,
        legacy: &legacy,
        nodes: &nodes,
    };
    let mut levels = vec![finest_level(
        &graph,
        &assignments,
        &context,
        &boundary_by_node,
        request.resolution,
    )];
    while levels.len() < budget.max_levels {
        let current = levels.last().ok_or(CommunityError::IncompleteHierarchy {
            level: levels.len(),
            missing: 1,
        })?;
        if current.groups.len() <= budget.root_target {
            break;
        }
        let level_index = levels.len();
        let group_graph =
            aggregate_graph(&current.graph, &current.assignments, current.groups.len());
        let boundary_by_group = current
            .groups
            .iter()
            .map(|group| group.quality.boundary_kinds.clone())
            .collect::<Vec<_>>();
        let remaining = budget.max_levels - levels.len();
        // Once a level fits the per-level budget, only the root budget is left
        // to chase, so the next level coarsens toward it.
        let target = if remaining <= 1 || current.groups.len() <= budget.level_target {
            budget.root_target
        } else {
            budget.level_target
        };
        let previous_resolution = current.resolution.unwrap_or(budget.min_level_resolution);
        let mut plan = coarsen(
            &group_graph,
            previous_resolution,
            target,
            level_index,
            request.limits,
            budget.min_level_resolution,
        )?;
        let reduced = plan.groups.len() < current.groups.len();
        let worth_publishing = plan.groups.len() <= target
            || (plan.groups.len() as f64) <= (current.groups.len() as f64) * MIN_LEVEL_REDUCTION;
        if !reduced || !worth_publishing {
            // Relationship evidence stopped reducing the level. Group the rest
            // by the source location they already cite, and record that rule.
            let keys = current
                .groups
                .iter()
                .map(|group| location_key(&group_member_ids(group, communities), &nodes))
                .collect::<Vec<_>>();
            let affinity = affinity_plan(&current.groups, &keys, target, level_index);
            if affinity.groups.len() >= current.groups.len() {
                break;
            }
            plan = affinity;
        }
        let metrics = aggregate_metrics(
            &group_graph,
            &plan.assignments,
            plan.groups.len(),
            &boundary_by_group,
        );
        let next = build_level(&group_graph, &plan, &metrics, current, &context)?;
        // Communities with no relationship evidence cannot be merged, so a
        // level that does not reduce the group count is the plateau: stop here
        // and record the achieved count rather than repeating one partition on
        // every remaining level.
        if next.groups.len() >= current.groups.len() {
            break;
        }
        levels.push(next);
    }

    // `levels` accumulates finest first, so the coarsest level is the last one.
    // The cut never merges a group it cannot name, so meeting the root budget
    // is the whole condition: groups with no location evidence stay singletons
    // and simply keep the root larger than the target.
    let root_fits = levels
        .last()
        .is_some_and(|level| level.groups.len() <= budget.root_target);
    let budget_satisfied = root_fits;
    let mut ordered = levels
        .into_iter()
        .rev()
        .enumerate()
        .map(|(level, state)| HierarchyLevel {
            level,
            signature: level_signature(&state.groups),
            merge: state.merge,
            resolution: state.resolution,
            merge_evidence: state.merge_evidence,
            group_count: state.groups.len(),
            groups: state
                .groups
                .iter()
                .enumerate()
                .map(|(index, group)| HierarchyGroup {
                    index,
                    id: group_id(level, &group.signature),
                    signature: group.signature.clone(),
                    community: group.community,
                    label: group.label.clone(),
                    member_count: group.member_count,
                    child_indices: group.children.clone(),
                    quality: group.quality.clone(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    for position in 0..ordered.len().saturating_sub(1) {
        if let (Some(coarser), Some(finer)) = (ordered.get(position), ordered.get(position + 1)) {
            verify_level_coverage(position, coarser, finer)?;
        }
    }
    let finest_community_count = communities.len();
    Ok(CommunityHierarchyDraft {
        identity: request.identity.clone(),
        limits: *request.limits,
        budget_identity: COMMUNITY_HIERARCHY_BUDGET.to_owned(),
        merge_policy: COMMUNITY_HIERARCHY_MERGE_POLICY.to_owned(),
        signature_algorithm: COMMUNITY_HIERARCHY_SIGNATURE_ALGORITHM.to_owned(),
        budget,
        boundary_kinds: boundary_kind_names(),
        finest_community_count,
        finest_signature: finest_signature(communities),
        budget_satisfied,
        levels: std::mem::take(&mut ordered),
    })
}

/// A level while it is being derived, before the hierarchy is ordered.
/// The member nodes of every group, per level.
///
/// The artifact deliberately stores no node ids, so the published partition is
/// supplied alongside it. Comparing raw node ids is the only way to see that a
/// community split: the successor communities are new communities whose
/// signatures have nothing in common with their predecessor's. Levels are
/// coarsest first, matching the artifact's order.
fn level_community_sets(
    hierarchy: &CommunityHierarchy,
    communities: &Communities,
) -> Vec<Vec<BTreeSet<String>>> {
    let mut levels = vec![Vec::new(); hierarchy.levels.len()];
    let Some(finest) = hierarchy.levels.last() else {
        return levels;
    };
    if let Some(slot) = levels.last_mut() {
        *slot = finest
            .groups
            .iter()
            .map(|group| {
                group
                    .community
                    .and_then(|community| communities.get(&community))
                    .map(|members| members.iter().cloned().collect::<BTreeSet<_>>())
                    .unwrap_or_default()
            })
            .collect();
    }
    for position in (0..hierarchy.levels.len().saturating_sub(1)).rev() {
        let Some(finer) = levels.get(position + 1).cloned() else {
            continue;
        };
        let Some(level) = hierarchy.levels.get(position) else {
            continue;
        };
        let sets = level
            .groups
            .iter()
            .map(|group| {
                let mut members = BTreeSet::new();
                for child in &group.child_indices {
                    if let Some(child_members) = finer.get(*child) {
                        members.extend(child_members.iter().cloned());
                    }
                }
                members
            })
            .collect::<Vec<_>>();
        if let Some(slot) = levels.get_mut(position) {
            *slot = sets;
        }
    }
    levels
}

/// Which group of the level above contains each group of a level.
fn level_parents(hierarchy: &CommunityHierarchy) -> Vec<Vec<Option<usize>>> {
    let mut parents = vec![Vec::new(); hierarchy.levels.len()];
    for position in 0..hierarchy.levels.len().saturating_sub(1) {
        let (Some(coarser), Some(finer)) = (
            hierarchy.levels.get(position),
            hierarchy.levels.get(position + 1),
        ) else {
            continue;
        };
        let mut map = vec![None; finer.groups.len()];
        for group in &coarser.groups {
            for child in &group.child_indices {
                if let Some(slot) = map.get_mut(*child) {
                    *slot = Some(group.index);
                }
            }
        }
        if let Some(slot) = parents.get_mut(position + 1) {
            *slot = map;
        }
    }
    parents
}

fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    let union = left.union(right).count();
    if union == 0 {
        return 0.0;
    }
    left.intersection(right).count() as f64 / union as f64
}

/// Every previous group with the new groups it overlaps enough to be related to,
/// strongest first.
fn overlap_matrix(
    previous: &[BTreeSet<String>],
    next: &[BTreeSet<String>],
    threshold: f64,
) -> Vec<Vec<(f64, usize)>> {
    previous
        .iter()
        .map(|previous_members| {
            let mut row = next
                .iter()
                .enumerate()
                .map(|(index, next_members)| (jaccard(previous_members, next_members), index))
                .filter(|(overlap, _)| *overlap >= threshold)
                .collect::<Vec<_>>();
            row.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.1.cmp(&right.1))
            });
            row
        })
        .collect()
}

/// Reconcile a freshly built hierarchy against the previous one.
///
/// Identity only: a group that survives keeps the previous build's id so a
/// reader's vocabulary outlives the rebuild, and the event list says exactly
/// what split, merged, appeared, or disappeared. Membership is never changed,
/// no label or evidence block is copied from the previous build, and a group
/// with two equally good successors is reported as `ambiguous` rather than
/// resolved by picking one.
pub fn reconcile_hierarchy(
    previous: &CommunityHierarchy,
    previous_communities: &Communities,
    next: &mut CommunityHierarchy,
    next_communities: &Communities,
    policy: &ReconcilePolicy,
) -> Result<HierarchyReconciliation, CommunityHierarchyArtifactError> {
    if !policy.keep_threshold.is_finite()
        || !(0.0..=1.0).contains(&policy.keep_threshold)
        || !policy.ambiguity_margin.is_finite()
        || policy.ambiguity_margin < 0.0
        || policy.max_events == 0
    {
        return Err(CommunityHierarchyArtifactError::InvalidEvidence(
            "reconcile policy is outside its domain".to_owned(),
        ));
    }
    let previous_sets = level_community_sets(previous, previous_communities);
    let next_sets = level_community_sets(next, next_communities);
    let previous_parents = level_parents(previous);
    let next_parents = level_parents(next);
    let mut events = Vec::new();
    let mut matched = 0usize;
    let mut ambiguous = 0usize;
    let mut parent_matches = Vec::<HashMap<usize, usize>>::new();
    for position in 0..next.levels.len().min(previous.levels.len()) {
        // Identity facts, owned up front: inheriting an id mutates the level
        // this loop also reads.
        let Some(previous_level_number) = previous.levels.get(position).map(|level| level.level)
        else {
            continue;
        };
        let Some(next_level_number) = next.levels.get(position).map(|level| level.level) else {
            continue;
        };
        let previous_ids = previous.levels[position]
            .groups
            .iter()
            .map(|group| group.id.clone())
            .collect::<Vec<_>>();
        let previous_members = previous.levels[position]
            .groups
            .iter()
            .map(|group| group.member_count)
            .collect::<Vec<_>>();
        let next_ids = next.levels[position]
            .groups
            .iter()
            .map(|group| group.id.clone())
            .collect::<Vec<_>>();
        let next_members = next.levels[position]
            .groups
            .iter()
            .map(|group| group.member_count)
            .collect::<Vec<_>>();
        let (Some(previous_sets), Some(next_sets)) =
            (previous_sets.get(position), next_sets.get(position))
        else {
            continue;
        };
        let rows = overlap_matrix(previous_sets, next_sets, policy.keep_threshold);
        let mut predecessors = vec![Vec::<usize>::new(); next_sets.len()];
        for (previous_index, row) in rows.iter().enumerate() {
            for (_, next_index) in row {
                if let Some(slot) = predecessors.get_mut(*next_index) {
                    slot.push(previous_index);
                }
            }
        }
        let parent_match = parent_matches.last().cloned().unwrap_or_default();
        let previous_parent_of = |index: usize| -> Option<usize> {
            previous_parents
                .get(position)
                .and_then(|parents| parents.get(index).copied().flatten())
        };
        let next_parent_of = |index: usize| -> Option<usize> {
            next_parents
                .get(position)
                .and_then(|parents| parents.get(index).copied().flatten())
        };
        let mut matched_here = HashMap::<usize, usize>::new();
        for (previous_index, row) in rows.iter().enumerate() {
            let Some(previous_id) = previous_ids.get(previous_index) else {
                continue;
            };
            match row.as_slice() {
                [] => {}
                [(overlap, next_index)] => {
                    // Two predecessors for one successor is a merge, not a
                    // rename, so only an unshared successor inherits an id.
                    if predecessors.get(*next_index).map_or(0, Vec::len) > 1 {
                        continue;
                    }
                    let Some(next_id) = next_ids.get(*next_index) else {
                        continue;
                    };
                    if position > 0
                        && let (Some(previous_parent), Some(next_parent)) = (
                            previous_parent_of(previous_index),
                            next_parent_of(*next_index),
                        )
                        && parent_match.get(&previous_parent) != Some(&next_parent)
                    {
                        continue;
                    }
                    if let Some(group) = next
                        .levels
                        .get_mut(position)
                        .and_then(|level| level.groups.get_mut(*next_index))
                    {
                        group.id = previous_id.clone();
                    }
                    matched_here.insert(previous_index, *next_index);
                    matched += 1;
                    events.push(HierarchyEvent {
                        kind: HierarchyEventKind::Stable,
                        level: next_level_number,
                        previous_ids: vec![previous_id.clone()],
                        next_ids: vec![next_id.clone()],
                        overlap: *overlap,
                        member_count: next_members.get(*next_index).copied().unwrap_or_default(),
                    });
                }
                [(best, best_index), (runner_up, _), ..] => {
                    // Two successors this close mean the evidence does not name
                    // a single heir, so no id is inherited.
                    if best - runner_up <= policy.ambiguity_margin {
                        ambiguous += 1;
                        events.push(HierarchyEvent {
                            kind: HierarchyEventKind::Ambiguous,
                            level: next_level_number,
                            previous_ids: vec![previous_id.clone()],
                            next_ids: row
                                .iter()
                                .filter_map(|(_, next_index)| next_ids.get(*next_index).cloned())
                                .collect(),
                            overlap: *best,
                            member_count: next_members
                                .get(*best_index)
                                .copied()
                                .unwrap_or_default(),
                        });
                    }
                }
            }
        }
        for (previous_index, row) in rows.iter().enumerate() {
            if row.len() < 2 {
                continue;
            }
            let Some(previous_id) = previous_ids.get(previous_index) else {
                continue;
            };
            events.push(HierarchyEvent {
                kind: HierarchyEventKind::Split,
                level: next_level_number,
                previous_ids: vec![previous_id.clone()],
                next_ids: row
                    .iter()
                    .filter_map(|(_, next_index)| next_ids.get(*next_index).cloned())
                    .collect(),
                overlap: row.first().map_or(0.0, |(overlap, _)| *overlap),
                member_count: row
                    .iter()
                    .map(|(_, next_index)| {
                        next_members.get(*next_index).copied().unwrap_or_default()
                    })
                    .sum(),
            });
        }
        for (next_index, previous_indices) in predecessors.iter().enumerate() {
            if previous_indices.len() < 2 {
                continue;
            }
            let Some(next_id) = next_ids.get(next_index) else {
                continue;
            };
            events.push(HierarchyEvent {
                kind: HierarchyEventKind::Merged,
                level: next_level_number,
                previous_ids: previous_indices
                    .iter()
                    .filter_map(|index| previous_ids.get(*index).cloned())
                    .collect(),
                next_ids: vec![next_id.clone()],
                overlap: previous_indices
                    .iter()
                    .filter_map(|index| {
                        rows.get(*index).and_then(|row| {
                            row.iter()
                                .find(|(_, candidate)| *candidate == next_index)
                                .map(|(overlap, _)| *overlap)
                        })
                    })
                    .fold(0.0_f64, f64::max),
                member_count: next_members.get(next_index).copied().unwrap_or_default(),
            });
        }
        for (previous_index, previous_id) in previous_ids.iter().enumerate() {
            let related = rows.get(previous_index).is_some_and(|row| !row.is_empty());
            if !related && !matched_here.contains_key(&previous_index) {
                events.push(HierarchyEvent {
                    kind: HierarchyEventKind::Disappeared,
                    level: previous_level_number,
                    previous_ids: vec![previous_id.clone()],
                    next_ids: Vec::new(),
                    overlap: 0.0,
                    member_count: previous_members
                        .get(previous_index)
                        .copied()
                        .unwrap_or_default(),
                });
            }
        }
        let claimed = matched_here.values().copied().collect::<BTreeSet<_>>();
        for (next_index, next_id) in next_ids.iter().enumerate() {
            if claimed.contains(&next_index) || !predecessors[next_index].is_empty() {
                continue;
            }
            events.push(HierarchyEvent {
                kind: HierarchyEventKind::Appeared,
                level: next_level_number,
                previous_ids: Vec::new(),
                next_ids: vec![next_id.clone()],
                overlap: 0.0,
                member_count: next_members.get(next_index).copied().unwrap_or_default(),
            });
        }
        parent_matches.push(matched_here);
    }
    events.sort_by(|left, right| {
        left.level
            .cmp(&right.level)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.previous_ids.cmp(&right.previous_ids))
            .then_with(|| left.next_ids.cmp(&right.next_ids))
    });
    let mut counts = BTreeMap::<HierarchyEventKind, usize>::new();
    for event in &events {
        *counts.entry(event.kind).or_default() += 1;
    }
    let omitted_events = events.len().saturating_sub(policy.max_events);
    events.truncate(policy.max_events);
    next.reseal()?;
    Ok(HierarchyReconciliation {
        policy: *policy,
        matched,
        stable: counts
            .get(&HierarchyEventKind::Stable)
            .copied()
            .unwrap_or_default(),
        split: counts
            .get(&HierarchyEventKind::Split)
            .copied()
            .unwrap_or_default(),
        merged: counts
            .get(&HierarchyEventKind::Merged)
            .copied()
            .unwrap_or_default(),
        appeared: counts
            .get(&HierarchyEventKind::Appeared)
            .copied()
            .unwrap_or_default(),
        disappeared: counts
            .get(&HierarchyEventKind::Disappeared)
            .copied()
            .unwrap_or_default(),
        ambiguous,
        events,
        omitted_events,
    })
}

struct LevelState {
    /// The graph this level partitions: the typed projection for the finest
    /// level, and the previous level's group graph above it.
    graph: WeightedGraph,
    /// Group index per graph node, in graph node order.
    assignments: Vec<Option<usize>>,
    groups: Vec<GroupState>,
    merge: LevelMerge,
    resolution: Option<f64>,
    merge_evidence: BTreeMap<String, Value>,
}

#[derive(Clone)]
struct GroupState {
    /// Leaf community ids this group contains, ascending.
    communities: Vec<usize>,
    /// Digest over this group's member-community signatures.
    signature: String,
    member_count: usize,
    /// Indices into the next finer level's groups, ascending.
    children: Vec<usize>,
    /// The published community this group is, on the finest level only.
    community: Option<usize>,
    smallest_member: String,
    quality: GroupQuality,
    label: HierarchyLabel,
}

struct LevelPlan {
    merge: LevelMerge,
    resolution: Option<f64>,
    /// Counts that justify an affinity level.
    evidence: BTreeMap<String, Value>,
    /// Child group indices per new group, ascending.
    groups: Vec<Vec<usize>>,
    /// New group index per graph node.
    assignments: Vec<Option<usize>>,
}

struct Metrics {
    cohesion: f64,
    conductance: f64,
    boundary_kinds: BTreeMap<String, usize>,
}

/// The published partition and the document it came from, shared by every
/// level the builder derives.
struct BuildContext<'a> {
    communities: &'a Communities,
    community_signatures: &'a BTreeMap<usize, String>,
    legacy: &'a compass_model::GraphDocument,
    nodes: &'a HashMap<&'a str, &'a NodeRecord>,
}

fn finest_level(
    graph: &WeightedGraph,
    assignments: &[Option<usize>],
    context: &BuildContext<'_>,
    boundary_by_node: &[BTreeMap<String, usize>],
    resolution: f64,
) -> LevelState {
    let BuildContext {
        communities,
        community_signatures,
        legacy,
        nodes,
    } = *context;
    let metrics = aggregate_metrics(graph, assignments, communities.len(), boundary_by_node);
    let groups = communities
        .iter()
        .enumerate()
        .map(|(index, (community, members))| {
            let mut members = members.clone();
            members.sort();
            let quality = metrics.get(index);
            GroupState {
                communities: vec![*community],
                // The finest level *is* the published partition, so its
                // signature is the community's own member signature: the same
                // community keeps that signature even when its numeric id
                // moves, which is what two builds can compare.
                signature: community_signatures
                    .get(community)
                    .cloned()
                    .unwrap_or_else(|| group_signature(&[*community], community_signatures)),
                member_count: members.len(),
                children: Vec::new(),
                community: Some(*community),
                smallest_member: members.first().cloned().unwrap_or_default(),
                quality: GroupQuality {
                    cohesion: quality.map_or(1.0, |metric| metric.cohesion),
                    conductance: quality.map_or(0.0, |metric| metric.conductance),
                    boundary_kinds: quality
                        .map(|metric| metric.boundary_kinds.clone())
                        .unwrap_or_default(),
                },
                label: derive_label(legacy, nodes, &members, *community),
            }
        })
        .collect::<Vec<_>>();
    LevelState {
        graph: graph.clone(),
        assignments: assignments.to_vec(),
        groups,
        merge: LevelMerge::Relationship,
        resolution: Some(resolution),
        merge_evidence: BTreeMap::new(),
    }
}

/// Coarsen one level over the finer level's group graph.
fn build_level(
    group_graph: &WeightedGraph,
    coarsening: &LevelPlan,
    metrics: &[Metrics],
    finer: &LevelState,
    context: &BuildContext<'_>,
) -> Result<LevelState, CommunityError> {
    let BuildContext {
        communities,
        community_signatures,
        legacy,
        nodes,
    } = *context;
    let mut groups = Vec::with_capacity(coarsening.groups.len());
    for (raw, children) in coarsening.groups.iter().enumerate() {
        let mut members = Vec::new();
        let mut member_count = 0usize;
        let mut leaf_communities = Vec::new();
        let mut boundary_kinds = BTreeMap::new();
        let mut smallest_member: Option<String> = None;
        for child in children {
            let Some(child_group) = finer.groups.get(*child) else {
                return Err(CommunityError::IncompleteHierarchy {
                    level: finer.groups.len(),
                    missing: 1,
                });
            };
            member_count = member_count.saturating_add(child_group.member_count);
            leaf_communities.extend(child_group.communities.iter().copied());
            merge_counts(&mut boundary_kinds, &child_group.quality.boundary_kinds);
            members.extend(group_member_ids(child_group, communities));
            keep_smallest(&mut smallest_member, &child_group.smallest_member);
        }
        members.sort();
        leaf_communities.sort_unstable();
        let signature = group_signature(&leaf_communities, community_signatures);
        let quality = metrics.get(raw);
        groups.push(GroupState {
            communities: leaf_communities,
            signature,
            member_count,
            children: children.clone(),
            community: None,
            smallest_member: smallest_member.unwrap_or_default(),
            quality: GroupQuality {
                cohesion: quality.map_or(1.0, |metric| metric.cohesion),
                conductance: quality.map_or(0.0, |metric| metric.conductance),
                boundary_kinds,
            },
            label: derive_label(legacy, nodes, &members, raw),
        });
    }
    Ok(finalise_level(group_graph, groups, coarsening))
}

/// Order a level's groups so identical input always produces identical indices.
fn finalise_level(
    group_graph: &WeightedGraph,
    groups: Vec<GroupState>,
    coarsening: &LevelPlan,
) -> LevelState {
    let mut order = (0..groups.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        let (Some(left_group), Some(right_group)) = (groups.get(*left), groups.get(*right)) else {
            return left.cmp(right);
        };
        right_group
            .member_count
            .cmp(&left_group.member_count)
            .then_with(|| left_group.label.text.cmp(&right_group.label.text))
            .then_with(|| left_group.smallest_member.cmp(&right_group.smallest_member))
            .then_with(|| left.cmp(right))
    });
    let position_of = order
        .iter()
        .enumerate()
        .map(|(position, raw)| (*raw, position))
        .collect::<HashMap<_, _>>();
    let mut ordered = order
        .iter()
        .filter_map(|raw| groups.get(*raw).cloned())
        .collect::<Vec<_>>();
    // A generic label has no evidence of its own, so it names the group by the
    // index the group just received rather than the coarsening's raw index.
    for (index, group) in ordered.iter_mut().enumerate() {
        if group.label.generic {
            group.label = community_id_label(index, group.member_count);
        }
    }
    // Children already index the finer level's final ordering, so only this
    // level's own indices move.
    let assignments = coarsening
        .assignments
        .iter()
        .map(|assignment| assignment.and_then(|raw| position_of.get(&raw).copied()))
        .collect::<Vec<_>>();
    LevelState {
        graph: group_graph.clone(),
        assignments,
        groups: ordered,
        merge: coarsening.merge,
        resolution: coarsening.resolution,
        merge_evidence: coarsening.evidence.clone(),
    }
}

/// Deterministic coarsening schedule: halve the resolution until the level fits
/// its target, keeping the smallest level achieved if the floor is reached
/// first. The artifact records the achieved count instead of truncating.
fn coarsen(
    graph: &WeightedGraph,
    previous_resolution: f64,
    target: usize,
    level: usize,
    limits: &CommunityLimits,
    min_resolution: f64,
) -> Result<LevelPlan, CommunityError> {
    let mut resolution = (previous_resolution / 2.0).max(min_resolution);
    let mut best = coarsening_at(graph, resolution, level, limits)?;
    let mut attempts = 1usize;
    while best.groups.len() > target
        && attempts < COARSEN_ATTEMPT_LIMIT
        && resolution > min_resolution
    {
        resolution = (resolution / 2.0).max(min_resolution);
        let candidate = coarsening_at(graph, resolution, level, limits)?;
        attempts = attempts.saturating_add(1);
        if candidate.groups.len() < best.groups.len() {
            best = candidate;
        }
    }
    Ok(best)
}

fn coarsening_at(
    graph: &WeightedGraph,
    resolution: f64,
    level: usize,
    limits: &CommunityLimits,
) -> Result<LevelPlan, CommunityError> {
    let raw = leiden(graph, resolution, limits.max_levels, limits.max_moves)?;
    let positions = graph
        .ids
        .iter()
        .enumerate()
        .map(|(position, id)| (id.as_str(), position))
        .collect::<HashMap<_, _>>();
    let mut assignments = vec![None; graph.len()];
    let mut groups = Vec::with_capacity(raw.len());
    for (index, members) in raw.iter().enumerate() {
        let mut children = Vec::with_capacity(members.len());
        for member in members {
            let Some(position) = positions.get(member.as_str()) else {
                return Err(CommunityError::IncompleteHierarchy { level, missing: 1 });
            };
            assignments[*position] = Some(index);
            children.push(*position);
        }
        children.sort_unstable();
        groups.push(children);
    }
    let missing = assignments.iter().filter(|entry| entry.is_none()).count();
    if missing > 0 {
        return Err(CommunityError::IncompleteHierarchy { level, missing });
    }
    Ok(LevelPlan {
        merge: LevelMerge::Relationship,
        resolution: Some(resolution),
        evidence: BTreeMap::new(),
        groups,
        assignments,
    })
}

/// Aggregate the projection into a graph whose nodes are a level's groups.
/// Group the level above by shared source location.
///
/// A repository can publish thousands of communities that share no relationship
/// at all — 98% of `colinhacks/zod`'s 2,781 communities have no cross-community
/// edge — and relationship evidence can never merge them. This cut walks the
/// directory tree those groups already cite: it starts at the repository root
/// and repeatedly expands the largest directory whose children still fit the
/// budget, so every merged group is a real directory the members share, and a
/// group without a dominant directory is only ever merged when it is named by
/// one. Nothing here invents a relationship: the rule, the key, and the
/// coverage counts are recorded on the level.
fn affinity_plan(groups: &[GroupState], keys: &[String], target: usize, level: usize) -> LevelPlan {
    #[derive(Default)]
    struct Node {
        children: BTreeMap<String, usize>,
        /// Group indices this node holds itself; only leaf holders use it.
        leaves: Vec<usize>,
        count: usize,
        parent: Option<usize>,
        name: Option<String>,
    }

    /// The node that holds the leaves of one directory. A directory can be a
    /// group's own key *and* the prefix of deeper keys, so its own leaves live
    /// in a holder that expands with the directory instead of being dropped by
    /// it.
    fn leaf_holder(nodes: &mut Vec<Node>, parent: usize) -> usize {
        if let Some(existing) = nodes
            .get(parent)
            .and_then(|node| node.children.get(""))
            .copied()
        {
            return existing;
        }
        nodes.push(Node {
            parent: Some(parent),
            name: None,
            ..Node::default()
        });
        let created = nodes.len().saturating_sub(1);
        if let Some(node) = nodes.get_mut(parent) {
            node.children.insert(String::new(), created);
        }
        created
    }

    fn node_path(nodes: &[Node], node: usize) -> String {
        let mut components = Vec::new();
        let mut current = node;
        while let Some(entry) = nodes.get(current) {
            if let Some(name) = &entry.name {
                components.push(name.clone());
            }
            let Some(parent) = entry.parent else {
                break;
            };
            current = parent;
        }
        components.reverse();
        components.join("/")
    }

    fn collect_node_leaves(nodes: &[Node], node: usize, leaves: &mut Vec<usize>) {
        if let Some(entry) = nodes.get(node) {
            leaves.extend(entry.leaves.iter().copied());
            for child in entry.children.values() {
                collect_node_leaves(nodes, *child, leaves);
            }
        }
    }

    // A group that cites no dominant directory has no location evidence, so it
    // stays a group of its own: this cut never merges what it cannot name.
    let mut pinned = Vec::new();
    let mut keyed = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        if key.is_empty() {
            pinned.push(index);
        } else {
            keyed.push(index);
        }
    }
    let mut nodes = vec![Node::default()];
    for index in &keyed {
        let Some(key) = keys.get(*index) else {
            continue;
        };
        let mut node = 0usize;
        for component in key.split('/').filter(|part| !part.is_empty()) {
            let next = match nodes[node].children.get(component) {
                Some(existing) => *existing,
                None => {
                    nodes.push(Node {
                        parent: Some(node),
                        name: Some(component.to_owned()),
                        ..Node::default()
                    });
                    let created = nodes.len().saturating_sub(1);
                    if let Some(parent) = nodes.get_mut(node) {
                        parent.children.insert(component.to_owned(), created);
                    }
                    created
                }
            };
            node = next;
        }
        let holder = leaf_holder(&mut nodes, node);
        if let Some(entry) = nodes.get_mut(holder) {
            entry.leaves.push(*index);
        }
    }
    for node in (0..nodes.len()).rev() {
        let mut count = nodes[node].leaves.len();
        for child in nodes[node].children.values() {
            count = count.saturating_add(nodes[*child].count);
        }
        nodes[node].count = count;
    }
    // Pinned groups occupy slots the cut cannot win back. When they already
    // exceed the target the budget is unreachable, so the cut still spends the
    // full target on the keyed groups: a bounded, directory-shaped root that
    // reports the overflow beats one bucket holding every named group.
    let budget = if target > pinned.len() {
        target.saturating_sub(pinned.len())
    } else {
        target.max(1)
    };
    let mut cut = vec![0usize];
    loop {
        let mut best: Option<(usize, usize, String)> = None;
        for node in &cut {
            let children = nodes[*node].children.len();
            if children == 0 || cut.len().saturating_sub(1).saturating_add(children) > budget {
                continue;
            }
            let candidate = (nodes[*node].count, *node, node_path(&nodes, *node));
            let better = match &best {
                None => true,
                Some((count, _, path)) => {
                    candidate.0 > *count || (candidate.0 == *count && candidate.2 < *path)
                }
            };
            if better {
                best = Some(candidate);
            }
        }
        let Some((_, node, _)) = best else {
            break;
        };
        cut.retain(|entry| *entry != node);
        cut.extend(nodes[node].children.values().copied());
        cut.sort_unstable();
    }
    let mut buckets = Vec::with_capacity(cut.len().saturating_add(pinned.len()));
    for node in &cut {
        let mut children = Vec::new();
        collect_node_leaves(&nodes, *node, &mut children);
        if !children.is_empty() {
            children.sort_unstable();
            buckets.push(children);
        }
    }
    for index in &pinned {
        buckets.push(vec![*index]);
    }
    let merged_groups = buckets
        .iter()
        .filter(|bucket| bucket.len() > 1)
        .map(Vec::len)
        .sum::<usize>();
    let singleton_groups = buckets.iter().filter(|bucket| bucket.len() == 1).count();
    let mut assignments = vec![None; groups.len()];
    for (bucket, children) in buckets.iter().enumerate() {
        for child in children {
            if let Some(slot) = assignments.get_mut(*child) {
                *slot = Some(bucket);
            }
        }
    }
    let mut evidence = BTreeMap::new();
    evidence.insert("rule".to_owned(), json!("location-affinity"));
    evidence.insert("mergedGroups".to_owned(), json!(merged_groups));
    evidence.insert("singletonGroups".to_owned(), json!(singleton_groups));
    evidence.insert("unkeyedGroups".to_owned(), json!(pinned.len()));
    evidence.insert("target".to_owned(), json!(target));
    evidence.insert("level".to_owned(), json!(level));
    LevelPlan {
        merge: LevelMerge::LocationAffinity,
        resolution: None,
        evidence,
        groups: buckets,
        assignments,
    }
}

fn aggregate_graph(
    graph: &WeightedGraph,
    assignments: &[Option<usize>],
    group_count: usize,
) -> WeightedGraph {
    let ids = (0..group_count)
        .map(|index| index.to_string())
        .collect::<Vec<_>>();
    let members = ids
        .iter()
        .map(|id| BTreeSet::from([id.clone()]))
        .collect::<Vec<_>>();
    let mut aggregated = WeightedGraph::new(ids, members);
    for (left, right, weight) in graph.edges() {
        let (Some(left_group), Some(right_group)) = (assignments[left], assignments[right]) else {
            continue;
        };
        if left_group == right_group || left_group >= group_count || right_group >= group_count {
            continue;
        }
        aggregated.add_edge(left_group, right_group, weight);
    }
    aggregated
}

/// Cohesion, conductance, and boundary accounting for one level's groups, over
/// the graph that level partitions. This mirrors the community quality
/// artifact's per-community metrics, so the finest level reports the same
/// numbers the published partition already carries.
fn aggregate_metrics(
    graph: &WeightedGraph,
    assignments: &[Option<usize>],
    group_count: usize,
    boundary_by_node: &[BTreeMap<String, usize>],
) -> Vec<Metrics> {
    let total_volume = (0..graph.len())
        .map(|node| graph.degree_weighted(node))
        .sum::<f64>();
    let mut member_counts = vec![0usize; group_count];
    let mut volume = vec![0.0f64; group_count];
    let mut internal_edges = vec![0usize; group_count];
    let mut boundary_weight = vec![0.0f64; group_count];
    let mut boundary_kinds = vec![BTreeMap::<String, usize>::new(); group_count];
    for (node, assignment) in assignments.iter().enumerate() {
        let Some(group) = assignment else {
            continue;
        };
        if *group >= group_count {
            continue;
        }
        member_counts[*group] = member_counts[*group].saturating_add(1);
        volume[*group] += graph.degree_weighted(node);
        if let Some(kinds) = boundary_by_node.get(node) {
            merge_counts(&mut boundary_kinds[*group], kinds);
        }
    }
    for (left, right, weight) in graph.edges() {
        match (assignments[left], assignments[right]) {
            (Some(left_group), Some(right_group)) if left_group == right_group => {
                if left_group < group_count {
                    internal_edges[left_group] = internal_edges[left_group].saturating_add(1);
                }
            }
            _ => {
                if let Some(group) = assignments[left]
                    && group < group_count
                {
                    boundary_weight[group] += weight;
                }
                if right != left
                    && let Some(group) = assignments[right]
                    && group < group_count
                {
                    boundary_weight[group] += weight;
                }
            }
        }
    }
    (0..group_count)
        .map(|group| {
            let members = member_counts[group];
            let possible = members.saturating_mul(members.saturating_sub(1)) / 2;
            let cohesion = if possible == 0 {
                1.0
            } else {
                internal_edges[group] as f64 / possible as f64
            };
            let denominator = volume[group].min(total_volume - volume[group]);
            let conductance = if denominator > 0.0 {
                boundary_weight[group] / denominator
            } else {
                0.0
            };
            Metrics {
                cohesion,
                conductance,
                boundary_kinds: boundary_kinds[group].clone(),
            }
        })
        .collect()
}

fn derive_label(
    legacy: &compass_model::GraphDocument,
    nodes: &HashMap<&str, &NodeRecord>,
    members: &[String],
    index: usize,
) -> HierarchyLabel {
    if let Some(label) = dominant_directory_label(nodes, members) {
        return label;
    }
    if let Some(label) = module_prefix_label(nodes, members) {
        return label;
    }
    hub_member_label(legacy, members).unwrap_or_else(|| community_id_label(index, members.len()))
}

fn dominant_directory_label(
    nodes: &HashMap<&str, &NodeRecord>,
    members: &[String],
) -> Option<HierarchyLabel> {
    let (prefixes, sourced) = directory_prefix_counts(nodes, members);
    if sourced == 0 {
        return None;
    }
    let threshold = LABEL_COVERAGE_THRESHOLD;
    let mut candidates = prefixes
        .iter()
        .filter(|(_, covered)| **covered as f64 >= threshold * sourced as f64)
        .collect::<Vec<_>>();
    candidates.sort_by(|(left, left_covered), (right, right_covered)| {
        left.matches('/')
            .count()
            .cmp(&right.matches('/').count())
            .reverse()
            .then_with(|| right_covered.cmp(left_covered))
            .then_with(|| left.cmp(right))
    });
    let (prefix, covered) = candidates.first()?;
    let mut evidence = BTreeMap::new();
    evidence.insert("rule".to_owned(), json!("dominant-directory"));
    evidence.insert("value".to_owned(), json!(prefix));
    evidence.insert("coveredMembers".to_owned(), json!(covered));
    evidence.insert("memberSourceCount".to_owned(), json!(sourced));
    evidence.insert("memberCount".to_owned(), json!(members.len()));
    evidence.insert("threshold".to_owned(), json!(threshold));
    Some(HierarchyLabel {
        text: (*prefix).clone(),
        rule: HierarchyLabelRule::DominantDirectory,
        generic: false,
        evidence,
    })
}

fn module_prefix_label(
    nodes: &HashMap<&str, &NodeRecord>,
    members: &[String],
) -> Option<HierarchyLabel> {
    let mut prefixes = BTreeMap::<String, usize>::new();
    let mut qualified = 0usize;
    for member in members {
        let Some(node) = nodes.get(member.as_str()) else {
            continue;
        };
        let Some(prefix) = qualified_prefix(&node.qualified_name) else {
            continue;
        };
        qualified = qualified.saturating_add(1);
        let entry = prefixes.entry(prefix).or_default();
        *entry = entry.saturating_add(1);
    }
    if qualified == 0 {
        return None;
    }
    let threshold = LABEL_COVERAGE_THRESHOLD;
    let mut candidates = prefixes
        .iter()
        .filter(|(_, covered)| **covered as f64 >= threshold * qualified as f64)
        .collect::<Vec<_>>();
    candidates.sort_by(|(left, left_covered), (right, right_covered)| {
        right_covered
            .cmp(left_covered)
            .then_with(|| left.cmp(right))
    });
    let (prefix, covered) = candidates.first()?;
    let mut evidence = BTreeMap::new();
    evidence.insert("rule".to_owned(), json!("module-prefix"));
    evidence.insert("value".to_owned(), json!(prefix));
    evidence.insert("coveredMembers".to_owned(), json!(covered));
    evidence.insert("memberQualifiedCount".to_owned(), json!(qualified));
    evidence.insert("memberCount".to_owned(), json!(members.len()));
    evidence.insert("threshold".to_owned(), json!(threshold));
    Some(HierarchyLabel {
        text: (*prefix).clone(),
        rule: HierarchyLabelRule::ModulePrefix,
        generic: false,
        evidence,
    })
}

/// Directory prefixes cited by a group's member source files, with the number
/// of members that cite each one.
fn directory_prefix_counts(
    nodes: &HashMap<&str, &NodeRecord>,
    members: &[String],
) -> (BTreeMap<String, usize>, usize) {
    let mut prefixes = BTreeMap::<String, usize>::new();
    let mut sourced = 0usize;
    for member in members {
        let Some(node) = nodes.get(member.as_str()) else {
            continue;
        };
        let Some(file) = node.source_file() else {
            continue;
        };
        sourced = sourced.saturating_add(1);
        let parts = file
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let limit = parts.len().saturating_sub(1).min(MAX_LABEL_PREFIX_DEPTH);
        for depth in 1..=limit {
            let Some(directory) = parts.get(..depth) else {
                continue;
            };
            let prefix = directory.join("/");
            let entry = prefixes.entry(prefix).or_default();
            *entry = entry.saturating_add(1);
        }
    }
    (prefixes, sourced)
}

/// The most specific directory that covers most of a group's members, or the
/// empty key when no directory does. This is the affinity key: two groups may
/// only be merged by location when they cite the same directory.
fn location_key(members: &[String], nodes: &HashMap<&str, &NodeRecord>) -> String {
    let (prefixes, sourced) = directory_prefix_counts(nodes, members);
    if sourced == 0 {
        return String::new();
    }
    let threshold = LABEL_COVERAGE_THRESHOLD * sourced as f64;
    let mut best: Option<(&String, usize)> = None;
    for (prefix, covered) in &prefixes {
        if (*covered as f64) < threshold {
            continue;
        }
        let depth = prefix.matches('/').count();
        let better = match best {
            None => true,
            Some((current, current_depth)) => {
                depth > current_depth || (depth == current_depth && prefix < current)
            }
        };
        if better {
            best = Some((prefix, depth));
        }
    }
    best.map_or_else(String::new, |(prefix, _)| prefix.clone())
}

/// Every leaf member of a group, sorted.
fn group_member_ids(group: &GroupState, communities: &Communities) -> Vec<String> {
    let mut members = Vec::new();
    for community in &group.communities {
        if let Some(group_members) = communities.get(community) {
            members.extend(group_members.iter().cloned());
        }
    }
    members.sort();
    members
}

fn hub_member_label(
    legacy: &compass_model::GraphDocument,
    members: &[String],
) -> Option<HierarchyLabel> {
    if members.is_empty() {
        return None;
    }
    let communities = Communities::from([(0usize, members.to_vec())]);
    let labels = label_communities_by_hub(legacy, &communities);
    let text = labels.get(&0)?.clone();
    // `label_communities_by_hub` falls back to `Community <n>` when no member
    // can name the group; that fallback is the community-id rule, not evidence.
    if text == "Community 0" {
        return None;
    }
    let mut evidence = BTreeMap::new();
    evidence.insert("rule".to_owned(), json!("hub-member"));
    evidence.insert("value".to_owned(), json!(text));
    evidence.insert("memberCount".to_owned(), json!(members.len()));
    Some(HierarchyLabel {
        text,
        rule: HierarchyLabelRule::HubMember,
        generic: false,
        evidence,
    })
}

fn community_id_label(index: usize, member_count: usize) -> HierarchyLabel {
    let text = format!("Community {index}");
    let mut evidence = BTreeMap::new();
    evidence.insert("rule".to_owned(), json!("community-id"));
    evidence.insert("value".to_owned(), json!(text));
    evidence.insert("community".to_owned(), json!(index));
    evidence.insert("memberCount".to_owned(), json!(member_count));
    HierarchyLabel {
        text,
        rule: HierarchyLabelRule::CommunityId,
        generic: true,
        evidence,
    }
}

fn qualified_prefix(qualified_name: &str) -> Option<String> {
    let trimmed = qualified_name.trim();
    let (index, _) = [("::", 2usize), (".", 1), ("/", 1), ("\\", 1)]
        .into_iter()
        .filter_map(|(separator, width)| trimmed.rfind(separator).map(|index| (index, width)))
        .max_by_key(|(index, _)| *index)?;
    let prefix = trimmed.get(..index)?;
    (!prefix.is_empty()).then(|| prefix.to_owned())
}

fn node_boundary_kinds(node: Option<&NodeRecord>) -> BTreeMap<String, usize> {
    let mut kinds = BTreeMap::new();
    if let Some(node) = node
        && BOUNDARY_KINDS.contains(&node.kind)
    {
        kinds.insert(node.kind.as_str().to_owned(), 1usize);
    }
    kinds
}

/// The exact boundary kind set an artifact records, sorted for stability.
#[must_use]
pub fn boundary_kind_names() -> Vec<String> {
    let mut names = BOUNDARY_KINDS
        .iter()
        .map(|kind| kind.as_str().to_owned())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

/// Map every graph node to the *dense* position of its community. Published
/// community ids can be sparse, but every per-level metric indexes groups by
/// position.
fn community_assignments(graph: &WeightedGraph, communities: &Communities) -> Vec<Option<usize>> {
    let positions = graph
        .ids
        .iter()
        .enumerate()
        .map(|(position, id)| (id.as_str(), position))
        .collect::<HashMap<_, _>>();
    let dense = communities
        .keys()
        .enumerate()
        .map(|(position, community)| (*community, position))
        .collect::<HashMap<_, _>>();
    let mut assignments = vec![None; graph.len()];
    for (community, members) in communities {
        let Some(position_of_group) = dense.get(community) else {
            continue;
        };
        for member in members {
            if let Some(position) = positions.get(member.as_str()) {
                assignments[*position] = Some(*position_of_group);
            }
        }
    }
    assignments
}

/// Prove that every finer level is exactly partitioned by the level above it.
fn verify_level_coverage(
    level: usize,
    coarser: &HierarchyLevel,
    finer: &HierarchyLevel,
) -> Result<(), CommunityError> {
    let mut covered = vec![false; finer.groups.len()];
    let mut placements = 0usize;
    for (group_index, group) in coarser.groups.iter().enumerate() {
        let mut member_sum = 0usize;
        for child in &group.child_indices {
            let Some(entry) = covered.get_mut(*child) else {
                return Err(CommunityError::IncompleteHierarchy { level, missing: 1 });
            };
            placements = placements.saturating_add(1);
            *entry = true;
            let Some(child_group) = finer.groups.get(*child) else {
                return Err(CommunityError::IncompleteHierarchy { level, missing: 1 });
            };
            member_sum = member_sum.saturating_add(child_group.member_count);
        }
        if member_sum != group.member_count {
            return Err(CommunityError::MemberCountMismatch {
                level,
                group: group_index,
                children: member_sum,
                parent: group.member_count,
            });
        }
    }
    let covered_once = covered.iter().filter(|entry| **entry).count();
    let duplicates = placements.saturating_sub(covered_once);
    let missing = finer.groups.len().saturating_sub(covered_once) + duplicates;
    if missing > 0 {
        return Err(CommunityError::IncompleteHierarchy { level, missing });
    }
    Ok(())
}

/// Digest a group's member-community signatures.
///
/// Hashing the signatures rather than node ids keeps an id alive when a symbol
/// is renamed inside a community whose membership is otherwise unchanged, and
/// keeps it independent of the order the members were discovered in.
fn group_signature(communities: &[usize], signatures: &BTreeMap<usize, String>) -> String {
    let mut hasher = Sha256::new();
    for community in communities {
        if let Some(signature) = signatures.get(community) {
            hasher.update(signature.as_bytes());
            hasher.update([0]);
        }
    }
    let digest = format!("{:x}", hasher.finalize());
    digest.chars().take(GROUP_SIGNATURE_LENGTH).collect()
}

/// Digest a set of signatures, independent of the order they arrive in.
fn signature_digest<'a>(signatures: impl IntoIterator<Item = &'a str>) -> String {
    let mut signatures = signatures.into_iter().collect::<Vec<_>>();
    signatures.sort_unstable();
    let mut hasher = Sha256::new();
    for signature in signatures {
        hasher.update(signature.as_bytes());
        hasher.update([0]);
    }
    let digest = format!("{:x}", hasher.finalize());
    digest.chars().take(GROUP_SIGNATURE_LENGTH).collect()
}

/// Digest a level's sorted group signatures.
fn level_signature(groups: &[GroupState]) -> String {
    signature_digest(groups.iter().map(|group| group.signature.as_str()))
}

/// The published id of a group: durable across builds that keep the group.
#[must_use]
pub fn group_id(level: usize, signature: &str) -> String {
    format!("h{level}-{signature}")
}

fn keep_smallest(current: &mut Option<String>, candidate: &str) {
    match current {
        Some(existing) if existing.as_str() <= candidate => {}
        _ => *current = Some(candidate.to_owned()),
    }
}

fn merge_counts(target: &mut BTreeMap<String, usize>, source: &BTreeMap<String, usize>) {
    for (key, count) in source {
        let entry = target.entry(key.clone()).or_default();
        *entry = entry.saturating_add(*count);
    }
}

fn finest_signature(communities: &Communities) -> String {
    let signatures = community_member_signatures(communities);
    let canonical = signatures
        .iter()
        .map(|(community, signature)| format!("{community}:{signature}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

/// A group signature is a fixed-length lowercase hex digest.
#[must_use]
pub fn is_group_signature(value: &str) -> bool {
    value.len() == GROUP_SIGNATURE_LENGTH
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// A published group id names its level and carries a 16-hex signature.
#[must_use]
pub fn is_group_id(level: usize, id: &str) -> bool {
    id.strip_prefix(&format!("h{level}-"))
        .is_some_and(is_group_signature)
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

fn is_sorted_unique(values: &[String]) -> bool {
    values.windows(2).all(|pair| {
        let (Some(left), Some(right)) = (pair.first(), pair.get(1)) else {
            return false;
        };
        left < right
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_model::code_graph::{BuildMetadata, EdgeKind, EdgeRecord};
    use compass_model::provenance::SourceAnchor;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn identity() -> CommunityIdentity {
        CommunityIdentity {
            algorithm: super::super::COMPATIBILITY_CLUSTER_ALGORITHM.to_owned(),
            topology: super::super::COMPATIBILITY_CLUSTER_TOPOLOGY.to_owned(),
            quality: super::super::COMPATIBILITY_CLUSTER_QUALITY.to_owned(),
            selector: super::super::COMPATIBILITY_CLUSTER_SELECTOR.to_owned(),
            seed: super::super::COMPATIBILITY_CLUSTER_SEED,
            limits: super::super::COMPATIBILITY_CLUSTER_LIMITS.to_owned(),
        }
    }

    fn node(id: &str, cluster: usize) -> NodeRecord {
        NodeRecord {
            id: id.to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: id.to_owned(),
            qualified_name: format!("app::group{cluster}::{id}"),
            language: Some("rust".to_owned()),
            framework: None,
            source: Some(SourceAnchor {
                file: format!("src/group{cluster}/{id}.rs"),
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

    fn edge(index: usize, source: &str, target: &str, kind: EdgeKind) -> EdgeRecord {
        EdgeRecord {
            id: format!("edge-{index}"),
            key: format!("edge-{index}"),
            source: source.to_owned(),
            target: target.to_owned(),
            kind,
            occurrence_rule: None,
            relationship_site: None,
            details: None,
            evidence: Vec::new(),
            weight: Some(1.0),
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        }
    }

    fn document(clusters: usize) -> GraphDocument {
        let mut document = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "test".to_owned(),
            source_tree_digest: "test".to_owned(),
            configuration_digest: "test".to_owned(),
            generation_id: "generation-test".to_owned(),
            source_commit: None,
        });
        let mut links = Vec::new();
        for cluster in 0..clusters {
            for index in 0..2 {
                document
                    .nodes
                    .push(node(&format!("c{cluster}n{index}"), cluster));
            }
            links.push(edge(
                links.len(),
                &format!("c{cluster}n0"),
                &format!("c{cluster}n1"),
                EdgeKind::Calls,
            ));
            if cluster > 0 {
                links.push(edge(
                    links.len(),
                    &format!("c{}n0", cluster - 1),
                    &format!("c{cluster}n0"),
                    EdgeKind::References,
                ));
            }
        }
        document.links = links;
        document
    }

    fn communities(clusters: usize) -> Communities {
        (0..clusters)
            .map(|cluster| {
                (
                    cluster,
                    vec![format!("c{cluster}n0"), format!("c{cluster}n1")],
                )
            })
            .collect()
    }

    fn draft(
        clusters: usize,
        budget: HierarchyBudget,
    ) -> Result<CommunityHierarchyDraft, CommunityError> {
        let identity = identity();
        let limits = CommunityLimits::default();
        build_community_hierarchy(
            &document(clusters),
            &communities(clusters),
            &HierarchyRequest {
                identity: &identity,
                limits: &limits,
                resolution: 1.0,
                budget,
            },
        )
    }

    fn artifact(
        clusters: usize,
        budget: HierarchyBudget,
    ) -> Result<CommunityHierarchy, Box<dyn std::error::Error>> {
        Ok(CommunityHierarchy::new(
            "generation-test".to_owned(),
            format!("sha256:{}", "0".repeat(64)),
            draft(clusters, budget)?,
        )?)
    }

    #[test]
    fn artifact_records_the_budget_and_the_resolution_schedule() -> TestResult {
        let budget = HierarchyBudget {
            root_target: 2,
            level_target: 3,
            max_levels: 3,
            ..HierarchyBudget::default()
        };
        let artifact = artifact(4, budget)?;
        assert_eq!(artifact.budget, budget);
        assert_eq!(artifact.budget_identity, COMMUNITY_HIERARCHY_BUDGET);
        assert_eq!(artifact.boundary_kinds, boundary_kind_names());
        let finest = artifact.finest().ok_or("missing finest level")?;
        assert_eq!(
            finest.resolution,
            Some(1.0),
            "the finest level carries the published resolution"
        );
        for pair in artifact.levels.windows(2) {
            let (Some(coarser), Some(finer)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let (Some(coarser_resolution), Some(finer_resolution)) =
                (coarser.resolution, finer.resolution)
            else {
                continue;
            };
            assert!(coarser_resolution <= finer_resolution);
        }
        Ok(())
    }

    #[test]
    fn finest_level_pairs_each_group_with_its_published_community() -> TestResult {
        let artifact = artifact(3, HierarchyBudget::default())?;
        let finest = artifact.finest().ok_or("missing finest level")?;
        assert_eq!(finest.groups.len(), 3);
        for (index, group) in finest.groups.iter().enumerate() {
            assert_eq!(group.index, index);
            assert_eq!(group.community, Some(index));
            assert!(group.child_indices.is_empty());
            assert_eq!(group.member_count, 2);
        }
        Ok(())
    }

    #[test]
    fn sparse_community_ids_are_preserved() -> TestResult {
        let identity = identity();
        let limits = CommunityLimits::default();
        let sparse = Communities::from([
            (2usize, vec!["c0n0".to_owned()]),
            (7usize, vec!["c1n0".to_owned()]),
        ]);
        let artifact = CommunityHierarchy::new(
            "generation-test".to_owned(),
            format!("sha256:{}", "0".repeat(64)),
            build_community_hierarchy(
                &document(2),
                &sparse,
                &HierarchyRequest {
                    identity: &identity,
                    limits: &limits,
                    resolution: 1.0,
                    budget: HierarchyBudget::default(),
                },
            )?,
        )?;
        let finest = artifact.finest().ok_or("missing finest level")?;
        assert_eq!(finest.groups.len(), 2);
        assert_eq!(
            finest.groups.first().and_then(|group| group.community),
            Some(2)
        );
        assert_eq!(
            finest.groups.get(1).and_then(|group| group.community),
            Some(7)
        );
        Ok(())
    }

    #[test]
    fn artifact_digest_detects_mutation() -> TestResult {
        let mut artifact = artifact(4, HierarchyBudget::default())?;
        let first = artifact.levels.first_mut().ok_or("missing level")?;
        first.resolution = first
            .resolution
            .map(|resolution| resolution / 2.0)
            .or(Some(0.5));
        assert!(matches!(
            artifact.validate(),
            Err(CommunityHierarchyArtifactError::DigestMismatch)
        ));
        Ok(())
    }

    #[test]
    fn artifact_rejects_unknown_major_and_wrong_graph() -> TestResult {
        let mut artifact = artifact(2, HierarchyBudget::default())?;
        assert!(matches!(
            artifact.validate_for_graph("other", &format!("sha256:{}", "0".repeat(64))),
            Err(CommunityHierarchyArtifactError::GraphIdentityMismatch)
        ));
        artifact.schema = "compass.community-hierarchy/2".to_owned();
        assert!(matches!(
            artifact.validate(),
            Err(CommunityHierarchyArtifactError::UnsupportedSchema(_))
        ));
        Ok(())
    }

    #[test]
    fn artifact_rejects_unknown_fields() -> TestResult {
        let artifact = artifact(2, HierarchyBudget::default())?;
        let mut value = serde_json::to_value(artifact)?;
        value
            .as_object_mut()
            .ok_or_else(|| std::io::Error::other("artifact must be an object"))?
            .insert("futureField".to_owned(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<CommunityHierarchy>(value).is_err());
        Ok(())
    }

    #[test]
    fn artifact_rejects_a_level_that_does_not_cover_its_children() -> TestResult {
        let budget = HierarchyBudget {
            root_target: 2,
            level_target: 3,
            max_levels: 3,
            ..HierarchyBudget::default()
        };
        let mut artifact = artifact(4, budget)?;
        assert!(artifact.levels.len() >= 2, "the fixture must coarsen");
        let dropped = artifact
            .levels
            .first()
            .and_then(|level| level.groups.first())
            .and_then(|group| group.child_indices.last().copied())
            .ok_or("missing child")?;
        let finer_members = artifact
            .levels
            .get(1)
            .and_then(|level| level.groups.get(dropped))
            .map(|group| group.member_count)
            .ok_or("missing finer group")?;
        let root = artifact.levels.first_mut().ok_or("missing root")?;
        let group = root.groups.first_mut().ok_or("missing root group")?;
        group.child_indices.pop();
        group.member_count = group.member_count.saturating_sub(finer_members);
        artifact.result_digest = artifact.calculate_digest()?;
        let error = artifact
            .validate()
            .err()
            .ok_or("expected invalid evidence")?;
        assert!(
            error.to_string().contains("not covered exactly once"),
            "unexpected error: {error}"
        );
        Ok(())
    }

    #[test]
    fn affinity_cut_partitions_every_level() -> TestResult {
        let group = |index: usize| GroupState {
            communities: vec![index],
            signature: format!("{:016x}", index),
            member_count: 1,
            children: vec![index],
            community: None,
            smallest_member: format!("n{index}"),
            quality: GroupQuality {
                cohesion: 1.0,
                conductance: 0.0,
                boundary_kinds: BTreeMap::new(),
            },
            label: community_id_label(index, 1),
        };
        let keys = [
            "src/flask".to_owned(),
            "src".to_owned(),
            "src/flask".to_owned(),
            "tests".to_owned(),
            "tests/unit".to_owned(),
            String::new(),
            "docs".to_owned(),
            "src/flask/app".to_owned(),
            "tests".to_owned(),
            String::new(),
        ];
        let groups = (0..keys.len()).map(group).collect::<Vec<_>>();
        for target in 1..=keys.len() {
            let plan = affinity_plan(&groups, &keys, target, 0);
            let mut covered = vec![0usize; groups.len()];
            for bucket in &plan.groups {
                for child in bucket {
                    covered[*child] += 1;
                }
            }
            assert!(
                covered.iter().all(|count| *count == 1),
                "target {target} produced buckets that do not partition the level: {:?}",
                plan.groups
            );
            // Groups with no location key stay singletons, so a small target
            // can only ever be met up to the number of pinned groups.
            let pinned = keys.iter().filter(|key| key.is_empty()).count();
            assert!(
                plan.groups.len() <= target || plan.groups.len() <= pinned.saturating_add(1),
                "target {target} produced {} buckets for {pinned} pinned groups",
                plan.groups.len(),
            );
        }
        Ok(())
    }

    #[test]
    fn empty_partition_is_rejected() -> TestResult {
        let empty = Communities::new();
        let identity = identity();
        let limits = CommunityLimits::default();
        let error = build_community_hierarchy(
            &document(1),
            &empty,
            &HierarchyRequest {
                identity: &identity,
                limits: &limits,
                resolution: 1.0,
                budget: HierarchyBudget::default(),
            },
        );
        assert!(matches!(
            error,
            Err(CommunityError::InvalidHierarchyBudget { .. })
        ));
        Ok(())
    }
}
