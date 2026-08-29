use serde::{Deserialize, Serialize};

use crate::code_graph::{
    EdgeDetails, EdgeKind, EdgeRecord, NodeDetails, NodeKind, NodeRecord, NodeRole,
};
use crate::provenance::{
    EvidenceConfidence, EvidenceOrigin, ResolutionCandidate, ResolutionState, SourceAnchor,
};

pub const CODE_QUERY_SCHEMA_V1: &str = "compass.query/1";
pub const STRUCTURAL_QUERY_SCHEMA_V1: &str = "compass.structural-query/1";

/// Normalize an exact structural symbol operand consistently across engines.
#[must_use]
pub fn normalize_query_symbol(value: &str) -> String {
    value
        .trim()
        .trim_end_matches("()")
        .trim_start_matches('.')
        .to_lowercase()
}

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

    /// Project transport-independent structural semantics for differential
    /// engine qualification.
    #[must_use]
    pub fn structural_view(
        &self,
        repository_id: impl Into<String>,
        generation_id: impl Into<String>,
    ) -> StructuralQueryResponse {
        let mut response = StructuralQueryResponse {
            schema: STRUCTURAL_QUERY_SCHEMA_V1.to_owned(),
            repository_id: repository_id.into(),
            generation_id: generation_id.into(),
            operation: self.operation,
            nodes: self.nodes.clone(),
            edges: self.edges.clone(),
            paths: self.paths.clone(),
            diagnostics: self.diagnostics.clone(),
            limits: self.limits.clone(),
            truncated: self.truncated,
        };
        response.sort_stable();
        response
    }
}

/// Canonical structural subset used to compare independent query engines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructuralQueryResponse {
    pub schema: String,
    pub repository_id: String,
    pub generation_id: String,
    pub operation: CodeQueryOperation,
    pub nodes: Vec<QueryNode>,
    pub edges: Vec<QueryEdge>,
    pub paths: Vec<QueryPath>,
    pub diagnostics: Vec<QueryDiagnostic>,
    pub limits: CodeQueryLimits,
    pub truncated: bool,
}

impl StructuralQueryResponse {
    pub fn sort_stable(&mut self) {
        self.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        self.edges.sort_by(|left, right| left.id.cmp(&right.id));
        self.paths.sort_by(|left, right| left.id.cmp(&right.id));
        self.diagnostics.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then_with(|| left.message.cmp(&right.message))
                .then_with(|| left.node_id.cmp(&right.node_id))
                .then_with(|| left.path.cmp(&right.path))
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
    DirectionMismatch,
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

/// Convert one canonical graph node into the shared structural query contract.
#[must_use]
pub fn query_node_from_record(node: &NodeRecord) -> QueryNode {
    QueryNode {
        id: node.id.clone(),
        kind: node.kind,
        roles: node.roles.clone(),
        name: node.name.clone(),
        qualified_name: node.qualified_name.clone(),
        language: node.language.clone(),
        framework: node.framework.clone(),
        source: node.source.clone(),
        details: node.details.clone(),
        evidence: node.evidence.iter().map(structural_evidence).collect(),
    }
}

/// Convert one canonical graph relation into the shared structural query contract.
#[must_use]
pub fn query_edge_from_record(edge: &EdgeRecord) -> QueryEdge {
    QueryEdge {
        id: edge.id.clone(),
        source: edge.source.clone(),
        target: edge.target.clone(),
        kind: edge.kind,
        relationship_site: edge.relationship_site.clone(),
        details: edge.details.clone(),
        evidence: edge.evidence.iter().map(structural_evidence).collect(),
    }
}

/// Build the deterministic path contract from canonical relation evidence.
#[must_use]
pub fn query_path_from_records(
    nodes: &[String],
    edges: &[String],
    selected: &[EdgeRecord],
) -> QueryPath {
    let weakest_confidence = selected
        .iter()
        .flat_map(|edge| &edge.evidence)
        .map(|evidence| evidence.confidence)
        .max_by_key(|confidence| match confidence {
            EvidenceConfidence::Exact => 0,
            EvidenceConfidence::Inferred => 1,
            EvidenceConfidence::Ambiguous => 2,
        })
        .unwrap_or(EvidenceConfidence::Exact);
    let weakest_resolution = if weakest_confidence == EvidenceConfidence::Ambiguous {
        ResolutionState::Ambiguous
    } else if weakest_confidence == EvidenceConfidence::Inferred {
        ResolutionState::Unresolved
    } else {
        ResolutionState::Exact
    };
    QueryPath {
        id: format!("path:{}", edges.join(":")),
        node_ids: nodes.to_vec(),
        edge_ids: edges.to_vec(),
        weakest_resolution,
        weakest_confidence,
    }
}

fn structural_evidence(evidence: &crate::provenance::Provenance) -> QueryEvidence {
    QueryEvidence {
        layer: QueryEvidenceLayer::StructuralGraph,
        origin: evidence.origin,
        extractor: evidence.extractor.clone(),
        confidence: evidence.confidence,
        anchor: evidence.anchors.first().cloned(),
        rule: evidence.rule.clone(),
        wiring_site: evidence.wiring_site.clone(),
        resolution: if evidence.confidence == EvidenceConfidence::Ambiguous
            || evidence.candidates.len() > 1
        {
            ResolutionState::Ambiguous
        } else if evidence.confidence == EvidenceConfidence::Inferred
            && evidence.candidates.is_empty()
        {
            ResolutionState::Unresolved
        } else {
            ResolutionState::Exact
        },
        candidates: evidence.candidates.clone(),
    }
}
