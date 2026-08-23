use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub const ARCHITECTURE_VIEWER_SCHEMA: &str = "compass.viewer.architecture/1";
pub const ARCHITECTURE_OVERLAY_SCHEMA: &str = "compass.architecture-overlay/1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureScope {
    Production,
    AllCode,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureSourceScope {
    Production,
    Test,
    Generated,
    Vendor,
    Documentation,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureRelationClass {
    Execution,
    Dependency,
    Type,
    Structure,
    Contextual,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureLens {
    Architecture,
    Execution,
    Dependency,
    Type,
    Structure,
    All,
}

impl ArchitectureLens {
    #[must_use]
    pub const fn admits(self, relation_class: ArchitectureRelationClass) -> bool {
        match self {
            Self::Architecture => matches!(
                relation_class,
                ArchitectureRelationClass::Execution | ArchitectureRelationClass::Dependency
            ),
            Self::Execution => matches!(relation_class, ArchitectureRelationClass::Execution),
            Self::Dependency => matches!(relation_class, ArchitectureRelationClass::Dependency),
            Self::Type => matches!(relation_class, ArchitectureRelationClass::Type),
            Self::Structure => matches!(relation_class, ArchitectureRelationClass::Structure),
            Self::All => !matches!(relation_class, ArchitectureRelationClass::Unknown),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureGroupKind {
    Owner,
    Subsystem,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureRouteLevel {
    Overview,
    Detail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureNameProvenance {
    Overlay,
    Persisted,
    Owner,
    Path,
    Declaration,
    Hub,
    Provider,
    Fallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureQualityStatus {
    Good,
    Degraded,
    Insufficient,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct ArchitectureProjectionOptions {
    pub scopes: BTreeSet<ArchitectureScope>,
    pub default_lens: ArchitectureLens,
    pub limits: ArchitectureProjectionLimits,
}

impl Default for ArchitectureProjectionOptions {
    fn default() -> Self {
        Self {
            scopes: BTreeSet::from([ArchitectureScope::Production, ArchitectureScope::AllCode]),
            default_lens: ArchitectureLens::Architecture,
            limits: ArchitectureProjectionLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureProjectionLimits {
    pub max_nodes: usize,
    pub max_relationships: usize,
    pub max_groups: usize,
    pub max_routes: usize,
    pub max_overview_groups: usize,
    pub max_overview_routes: usize,
    pub max_name_candidates: usize,
    pub max_name_evidence: usize,
    pub max_diagnostics: usize,
    pub max_omission_witnesses: usize,
}

impl Default for ArchitectureProjectionLimits {
    fn default() -> Self {
        Self {
            max_nodes: 250_000,
            max_relationships: 1_000_000,
            max_groups: 100_000,
            max_routes: 250_000,
            max_overview_groups: 24,
            max_overview_routes: 64,
            max_name_candidates: 12,
            max_name_evidence: 4,
            max_diagnostics: 128,
            max_omission_witnesses: 8,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureViewModel {
    pub schema: &'static str,
    pub title: String,
    pub nodes: Vec<ArchitectureNode>,
    pub relationships: Vec<ArchitectureRelationship>,
    pub projections: Vec<ArchitectureScopeProjection>,
    pub statistics: ArchitectureStatistics,
    pub provenance: ArchitectureProvenance,
    pub limits: ArchitectureProjectionLimits,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub source_file: Option<String>,
    pub source_scope: ArchitectureSourceScope,
    pub scope_reason: String,
    pub community: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureRelationship {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    pub relation_class: ArchitectureRelationClass,
    pub confidence: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureScopeProjection {
    pub scope: ArchitectureScope,
    pub default_lens: ArchitectureLens,
    pub groups: Vec<ArchitectureGroup>,
    pub memberships: Vec<ArchitectureMembership>,
    pub routes: Vec<ArchitectureRoute>,
    pub overview_group_ids: Vec<String>,
    pub overview_route_ids: Vec<String>,
    pub coverage: ArchitectureCoverage,
    pub omissions: ArchitectureOmissions,
    pub quality: ArchitectureQuality,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureGroup {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: ArchitectureGroupKind,
    pub rank: usize,
    pub name: ArchitectureGroupName,
    pub owner_key: String,
    pub community_ids: Vec<usize>,
    pub node_count: usize,
    pub relationship_count: usize,
    pub neighbor_count: usize,
    pub cohesion: f64,
    pub source_scopes: ArchitectureSourceCounts,
    pub pinned: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureGroupName {
    pub value: String,
    pub provenance: ArchitectureNameProvenance,
    pub membership_signature: String,
    pub quality: u8,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureMembership {
    /// Index into the top-level, ID-sorted `nodes` array.
    pub node_index: usize,
    /// Index into this projection's deterministic `groups` array.
    pub group_index: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureRoute {
    pub id: String,
    pub level: ArchitectureRouteLevel,
    pub owner_id: Option<String>,
    pub source_group: String,
    pub target_group: String,
    pub relationship_count: usize,
    pub relation_classes: ArchitectureClassCounts,
    pub evidence: ArchitectureEvidenceCounts,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureClassCounts {
    pub execution: usize,
    pub dependency: usize,
    pub r#type: usize,
    pub structure: usize,
    pub contextual: usize,
    pub unknown: usize,
}

impl ArchitectureClassCounts {
    pub fn increment(&mut self, relation_class: ArchitectureRelationClass) {
        match relation_class {
            ArchitectureRelationClass::Execution => self.execution += 1,
            ArchitectureRelationClass::Dependency => self.dependency += 1,
            ArchitectureRelationClass::Type => self.r#type += 1,
            ArchitectureRelationClass::Structure => self.structure += 1,
            ArchitectureRelationClass::Contextual => self.contextual += 1,
            ArchitectureRelationClass::Unknown => self.unknown += 1,
        }
    }

    #[must_use]
    pub const fn admitted(self, lens: ArchitectureLens) -> usize {
        match lens {
            ArchitectureLens::Architecture => self.execution + self.dependency,
            ArchitectureLens::Execution => self.execution,
            ArchitectureLens::Dependency => self.dependency,
            ArchitectureLens::Type => self.r#type,
            ArchitectureLens::Structure => self.structure,
            ArchitectureLens::All => {
                self.execution + self.dependency + self.r#type + self.structure + self.contextual
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ArchitectureEvidenceCounts {
    pub extracted: usize,
    pub inferred: usize,
    pub ambiguous: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureSourceCounts {
    pub production: usize,
    pub test: usize,
    pub generated: usize,
    pub vendor: usize,
    pub documentation: usize,
    pub unknown: usize,
}

impl ArchitectureSourceCounts {
    pub fn increment(&mut self, source_scope: ArchitectureSourceScope) {
        match source_scope {
            ArchitectureSourceScope::Production => self.production += 1,
            ArchitectureSourceScope::Test => self.test += 1,
            ArchitectureSourceScope::Generated => self.generated += 1,
            ArchitectureSourceScope::Vendor => self.vendor += 1,
            ArchitectureSourceScope::Documentation => self.documentation += 1,
            ArchitectureSourceScope::Unknown => self.unknown += 1,
        }
    }

    #[must_use]
    pub const fn total(self) -> usize {
        self.production
            + self.test
            + self.generated
            + self.vendor
            + self.documentation
            + self.unknown
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureCoverage {
    pub admitted: usize,
    pub internal: usize,
    pub cross_group: usize,
    pub unassigned: usize,
    pub relation_classes: ArchitectureClassCounts,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureOmissions {
    pub total_groups: usize,
    pub shown_groups: usize,
    pub omitted_groups: usize,
    pub represented_nodes: usize,
    pub omitted_nodes: usize,
    pub represented_relationships: usize,
    pub omitted_relationships: usize,
    pub witness_group_ids: Vec<String>,
    pub max_overview_groups: usize,
    pub max_overview_routes: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureQuality {
    pub status: ArchitectureQualityStatus,
    pub metrics: ArchitectureQualityMetrics,
    pub diagnostics: Vec<ArchitectureQualityDiagnostic>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureQualityMetrics {
    pub source_scopes: ArchitectureSourceCounts,
    pub unknown_source_fraction: f64,
    pub generated_vendor_leakage: usize,
    pub represented_node_fraction: f64,
    pub represented_relationship_fraction: f64,
    pub duplicate_names: usize,
    pub fallback_names: usize,
    pub largest_group_fraction: f64,
    pub unknown_relations: usize,
    pub unassigned_nodes: usize,
    pub unassigned_relationships: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureQualityDiagnostic {
    pub code: String,
    pub severity: ArchitectureDiagnosticSeverity,
    pub message: String,
    pub observed: Option<f64>,
    pub threshold: Option<f64>,
    pub witnesses: Vec<String>,
    pub recommended_action: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArchitectureStatistics {
    pub nodes: usize,
    pub relationships: usize,
    pub communities: usize,
    pub extracted: usize,
    pub inferred: usize,
    pub ambiguous: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchitectureProvenance {
    pub project_name: String,
    pub built_at_commit: Option<String>,
    pub generated_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchitectureOverlay {
    pub schema: String,
    #[serde(default)]
    pub source_rules: Vec<ArchitectureOverlaySourceRule>,
    #[serde(default)]
    pub groups: Vec<ArchitectureOverlayGroup>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchitectureOverlaySourceRule {
    pub path_prefix: String,
    pub scope: ArchitectureSourceScope,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchitectureOverlayGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub path_prefixes: Vec<String>,
    #[serde(default)]
    pub communities: Vec<usize>,
    #[serde(default)]
    pub pin: bool,
}
