//! Deterministic, bounded projections for coding-agent query consumers.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use compass_model::code_graph::NodeRole;
use compass_model::provenance::{EvidenceConfidence, ResolutionState, SourceAnchor};
use compass_model::query_contract::{
    CodeQueryOperation, CodeQueryResponse, DiscoveryEdge, DiscoveryQueryResponse, QueryDiagnostic,
    QueryDiagnosticCode, QueryEdge, QueryEvidence, QueryEvidenceLayer, QueryNode, QueryPath,
};
use compass_query::{
    CursorTokenError, code_query_response_digest, decode_cursor_token, discovery_response_digest,
    encode_cursor_token, normalize_code_query_symbol,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::OutputError;

pub const AGENT_QUERY_VIEW_SCHEMA: &str = "compass.query.agent-view/1";
/// Version of the compact agent projection that omits audit-only detail.
pub const AGENT_BRIEF_VIEW_SCHEMA: &str = "compass.query.agent-view.brief/1";
pub const AGENT_VIEW_MAX_PRIMARY_RESULTS: usize = 12;
pub const AGENT_VIEW_MAX_RELATIONSHIPS: usize = 24;
pub const AGENT_VIEW_MAX_PATHS: usize = 5;
pub const AGENT_VIEW_MAX_CAVEATS: usize = 16;
pub const AGENT_VIEW_MAX_NEXT_ACTIONS: usize = 5;
pub const AGENT_VIEW_MAX_BYTES: usize = 256 * 1024;
pub const AGENT_VIEW_TEXT_MAX_BYTES: usize = 64 * 1024;
pub const AGENT_VIEW_MAX_SCALAR_CHARS: usize = 512;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOperation {
    Discovery,
    Search,
    Callers,
    Callees,
    Impact,
    Explore,
    NodeTrail,
}

impl AgentOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Search => "search",
            Self::Callers => "callers",
            Self::Callees => "callees",
            Self::Impact => "impact",
            Self::Explore => "explore",
            Self::NodeTrail => "node_trail",
        }
    }
}

