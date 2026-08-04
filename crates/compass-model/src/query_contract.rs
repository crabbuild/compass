use serde::{Deserialize, Serialize};

use crate::code_graph::{EdgeDetails, EdgeKind, NodeDetails, NodeKind, NodeRole};
use crate::provenance::{
    EvidenceConfidence, EvidenceOrigin, ResolutionCandidate, ResolutionState, SourceAnchor,
};

pub const CODE_QUERY_SCHEMA_V1: &str = "compass.query/1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeQueryOperation {
    Search,
    Callers,
    Callees,
    Impact,
    Explore,
    NodeTrail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryEvidenceLayer {
    StructuralGraph,
    ProgramIr,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeQueryLimits {
    pub max_depth: u32,
    pub max_nodes: u32,
    pub max_edges: u32,
    pub max_paths: u32,
    pub max_candidates: u32,
    pub max_source_bytes: u64,
    pub max_response_bytes: u64,
}

impl Default for CodeQueryLimits {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_nodes: 500,
            max_edges: 1_000,
            max_paths: 100,
            max_candidates: 20,
            max_source_bytes: 1_048_576,
            max_response_bytes: 8_388_608,
        }
    }
}

impl CodeQueryLimits {
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.max_depth > 0
            && self.max_nodes > 0
            && self.max_edges > 0
            && self.max_paths > 0
            && self.max_candidates > 0
            && self.max_source_bytes > 0
            && self.max_response_bytes > 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchRequest {
    pub query: String,
    pub limits: CodeQueryLimits,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallRequest {
    pub symbol: String,
    #[serde(default)]
    pub include_heuristic: bool,
    pub limits: CodeQueryLimits,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactRequest {
    pub symbol: String,
    pub include_heuristic: bool,
    pub limits: CodeQueryLimits,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExploreRequest {
    pub symbols: Vec<String>,
    pub root: String,
    #[serde(default)]
    pub include_heuristic: bool,
    pub limits: CodeQueryLimits,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeTrailRequest {
    pub source: String,
    pub target: String,
    pub include_heuristic: bool,
    pub limits: CodeQueryLimits,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeQueryResponse {
    pub schema: String,
    pub operation: CodeQueryOperation,
    pub results: Vec<SearchHit>,
    pub nodes: Vec<QueryNode>,
    pub edges: Vec<QueryEdge>,
    pub files: Vec<QueryFile>,
    pub paths: Vec<QueryPath>,
    pub diagnostics: Vec<QueryDiagnostic>,
    pub limits: CodeQueryLimits,
    pub truncated: bool,
}

impl CodeQueryResponse {
    #[must_use]
    pub fn empty(operation: CodeQueryOperation, limits: CodeQueryLimits) -> Self {
        Self {
            schema: CODE_QUERY_SCHEMA_V1.to_owned(),
            operation,
            results: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            files: Vec::new(),
            paths: Vec::new(),
            diagnostics: Vec::new(),
            limits,
            truncated: false,
        }
    }

    pub fn sort_stable(&mut self) {
        self.results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.node_id.cmp(&right.node_id))
        });
        self.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        self.edges.sort_by(|left, right| left.id.cmp(&right.id));
        self.files.sort_by(|left, right| left.path.cmp(&right.path));
        self.paths.sort_by(|left, right| left.id.cmp(&right.id));
        self.diagnostics.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| left.message.cmp(&right.message))
                .then_with(|| left.node_id.cmp(&right.node_id))
        });
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchHit {
    pub node_id: String,
    pub score: f64,
    pub matched_fields: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryNode {
    pub id: String,
    pub kind: NodeKind,
    pub roles: Vec<NodeRole>,
    pub name: String,
    pub qualified_name: String,
    pub language: Option<String>,
    pub framework: Option<String>,
    pub source: Option<SourceAnchor>,
    pub details: Option<NodeDetails>,
    pub evidence: Vec<QueryEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    pub relationship_site: Option<SourceAnchor>,
    pub details: Option<EdgeDetails>,
    pub evidence: Vec<QueryEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryFile {
    pub path: String,
    pub content_digest: String,
    pub source: Option<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryPath {
    pub id: String,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
    pub weakest_resolution: ResolutionState,
    pub weakest_confidence: EvidenceConfidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryDiagnostic {
    pub code: QueryDiagnosticCode,
    pub message: String,
    pub node_id: Option<String>,
    pub path: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryDiagnosticCode {
    NoMatch,
    AmbiguousMatch,
    UnresolvedHandler,
    IncompleteCoverage,
    StaleSourceDigest,
    BoundedTruncation,
    ProgramOrphan,
    ProgramConflict,
    ProgramUnavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryEvidence {
    pub layer: QueryEvidenceLayer,
    pub origin: EvidenceOrigin,
    pub extractor: String,
    pub confidence: EvidenceConfidence,
    pub anchor: Option<SourceAnchor>,
    pub rule: Option<String>,
    pub wiring_site: Option<SourceAnchor>,
    pub resolution: ResolutionState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<ResolutionCandidate>,
}
