//! Deterministic, bounded projections for coding-agent query consumers.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use compass_model::code_graph::NodeRole;
use compass_model::provenance::{EvidenceConfidence, ResolutionState, SourceAnchor};
use compass_model::query_contract::{
    CodeQueryOperation, CodeQueryResponse, DiscoveryEdge, DiscoveryQueryResponse, QueryDiagnostic,
    QueryDiagnosticCode, QueryEdge, QueryEvidence, QueryEvidenceLayer, QueryNode, QueryPath,
};
use compass_query::{code_query_response_digest, discovery_response_digest};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::OutputError;

pub const AGENT_QUERY_VIEW_SCHEMA: &str = "compass.query.agent-view/1";
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
    let primary_ids = primary_node_ids(context.operation, &context.operands, response, &nodes);
    let mut primary_results = primary_ids
        .iter()
        .filter_map(|id| nodes.get(id).copied())
        .map(agent_entity)
        .collect::<Vec<_>>();
    let before_primary = primary_results.len();
    primary_results.truncate(AGENT_VIEW_MAX_PRIMARY_RESULTS);
    let primary_omitted = before_primary.saturating_sub(primary_results.len());

    let mut all_relationships = response
        .edges
        .iter()
        .enumerate()
        .map(|(index, edge)| agent_relationship(edge, index, &nodes))
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
        lines.extend(view.caveats.iter().map(render_caveat));
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
        lines.extend(view.primary_results.iter().map(render_entity));
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

fn render_result_lines(view: &AgentQueryView) -> Vec<String> {
    vec![
        "RESULT".to_owned(),
        format!("State: {}", result_state_name(view.status.result_state)),
        format!("Match: {}", match_state_name(view.status.match_state)),
        format!(
            "Evidence: {}",
            evidence_state_name(view.status.evidence_state)
        ),
        format!(
            "Execution: {} within requested bounds",
            execution_state_name(view.status.source_execution)
        ),
        format!("Coverage: {}", coverage_state_name(view.status.coverage)),
    ]
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

fn render_entity(entity: &AgentEntity) -> String {
    let source = entity
        .source
        .as_ref()
        .map(render_source)
        .unwrap_or_else(|| "source unavailable".to_owned());
    format!(
        "- {} [{}] {}\n  id: {}",
        escape_scalar(&entity.label),
        escape_scalar(&entity.kind),
        escape_scalar(&source),
        escape_scalar(&entity.id)
    )
}

fn render_relationship(relationship: &AgentRelationship) -> String {
    let site = relationship
        .site
        .as_ref()
        .map(render_source)
        .unwrap_or_else(|| "site unavailable".to_owned());
    format!(
        "- {} --{}--> {}\n  {} · {} · {}",
        escape_scalar(&relationship.source.label),
        escape_scalar(&relationship.relation),
        escape_scalar(&relationship.target.label),
        escape_scalar(&site),
        escape_scalar(&relationship.evidence.confidence),
        escape_scalar(&relationship.evidence.resolution)
    )
}

fn render_path(path: &AgentPath) -> String {
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
    format!(
        "- {} ({} hop(s)): {}",
        escape_scalar(&path.id),
        path.steps.len(),
        segments.join(" ")
    )
}

fn render_caveat(caveat: &AgentCaveat) -> String {
    format!(
        "- [{}] {}: {}",
        severity_name(caveat.severity),
        escape_scalar(&caveat.code),
        escape_scalar(&caveat.statement)
    )
}

fn severity_name(value: AgentSeverity) -> &'static str {
    match value {
        AgentSeverity::Blocker => "blocker",
        AgentSeverity::Warning => "warning",
        AgentSeverity::Info => "info",
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
                    response
                        .edges
                        .iter()
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
                    response
                        .edges
                        .iter()
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
            ordered.extend(
                response
                    .paths
                    .iter()
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
    let mut matches = nodes
        .values()
        .filter(|node| node.name == value || node.qualified_name == value)
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
                "Absence is not proof that the relationship does not exist. {}",
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
        AgentOperation::Callers => format!(
            "Found {} incoming call or route relationship(s) for {subject}.",
            relationships.len()
        ),
        AgentOperation::Callees => format!(
            "Found {} direct callee relationship(s) for {subject}.",
            relationships.len()
        ),
        AgentOperation::Impact => format!(
            "Found {} potentially affected node(s) within depth {}.",
            primary_results.len(),
            response.limits.max_depth
        ),
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
