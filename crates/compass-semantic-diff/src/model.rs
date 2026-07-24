use std::collections::BTreeMap;

use compass_analysis::FunctionSummary;
use compass_history::{SourceFileDelta, SourceHunk};
use compass_ir::{FunctionIr, ModuleIr};
use compass_model::NodeRecord;
use serde::{Deserialize, Serialize};

use crate::SemanticDiffError;

pub const REPORT_SCHEMA: &str = "compass.semantic_diff.report/1";
pub const CLASSIFIER_VERSION: u32 = 1;
pub const MAX_DIRECT_ENTITIES: usize = 10_000;
pub const MAX_TRAVERSED_CALL_EDGES: usize = 200_000;
pub const MAX_IMPACT_DEPTH: u8 = 4;
pub const MAX_FINDINGS: usize = 5_000;
pub const MAX_EVIDENCE_PER_FINDING: usize = 20;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingType {
    ContractChange,
    BehaviorChange,
    DependencyChange,
    ImpactChange,
    VerificationGap,
    StructuralChange,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingOrigin {
    Direct,
    Derived,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    ProvenBreak,
    PossibleBreak,
    Compatible,
    Behavioral,
    NotApplicable,
    Indeterminate,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Exact,
    Probable,
    Inferred,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    Unknown,
    Covered,
    Gap,
    Partial,
    Stale,
    Failing,
    NotRun,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeDirection {
    Added,
    Removed,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_byte: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_byte: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_key: Option<String>,
    pub capability: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct AffectedConsumer {
    pub symbol_id: String,
    pub display_name: String,
    pub source_file: String,
    pub distance: u8,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WitnessHop {
    pub source: String,
    pub relation: String,
    pub target: String,
    pub confidence: Confidence,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WitnessPath {
    pub consumer: String,
    pub confidence: Confidence,
    pub hops: Vec<WitnessHop>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Verification {
    pub state: VerificationState,
    pub exact_tests: Vec<String>,
    pub recommended_tests: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TestEvidence {
    pub completeness: Completeness,
    pub exact_tests: Vec<String>,
    pub suggested_tests: Vec<String>,
}

pub trait TestEvidenceProvider {
    fn tests_for(&self, symbol_id: &str) -> TestEvidence;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoTestEvidence;

impl TestEvidenceProvider for NoTestEvidence {
    fn tests_for(&self, _symbol_id: &str) -> TestEvidence {
        TestEvidence {
            completeness: Completeness::Unavailable,
            exact_tests: Vec::new(),
            suggested_tests: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticFinding {
    pub id: String,
    pub finding_type: FindingType,
    pub subject: String,
    pub origin: FindingOrigin,
    pub headline: String,
    pub explanation: String,
    pub compatibility: Compatibility,
    pub confidence: Confidence,
    pub review_priority: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
    pub affected_consumers: Vec<AffectedConsumer>,
    pub witness_paths: Vec<WitnessPath>,
    pub verification: Verification,
    pub reviewer_action: String,
    pub evidence: Vec<EvidenceRef>,
    pub completeness: BTreeMap<String, Completeness>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CollapsedGroup {
    pub label: String,
    pub count: usize,
    pub finding_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub old_commit: String,
    pub new_commit: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticDiffReport {
    pub schema: String,
    pub comparison: Comparison,
    pub findings: Vec<SemanticFinding>,
    pub collapsed_groups: Vec<CollapsedGroup>,
    pub completeness: BTreeMap<String, Completeness>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotSide {
    Old,
    New,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotIdentity {
    pub commit: String,
    pub realization: String,
    pub fingerprint: String,
}

pub trait SnapshotReader {
    fn node(
        &self,
        side: SnapshotSide,
        node_id: &str,
    ) -> Result<Option<NodeRecord>, SemanticDiffError>;

    fn module(
        &self,
        side: SnapshotSide,
        source_file: &str,
    ) -> Result<Option<ModuleIr>, SemanticDiffError>;

    fn function(
        &self,
        _side: SnapshotSide,
        _symbol_id: &str,
    ) -> Result<Option<FunctionIr>, SemanticDiffError> {
        Ok(None)
    }

    fn summary(
        &self,
        side: SnapshotSide,
        symbol_id: &str,
    ) -> Result<Option<FunctionSummary>, SemanticDiffError>;

    fn reverse_callers(
        &self,
        side: SnapshotSide,
        symbol_id: &str,
    ) -> Result<Vec<String>, SemanticDiffError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyDelta {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub change: ChangeDirection,
    pub evidence: Vec<EvidenceRef>,
}

pub struct SemanticDiffInput<'a> {
    pub old: SnapshotIdentity,
    pub new: SnapshotIdentity,
    pub source_deltas: &'a [SourceFileDelta],
    pub changed_node_ids: &'a [String],
    pub dependency_deltas: &'a [DependencyDelta],
    pub snapshots: &'a dyn SnapshotReader,
    pub test_evidence: &'a dyn TestEvidenceProvider,
}

#[derive(Clone, Debug)]
pub(crate) struct EntitySnapshot {
    pub language: String,
    pub source_file: String,
    pub function: compass_ir::FunctionIr,
}

#[derive(Clone, Debug)]
pub(crate) struct ChangedEntity {
    pub old: Option<EntitySnapshot>,
    pub new: Option<EntitySnapshot>,
    pub hunks: Vec<SourceHunk>,
}
