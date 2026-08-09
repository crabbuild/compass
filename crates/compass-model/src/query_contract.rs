use serde::{Deserialize, Serialize};

use crate::code_graph::{EdgeDetails, EdgeKind, NodeDetails, NodeKind, NodeRole};
use crate::provenance::{
    EvidenceConfidence, EvidenceOrigin, OccurrenceRule, ResolutionCandidate, ResolutionState,
    SourceAnchor,
};

pub const CODE_QUERY_SCHEMA_V1: &str = "compass.query/1";
pub const DISCOVERY_QUERY_SCHEMA_V1: &str = "compass.query.discovery/1";

pub const MAX_DISCOVERY_DEPTH: u32 = 8;
pub const MAX_DISCOVERY_SEEDS: u32 = 3;
pub const MAX_DISCOVERY_CANDIDATES: u32 = 256;
pub const MAX_DISCOVERY_NODES: u32 = 500;
pub const MAX_DISCOVERY_EDGES: u32 = 1_000;
pub const MAX_DISCOVERY_EXPANDED_RELATIONSHIPS: u64 = 10_000;
/// Maximum indexed candidate records read across exact-ID, exact-name, alias,
/// term, and fuzzy recall for one typed query. This bound is independent of
/// graph size and may exceed the admitted-candidate limit because the shared
/// recall engine probes several independently bounded sources.
pub const MAX_INDEXED_CANDIDATE_NODES_READ: u64 = 12_801;
/// Maximum exact/name/alias/term/fuzzy index probes for one typed query.
pub const MAX_INDEXED_CANDIDATE_PROBES: u64 = 291;
/// Discovery uses the shared indexed-recall work ceiling.
pub const MAX_DISCOVERY_CANDIDATE_NODES_READ: u64 = MAX_INDEXED_CANDIDATE_NODES_READ;
/// Discovery uses the shared indexed-recall probe ceiling.
pub const MAX_DISCOVERY_CANDIDATE_PROBES: u64 = MAX_INDEXED_CANDIDATE_PROBES;
pub const MAX_DISCOVERY_RESPONSE_BYTES: u64 = 8_388_608;
pub const MAX_DISCOVERY_TIMEOUT_MS: u64 = 30_000;
pub const MAX_INDEXED_QUERY_BYTES: usize = 4_096;
pub const MAX_INDEXED_QUERY_TERMS: usize = 32;
pub const MAX_DISCOVERY_QUESTION_BYTES: usize = MAX_INDEXED_QUERY_BYTES;
pub const MAX_DISCOVERY_QUERY_TERMS: usize = MAX_INDEXED_QUERY_TERMS;
pub const MAX_DISCOVERY_FILTERS: usize = 32;
pub const MAX_DISCOVERY_FILTER_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryDirection {
    #[default]
    Auto,
    Incoming,
    Outgoing,
    Both,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryDirectionSource {
    Explicit,
    Heuristic,
    #[default]
    Neutral,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryScopeKind {
    Community,
    Source,
    Package,
    Node,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryScope {
    pub kind: DiscoveryScopeKind,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryTraversal {
    #[default]
    Bfs,
    Dfs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryLimits {
    pub max_depth: u32,
    pub max_seeds: u32,
    pub max_candidates: u32,
    pub max_nodes: u32,
    pub max_edges: u32,
    pub max_expanded_relationships: u64,
    pub max_response_bytes: u64,
    pub timeout_ms: u64,
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_seeds: 3,
            max_candidates: MAX_DISCOVERY_CANDIDATES,
            max_nodes: MAX_DISCOVERY_NODES,
            max_edges: MAX_DISCOVERY_EDGES,
            max_expanded_relationships: MAX_DISCOVERY_EXPANDED_RELATIONSHIPS,
            max_response_bytes: MAX_DISCOVERY_RESPONSE_BYTES,
            timeout_ms: MAX_DISCOVERY_TIMEOUT_MS,
        }
    }
}

impl DiscoveryLimits {
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.max_depth > 0
            && self.max_depth <= MAX_DISCOVERY_DEPTH
            && self.max_seeds > 0
            && self.max_seeds <= MAX_DISCOVERY_SEEDS
            && self.max_candidates > 0
            && self.max_candidates <= MAX_DISCOVERY_CANDIDATES
            && self.max_nodes > 0
            && self.max_nodes <= MAX_DISCOVERY_NODES
            && self.max_edges > 0
            && self.max_edges <= MAX_DISCOVERY_EDGES
            && self.max_expanded_relationships > 0
            && self.max_expanded_relationships <= MAX_DISCOVERY_EXPANDED_RELATIONSHIPS
            && self.max_response_bytes > 0
            && self.max_response_bytes <= MAX_DISCOVERY_RESPONSE_BYTES
            && self.timeout_ms > 0
            && self.timeout_ms <= MAX_DISCOVERY_TIMEOUT_MS
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryQueryRequest {
    pub question: String,
    #[serde(default)]
    pub direction: DiscoveryDirection,
    #[serde(default)]
    pub relation_contexts: Vec<String>,
    #[serde(default)]
    pub scope: Vec<DiscoveryScope>,
    #[serde(default)]
    pub traversal: DiscoveryTraversal,
    #[serde(default)]
    pub include_heuristic: bool,
    #[serde(default)]
    pub limits: DiscoveryLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySeedSource {
    ExactId,
    ExactName,
    Alias,
    TermIndex,
    RelationSeed,
    Fuzzy,
    HeuristicFallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryScoreTier {
    ExactId,
    ExactName,
    Lexical,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryAlternative {
    pub node_id: String,
    pub qualified_name: String,
    pub source: Option<SourceAnchor>,
    pub score: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoverySeed {
    pub node_id: String,
    pub score: String,
    pub score_tier: DiscoveryScoreTier,
    pub rank: u32,
    pub matched_terms: Vec<String>,
    pub matched_fields: Vec<String>,
    pub source: Option<SourceAnchor>,
    pub candidate_source: DiscoverySeedSource,
    pub alternatives: Vec<DiscoveryAlternative>,
    pub ambiguous: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryOmissions {
    pub candidates: Option<u64>,
    pub nodes: Option<u64>,
    pub edges: Option<u64>,
    pub expanded_relationships: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryStats {
    /// Independently bounded index probes performed by candidate recall.
    pub candidate_probes: u64,
    /// Candidate records read from all bounded recall sources before
    /// deduplication, scope filtering, and ranking.
    pub candidate_nodes: u64,
    /// Deduplicated, scoped, ranked candidates admitted to seed selection.
    pub candidates_admitted: u64,
    pub visited_nodes: u64,
    pub expanded_relationships: u64,
    pub returned_nodes: u64,
    pub returned_edges: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryQueryResponse {
    pub schema: String,
    pub question: String,
    pub selected_direction: DiscoveryDirection,
    pub direction_source: DiscoveryDirectionSource,
    pub relation_contexts: Vec<String>,
    pub scope: Vec<DiscoveryScope>,
    pub traversal: DiscoveryTraversal,
    pub seeds: Vec<DiscoverySeed>,
    pub nodes: Vec<QueryNode>,
    pub edges: Vec<DiscoveryEdge>,
    pub diagnostics: Vec<QueryDiagnostic>,
    pub limits: DiscoveryLimits,
    pub stats: DiscoveryStats,
    pub omissions: DiscoveryOmissions,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryEdge {
    pub id: Option<String>,
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    pub occurrence_rule: Option<OccurrenceRule>,
    pub relationship_site: Option<SourceAnchor>,
    pub details: Option<EdgeDetails>,
    pub evidence: Vec<QueryEvidence>,
    pub context: Option<String>,
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

#[cfg(test)]
mod discovery_contract_tests {
    use serde_json::json;

    use super::{
        DISCOVERY_QUERY_SCHEMA_V1, DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest,
        DiscoveryTraversal, MAX_DISCOVERY_CANDIDATES, MAX_DISCOVERY_DEPTH, MAX_DISCOVERY_EDGES,
        MAX_DISCOVERY_EXPANDED_RELATIONSHIPS, MAX_DISCOVERY_NODES, MAX_DISCOVERY_RESPONSE_BYTES,
        MAX_DISCOVERY_SEEDS, MAX_DISCOVERY_TIMEOUT_MS,
    };

    #[test]
    fn discovery_defaults_are_bounded_and_reuse_code_query_graph_bounds() {
        let limits = DiscoveryLimits::default();
        assert!(limits.is_valid());
        assert_eq!(limits.max_depth, 2);
        assert_eq!(limits.max_seeds, 3);
        assert_eq!(limits.max_candidates, MAX_DISCOVERY_CANDIDATES);
        assert_eq!(limits.max_nodes, MAX_DISCOVERY_NODES);
        assert_eq!(limits.max_edges, MAX_DISCOVERY_EDGES);
        assert_eq!(
            limits.max_expanded_relationships,
            MAX_DISCOVERY_EXPANDED_RELATIONSHIPS
        );
        assert_eq!(limits.max_response_bytes, MAX_DISCOVERY_RESPONSE_BYTES);
        assert_eq!(limits.timeout_ms, MAX_DISCOVERY_TIMEOUT_MS);
        assert_eq!(DISCOVERY_QUERY_SCHEMA_V1, "compass.query.discovery/1");
    }

    #[test]
    fn discovery_hard_ceilings_are_rejected_by_the_model() {
        let mut invalid = Vec::new();
        macro_rules! invalid_limit {
            ($field:ident, $value:expr) => {{
                let mut limits = DiscoveryLimits::default();
                limits.$field = $value;
                invalid.push((stringify!($field), limits));
            }};
        }
        invalid_limit!(max_depth, 0);
        invalid_limit!(max_depth, MAX_DISCOVERY_DEPTH + 1);
        invalid_limit!(max_seeds, 0);
        invalid_limit!(max_seeds, MAX_DISCOVERY_SEEDS + 1);
        invalid_limit!(max_candidates, 0);
        invalid_limit!(max_candidates, MAX_DISCOVERY_CANDIDATES + 1);
        invalid_limit!(max_nodes, 0);
        invalid_limit!(max_nodes, MAX_DISCOVERY_NODES + 1);
        invalid_limit!(max_edges, 0);
        invalid_limit!(max_edges, MAX_DISCOVERY_EDGES + 1);
        invalid_limit!(max_expanded_relationships, 0);
        invalid_limit!(
            max_expanded_relationships,
            MAX_DISCOVERY_EXPANDED_RELATIONSHIPS + 1
        );
        invalid_limit!(max_response_bytes, 0);
        invalid_limit!(max_response_bytes, MAX_DISCOVERY_RESPONSE_BYTES + 1);
        invalid_limit!(timeout_ms, 0);
        invalid_limit!(timeout_ms, MAX_DISCOVERY_TIMEOUT_MS + 1);

        for (field, limits) in invalid {
            assert!(!limits.is_valid(), "{field} unexpectedly accepted");
        }
    }

    #[test]
    fn discovery_request_rejects_unknown_fields() -> Result<(), Box<dyn std::error::Error>> {
        let value = json!({
            "question": "where is routing handled",
            "direction": "auto",
            "relationContexts": [],
            "scope": [],
            "traversal": "bfs",
            "includeHeuristic": false,
            "limits": DiscoveryLimits::default(),
            "unversionedGuess": true
        });
        assert!(serde_json::from_value::<DiscoveryQueryRequest>(value).is_err());
        Ok(())
    }

    #[test]
    fn discovery_request_defaults_are_explicit_on_decode() -> Result<(), Box<dyn std::error::Error>>
    {
        let request = serde_json::from_value::<DiscoveryQueryRequest>(json!({
            "question": "where is routing handled"
        }))?;
        assert_eq!(request.direction, DiscoveryDirection::Auto);
        assert_eq!(request.traversal, DiscoveryTraversal::Bfs);
        assert_eq!(request.limits, DiscoveryLimits::default());
        Ok(())
    }
}