impl From<CodeQueryOperation> for AgentOperation {
    fn from(operation: CodeQueryOperation) -> Self {
        match operation {
            CodeQueryOperation::Search => Self::Search,
            CodeQueryOperation::Callers => Self::Callers,
            CodeQueryOperation::Callees => Self::Callees,
            CodeQueryOperation::Impact => Self::Impact,
            CodeQueryOperation::Explore => Self::Explore,
            CodeQueryOperation::NodeTrail => Self::NodeTrail,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentQueryContext {
    pub operation: AgentOperation,
    pub question: Option<String>,
    pub operands: Vec<AgentOperand>,
    pub graph_identity: String,
    pub build_generation_identity: String,
    pub continuation_cursor: Option<String>,
    pub evidence_hidden: bool,
}

impl AgentQueryContext {
    #[must_use]
    pub fn new(
        operation: AgentOperation,
        graph_identity: impl Into<String>,
        build_generation_identity: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            question: None,
            operands: Vec::new(),
            graph_identity: graph_identity.into(),
            build_generation_identity: build_generation_identity.into(),
            continuation_cursor: None,
            evidence_hidden: false,
        }
    }

    #[must_use]
    pub fn with_question(mut self, question: impl Into<String>) -> Self {
        self.question = Some(question.into());
        self
    }

    #[must_use]
    pub fn with_operand(mut self, role: AgentOperandRole, value: impl Into<String>) -> Self {
        self.operands.push(AgentOperand {
            role,
            value: value.into(),
        });
        self
    }

    #[must_use]
    pub fn with_cursor(mut self, cursor: Option<String>) -> Self {
        self.continuation_cursor = cursor;
        self
    }

    #[must_use]
    pub const fn with_evidence_hidden(mut self, hidden: bool) -> Self {
        self.evidence_hidden = hidden;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOperandRole {
    Query,
    Symbol,
    Source,
    Target,
    Root,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentOperand {
    pub role: AgentOperandRole,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentQueryView {
    pub schema: String,
    pub request: AgentRequest,
    pub status: AgentStatus,
    pub answer: AgentAnswer,
    pub primary_results: Vec<AgentEntity>,
    pub relationships: Vec<AgentRelationship>,
    pub paths: Vec<AgentPath>,
    pub caveats: Vec<AgentCaveat>,
    pub next_actions: Vec<AgentNextAction>,
    pub omissions: AgentOmissions,
    pub identity: AgentIdentity,
    pub source_truncated: bool,
    pub projection_truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentRequest {
    pub operation: AgentOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    pub operands: Vec<AgentOperand>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResultState {
    Answered,
    Candidates,
    NeedsResolution,
    NoMatch,
    NoPath,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMatch {
    Exact,
    Fuzzy,
    Ambiguous,
    None,
    NotApplicable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvidence {
    Exact,
    Inferred,
    Mixed,
    Ambiguous,
    None,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecution {
    Complete,
    Partial,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProjection {
    Complete,
    Partial,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCoverage {
    Incomplete,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentStatus {
    pub result_state: AgentResultState,
    pub match_state: AgentMatch,
    pub evidence_state: AgentEvidence,
    pub source_execution: AgentExecution,
    pub projection: AgentProjection,
    pub coverage: AgentCoverage,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentAnswer {
    pub headline: String,
    pub basis: Vec<AgentBasis>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentBasis {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSource {
    pub file: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentEntity {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub roles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<AgentSource>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentEndpoint {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentRelationship {
    pub id: String,
    pub source: AgentEndpoint,
    pub relation: String,
    pub target: AgentEndpoint,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<AgentSource>,
    pub evidence: AgentRelationshipEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentRelationshipEvidence {
    pub confidence: String,
    pub resolution: String,
    pub layers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPathDirection {
    Forward,
    Reverse,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPathStep {
    pub from: AgentEndpoint,
    pub edge_id: String,
    pub relation: String,
    pub direction: AgentPathDirection,
    pub to: AgentEndpoint,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<AgentSource>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPath {
    pub id: String,
    pub steps: Vec<AgentPathStep>,
    pub weakest_resolution: String,
    pub weakest_confidence: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSeverity {
    Blocker,
    Warning,
    Info,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentCaveat {
    pub severity: AgentSeverity,
    pub code: String,
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentActionCli {
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentActionMcp {
    pub tool: String,
    pub arguments: Map<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentNextAction {
    pub kind: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli: Option<AgentActionCli>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<AgentActionMcp>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentOmissions {
    pub primary_results: usize,
    pub relationships: usize,
    pub paths: usize,
    pub caveats: usize,
    pub next_actions: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentIdentity {
    pub raw_schema: String,
    pub graph_identity: String,
    pub build_generation_identity: String,
    pub source_result_digest: String,
    pub view_digest: String,
}

fn invalid(reason: impl Into<String>) -> OutputError {
    OutputError::InvalidAgentQuery(reason.into())
}

/// Order response edges so direct usage survives the bounded projection.
///
/// A resolved callers or callees response may carry hundreds of owner-level
/// references beside a handful of exact call or route edges. The projection
/// cap must never drop the direct usage evidence an agent asked for, so edges
/// are ordered by relation strength first and by exact ID only as the
/// deterministic tie-break.
fn ordered_relationship_edges(response: &CodeQueryResponse) -> Vec<(usize, &QueryEdge)> {
    let mut edges = response
        .edges
        .iter()
        .enumerate()
        .collect::<Vec<(usize, &QueryEdge)>>();
    edges.sort_by(|(_, left), (_, right)| {
        relationship_priority(left)
            .cmp(&relationship_priority(right))
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.target.cmp(&right.target))
    });
    edges
}

/// Lower ranks are stronger evidence of a direct dependency.
fn relationship_priority(edge: &QueryEdge) -> u8 {
    edge.kind.dependency_strength()
}

/// Strength of the last hop of a reverse-impact trail.
fn trail_strength(
    path: &compass_model::query_contract::QueryPath,
    edge_kinds: &BTreeMap<&str, compass_model::code_graph::EdgeKind>,
) -> u8 {
    path.edge_ids
        .last()
        .and_then(|id| edge_kinds.get(id.as_str()))
        .map_or(3, |kind| kind.dependency_strength())
}

pub fn build_code_query_view(
    response: &CodeQueryResponse,
    context: AgentQueryContext,
) -> Result<AgentQueryView, OutputError> {
    if response.schema != compass_model::query_contract::CODE_QUERY_SCHEMA_V1 {
        return Err(invalid(format!(
            "unsupported raw query schema {}",
            response.schema
        )));
    }
    if context.operation == AgentOperation::Discovery {
        return Err(invalid("code query context cannot use discovery operation"));
    }
    let source_digest = format!(
        "sha256:{}",
        code_query_response_digest(response).map_err(|error| invalid(error.to_string()))?
    );
    let nodes = response
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let ordered_edges = ordered_relationship_edges(response);
    let primary_ids = primary_node_ids(
        context.operation,
        &context.operands,
        response,
        &nodes,
        &ordered_edges,
    );
    let mut primary_results = primary_ids
        .iter()
        .filter_map(|id| nodes.get(id).copied())
        .map(agent_entity)
        .collect::<Vec<_>>();
    deduplicate_entities(&mut primary_results);
    let before_primary = primary_results.len();
    primary_results.truncate(AGENT_VIEW_MAX_PRIMARY_RESULTS);
    let primary_omitted = before_primary.saturating_sub(primary_results.len());

    let all_relationships = ordered_edges
        .iter()
        .map(|(index, edge)| agent_relationship(edge, *index, &nodes))
        .collect::<Vec<_>>();
    let before_relationships = all_relationships.len();
    let relationships = all_relationships
        .into_iter()
        .take(AGENT_VIEW_MAX_RELATIONSHIPS)
        .collect::<Vec<_>>();
    let relationship_omitted = before_relationships.saturating_sub(relationships.len());

    let mut all_paths = response
        .paths
        .iter()
        .map(|path| agent_path(path, &nodes, &response.edges))
        .collect::<Vec<_>>();
    all_paths.sort_by(|left, right| left.id.cmp(&right.id));
    let before_paths = all_paths.len();
    let paths = all_paths
        .into_iter()
        .take(AGENT_VIEW_MAX_PATHS)
        .collect::<Vec<_>>();
    let path_omitted = before_paths.saturating_sub(paths.len());

    let mut all_caveats = response
        .diagnostics
        .iter()
        .map(agent_caveat)
        .collect::<Vec<_>>();
    all_caveats.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.node_id.cmp(&right.node_id))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.statement.cmp(&right.statement))
    });
    let before_caveats = all_caveats.len();
    let caveats = all_caveats
        .into_iter()
        .take(AGENT_VIEW_MAX_CAVEATS)
        .collect::<Vec<_>>();
    let caveat_omitted = before_caveats.saturating_sub(caveats.len());

    let source_truncated = response.truncated
        || response
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::BoundedTruncation);
    let match_state = code_match_state(&context, response, &nodes);
    let result_state = code_result_state(context.operation, match_state, response);
    let evidence_state = evidence_state(
        response
            .nodes
            .iter()
            .flat_map(|node| node.evidence.iter())
            .chain(response.edges.iter().flat_map(|edge| edge.evidence.iter())),
    );
    let coverage = if has_diagnostic(
        response.diagnostics.as_slice(),
        QueryDiagnosticCode::IncompleteCoverage,
    ) {
        AgentCoverage::Incomplete
    } else {
        AgentCoverage::Unknown
    };
    let answer = answer_for_code(
        &context,
        result_state,
        match_state,
        response,
        &primary_results,
        &relationships,
        &paths,
    );
    let next_actions = next_actions_for_code(
        &context,
        &primary_results,
        &caveats,
        source_truncated,
        &response.paths,
    );
    let mut view = AgentQueryView {
        schema: AGENT_QUERY_VIEW_SCHEMA.to_owned(),
        request: AgentRequest {
            operation: context.operation,
            question: context.question,
            operands: context.operands,
        },
        status: AgentStatus {
            result_state,
            match_state,
            evidence_state,
            source_execution: if source_truncated {
                AgentExecution::Partial
            } else {
                AgentExecution::Complete
            },
            projection: AgentProjection::Complete,
            coverage,
        },
        answer,
        primary_results,
        relationships,
        paths,
        caveats,
        next_actions,
        omissions: AgentOmissions {
            primary_results: primary_omitted,
            relationships: relationship_omitted,
            paths: path_omitted,
            caveats: caveat_omitted,
            next_actions: 0,
            total: primary_omitted + relationship_omitted + path_omitted + caveat_omitted,
        },
        identity: AgentIdentity {
            raw_schema: response.schema.clone(),
            graph_identity: context.graph_identity,
            build_generation_identity: context.build_generation_identity,
            source_result_digest: source_digest,
            view_digest: String::new(),
        },
        source_truncated,
        projection_truncated: false,
    };
    let before_actions = view.next_actions.len();
    view.next_actions.truncate(AGENT_VIEW_MAX_NEXT_ACTIONS);
    view.omissions.next_actions = before_actions.saturating_sub(view.next_actions.len());
    view.omissions.total = view
        .omissions
        .primary_results
        .saturating_add(view.omissions.relationships)
        .saturating_add(view.omissions.paths)
        .saturating_add(view.omissions.caveats)
        .saturating_add(view.omissions.next_actions);
    view.projection_truncated = view.omissions.total > 0;
    view.status.projection = if view.projection_truncated {
        AgentProjection::Partial
    } else {
        AgentProjection::Complete
    };
    finish_view(view)
}

pub fn build_discovery_query_view(
    response: &DiscoveryQueryResponse,
    context: AgentQueryContext,
) -> Result<AgentQueryView, OutputError> {
    if response.schema != compass_model::query_contract::DISCOVERY_QUERY_SCHEMA_V1 {
        return Err(invalid(format!(
            "unsupported raw discovery schema {}",
            response.schema
        )));
    }
    if context.operation != AgentOperation::Discovery {
        return Err(invalid(
            "discovery query context must use discovery operation",
        ));
    }
    let source_digest = format!(
        "sha256:{}",
        discovery_response_digest(response).map_err(|error| invalid(error.to_string()))?
    );
    let nodes = response
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let seed_ids = response
        .seeds
        .iter()
        .map(|seed| seed.node_id.clone())
        .collect::<Vec<_>>();
    let seed_set = seed_ids.iter().cloned().collect::<HashSet<_>>();
    let primary_ids = seed_ids
        .into_iter()
        .chain(nodes.keys().filter(|id| !seed_set.contains(*id)).cloned())
        .collect::<Vec<_>>();
    let mut primary_results = primary_ids
        .iter()
        .filter_map(|id| nodes.get(id).copied())
        .map(agent_entity)
        .collect::<Vec<_>>();
    deduplicate_entities(&mut primary_results);
    let before_primary = primary_results.len();
    primary_results.truncate(AGENT_VIEW_MAX_PRIMARY_RESULTS);
    let primary_omitted = before_primary.saturating_sub(primary_results.len());

    let mut all_relationships = response
        .edges
        .iter()
        .enumerate()
        .map(|(index, edge)| agent_discovery_relationship(edge, index, &nodes))
        .collect::<Vec<_>>();
    all_relationships.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.source.id.cmp(&right.source.id))
            .then_with(|| left.target.id.cmp(&right.target.id))
    });
    let before_relationships = all_relationships.len();
    let relationships = all_relationships
        .into_iter()
        .take(AGENT_VIEW_MAX_RELATIONSHIPS)
        .collect::<Vec<_>>();
    let relationship_omitted = before_relationships.saturating_sub(relationships.len());
    let mut all_caveats = response
        .diagnostics
        .iter()
        .map(agent_caveat)
        .collect::<Vec<_>>();
    all_caveats.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.node_id.cmp(&right.node_id))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.statement.cmp(&right.statement))
    });
    let before_caveats = all_caveats.len();
    let caveats = all_caveats
        .into_iter()
        .take(AGENT_VIEW_MAX_CAVEATS)
        .collect::<Vec<_>>();
    let caveat_omitted = before_caveats.saturating_sub(caveats.len());
    let ambiguous = response.seeds.iter().any(|seed| seed.ambiguous)
        || has_diagnostic(
            response.diagnostics.as_slice(),
            QueryDiagnosticCode::AmbiguousMatch,
        );
    let no_match = has_diagnostic(
        response.diagnostics.as_slice(),
        QueryDiagnosticCode::NoMatch,
    );
    let match_state = if ambiguous {
        AgentMatch::Ambiguous
    } else if no_match {
        AgentMatch::None
    } else {
        AgentMatch::Fuzzy
    };
    let result_state = if ambiguous {
        AgentResultState::NeedsResolution
    } else if no_match {
        AgentResultState::NoMatch
    } else {
        AgentResultState::Candidates
    };
    let source_truncated = response.truncated
        || has_diagnostic(
            response.diagnostics.as_slice(),
            QueryDiagnosticCode::BoundedTruncation,
        );
    let evidence_state = evidence_state(
        response
            .nodes
            .iter()
            .flat_map(|node| node.evidence.iter())
            .chain(response.edges.iter().flat_map(|edge| edge.evidence.iter())),
    );
    let coverage = if has_diagnostic(
        response.diagnostics.as_slice(),
        QueryDiagnosticCode::IncompleteCoverage,
    ) {
        AgentCoverage::Incomplete
    } else {
        AgentCoverage::Unknown
    };
    let answer = answer_for_discovery(
        result_state,
        &response.question,
        response.seeds.len(),
        relationships.len(),
        &primary_results,
    );
    let next_actions = next_actions_for_discovery(&context, &primary_results, source_truncated);
    let mut view = AgentQueryView {
        schema: AGENT_QUERY_VIEW_SCHEMA.to_owned(),
        request: AgentRequest {
            operation: AgentOperation::Discovery,
            question: context.question.or_else(|| Some(response.question.clone())),
            operands: context.operands,
        },
        status: AgentStatus {
            result_state,
            match_state,
            evidence_state,
            source_execution: if source_truncated {
                AgentExecution::Partial
            } else {
                AgentExecution::Complete
            },
            projection: AgentProjection::Complete,
            coverage,
        },
        answer,
        primary_results,
        relationships,
        paths: Vec::new(),
        caveats,
        next_actions,
        omissions: AgentOmissions {
            primary_results: primary_omitted,
            relationships: relationship_omitted,
            paths: 0,
            caveats: caveat_omitted,
            next_actions: 0,
            total: primary_omitted + relationship_omitted + caveat_omitted,
        },
        identity: AgentIdentity {
            raw_schema: response.schema.clone(),
            graph_identity: context.graph_identity,
            build_generation_identity: context.build_generation_identity,
            source_result_digest: source_digest,
            view_digest: String::new(),
        },
        source_truncated,
        projection_truncated: false,
    };
    let before_actions = view.next_actions.len();
    view.next_actions.truncate(AGENT_VIEW_MAX_NEXT_ACTIONS);
    view.omissions.next_actions = before_actions.saturating_sub(view.next_actions.len());
    view.omissions.total = view
        .omissions
        .primary_results
        .saturating_add(view.omissions.relationships)
        .saturating_add(view.omissions.paths)
        .saturating_add(view.omissions.caveats)
        .saturating_add(view.omissions.next_actions);
    view.projection_truncated = view.omissions.total > 0;
    view.status.projection = if view.projection_truncated {
        AgentProjection::Partial
    } else {
        AgentProjection::Complete
    };
    finish_view(view)
}

fn finish_view(mut view: AgentQueryView) -> Result<AgentQueryView, OutputError> {
    if view.omissions.total > 0 {
        view.projection_truncated = true;
    }
    view.identity.view_digest = view_digest(&view)?;
    view.validate()?;
    let bytes = serde_json::to_vec(&view).map_err(|error| invalid(error.to_string()))?;
    if bytes.len() > AGENT_VIEW_MAX_BYTES {
        return Err(invalid(format!(
            "serialized view is {} bytes; limit is {}",
            bytes.len(),
            AGENT_VIEW_MAX_BYTES
        )));
    }
    Ok(view)
}

impl AgentQueryView {
    pub fn from_json(bytes: &[u8]) -> Result<Self, OutputError> {
        let view: Self =
            serde_json::from_slice(bytes).map_err(|error| invalid(error.to_string()))?;
        view.validate()?;
        Ok(view)
    }

    pub fn validate(&self) -> Result<(), OutputError> {
        if self.schema != AGENT_QUERY_VIEW_SCHEMA {
            return Err(invalid(format!(
                "unsupported Agent View schema {}",
                self.schema
            )));
        }
        for (name, value) in [
            ("graph identity", self.identity.graph_identity.as_str()),
            (
                "build generation identity",
                self.identity.build_generation_identity.as_str(),
            ),
            (
                "source result digest",
                self.identity.source_result_digest.as_str(),
            ),
            ("view digest", self.identity.view_digest.as_str()),
        ] {
            if value.is_empty() {
                return Err(invalid(format!("{name} must not be empty")));
            }
        }
        if !valid_sha256(&self.identity.source_result_digest)
            || !valid_sha256(&self.identity.view_digest)
        {
            return Err(invalid(
                "Agent View digests must be sha256:<64 lowercase hex>",
            ));
        }
        if self.primary_results.len() > AGENT_VIEW_MAX_PRIMARY_RESULTS
            || self.relationships.len() > AGENT_VIEW_MAX_RELATIONSHIPS
            || self.paths.len() > AGENT_VIEW_MAX_PATHS
            || self.caveats.len() > AGENT_VIEW_MAX_CAVEATS
            || self.next_actions.len() > AGENT_VIEW_MAX_NEXT_ACTIONS
        {
            return Err(invalid("Agent View item bound exceeded"));
        }
        if self.status.source_execution == AgentExecution::Complete && self.source_truncated {
            return Err(invalid("complete execution cannot be source-truncated"));
        }
        if self.status.projection == AgentProjection::Complete
            && (self.projection_truncated || self.omissions.total > 0)
        {
            return Err(invalid("complete projection cannot contain omissions"));
        }
        if self.status.result_state == AgentResultState::NoMatch
            && !self.caveats.iter().any(|caveat| caveat.code == "no_match")
        {
            return Err(invalid("no_match requires a no_match caveat"));
        }
        if self.status.result_state == AgentResultState::NeedsResolution
            && self.primary_results.len() < 2
            && self.omissions.primary_results == 0
            && !self
                .caveats
                .iter()
                .any(|caveat| caveat.code == "ambiguous_match")
        {
            return Err(invalid(
                "needs_resolution requires two retained candidates or an omission",
            ));
        }
        if self.answer.headline.is_empty() || self.answer.basis.is_empty() {
            return Err(invalid("answer headline and basis are required"));
        }
        let primary_ids = self
            .primary_results
            .iter()
            .map(|entity| entity.id.as_str())
            .collect::<HashSet<_>>();
        let relationship_ids = self
            .relationships
            .iter()
            .map(|relationship| relationship.id.as_str())
            .collect::<HashSet<_>>();
        let path_ids = self
            .paths
            .iter()
            .map(|path| path.id.as_str())
            .collect::<HashSet<_>>();
        for basis in &self.answer.basis {
            if basis.id.is_empty() {
                return Err(invalid("answer basis IDs are required"));
            }
            let valid = match basis.kind.as_str() {
                "node" => primary_ids.contains(basis.id.as_str()),
                "relationship" => relationship_ids.contains(basis.id.as_str()),
                "path" => path_ids.contains(basis.id.as_str()),
                "operation" => basis.id == self.request.operation.label(),
                _ => false,
            };
            if !valid {
                return Err(invalid(format!(
                    "answer basis {} does not reference a retained result",
                    basis.id
                )));
            }
        }
        if self.status.result_state == AgentResultState::Answered
            && matches!(
                self.status.match_state,
                AgentMatch::Ambiguous | AgentMatch::None
            )
        {
            return Err(invalid(
                "answered cannot have ambiguous or none match state",
            ));
        }
        for entity in &self.primary_results {
            if entity.id.is_empty() || entity.label.is_empty() {
                return Err(invalid("Agent entity IDs and labels are required"));
            }
        }
        for relationship in &self.relationships {
            validate_endpoint(&relationship.source)?;
            validate_endpoint(&relationship.target)?;
        }
        for path in &self.paths {
            if path.id.is_empty() {
                return Err(invalid("path IDs are required"));
            }
            for (index, step) in path.steps.iter().enumerate() {
                validate_endpoint(&step.from)?;
                validate_endpoint(&step.to)?;
                if step.edge_id.is_empty() || step.relation.is_empty() {
                    return Err(invalid("path edge IDs and relations are required"));
                }
                if index > 0 && path.steps[index - 1].to.id != step.from.id {
                    return Err(invalid("path steps must form a connected trail"));
                }
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|error| invalid(error.to_string()))?;
        if bytes.len() > AGENT_VIEW_MAX_BYTES {
            return Err(invalid(format!(
                "serialized view is {} bytes; limit is {}",
                bytes.len(),
                AGENT_VIEW_MAX_BYTES
            )));
        }
        if view_digest(self)? != self.identity.view_digest {
            return Err(invalid("Agent View digest does not match its contents"));
        }
        Ok(())
    }
}

fn validate_endpoint(endpoint: &AgentEndpoint) -> Result<(), OutputError> {
    if endpoint.id.is_empty() || endpoint.label.is_empty() {
        return Err(invalid("relationship endpoint IDs and labels are required"));
    }
    Ok(())
}

pub fn render_agent_query_header_lines(view: &AgentQueryView) -> Result<Vec<String>, OutputError> {
    view.validate()?;
    let mut lines = render_result_lines(view);
    lines.push(String::new());
    lines.push("ANSWER".to_owned());
    lines.push(escape_scalar(&view.answer.headline));
    if !view.caveats.is_empty() {
        lines.push(String::new());
        lines.push("CAVEATS".to_owned());
        // One caveat can be retained twice under the same code - the same
        // truncated sentence is the same warning, so it is stated once.
        let mut seen = HashSet::new();
        lines.extend(
            view.caveats
                .iter()
                .map(render_caveat)
                .filter(|line| seen.insert(line.clone())),
        );
    }
    Ok(lines)
}

pub fn render_agent_query_text(view: &AgentQueryView) -> Result<String, OutputError> {
    let mut lines = render_agent_query_header_lines(view)?;
    lines.push(String::new());
    lines.push("PRIMARY RESULTS".to_owned());
    if view.primary_results.is_empty() {
        lines.push("- None retained.".to_owned());
    } else {
        let ambiguous_labels = duplicated_entity_labels(&view.primary_results);
        lines.extend(view.primary_results.iter().map(|entity| {
            render_entity(
                entity,
                entity_needs_identity(entity, view.status.match_state, &ambiguous_labels),
            )
        }));
    }
    lines.push(String::new());
    lines.push("PATHS".to_owned());
    if view.paths.is_empty() {
        lines.push("- None retained.".to_owned());
    } else {
        lines.extend(view.paths.iter().map(render_path));
    }
    lines.push(String::new());
    lines.push("RELATIONSHIPS".to_owned());
    if view.relationships.is_empty() {
        lines.push("- None retained.".to_owned());
    } else {
        lines.extend(view.relationships.iter().map(render_relationship));
    }
    lines.push(String::new());
    lines.push("NEXT ACTIONS".to_owned());
    if view.next_actions.is_empty() {
        lines.push("- None.".to_owned());
    } else {
        lines.extend(view.next_actions.iter().map(render_action));
    }
    lines.push(String::new());
    lines.push("DETAILS".to_owned());
    lines.push(format!(
        "{} primary result(s) · {} relationship(s) · {} path(s)",
        view.primary_results.len(),
        view.relationships.len(),
        view.paths.len()
    ));
    if view.omissions.total > 0 {
        lines.push(format!(
            "{} record(s) omitted by the Agent View bound; raw evidence is unchanged.",
            view.omissions.total
        ));
    }
    lines.push("Full provenance is available in the raw JSON/evidence view.".to_owned());
    let text = lines.join("\n");
    if text.len() > AGENT_VIEW_TEXT_MAX_BYTES {
        return Err(OutputError::AgentQueryTextBudgetExceeded {
            rendered_bytes: text.len(),
            limit: AGENT_VIEW_TEXT_MAX_BYTES,
        });
    }
    Ok(text)
}

/// The compact agent projection.
///
/// The brief view keeps the reviewed answer semantics of
/// `compass.query.agent-view/1` - status, caveats, source-located entities,
/// relationships, paths, and next actions - while dropping audit-only detail
/// such as graph identities, response digests, per-record IDs for
/// relationships, and per-edge evidence layers. Consumers that need exact
/// provenance read the raw `compass.query/1` response instead.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentBriefView {
    pub schema: String,
    pub operation: AgentOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    pub operands: Vec<AgentOperand>,
    pub status: AgentBriefStatus,
    pub answer: String,
    pub caveats: Vec<String>,
    pub primary_results: Vec<AgentBriefEntity>,
    pub relationships: Vec<AgentBriefRelationship>,
    pub paths: Vec<AgentBriefPath>,
    pub next_actions: Vec<AgentBriefAction>,
}

/// Answer-level state of a brief view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentBriefStatus {
    pub result_state: String,
    pub match_state: String,
    pub coverage: String,
}

/// One source-located entity in a brief view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentBriefEntity {
    pub id: String,
    pub label: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// One usage or structural relationship in a brief view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentBriefRelationship {
    pub source: String,
    pub relation: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    pub confidence: String,
}

/// One connecting path in a brief view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentBriefPath {
    pub id: String,
    pub hops: usize,
    pub summary: String,
}

/// One bounded follow-up action in a brief view.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentBriefAction {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli: Option<Vec<String>>,
}

/// Build the compact projection from the same bounded agent view.
pub fn build_code_query_brief(
    response: &CodeQueryResponse,
    context: AgentQueryContext,
) -> Result<AgentBriefView, OutputError> {
    let view = build_code_query_view(response, context)?;
    Ok(AgentBriefView {
        schema: AGENT_BRIEF_VIEW_SCHEMA.to_owned(),
        operation: view.request.operation,
        question: view.request.question.clone(),
        operands: view.request.operands.clone(),
        status: AgentBriefStatus {
            result_state: result_state_name(view.status.result_state).to_owned(),
            match_state: match_state_name(view.status.match_state).to_owned(),
            coverage: coverage_state_name(view.status.coverage).to_owned(),
        },
        answer: view.answer.headline.clone(),
        caveats: view
            .caveats
            .iter()
            .map(|caveat| {
                format!(
                    "[{}] {}: {}",
                    severity_name(caveat.severity),
                    caveat.code,
                    caveat.statement
                )
            })
            .collect(),
        primary_results: view
            .primary_results
            .iter()
            .map(|entity| AgentBriefEntity {
                id: entity.id.clone(),
                label: entity.label.clone(),
                kind: entity.kind.clone(),
                source: entity.source.as_ref().map(render_source),
            })
            .collect(),
        relationships: view
            .relationships
            .iter()
            .map(|relationship| AgentBriefRelationship {
                source: relationship.source.label.clone(),
                relation: relationship.relation.clone(),
                target: relationship.target.label.clone(),
                site: relationship.site.as_ref().map(render_source),
                confidence: relationship.evidence.confidence.clone(),
            })
            .collect(),
        paths: view
            .paths
            .iter()
            .map(|path| AgentBriefPath {
                id: path.id.clone(),
                hops: path.steps.len(),
                summary: render_path_summary(path),
            })
            .collect(),
        next_actions: view
            .next_actions
            .iter()
            .map(|action| AgentBriefAction {
                kind: action.kind.clone(),
                cli: action.cli.as_ref().map(|cli| cli.argv.clone()),
            })
            .collect(),
    })
}

/// Version of the paged agent text projection.
pub const AGENT_TEXT_PAGE_VERSION: &str = "compass.query.agent-text-page/1";
/// Default page budget, in approximate tokens, for paged agent text output.
pub const DEFAULT_AGENT_TEXT_PAGE_TOKENS: usize = 2_000;

const AGENT_TEXT_PAGE_FOOTER_RESERVE_CHARS: usize = 384;
const AGENT_TEXT_PAGE_MAX_CURSOR_BYTES: usize = 4_096;

/// Continuation state for one paged agent text response.
///
/// The cursor binds the page to the reviewed prefix of the deterministic
/// agent-view ledger. A later page re-runs the same query with wider internal
/// bounds, so the decoded cursor carries only the page number and the verified
/// prefix identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTextPageCursor {
    pub version: String,
    pub operation: String,
    pub graph_identity: String,
    pub page: u32,
    pub prefix_count: usize,
    pub prefix_digest: String,
}

/// Cursor wire version. Older encodings fail with an explicit version error.
const AGENT_TEXT_PAGE_CURSOR_WIRE_VERSION: u8 = 2;
/// Hex characters stored for each digest the cursor binds.
const AGENT_TEXT_PAGE_CURSOR_DIGEST_CHARS: usize = 16;
/// Fields this cursor carries behind its wire version.
const AGENT_TEXT_PAGE_CURSOR_FIELDS: usize = 5;

fn cursor_digest_prefix(value: &str) -> String {
    value
        .chars()
        .take(AGENT_TEXT_PAGE_CURSOR_DIGEST_CHARS)
        .collect()
}

/// Encode one cursor as the versioned field list plus its payload checksum.
fn encode_cursor_fields(cursor: &AgentTextPageCursor) -> String {
    encode_cursor_token(
        AGENT_TEXT_PAGE_CURSOR_WIRE_VERSION,
        &[
            &cursor.operation,
            &cursor_digest_prefix(&cursor.graph_identity),
            &cursor.page.to_string(),
            &cursor.prefix_count.to_string(),
            &cursor_digest_prefix(&cursor.prefix_digest),
        ],
    )
}

/// Decode the field list back into a continuation cursor.
///
/// Every field is re-validated here - the operation label, the hex digests, and
/// the bounded integers - so a cursor that was not produced by
/// [`encode_cursor_fields`] fails as an unreadable cursor rather than as a
/// silently reinterpreted field.
fn decode_cursor_fields(payload: &str) -> Result<AgentTextPageCursor, OutputError> {
    let fields = decode_cursor_token(
        payload,
        AGENT_TEXT_PAGE_CURSOR_WIRE_VERSION,
        AGENT_TEXT_PAGE_CURSOR_FIELDS,
    )
    .map_err(|error| match error {
        CursorTokenError::UnsupportedVersion => OutputError::InvalidAgentTextPage(
            "unsupported page cursor version; reissue the page query".to_owned(),
        ),
        CursorTokenError::MissingChecksum
        | CursorTokenError::ChecksumMismatch
        | CursorTokenError::Malformed => unreadable_cursor(),
    })?;
    let [operation, graph_identity, page, prefix_count, prefix_digest] =
        <[String; AGENT_TEXT_PAGE_CURSOR_FIELDS]>::try_from(fields)
            .map_err(|_| unreadable_cursor())?;
    let operation = operation
        .chars()
        .all(|character| character.is_ascii_lowercase() || character == '_')
        .then_some(operation)
        .filter(|value| !value.is_empty())
        .ok_or_else(unreadable_cursor)?;
    let page = page.parse::<u32>().map_err(|_| unreadable_cursor())?;
    let prefix_count = prefix_count
        .parse::<usize>()
        .map_err(|_| unreadable_cursor())?;
    if page == 0 {
        return Err(OutputError::InvalidAgentTextPage(
            "cursor page must be greater than zero".to_owned(),
        ));
    }
    if !is_digest_prefix(&prefix_digest) {
        return Err(unreadable_cursor());
    }
    Ok(AgentTextPageCursor {
        version: AGENT_TEXT_PAGE_VERSION.to_owned(),
        operation,
        graph_identity,
        page,
        prefix_count,
        prefix_digest,
    })
}

fn is_digest_prefix(value: &str) -> bool {
    compass_query::is_cursor_digest(value, AGENT_TEXT_PAGE_CURSOR_DIGEST_CHARS)
}

fn unreadable_cursor() -> OutputError {
    OutputError::InvalidAgentTextPage(
        "unsupported page cursor version; reissue the page query".to_owned(),
    )
}

/// Options for one paged agent text response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentTextPageOptions<'a> {
    pub token_budget: usize,
    pub cursor: Option<&'a str>,
}

/// One bounded page of agent text with an optional continuation cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTextPage {
    pub text: String,
    pub next_cursor: Option<String>,
    pub page: u32,
    pub entry_start: usize,
    pub entry_end: usize,
    pub entry_total: usize,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TextPageSection {
    PrimaryResults,
    Paths,
    Relationships,
    Source,
}

impl TextPageSection {
    fn heading(self) -> &'static str {
        match self {
            Self::PrimaryResults => "PRIMARY RESULTS",
            Self::Paths => "PATHS",
            Self::Relationships => "RELATIONSHIPS",
            Self::Source => "SOURCE",
        }
    }

    /// Most entries of this section one text page renders.
    ///
    /// The text page keeps the same profile the Agent View documents, so a page
    /// stops at the answer's strongest evidence instead of spending the whole
    /// token budget on the tail of a long relation list. The pagination line
    /// still reports the ledger's true total and `next=` continues it.
    fn max_per_page(self) -> usize {
        match self {
            Self::PrimaryResults => AGENT_VIEW_MAX_PRIMARY_RESULTS,
            Self::Paths => AGENT_VIEW_MAX_PATHS,
            Self::Relationships => AGENT_VIEW_MAX_RELATIONSHIPS,
            // Source blocks are already bounded per anchor and by the budget.
            Self::Source => usize::MAX,
        }
    }
}

/// Decode and verify the self-contained envelope of one page cursor.
///
/// The ledger digest is verified when the page is rendered; this decoder only
/// proves that the token is intact and readable so a caller can learn which
/// page the token continues.
pub fn decode_agent_text_page_cursor(value: &str) -> Result<AgentTextPageCursor, OutputError> {
    if value.len() > AGENT_TEXT_PAGE_MAX_CURSOR_BYTES {
        return Err(OutputError::InvalidAgentTextPage(
            "cursor exceeds the supported size".to_owned(),
        ));
    }
    decode_cursor_fields(value)
}

/// Render one bounded page of agent text and its continuation cursor.
///
/// The ledger is derived from the authoritative query response rather than the
/// capped agent view, so paging can reach records that the view's own bounds
/// omit. The page keeps at least one entry so a pathological budget cannot
/// return an empty page, and a continuation cursor only becomes valid when the
/// next response reproduces the same reviewed prefix.
pub fn render_code_query_text_page(
    response: &CodeQueryResponse,
    context: AgentQueryContext,
    options: AgentTextPageOptions<'_>,
) -> Result<AgentTextPage, OutputError> {
    if options.token_budget == 0 {
        return Err(OutputError::InvalidAgentTextPage(
            "token budget must be greater than zero".to_owned(),
        ));
    }
    let view = build_code_query_view(response, context)?;
    view.validate()?;
    let entries = text_page_entries(
        response,
        view.request.operation,
        &view.request.operands,
        view.status.match_state,
    );
    let (page, start) = match options.cursor {
        None => (1_u32, 0_usize),
        Some(cursor) => {
            let envelope = decode_agent_text_page_cursor(cursor)?;
            if envelope.operation != view.request.operation.label() {
                return Err(OutputError::InvalidAgentTextPage(format!(
                    "cursor continues operation {} but this response is {}",
                    envelope.operation,
                    view.request.operation.label()
                )));
            }
            if !view
                .identity
                .graph_identity
                .starts_with(&envelope.graph_identity)
            {
                return Err(OutputError::InvalidAgentTextPage(
                    "cursor belongs to a different graph identity".to_owned(),
                ));
            }
            if envelope.prefix_count > entries.len() {
                return Err(OutputError::InvalidAgentTextPage(
                    "cursor prefix is longer than the retained ledger".to_owned(),
                ));
            }
            if !prefix_digest(&entries[..envelope.prefix_count])
                .starts_with(&envelope.prefix_digest)
            {
                return Err(OutputError::InvalidAgentTextPage(
                    "cursor prefix does not match the current response".to_owned(),
                ));
            }
            (envelope.page, envelope.prefix_count)
        }
    };
    let max_chars = options.token_budget.saturating_mul(4);
    let header = render_agent_query_header_lines(&view)?;
    let header_chars = rendered_chars(&header);
    let mut end = start;
    let mut used = header_chars.saturating_add(AGENT_TEXT_PAGE_FOOTER_RESERVE_CHARS);
    let mut rendered_per_section: BTreeMap<TextPageSection, usize> = BTreeMap::new();
    while end < entries.len() {
        let (section, text) = &entries[end];
        if rendered_per_section.get(section).copied().unwrap_or(0) >= section.max_per_page() {
            break;
        }
        let mut cost = text.chars().count().saturating_add(1);
        if end == start || Some(*section) != entries.get(end.saturating_sub(1)).map(|entry| entry.0)
        {
            cost = cost.saturating_add(section.heading().chars().count() + 2);
        }
        if used.saturating_add(cost) > max_chars && end > start {
            break;
        }
        used = used.saturating_add(cost);
        *rendered_per_section.entry(*section).or_default() += 1;
        end += 1;
    }
    loop {
        let next_cursor = if end < entries.len() {
            Some(encode_agent_text_page_cursor(&AgentTextPageCursor {
                version: AGENT_TEXT_PAGE_VERSION.to_owned(),
                operation: view.request.operation.label().to_owned(),
                graph_identity: view.identity.graph_identity.clone(),
                page: page.saturating_add(1),
                prefix_count: end,
                prefix_digest: prefix_digest(&entries[..end]),
            })?)
        } else {
            None
        };
        let text = render_agent_text_page_body(
            &view,
            response,
            &header,
            &entries,
            start,
            end,
            page,
            next_cursor.as_deref(),
        );
        if text.chars().count() <= max_chars || end <= start.saturating_add(1) {
            return Ok(AgentTextPage {
                text,
                next_cursor,
                page,
                entry_start: start,
                entry_end: end,
                entry_total: entries.len(),
            });
        }
        end = end.saturating_sub(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_agent_text_page_body(
    view: &AgentQueryView,
    response: &CodeQueryResponse,
    header: &[String],
    entries: &[(TextPageSection, String)],
    start: usize,
    end: usize,
    page: u32,
    next_cursor: Option<&str>,
) -> String {
    let mut lines = if page > 1 {
        continuation_header(view, header)
    } else {
        header.to_vec()
    };
    let mut section: Option<TextPageSection> = None;
    for (entry_section, text) in &entries[start..end] {
        if section != Some(*entry_section) {
            lines.push(String::new());
            lines.push(entry_section.heading().to_owned());
            section = Some(*entry_section);
        }
        lines.push(text.clone());
    }
    lines.push(String::new());
    lines.push(format!(
        "Pagination: page={page} range={}-{} of {} next={}",
        if end > start { start + 1 } else { 0 },
        end,
        entries.len(),
        next_cursor.unwrap_or("none")
    ));
    if view.omissions.total > 0 || response.truncated {
        lines.push(if response.truncated {
            format!(
                "Bound: {} record(s) beyond this page; next= continues, --max-nodes/--max-edges widens the query bound.",
                view.omissions.total
            )
        } else {
            format!(
                "Bound: {} record(s) beyond this page; next= continues.",
                view.omissions.total
            )
        });
    }
    lines.join("\n")
}

/// Compact follow-up header: keep the state and answer, summarize caveats.
///
/// Page one states every caveat in full. A continuation page repeats the same
/// immutable result, so re-printing paragraphs of caveat text would spend the
/// page budget without adding information.
fn continuation_header(view: &AgentQueryView, header: &[String]) -> Vec<String> {
    let mut kept = Vec::new();
    for line in header {
        if line == "CAVEATS" {
            if view.caveats.is_empty() {
                continue;
            }
            let mut counts = BTreeMap::<&str, usize>::new();
            for caveat in &view.caveats {
                *counts.entry(caveat.code.as_str()).or_default() += 1;
            }
            let codes = counts
                .iter()
                .map(|(code, count)| format!("{code}×{count}"))
                .collect::<Vec<_>>();
            kept.push(format!(
                "CAVEATS: {} unchanged from page 1 ({})",
                view.caveats.len(),
                codes.join(", ")
            ));
            break;
        }
        kept.push(line.clone());
    }
    kept
}

/// Build the compact header for a continuation page.
///
/// Callers that pre-render a page prefix (the discovery text pager) use this
/// instead of [`render_agent_query_header_lines`] whenever a cursor continues
/// an immutable result, so a page budget is not spent re-printing caveat
/// paragraphs that page one already stated in full.
pub fn render_agent_query_continuation_header(
    view: &AgentQueryView,
) -> Result<Vec<String>, OutputError> {
    let header = render_agent_query_header_lines(view)?;
    Ok(continuation_header(view, &header))
}

fn text_page_entries(
    response: &CodeQueryResponse,
    operation: AgentOperation,
    operands: &[AgentOperand],
    match_state: AgentMatch,
) -> Vec<(TextPageSection, String)> {
    let nodes = response
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let ordered_edges = ordered_relationship_edges(response);
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let primary_entities = primary_node_ids(operation, operands, response, &nodes, &ordered_edges)
        .iter()
        .filter(|id| seen.insert((*id).clone()))
        .filter_map(|id| nodes.get(id).copied())
        .map(agent_entity)
        .collect::<Vec<_>>();
    let ambiguous_labels = duplicated_entity_labels(&primary_entities);
    entries.extend(primary_entities.iter().map(|entity| {
        (
            TextPageSection::PrimaryResults,
            render_entity(
                entity,
                entity_needs_identity(entity, match_state, &ambiguous_labels),
            ),
        )
    }));
    let mut paths = response
        .paths
        .iter()
        .map(|path| (path.id.clone(), agent_path(path, &nodes, &response.edges)))
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    entries.extend(
        paths
            .iter()
            .map(|(_, path)| (TextPageSection::Paths, render_path(path))),
    );
    let relationships = ordered_edges
        .iter()
        .map(|(index, edge)| {
            let relationship = agent_relationship(edge, *index, &nodes);
            (relationship.id.clone(), relationship)
        })
        .collect::<Vec<_>>();
    entries.extend(relationships.iter().map(|(_, relationship)| {
        (
            TextPageSection::Relationships,
            render_relationship(relationship),
        )
    }));
    entries.extend(source_context_entries(response, &nodes, &ordered_edges));
    entries
}

/// Bound for one rendered source block in a paged map.
const MAP_SOURCE_BLOCK_CHARS: usize = 2_000;

/// Render digest-verified source blocks for the map's primary anchors.
///
/// `explore` verifies the anchored files into the response; a bounded text map
/// is only useful when that context is visible, so each primary anchor gets the
/// recorded line range of its declaring file. Missing or stale source is
/// skipped here and stays visible through the response's file records.
fn source_context_entries(
    response: &CodeQueryResponse,
    nodes: &BTreeMap<String, &QueryNode>,
    ordered_edges: &[(usize, &QueryEdge)],
) -> Vec<(TextPageSection, String)> {
    if response.files.is_empty() {
        return Vec::new();
    }
    let files = response
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    let operands = response
        .nodes
        .iter()
        .filter_map(|node| {
            node.source.as_ref().map(|_| AgentOperand {
                role: AgentOperandRole::Symbol,
                value: node.id.clone(),
            })
        })
        .collect::<Vec<_>>();
    let mut anchors = primary_node_ids(
        AgentOperation::Explore,
        &operands,
        response,
        nodes,
        ordered_edges,
    );
    anchors.extend(response.paths.iter().flat_map(|path| {
        [
            path.node_ids.first().cloned(),
            path.node_ids.last().cloned(),
        ]
        .into_iter()
        .flatten()
    }));
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for id in anchors {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(node) = nodes.get(&id).copied() else {
            continue;
        };
        let Some(anchor) = node.source.as_ref() else {
            continue;
        };
        let Some(file) = files.get(anchor.file.as_str()).copied() else {
            continue;
        };
        let Some(source) = file.source.as_deref() else {
            continue;
        };
        let block = source_line_range(source, anchor.start_line, anchor.end_line);
        if block.is_empty() {
            continue;
        }
        entries.push((
            TextPageSection::Source,
            format!(
                "- {} L{}-L{} ({})\n{}",
                anchor.file,
                anchor.start_line,
                anchor.end_line,
                if file.truncated {
                    "verified, file read truncated"
                } else {
                    "verified"
                },
                block
            ),
        ));
    }
    entries
}

/// Extract the recorded line range, bounded by `MAP_SOURCE_BLOCK_CHARS`.
fn source_line_range(source: &str, start_line: u32, end_line: u32) -> String {
    if start_line == 0 || end_line < start_line {
        return String::new();
    }
    let mut block = String::new();
    for (offset, line) in source
        .lines()
        .skip(usize::try_from(start_line - 1).unwrap_or(usize::MAX))
        .take(usize::try_from(end_line - start_line).unwrap_or(usize::MAX) + 1)
        .enumerate()
    {
        let number = u64::from(start_line) + u64::try_from(offset).unwrap_or(u64::MAX);
        let rendered = format!("  {number:>6}: {line}");
        if block.chars().count() + rendered.chars().count() + 1 > MAP_SOURCE_BLOCK_CHARS {
            if !block.is_empty() {
                block.push('\n');
            }
            block.push_str("  …[source block truncated]");
            break;
        }
        if !block.is_empty() {
            block.push('\n');
        }
        block.push_str(&rendered);
    }
    block
}

fn prefix_digest(entries: &[(TextPageSection, String)]) -> String {
    let joined = entries
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    hex_digest(joined.as_bytes())
}

fn rendered_chars(lines: &[String]) -> usize {
    lines.iter().map(|line| line.chars().count() + 1).sum()
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn encode_agent_text_page_cursor(cursor: &AgentTextPageCursor) -> Result<String, OutputError> {
    Ok(encode_cursor_fields(cursor))
}

fn render_result_lines(view: &AgentQueryView) -> Vec<String> {
    // One line: the state words carry the page's entire envelope, and a
    // six-line block costs every page - including the pages whose answer is a
    // single no-match sentence - the same budget. `match=exact`, `evidence=exact`
    // and `execution=complete` are the resolved answer; printing them on every
    // page spends budget on the state a caller already assumes and would only
    // read as a caveat. The exceptions are what a caller weighs, so only they
    // are printed, after the coverage that qualifies the whole answer.
    let mut states = Vec::new();
    if view.status.match_state != AgentMatch::Exact {
        states.push(format!(
            "match={}",
            match_state_name(view.status.match_state)
        ));
    }
    if view.status.evidence_state != AgentEvidence::Exact {
        states.push(format!(
            "evidence={}",
            evidence_state_name(view.status.evidence_state)
        ));
    }
    if view.status.source_execution != AgentExecution::Complete {
        states.push(format!(
            "execution={}",
            execution_state_name(view.status.source_execution)
        ));
    }
    states.push(format!(
        "coverage={}",
        coverage_state_name(view.status.coverage)
    ));
    vec![format!(
        "RESULT {} · {}",
        result_state_name(view.status.result_state),
        states.join(" · ")
    )]
}

fn result_state_name(value: AgentResultState) -> &'static str {
    match value {
        AgentResultState::Answered => "answered",
        AgentResultState::Candidates => "candidates",
        AgentResultState::NeedsResolution => "needs_resolution",
        AgentResultState::NoMatch => "no_match",
        AgentResultState::NoPath => "no_path",
    }
}

fn match_state_name(value: AgentMatch) -> &'static str {
    match value {
        AgentMatch::Exact => "exact",
        AgentMatch::Fuzzy => "fuzzy",
        AgentMatch::Ambiguous => "ambiguous",
        AgentMatch::None => "none",
        AgentMatch::NotApplicable => "not_applicable",
        AgentMatch::Unknown => "unknown",
    }
}

fn evidence_state_name(value: AgentEvidence) -> &'static str {
    match value {
        AgentEvidence::Exact => "exact",
        AgentEvidence::Inferred => "inferred",
        AgentEvidence::Mixed => "mixed",
        AgentEvidence::Ambiguous => "ambiguous",
        AgentEvidence::None => "none",
    }
}

fn execution_state_name(value: AgentExecution) -> &'static str {
    match value {
        AgentExecution::Complete => "complete",
        AgentExecution::Partial => "partial",
    }
}

fn coverage_state_name(value: AgentCoverage) -> &'static str {
    match value {
        AgentCoverage::Incomplete => "incomplete",
        AgentCoverage::Unknown => "unknown",
    }
}

fn severity_name(value: AgentSeverity) -> &'static str {
    match value {
        AgentSeverity::Blocker => "blocker",
        AgentSeverity::Warning => "warning",
        AgentSeverity::Info => "info",
    }
}

fn render_path_summary(path: &AgentPath) -> String {
    let mut segments = Vec::new();
    if let Some(first) = path.steps.first() {
        segments.push(escape_scalar(&first.from.label));
    }
    for step in &path.steps {
        let arrow = match step.direction {
            AgentPathDirection::Forward => format!("--{}-->", escape_scalar(&step.relation)),
            AgentPathDirection::Reverse => format!("<--{}--", escape_scalar(&step.relation)),
        };
        segments.push(arrow);
        segments.push(escape_scalar(&step.to.label));
    }
    segments.join(" ")
}

/// Render one entity line.
///
/// The stable identifier is printed only where a page has to resolve a name
/// that is not unique; every other row stays addressable by the qualified name
/// and source anchor it prints, which keeps the answer's evidence per token
/// high. `--format json` carries every identifier unchanged.
fn render_entity(entity: &AgentEntity, include_id: bool) -> String {
    let source = entity
        .source
        .as_ref()
        .map(render_source)
        .unwrap_or_else(|| "source unavailable".to_owned());
    let line = format!(
        "- {} [{}] {}",
        escape_scalar(&entity.label),
        escape_scalar(&entity.kind),
        escape_scalar(&source),
    );
    if include_id {
        format!("{line}\n  id: {}", escape_scalar(&entity.id))
    } else {
        line
    }
}

/// Labels that more than one retained entity carries.
///
/// A label is the address a caller repeats back to the tool, so a row needs its
/// stable identifier exactly when that address does not pick the row out of the
/// answer it was read from.
fn duplicated_entity_labels(entities: &[AgentEntity]) -> HashSet<&str> {
    let mut counts = HashMap::<&str, usize>::new();
    for entity in entities {
        *counts.entry(entity.label.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|(label, count)| (count > 1).then_some(label))
        .collect()
}

/// Whether a page must print the stable identifier of one entity.
///
/// An ambiguous query asks the user to select an exact ID, so every retained
/// candidate carries that ID even when its qualified label is distinct. Other
/// unresolved candidate lists need IDs only for colliding labels. Resolved
/// answers stay compact; JSON carries every identifier unchanged.
fn entity_needs_identity(
    entity: &AgentEntity,
    match_state: AgentMatch,
    duplicated_labels: &HashSet<&str>,
) -> bool {
    match_state == AgentMatch::Ambiguous
        || (!matches!(match_state, AgentMatch::Exact)
            && duplicated_labels.contains(entity.label.as_str()))
}

fn render_relationship(relationship: &AgentRelationship) -> String {
    let site = relationship
        .site
        .as_ref()
        .map(render_source)
        .unwrap_or_else(|| "site unavailable".to_owned());
    let mut line = format!(
        "- {} --{}--> {} · {}",
        escape_scalar(&relationship.source.label),
        escape_scalar(&relationship.relation),
        escape_scalar(&relationship.target.label),
        escape_scalar(&site),
    );
    // The strongest confidence and resolution are the default; what a caller
    // has to weigh is the exception, so only that is spelled out.
    if relationship.evidence.confidence != "exact" || relationship.evidence.resolution != "exact" {
        line.push_str(&format!(
            " · {} · {}",
            escape_scalar(&relationship.evidence.confidence),
            escape_scalar(&relationship.evidence.resolution)
        ));
    }
    line
}

fn render_path(path: &AgentPath) -> String {
    // The path identity is a concatenation of its node identifiers, so a
    // three-hop trail spends more of the page on `sha256:` text than on the
    // trail itself. The hop count and the labelled chain are the answer; the
    // identity stays in `--format json`.
    format!(
        "- {} hop(s): {}",
        path.steps.len(),
        render_path_summary(path)
    )
}

fn render_caveat(caveat: &AgentCaveat) -> String {
    let line = format!(
        "- [{}] {}: {}",
        severity_name(caveat.severity),
        escape_scalar(&caveat.code),
        escape_scalar(&caveat_statement(caveat))
    );
    line
}

/// Statement to print for one caveat.
///
/// A blocker changes how the answer must be read, so it is stated in full. A
/// warning states its actionable sentence and leaves the explanatory remainder
/// to `--format json`: the six caveat paragraphs that accompany a large
/// relation answer otherwise cost more of the page budget than the evidence
/// they qualify.
fn caveat_statement(caveat: &AgentCaveat) -> String {
    if caveat.severity != AgentSeverity::Warning {
        return caveat.statement.clone();
    }
    match caveat.statement.split_once(". ") {
        Some((first, _)) if !first.is_empty() => format!("{first}."),
        _ => caveat.statement.clone(),
    }
}

fn render_action(action: &AgentNextAction) -> String {
    if let Some(mcp) = &action.mcp {
        let arguments = serde_json::to_string(&mcp.arguments).unwrap_or_else(|_| "{}".to_owned());
        return format!(
            "- {}: {} · MCP {} {}",
            escape_scalar(&action.kind),
            escape_scalar(&action.reason),
            escape_scalar(&mcp.tool),
            escape_scalar(&arguments)
        );
    }
    let cli = action
        .cli
        .as_ref()
        .map(|action| {
            action
                .argv
                .iter()
                .map(|value| escape_scalar(value))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| "unavailable".to_owned());
    format!(
        "- {}: {} · CLI {}",
        escape_scalar(&action.kind),
        escape_scalar(&action.reason),
        cli
    )
}

fn render_source(source: &AgentSource) -> String {
    format!(
        "{}:L{}:{}-L{}:{}",
        escape_scalar(&source.file),
        source.start_line,
        source.start_column,
        source.end_line,
        source.end_column
    )
}

fn escape_scalar(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars().take(AGENT_VIEW_MAX_SCALAR_CHARS) {
        let code = u32::from(character);
        let bidi = matches!(
            code,
            0x061c | 0x200e..=0x200f | 0x202a..=0x202e | 0x2066..=0x2069
        );
        if character.is_control() || bidi {
            let _ = write!(output, "\\u{{{code:x}}}");
        } else {
            output.push(character);
        }
    }
    if value.chars().count() > AGENT_VIEW_MAX_SCALAR_CHARS {
        output.push('…');
    }
    output
}

fn primary_node_ids(
    operation: AgentOperation,
    operands: &[AgentOperand],
    response: &CodeQueryResponse,
    nodes: &BTreeMap<String, &QueryNode>,
    ordered_edges: &[(usize, &QueryEdge)],
) -> Vec<String> {
    let mut ordered = Vec::new();
    let requested = operands
        .iter()
        .filter_map(|operand| unique_node_id(&operand.value, nodes))
        .collect::<Vec<_>>();
    match operation {
        AgentOperation::Search => {
            ordered.extend(response.results.iter().map(|hit| hit.node_id.clone()));
            ordered.extend(requested);
        }
        AgentOperation::Callers => {
            if let Some(target) = requested.first() {
                ordered.push(target.clone());
                ordered.extend(
                    ordered_edges
                        .iter()
                        .map(|(_, edge)| *edge)
                        .filter(|edge| edge.target == *target)
                        .map(|edge| edge.source.clone()),
                );
            } else {
                ordered.extend(response.results.iter().map(|hit| hit.node_id.clone()));
                ordered.extend(requested);
            }
        }
        AgentOperation::Callees => {
            if let Some(source) = requested.first() {
                ordered.push(source.clone());
                ordered.extend(
                    ordered_edges
                        .iter()
                        .map(|(_, edge)| *edge)
                        .filter(|edge| edge.source == *source)
                        .map(|edge| edge.target.clone()),
                );
            } else {
                ordered.extend(response.results.iter().map(|hit| hit.node_id.clone()));
                ordered.extend(requested);
            }
        }
        AgentOperation::Impact => {
            ordered.extend(requested);
            // A reverse walk reaches far more owner-level dependents than
            // direct ones, and the raw trail ledger is ordered by identity.
            // Order the impacted nodes by how close and how direct their
            // evidence is, so the bounded answer names the callers and
            // dependents that a change actually breaks before it lists the
            // symbols that only touch a containing owner.
            let edge_kinds = response
                .edges
                .iter()
                .map(|edge| (edge.id.as_str(), edge.kind))
                .collect::<BTreeMap<_, _>>();
            let mut trails = response.paths.iter().collect::<Vec<_>>();
            trails.sort_by(|left, right| {
                left.node_ids
                    .len()
                    .cmp(&right.node_ids.len())
                    .then_with(|| {
                        trail_strength(left, &edge_kinds).cmp(&trail_strength(right, &edge_kinds))
                    })
                    .then_with(|| left.node_ids.cmp(&right.node_ids))
                    .then_with(|| left.id.cmp(&right.id))
            });
            ordered.extend(
                trails
                    .into_iter()
                    .filter_map(|path| path.node_ids.last().cloned()),
            );
        }
        AgentOperation::Explore => {
            ordered.extend(
                response
                    .paths
                    .iter()
                    .flat_map(|path| [path.node_ids.first(), path.node_ids.last()])
                    .flatten()
                    .cloned(),
            );
            ordered.extend(requested);
            ordered.extend(response.results.iter().map(|hit| hit.node_id.clone()));
        }
        AgentOperation::NodeTrail => {
            if let Some(path) = response.paths.first() {
                ordered.extend(path.node_ids.iter().cloned());
            } else {
                ordered.extend(requested);
            }
        }
        AgentOperation::Discovery => {}
    }
    ordered.extend(nodes.keys().cloned());
    ordered
}

fn unique_node_id(value: &str, nodes: &BTreeMap<String, &QueryNode>) -> Option<String> {
    if nodes.contains_key(value) {
        return Some(value.to_owned());
    }
    let normalized = normalize_code_query_symbol(value);
    let mut matches = nodes
        .values()
        .filter(|node| {
            normalize_code_query_symbol(&node.name) == normalized
                || normalize_code_query_symbol(&node.qualified_name) == normalized
        })
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    (matches.len() == 1).then(|| matches.remove(0))
}

fn deduplicate_entities(entities: &mut Vec<AgentEntity>) {
    let mut seen = HashSet::new();
    entities.retain(|entity| seen.insert(entity.id.clone()));
}

fn agent_entity(node: &QueryNode) -> AgentEntity {
    AgentEntity {
        id: node.id.clone(),
        label: display_label(node),
        kind: node.kind.as_str().to_owned(),
        roles: node
            .roles
            .iter()
            .map(|role| role_name(*role).to_owned())
            .collect(),
        language: node.language.clone(),
        framework: node.framework.clone(),
        source: node.source.as_ref().map(agent_source),
    }
}

fn endpoint(id: &str, nodes: &BTreeMap<String, &QueryNode>) -> AgentEndpoint {
    AgentEndpoint {
        id: id.to_owned(),
        label: nodes
            .get(id)
            .map_or_else(|| id.to_owned(), |node| display_label(node)),
    }
}

fn agent_relationship(
    edge: &QueryEdge,
    index: usize,
    nodes: &BTreeMap<String, &QueryNode>,
) -> AgentRelationship {
    AgentRelationship {
        id: edge.id.clone(),
        source: endpoint(&edge.source, nodes),
        relation: edge.kind.as_str().to_owned(),
        target: endpoint(&edge.target, nodes),
        site: edge.relationship_site.as_ref().map(agent_source),
        evidence: relationship_evidence(&edge.evidence, format!("edge-{index}")),
    }
}

fn agent_discovery_relationship(
    edge: &DiscoveryEdge,
    index: usize,
    nodes: &BTreeMap<String, &QueryNode>,
) -> AgentRelationship {
    AgentRelationship {
        id: edge
            .id
            .clone()
            .unwrap_or_else(|| format!("anonymous-edge-{index}")),
        source: endpoint(&edge.source, nodes),
        relation: edge.kind.as_str().to_owned(),
        target: endpoint(&edge.target, nodes),
        site: edge.relationship_site.as_ref().map(agent_source),
        evidence: relationship_evidence(&edge.evidence, format!("edge-{index}")),
    }
}

fn agent_path(
    path: &QueryPath,
    nodes: &BTreeMap<String, &QueryNode>,
    edges: &[QueryEdge],
) -> AgentPath {
    let edges = edges
        .iter()
        .map(|edge| (edge.id.as_str(), edge))
        .collect::<HashMap<_, _>>();
    let mut steps = Vec::new();
    for (index, edge_id) in path.edge_ids.iter().enumerate() {
        let Some(edge) = edges.get(edge_id.as_str()) else {
            continue;
        };
        let Some(from) = path.node_ids.get(index) else {
            continue;
        };
        let Some(to) = path.node_ids.get(index + 1) else {
            continue;
        };
        let direction = if edge.source == *from && edge.target == *to {
            AgentPathDirection::Forward
        } else {
            AgentPathDirection::Reverse
        };
        steps.push(AgentPathStep {
            from: endpoint(from, nodes),
            edge_id: edge.id.clone(),
            relation: edge.kind.as_str().to_owned(),
            direction,
            to: endpoint(to, nodes),
            site: edge.relationship_site.as_ref().map(agent_source),
        });
    }
    AgentPath {
        id: path.id.clone(),
        steps,
        weakest_resolution: resolution_name(path.weakest_resolution).to_owned(),
        weakest_confidence: confidence_name(path.weakest_confidence).to_owned(),
    }
}

fn display_label(node: &QueryNode) -> String {
    if !node.qualified_name.is_empty() {
        node.qualified_name.clone()
    } else if !node.name.is_empty() {
        node.name.clone()
    } else {
        node.id.clone()
    }
}

fn agent_source(anchor: &SourceAnchor) -> AgentSource {
    AgentSource {
        file: anchor.file.clone(),
        start_line: anchor.start_line,
        start_column: anchor.start_column,
        end_line: anchor.end_line,
        end_column: anchor.end_column,
    }
}

fn role_name(role: NodeRole) -> &'static str {
    match role {
        NodeRole::Controller => "controller",
        NodeRole::RouteHandler => "route_handler",
        NodeRole::Middleware => "middleware",
        NodeRole::Service => "service",
        NodeRole::Resolver => "resolver",
        NodeRole::Consumer => "consumer",
        NodeRole::Producer => "producer",
        NodeRole::Subscriber => "subscriber",
        NodeRole::Repository => "repository",
        NodeRole::Model => "model",
        NodeRole::Test => "test",
        NodeRole::Fixture => "fixture",
        NodeRole::Generated => "generated",
        NodeRole::UiComponent => "ui_component",
        NodeRole::Hook => "hook",
        NodeRole::ClientBoundary => "client_boundary",
        NodeRole::ClientComponent => "client_component",
        NodeRole::ServerComponent => "server_component",
        NodeRole::ServerFunction => "server_function",
        NodeRole::DataLoader => "data_loader",
    }
}

fn resolution_name(value: ResolutionState) -> &'static str {
    match value {
        ResolutionState::Exact => "exact",
        ResolutionState::Ambiguous => "ambiguous",
        ResolutionState::Unresolved => "unresolved",
    }
}

fn confidence_name(value: EvidenceConfidence) -> &'static str {
    match value {
        EvidenceConfidence::Exact => "exact",
        EvidenceConfidence::Inferred => "inferred",
        EvidenceConfidence::Ambiguous => "ambiguous",
    }
}

fn relationship_evidence(
    evidence: &[QueryEvidence],
    fallback: String,
) -> AgentRelationshipEvidence {
    let confidence = evidence
        .iter()
        .map(|item| item.confidence)
        .min_by_key(|value| confidence_rank(*value))
        .map(confidence_name)
        .unwrap_or("unknown");
    let resolution = evidence
        .iter()
        .map(|item| item.resolution)
        .min_by_key(|value| resolution_rank(*value))
        .map(resolution_name)
        .unwrap_or("unknown");
    let mut layers = evidence
        .iter()
        .map(|item| match item.layer {
            QueryEvidenceLayer::StructuralGraph => "structural_graph".to_owned(),
            QueryEvidenceLayer::ProgramIr => "program_ir".to_owned(),
        })
        .collect::<Vec<_>>();
    layers.sort();
    layers.dedup();
    if layers.is_empty() {
        layers.push(fallback);
    }
    AgentRelationshipEvidence {
        confidence: confidence.to_owned(),
        resolution: resolution.to_owned(),
        layers,
    }
}

fn evidence_state<'a>(evidence: impl Iterator<Item = &'a QueryEvidence>) -> AgentEvidence {
    let mut has_exact = false;
    let mut has_inferred = false;
    let mut has_ambiguous = false;
    for item in evidence {
        match item.confidence {
            EvidenceConfidence::Exact => has_exact = true,
            EvidenceConfidence::Inferred => has_inferred = true,
            EvidenceConfidence::Ambiguous => has_ambiguous = true,
        }
        if item.resolution != ResolutionState::Exact {
            has_inferred = true;
        }
    }
    if has_ambiguous {
        AgentEvidence::Ambiguous
    } else if has_exact && has_inferred {
        AgentEvidence::Mixed
    } else if has_inferred {
        AgentEvidence::Inferred
    } else if has_exact {
        AgentEvidence::Exact
    } else {
        AgentEvidence::None
    }
}

fn confidence_rank(value: EvidenceConfidence) -> u8 {
    match value {
        EvidenceConfidence::Ambiguous => 0,
        EvidenceConfidence::Inferred => 1,
        EvidenceConfidence::Exact => 2,
    }
}

fn resolution_rank(value: ResolutionState) -> u8 {
    match value {
        ResolutionState::Ambiguous => 0,
        ResolutionState::Unresolved => 1,
        ResolutionState::Exact => 2,
    }
}

fn agent_caveat(diagnostic: &QueryDiagnostic) -> AgentCaveat {
    let (severity, statement) = match diagnostic.code {
        QueryDiagnosticCode::AmbiguousMatch => (
            AgentSeverity::Blocker,
            format!(
                "Do not select a candidate automatically. {}",
                diagnostic.message
            ),
        ),
        QueryDiagnosticCode::RelationshipInconsistency => (
            AgentSeverity::Warning,
            format!(
                "Relationship traversal disagrees with indexed source-backed usage evidence. {}",
                diagnostic.message
            ),
        ),
        QueryDiagnosticCode::NoMatch => (
            AgentSeverity::Blocker,
            format!(
                "Fallback candidates are suggestions, not an exact answer. {}",
                diagnostic.message
            ),
        ),
        QueryDiagnosticCode::DirectionMismatch => (
            AgentSeverity::Blocker,
            format!(
                "A reverse-only connection is not a valid directed path. {}",
                diagnostic.message
            ),
        ),
        QueryDiagnosticCode::StaleSourceDigest => (
            AgentSeverity::Blocker,
            format!(
                "Do not quote or edit the stale source excerpt. {}",
                diagnostic.message
            ),
        ),
        QueryDiagnosticCode::IncompleteCoverage => (
            AgentSeverity::Warning,
            format!(
                "Coverage or precision is limited; verify before drawing a conclusion. {}",
                diagnostic.message
            ),
        ),
        QueryDiagnosticCode::BoundedTruncation => (
            AgentSeverity::Warning,
            format!(
                "More retained facts may exist beyond the response bound. {}",
                diagnostic.message
            ),
        ),
        QueryDiagnosticCode::UnresolvedHandler => (
            AgentSeverity::Warning,
            format!("The framework target is unresolved. {}", diagnostic.message),
        ),
        QueryDiagnosticCode::ProgramConflict => (
            AgentSeverity::Warning,
            format!(
                "Structural and Program IR evidence disagree. {}",
                diagnostic.message
            ),
        ),
        QueryDiagnosticCode::ProgramOrphan => (
            AgentSeverity::Info,
            format!(
                "Program evidence could not join to a graph entity. {}",
                diagnostic.message
            ),
        ),
        QueryDiagnosticCode::ProgramUnavailable => (
            AgentSeverity::Info,
            format!(
                "Optional Program IR evidence was unavailable. {}",
                diagnostic.message
            ),
        ),
    };
    AgentCaveat {
        severity,
        code: diagnostic_code_name(diagnostic.code).to_owned(),
        statement,
        node_id: diagnostic.node_id.clone(),
        path: diagnostic.path.clone(),
    }
}

fn diagnostic_code_name(code: QueryDiagnosticCode) -> &'static str {
    match code {
        QueryDiagnosticCode::NoMatch => "no_match",
        QueryDiagnosticCode::AmbiguousMatch => "ambiguous_match",
        QueryDiagnosticCode::RelationshipInconsistency => "relationship_inconsistency",
        QueryDiagnosticCode::DirectionMismatch => "direction_mismatch",
        QueryDiagnosticCode::UnresolvedHandler => "unresolved_handler",
        QueryDiagnosticCode::IncompleteCoverage => "incomplete_coverage",
        QueryDiagnosticCode::StaleSourceDigest => "stale_source_digest",
        QueryDiagnosticCode::BoundedTruncation => "bounded_truncation",
        QueryDiagnosticCode::ProgramOrphan => "program_orphan",
        QueryDiagnosticCode::ProgramConflict => "program_conflict",
        QueryDiagnosticCode::ProgramUnavailable => "program_unavailable",
    }
}

fn code_match_state(
    context: &AgentQueryContext,
    response: &CodeQueryResponse,
    nodes: &BTreeMap<String, &QueryNode>,
) -> AgentMatch {
    if has_diagnostic(
        response.diagnostics.as_slice(),
        QueryDiagnosticCode::AmbiguousMatch,
    ) {
        return AgentMatch::Ambiguous;
    }
    if has_diagnostic(
        response.diagnostics.as_slice(),
        QueryDiagnosticCode::NoMatch,
    ) {
        return AgentMatch::None;
    }
    if response.operation == CodeQueryOperation::Search {
        let query = context
            .operands
            .iter()
            .find(|operand| operand.role == AgentOperandRole::Query)
            .map(|operand| operand.value.as_str());
        if query.is_some_and(|query| {
            response.results.first().is_some_and(|hit| {
                nodes.get(&hit.node_id).is_some_and(|node| {
                    node.id == query || node.name == query || node.qualified_name == query
                })
            })
        }) {
            AgentMatch::Exact
        } else if response.results.is_empty() {
            AgentMatch::None
        } else {
            AgentMatch::Fuzzy
        }
    } else if response.nodes.is_empty() {
        AgentMatch::Unknown
    } else {
        AgentMatch::Exact
    }
}

fn code_result_state(
    operation: AgentOperation,
    match_state: AgentMatch,
    response: &CodeQueryResponse,
) -> AgentResultState {
    if match_state == AgentMatch::Ambiguous {
        return AgentResultState::NeedsResolution;
    }
    if match_state == AgentMatch::None {
        return AgentResultState::NoMatch;
    }
    if has_diagnostic(
        response.diagnostics.as_slice(),
        QueryDiagnosticCode::DirectionMismatch,
    ) {
        return AgentResultState::NoPath;
    }
    if operation == AgentOperation::NodeTrail
        && response.paths.is_empty()
        && !response.nodes.is_empty()
    {
        return AgentResultState::NoPath;
    }
    if operation == AgentOperation::Search && match_state == AgentMatch::Fuzzy {
        AgentResultState::Candidates
    } else {
        AgentResultState::Answered
    }
}

/// Headline for a relationship answer whose evidence belongs to one subject.
///
/// When the query resolved exactly, the count describes that subject. When it
/// did not, the evidence belongs to a fallback candidate, and the headline says
/// so instead of attributing another symbol's relationships to the requested
/// one: the Agent View never selects a candidate silently, and `no_match`
/// answers keep their candidates in the caveats.
fn relationship_headline(
    result_state: AgentResultState,
    requested: &str,
    subject: &str,
    count: usize,
    noun: &str,
) -> String {
    if result_state == AgentResultState::Answered {
        return format!("Found {count} {noun} for {subject}.");
    }
    if subject == requested {
        return format!("No exact match for \"{requested}\"; no {noun} are attributed to it.");
    }
    format!(
        "No exact match for \"{requested}\"; the {count} {noun} below belong to the fallback candidate {subject}."
    )
}

fn answer_for_code(
    context: &AgentQueryContext,
    result_state: AgentResultState,
    match_state: AgentMatch,
    response: &CodeQueryResponse,
    primary_results: &[AgentEntity],
    relationships: &[AgentRelationship],
    paths: &[AgentPath],
) -> AgentAnswer {
    let requested = context
        .operands
        .first()
        .map(|operand| operand.value.clone())
        .unwrap_or_else(|| "the requested query".to_owned());
    if result_state == AgentResultState::NeedsResolution {
        return AgentAnswer {
            headline: "The query matches multiple candidates; retry with an exact node ID for each ambiguous operand.".to_owned(),
            basis: vec![AgentBasis {
                kind: "operation".to_owned(),
                id: context.operation.label().to_owned(),
            }],
        };
    }
    let subject = primary_results
        .first()
        .map(|entity| entity.label.clone())
        .unwrap_or_else(|| requested.clone());
    let headline = match context.operation {
        AgentOperation::Search => match result_state {
            AgentResultState::NoMatch => {
                format!("No exact match for \"{requested}\"; fallback candidates are shown.")
            }
            AgentResultState::Candidates => format!(
                "Found {} candidate matches for \"{requested}\".",
                primary_results.len()
            ),
            _ if match_state == AgentMatch::Exact => {
                format!("Found an exact match for \"{requested}\".")
            }
            _ => format!("No exact answer was proven for \"{requested}\"."),
        },
        AgentOperation::Callers => relationship_headline(
            result_state,
            &requested,
            &subject,
            response.edges.len(),
            "incoming usage relationship(s)",
        ),
        AgentOperation::Callees => relationship_headline(
            result_state,
            &requested,
            &subject,
            response.edges.len(),
            "direct callee relationship(s)",
        ),
        AgentOperation::Impact => {
            if result_state == AgentResultState::Answered {
                format!(
                    "Found {} potentially affected node(s) within depth {}.",
                    primary_results.len(),
                    response.limits.max_depth
                )
            } else if subject == requested {
                format!(
                    "No exact match for \"{requested}\"; no potentially affected nodes are attributed to it."
                )
            } else {
                format!(
                    "No exact match for \"{requested}\"; the {} potentially affected node(s) below belong to the fallback candidate {subject}.",
                    primary_results.len()
                )
            }
        }
        AgentOperation::Explore => format!(
            "Found {} candidate anchor(s) and {} relationship(s) for the question.",
            primary_results.len(),
            relationships.len()
        ),
        AgentOperation::NodeTrail => {
            let source = context
                .operands
                .iter()
                .find(|operand| operand.role == AgentOperandRole::Source)
                .map(|operand| operand.value.as_str())
                .unwrap_or("source");
            let target = context
                .operands
                .iter()
                .find(|operand| operand.role == AgentOperandRole::Target)
                .map(|operand| operand.value.as_str())
                .unwrap_or("target");
            if let Some(path) = paths.first() {
                format!(
                    "Found a {}-hop directed path from {source} to {target}.",
                    path.steps.len()
                )
            } else {
                "No directed path reaches the exact target within the requested bounds.".to_owned()
            }
        }
        AgentOperation::Discovery => "Discovery returned candidate anchors.".to_owned(),
    };
    let mut basis = Vec::new();
    if let Some(entity) = primary_results.first() {
        basis.push(AgentBasis {
            kind: "node".to_owned(),
            id: entity.id.clone(),
        });
    } else if let Some(relationship) = relationships.first() {
        basis.push(AgentBasis {
            kind: "relationship".to_owned(),
            id: relationship.id.clone(),
        });
    } else if let Some(path) = paths.first() {
        basis.push(AgentBasis {
            kind: "path".to_owned(),
            id: path.id.clone(),
        });
    } else {
        basis.push(AgentBasis {
            kind: "operation".to_owned(),
            id: context.operation.label().to_owned(),
        });
    }
    AgentAnswer { headline, basis }
}

fn answer_for_discovery(
    result_state: AgentResultState,
    question: &str,
    seeds: usize,
    relationships: usize,
    primary_results: &[AgentEntity],
) -> AgentAnswer {
    let headline = match result_state {
        AgentResultState::NoMatch => {
            format!("No exact match for \"{question}\"; fallback candidates are shown.")
        }
        AgentResultState::NeedsResolution => {
            format!("Multiple candidate matches remain for \"{question}\"; choose an exact target.")
        }
        _ => format!(
            "Found {seeds} candidate anchor(s) and {relationships} relationship(s) for the question."
        ),
    };
    let basis = primary_results
        .first()
        .map(|entity| AgentBasis {
            kind: "node".to_owned(),
            id: entity.id.clone(),
        })
        .unwrap_or_else(|| AgentBasis {
            kind: "operation".to_owned(),
            id: "discovery".to_owned(),
        });
    AgentAnswer {
        headline,
        basis: vec![basis],
    }
}

fn next_actions_for_code(
    context: &AgentQueryContext,
    primary_results: &[AgentEntity],
    caveats: &[AgentCaveat],
    source_truncated: bool,
    paths: &[QueryPath],
) -> Vec<AgentNextAction> {
    let mut actions = Vec::new();
    if caveats
        .iter()
        .any(|caveat| caveat.code == "direction_mismatch")
    {
        let source = context
            .operands
            .iter()
            .find(|operand| operand.role == AgentOperandRole::Source);
        let target = context
            .operands
            .iter()
            .find(|operand| operand.role == AgentOperandRole::Target);
        if let (Some(source), Some(target)) = (source, target) {
            actions.push(AgentNextAction {
                kind: "inspect_undirected_path".to_owned(),
                reason: "A connection exists only when relationship direction is ignored."
                    .to_owned(),
                cli: Some(AgentActionCli {
                    argv: vec![
                        "compass".to_owned(),
                        "path".to_owned(),
                        source.value.clone(),
                        target.value.clone(),
                    ],
                }),
                mcp: None,
            });
        }
    }
    if caveats
        .iter()
        .any(|caveat| caveat.code == "ambiguous_match")
    {
        for entity in primary_results.iter().take(3) {
            actions.push(AgentNextAction {
                kind: "retry_with_exact_id".to_owned(),
                reason: "Resolve the ambiguous symbol without selecting by position.".to_owned(),
                cli: Some(AgentActionCli {
                    argv: vec!["compass".to_owned(), "search".to_owned(), entity.id.clone()],
                }),
                mcp: Some(AgentActionMcp {
                    tool: "search_symbols".to_owned(),
                    arguments: Map::from_iter([("query".to_owned(), json!(entity.id))]),
                }),
            });
        }
    }
    if let Some(entity) = primary_results.first()
        && !caveats
            .iter()
            .any(|caveat| caveat.code == "ambiguous_match")
    {
        actions.push(AgentNextAction {
            kind: "inspect_target".to_owned(),
            reason: "Inspect the exact retained target with source-grounded context.".to_owned(),
            cli: Some(AgentActionCli {
                argv: vec![
                    "compass".to_owned(),
                    "context".to_owned(),
                    "explain".to_owned(),
                    entity.id.clone(),
                ],
            }),
            mcp: Some(AgentActionMcp {
                tool: "task_context".to_owned(),
                arguments: Map::from_iter([
                    ("intent".to_owned(), json!("explain")),
                    ("target".to_owned(), json!(entity.id)),
                ]),
            }),
        });
    }
    if source_truncated && paths.is_empty() {
        actions.push(AgentNextAction {
            kind: "narrow_query".to_owned(),
            reason: "The source query reached a bound before retaining every fact.".to_owned(),
            cli: None,
            mcp: None,
        });
    }
    if context.evidence_hidden {
        actions.push(AgentNextAction {
            kind: "show_evidence".to_owned(),
            reason: "Provenance records were hidden by the request.".to_owned(),
            cli: None,
            mcp: None,
        });
    }
    actions
}

fn next_actions_for_discovery(
    context: &AgentQueryContext,
    primary_results: &[AgentEntity],
    source_truncated: bool,
) -> Vec<AgentNextAction> {
    let mut actions = Vec::new();
    if let Some(cursor) = &context.continuation_cursor {
        actions.push(AgentNextAction {
            kind: "continue_result".to_owned(),
            reason: "Continue the bounded discovery result at its next page.".to_owned(),
            cli: Some(AgentActionCli {
                argv: vec![
                    "compass".to_owned(),
                    "query".to_owned(),
                    "--cursor".to_owned(),
                    cursor.clone(),
                ],
            }),
            mcp: None,
        });
    }
    if let Some(entity) = primary_results.first() {
        actions.push(AgentNextAction {
            kind: "inspect_target".to_owned(),
            reason: "Inspect the strongest retained discovery anchor.".to_owned(),
            cli: None,
            mcp: Some(AgentActionMcp {
                tool: "task_context".to_owned(),
                arguments: Map::from_iter([
                    ("intent".to_owned(), json!("explain")),
                    ("target".to_owned(), json!(entity.id)),
                ]),
            }),
        });
    }
    if source_truncated {
        actions.push(AgentNextAction {
            kind: "narrow_query".to_owned(),
            reason: "Discovery reached a source bound.".to_owned(),
            cli: None,
            mcp: None,
        });
    }
    actions
}

fn has_diagnostic(diagnostics: &[QueryDiagnostic], code: QueryDiagnosticCode) -> bool {
    diagnostics.iter().any(|diagnostic| diagnostic.code == code)
}

fn view_digest(view: &AgentQueryView) -> Result<String, OutputError> {
    let mut canonical = view.clone();
    canonical.identity.view_digest.clear();
    let bytes = serde_json::to_vec(&canonical).map_err(|error| invalid(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn valid_sha256(value: &str) -> bool {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
