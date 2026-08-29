use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::code_graph::{
    EdgeDetails, EdgeKind, EdgeRecord, NodeDetails, NodeKind, NodeRecord, NodeRole,
};
use crate::provenance::{
    EvidenceConfidence, EvidenceOrigin, OccurrenceRule, ResolutionCandidate, ResolutionState,
    SourceAnchor,
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

pub const DISCOVERY_QUERY_SCHEMA_V1: &str = "compass.query.discovery/1";

pub const MAX_DISCOVERY_DEPTH: u32 = 8;
pub const MAX_DISCOVERY_SEEDS: u32 = 3;
pub const MAX_DISCOVERY_CANDIDATES: u32 = 256;
pub const MAX_DISCOVERY_NODES: u32 = 500;
pub const MAX_DISCOVERY_EDGES: u32 = 1_000;
pub const DEFAULT_DISCOVERY_NODES: u32 = 64;
pub const DEFAULT_DISCOVERY_EDGES: u32 = 128;
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
pub const MAX_DISCOVERY_ALTERNATIVES_PER_SEED: usize = 8;

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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
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

/// Canonicalize a discovery scope without consulting the host filesystem.
/// Source paths use `/`; package names retain language namespace separators.
#[must_use]
pub fn canonical_discovery_scope_value(kind: DiscoveryScopeKind, value: &str) -> Option<String> {
    let value = match kind {
        DiscoveryScopeKind::Source => canonical_source_scope(value),
        DiscoveryScopeKind::Package => {
            let value = value.trim().trim_matches('/');
            if value.contains('/') || value.contains('\\') {
                canonical_source_scope(value)
            } else {
                value.trim_matches('.').trim_matches(':').to_owned()
            }
        }
        DiscoveryScopeKind::Community | DiscoveryScopeKind::Node => value.trim().to_owned(),
    };
    (!value.is_empty() && !value.split('/').any(|part| part == "..")).then_some(value)
}

/// Stable scope postings as `(posting kind, requested value, canonical value)`.
/// Canonical values are community/node IDs or the normalized prefix itself.
#[must_use]
pub fn discovery_scope_postings(
    node: &crate::code_graph::NodeRecord,
) -> BTreeSet<(String, String, String)> {
    let mut postings = BTreeSet::from([
        ("node-id".to_owned(), node.id.clone(), node.id.clone()),
        (
            "node-qname".to_owned(),
            node.qualified_name.clone(),
            node.id.clone(),
        ),
    ]);
    if let Some(community) = &node.community {
        let id = community.id.to_string();
        postings.insert(("community-id".to_owned(), id.clone(), id.clone()));
        if let Some(label) = &community.label {
            postings.insert(("community-label".to_owned(), label.clone(), id));
        }
    }
    if let Some(source) = &node.source
        && let Some(source) =
            canonical_discovery_scope_value(DiscoveryScopeKind::Source, &source.file)
    {
        for prefix in slash_prefixes(&source) {
            postings.insert(("source".to_owned(), prefix.clone(), prefix.clone()));
            postings.insert(("package".to_owned(), prefix.clone(), prefix));
        }
    }
    for prefix in qname_prefixes(&node.qualified_name) {
        postings.insert(("package".to_owned(), prefix.clone(), prefix));
    }
    postings.retain(|(_, value, canonical)| {
        value.len() <= MAX_DISCOVERY_FILTER_BYTES && canonical.len() <= MAX_DISCOVERY_FILTER_BYTES
    });
    postings
}

/// Match a node against the deterministic OR-union of canonical scopes.
#[must_use]
pub fn discovery_scope_matches(
    node: &crate::code_graph::NodeRecord,
    scopes: &[DiscoveryScope],
) -> bool {
    scopes.is_empty()
        || scopes.iter().any(|scope| match scope.kind {
            DiscoveryScopeKind::Community => node
                .community
                .as_ref()
                .is_some_and(|community| community.id.to_string() == scope.value),
            DiscoveryScopeKind::Source => node.source.as_ref().is_some_and(|source| {
                canonical_source_scope(&source.file) == scope.value
                    || canonical_source_scope(&source.file)
                        .strip_prefix(&scope.value)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            }),
            DiscoveryScopeKind::Package => {
                node.qualified_name == scope.value
                    || node
                        .qualified_name
                        .strip_prefix(&scope.value)
                        .is_some_and(|suffix| {
                            suffix.starts_with("::")
                                || suffix.starts_with('.')
                                || suffix.starts_with('/')
                        })
                    || node.source.as_ref().is_some_and(|source| {
                        let source = canonical_source_scope(&source.file);
                        source == scope.value
                            || source
                                .strip_prefix(&scope.value)
                                .is_some_and(|suffix| suffix.starts_with('/'))
                    })
            }
            DiscoveryScopeKind::Node => node.id == scope.value,
        })
}

fn canonical_source_scope(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn slash_prefixes(value: &str) -> Vec<String> {
    let mut prefixes = Vec::new();
    let mut current = String::new();
    for part in value.split('/').filter(|part| !part.is_empty()) {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(part);
        prefixes.push(current.clone());
    }
    prefixes
}

fn qname_prefixes(value: &str) -> Vec<String> {
    let mut cuts = BTreeSet::from([value.len()]);
    for (index, character) in value.char_indices() {
        if matches!(character, '.' | '/' | ':') && index > 0 {
            cuts.insert(index);
        }
    }
    cuts.into_iter()
        .filter_map(|cut| value.get(..cut))
        .filter_map(|value| canonical_discovery_scope_value(DiscoveryScopeKind::Package, value))
        .collect()
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
            max_nodes: DEFAULT_DISCOVERY_NODES,
            max_edges: DEFAULT_DISCOVERY_EDGES,
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
    pub alternatives: Option<u64>,
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

pub const DISCOVERY_RESULT_ENVELOPE_SCHEMA_V1: &str = "compass.query.discovery-result/1";

/// Opt-in transport envelope for a discovery result and its query-owned digest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscoveryResultEnvelope {
    pub schema: String,
    pub result: DiscoveryQueryResponse,
    pub semantic_result_digest: String,
}

impl DiscoveryResultEnvelope {
    pub fn new(
        result: DiscoveryQueryResponse,
        semantic_result_digest: String,
    ) -> Result<Self, &'static str> {
        let envelope = Self {
            schema: DISCOVERY_RESULT_ENVELOPE_SCHEMA_V1.to_owned(),
            result,
            semantic_result_digest,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != DISCOVERY_RESULT_ENVELOPE_SCHEMA_V1 {
            return Err("unsupported discovery result envelope schema");
        }
        let Some(digest) = self.semantic_result_digest.strip_prefix("sha256:") else {
            return Err("invalid discovery semantic result digest");
        };
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("invalid discovery semantic result digest");
        }
        Ok(())
    }
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

#[cfg(test)]
mod discovery_contract_tests {
    use serde_json::json;

    use super::{
        DEFAULT_DISCOVERY_EDGES, DEFAULT_DISCOVERY_NODES, DISCOVERY_QUERY_SCHEMA_V1,
        DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
        MAX_DISCOVERY_CANDIDATES, MAX_DISCOVERY_DEPTH, MAX_DISCOVERY_EDGES,
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
        assert_eq!(limits.max_nodes, DEFAULT_DISCOVERY_NODES);
        assert_eq!(limits.max_edges, DEFAULT_DISCOVERY_EDGES);
        assert!(limits.max_nodes < MAX_DISCOVERY_NODES);
        assert!(limits.max_edges < MAX_DISCOVERY_EDGES);
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
