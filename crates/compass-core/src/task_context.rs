use std::collections::{BTreeMap, BTreeSet};

use compass_agent_graph::{
    ChallengeId, CompositionOmissions, EffectiveGraph, GroundingEvidence, OverlayState,
};
use compass_model::code_graph::{
    EdgeDetails, EdgeKind, NodeDetails, NodeKind, NodeRole, RouteStage,
};
use compass_model::provenance::{ResolutionState, SourceAnchor};
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, ExploreRequest,
    ImpactRequest, QueryDiagnosticCode, QueryEdge, QueryEvidence, QueryNode, SearchHit,
    SearchRequest,
};
use compass_query::CodeQueryEngine;
use compass_reflect::MemoryDoc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const TASK_CONTEXT_SCHEMA: &str = "compass.task-context/2";
pub const TASK_CONTEXT_SCHEMA_V1: &str = "compass.task-context/1";
pub const TASK_CONTEXT_PROFILE_SCHEMA: &str = "compass.task-context-profile/1";
pub const FRAMEWORK_CONTEXT_SCHEMA: &str = "compass.framework-context/1";
const MAX_TASK_TARGET_BYTES: usize = 16 * 1024;
const MAX_REPOSITORY_ROOT_BYTES: usize = 32 * 1024;
const MAX_KNOWLEDGE_ITEMS: u32 = 100;
const MAX_MEMORY_DOCS: usize = 10_000;
const MAX_MEMORY_SOURCE_NODES: usize = 100_000;
const MAX_MEMORY_INPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TASK_CONTEXT_BYTES: usize = 64 * 1024 * 1024;
const MAX_FRAMEWORK_CONTEXT_RECORDS: usize = 256;
const MAX_FRAMEWORK_CONTEXT_BYTES: usize = 256 * 1024;
const MAX_FRAMEWORK_TEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskContextIntent {
    Explain,
    Modify,
    Debug,
    Test,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskContextLimits {
    pub query: CodeQueryLimits,
    pub max_knowledge_items: u32,
    pub max_response_bytes: u64,
}

impl Default for TaskContextLimits {
    fn default() -> Self {
        Self {
            query: CodeQueryLimits::default(),
            max_knowledge_items: 20,
            max_response_bytes: 16 * 1024 * 1024,
        }
    }
}

impl TaskContextLimits {
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.query.is_valid()
            && self.max_knowledge_items > 0
            && self.max_knowledge_items <= MAX_KNOWLEDGE_ITEMS
            && self.max_response_bytes > 0
            && self.max_response_bytes <= 64 * 1024 * 1024
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskContextRequest {
    pub intent: TaskContextIntent,
    pub target: String,
    pub repository_root: String,
    pub limits: TaskContextLimits,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskContextTarget {
    Exact { node_id: String },
    Ambiguous { candidates: Vec<SearchHit> },
    NotFound { candidates: Vec<SearchHit> },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskContextSectionKind {
    DeclarationSource,
    ExactCallers,
    ExactCallees,
    ImplementationType,
    RelatedTests,
    TransitiveImpact,
    Framework,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskContextSection {
    pub kind: TaskContextSectionKind,
    pub evidence: CodeQueryResponse,
}

/// Qualification state is deliberately closed. Consumers must reject a
/// future state rather than treating it as a successful framework claim.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkQualificationState {
    Qualified,
    Qualifying,
    Incomplete,
    Unsupported,
    Ambiguous,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkPackContext {
    pub id: String,
    pub version: u32,
    pub qualification: FrameworkQualificationState,
    pub capabilities: Vec<String>,
    pub observed_nodes: u32,
    pub observed_relations: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkStageContext {
    pub stage: RouteStage,
    pub position: u32,
    pub reference: String,
    pub resolution: ResolutionState,
    pub source: Option<SourceAnchor>,
    pub target: Option<String>,
    pub provenance: Vec<QueryEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkRouteContext {
    pub node_id: String,
    pub framework: String,
    pub operation: String,
    pub path: String,
    pub declaring_scope: String,
    pub resolution: ResolutionState,
    pub stages: Vec<FrameworkStageContext>,
    pub provenance: Vec<QueryEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkRelationContext {
    pub id: String,
    pub relation: EdgeKind,
    pub source: String,
    pub target: String,
    pub details: Option<EdgeDetails>,
    pub relationship_site: Option<SourceAnchor>,
    pub provenance: Vec<QueryEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkBoundaryContext {
    pub node_id: String,
    pub framework: String,
    pub roles: Vec<NodeRole>,
    pub source: Option<SourceAnchor>,
    pub provenance: Vec<QueryEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkCapabilityStatus {
    pub framework: String,
    pub capability: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkAmbiguity {
    pub kind: String,
    pub reference: String,
    pub candidates: Vec<String>,
}

/// Typed framework evidence attached to one task-context response. The
/// section contains only graph evidence already admitted by the query engine;
/// it never reparses source or executes a framework configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameworkContext {
    pub schema: String,
    pub graph_identity: String,
    pub build_generation_identity: String,
    pub focus_node_id: Option<String>,
    pub packs: Vec<FrameworkPackContext>,
    pub routes: Vec<FrameworkRouteContext>,
    pub relations: Vec<FrameworkRelationContext>,
    pub rendered_by: Vec<FrameworkRelationContext>,
    pub renders: Vec<FrameworkRelationContext>,
    pub config_dependencies: Vec<FrameworkRelationContext>,
    pub runtime_boundaries: Vec<FrameworkBoundaryContext>,
    pub unsupported: Vec<FrameworkCapabilityStatus>,
    pub incomplete: Vec<FrameworkCapabilityStatus>,
    pub ambiguities: Vec<FrameworkAmbiguity>,
    pub truncated: bool,
    pub record_limit: u32,
    pub byte_limit: u64,
}

impl FrameworkContext {
    fn trim_one(&mut self) -> bool {
        self.relations.pop().is_some()
            || self.rendered_by.pop().is_some()
            || self.renders.pop().is_some()
            || self.config_dependencies.pop().is_some()
            || self.routes.pop().is_some()
            || self.runtime_boundaries.pop().is_some()
            || self.ambiguities.pop().is_some()
            || self.incomplete.pop().is_some()
            || self.unsupported.pop().is_some()
            || self.packs.pop().is_some()
    }

    fn validate(&self) -> Result<(), TaskContextError> {
        if self.schema != FRAMEWORK_CONTEXT_SCHEMA {
            return Err(TaskContextError::UnsupportedSchema(self.schema.clone()));
        }
        if self.graph_identity.is_empty() || self.build_generation_identity.is_empty() {
            return Err(TaskContextError::InvalidResult(
                "framework context is missing graph or build identity".to_owned(),
            ));
        }
        if let Some(focus) = &self.focus_node_id {
            validate_framework_strings(std::slice::from_ref(focus))?;
        }
        if self.record_limit == 0
            || usize::try_from(self.record_limit).unwrap_or(usize::MAX)
                > MAX_FRAMEWORK_CONTEXT_RECORDS
            || self.byte_limit == 0
            || usize::try_from(self.byte_limit).unwrap_or(usize::MAX) > MAX_FRAMEWORK_CONTEXT_BYTES
        {
            return Err(TaskContextError::InvalidResult(
                "framework context limits are outside the supported bounds".to_owned(),
            ));
        }
        for pack in &self.packs {
            let Some(expected) = compass_languages::framework_pack_semantics_version(&pack.id)
            else {
                return Err(TaskContextError::InvalidResult(format!(
                    "unknown framework pack ID {:?}",
                    pack.id
                )));
            };
            if pack.version != expected || pack.id.trim().is_empty() {
                return Err(TaskContextError::InvalidResult(format!(
                    "invalid version for framework pack {:?}",
                    pack.id
                )));
            }
            validate_framework_strings(&pack.capabilities)?;
        }
        validate_framework_strings(
            &self
                .routes
                .iter()
                .flat_map(|route| {
                    [
                        route.node_id.clone(),
                        route.framework.clone(),
                        route.operation.clone(),
                        route.path.clone(),
                        route.declaring_scope.clone(),
                    ]
                })
                .collect::<Vec<_>>(),
        )?;
        for route in &self.routes {
            validate_framework_strings(
                &route
                    .stages
                    .iter()
                    .flat_map(|stage| {
                        [
                            stage.reference.clone(),
                            stage.target.clone().unwrap_or_default(),
                        ]
                    })
                    .collect::<Vec<_>>(),
            )?;
        }
        for status in self.unsupported.iter().chain(self.incomplete.iter()) {
            validate_framework_strings(&[
                status.framework.clone(),
                status.capability.clone(),
                status.reason.clone(),
            ])?;
        }
        for ambiguity in &self.ambiguities {
            validate_framework_strings(&[ambiguity.kind.clone(), ambiguity.reference.clone()])?;
            validate_framework_strings(&ambiguity.candidates)?;
        }
        if framework_record_count(self) > usize::try_from(self.record_limit).unwrap_or(usize::MAX)
            || canonical_bytes(self)?.len() > usize::try_from(self.byte_limit).unwrap_or(usize::MAX)
        {
            return Err(TaskContextError::InvalidResult(
                "framework context exceeds its declared record or byte limit".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskContextKnowledge {
    pub path: String,
    pub date: String,
    pub question: String,
    pub outcome: String,
    pub correction: String,
    pub source_nodes: Vec<String>,
    pub provenance: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskContextOmission {
    pub category: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskContextWork {
    pub schema: String,
    pub query_count: u64,
    pub candidates_returned: u64,
    pub nodes_returned: u64,
    pub edges_returned: u64,
    pub files_verified: u64,
    pub source_bytes: u64,
    pub knowledge_items_read: u64,
    pub framework_records: u64,
    pub framework_bytes: u64,
    #[serde(default)]
    pub agent_knowledge_records: u64,
    #[serde(default)]
    pub agent_knowledge_bytes: u64,
    pub response_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentKnowledgeAssertion {
    pub assertion_id: compass_agent_graph::AssertionId,
    pub projected_id: String,
    pub owner: compass_agent_graph::PrincipalId,
    pub version: u64,
    pub grounding_status: String,
    pub structural_confidence: String,
    pub certificate_digest: compass_agent_graph::GroundingCertificateDigest,
    pub summary: String,
    pub citations: Vec<GroundingEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentKnowledgeChallenge {
    pub challenge_id: ChallengeId,
    pub target_id: String,
    pub effect: compass_agent_graph::ChallengeEffect,
    pub masked: bool,
    pub grounding_status: String,
    pub certificate_digest: compass_agent_graph::GroundingCertificateDigest,
    pub summary: String,
    pub citations: Vec<GroundingEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentKnowledgeSection {
    pub schema: String,
    pub effective_identity: compass_agent_graph::Digest,
    pub base_generation: compass_agent_graph::BaseGenerationId,
    pub overlay_revision: compass_agent_graph::OverlayRevisionId,
    pub composition_profile: compass_agent_graph::CompositionProfile,
    pub assertions: Vec<AgentKnowledgeAssertion>,
    pub challenges: Vec<AgentKnowledgeChallenge>,
    pub omissions: CompositionOmissions,
    pub truncated: bool,
    pub omitted_records: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskContext {
    pub schema: String,
    pub intent: TaskContextIntent,
    pub requested_target: String,
    pub target: TaskContextTarget,
    pub graph_identity: String,
    pub build_generation_identity: String,
    pub sections: Vec<TaskContextSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<FrameworkContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_knowledge: Option<AgentKnowledgeSection>,
    pub project_knowledge: Vec<TaskContextKnowledge>,
    pub omissions: Vec<TaskContextOmission>,
    pub truncated: bool,
    pub work: TaskContextWork,
    pub result_digest: String,
}

impl TaskContext {
    pub fn from_json(bytes: &[u8]) -> Result<Self, TaskContextError> {
        if bytes.len() > MAX_TASK_CONTEXT_BYTES {
            return Err(TaskContextError::InvalidResult(format!(
                "encoded task context exceeds {MAX_TASK_CONTEXT_BYTES} bytes"
            )));
        }
        let context: Self = serde_json::from_slice(bytes)?;
        context.validate()?;
        Ok(context)
    }

    pub fn validate(&self) -> Result<(), TaskContextError> {
        if self.schema != TASK_CONTEXT_SCHEMA {
            return Err(TaskContextError::UnsupportedSchema(self.schema.clone()));
        }
        if self.work.schema != TASK_CONTEXT_PROFILE_SCHEMA {
            return Err(TaskContextError::UnsupportedSchema(
                self.work.schema.clone(),
            ));
        }
        if let Some(framework) = &self.framework {
            framework.validate()?;
            let encoded = canonical_bytes(framework)?;
            if encoded.len() > MAX_FRAMEWORK_CONTEXT_BYTES {
                return Err(TaskContextError::InvalidResult(
                    "framework context exceeds its byte bound".to_owned(),
                ));
            }
        }
        if let Some(agent) = &self.agent_knowledge {
            if agent.schema != "compass.agent-knowledge/1"
                || agent.effective_identity.as_str() != self.graph_identity
                || agent
                    .assertions
                    .len()
                    .saturating_add(agent.challenges.len())
                    > MAX_KNOWLEDGE_ITEMS as usize
            {
                return Err(TaskContextError::InvalidResult(
                    "Agent knowledge schema, identity, or record bound is invalid".to_owned(),
                ));
            }
            if !agent.truncated && agent.omitted_records != 0 {
                return Err(TaskContextError::InvalidResult(
                    "complete Agent knowledge cannot report omitted records".to_owned(),
                ));
            }
            for assertion in &agent.assertions {
                if assertion.grounding_status != "GROUNDED"
                    || assertion.structural_confidence != "inferred"
                {
                    return Err(TaskContextError::InvalidResult(
                        "Agent knowledge must keep GROUNDED separate from structural confidence"
                            .to_owned(),
                    ));
                }
            }
            for challenge in &agent.challenges {
                if challenge.grounding_status != "GROUNDED" {
                    return Err(TaskContextError::InvalidResult(
                        "Agent knowledge Challenge is missing GROUNDED status".to_owned(),
                    ));
                }
            }
        }
        let expected_digest = task_context_digest(self)?;
        if self.result_digest != expected_digest {
            return Err(TaskContextError::InvalidResult(format!(
                "result digest mismatch: expected {expected_digest}, found {}",
                self.result_digest
            )));
        }
        let actual_bytes = u64::try_from(canonical_bytes(self)?.len()).unwrap_or(u64::MAX);
        if self.work.response_bytes != actual_bytes {
            return Err(TaskContextError::InvalidResult(format!(
                "response byte count mismatch: expected {actual_bytes}, found {}",
                self.work.response_bytes
            )));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TaskContextError {
    #[error("invalid task-context request: {0}")]
    InvalidRequest(String),
    #[error("unsupported task-context schema {0:?}")]
    UnsupportedSchema(String),
    #[error("invalid task-context result: {0}")]
    InvalidResult(String),
    #[error("task-context query failed: {0}")]
    Query(#[from] compass_query::QueryError),
    #[error("could not encode task context: {0}")]
    Json(#[from] serde_json::Error),
    #[error("task context cannot fit the {limit}-byte response bound")]
    ResponseLimit { limit: u64 },
}

pub fn build_task_context(
    engine: &CodeQueryEngine,
    request: &TaskContextRequest,
    memory: &[MemoryDoc],
) -> Result<TaskContext, TaskContextError> {
    validate_request(request)?;
    validate_memory(memory)?;
    let preliminary = engine.explore(ExploreRequest {
        symbols: vec![request.target.clone()],
        root: request.repository_root.clone(),
        include_heuristic: false,
        limits: request.limits.query.clone(),
    })?;
    let preliminary_truncated = preliminary.truncated;
    let preliminary_target = resolve_target(&request.target, &preliminary);
    let search = if matches!(preliminary_target, TaskContextTarget::Exact { .. }) {
        None
    } else {
        Some(engine.search(SearchRequest {
            query: request.target.clone(),
            limits: request.limits.query.clone(),
        })?)
    };
    let target = match (&preliminary_target, search.as_ref()) {
        (TaskContextTarget::Ambiguous { .. }, Some(search)) => TaskContextTarget::Ambiguous {
            candidates: search.results.clone(),
        },
        (TaskContextTarget::NotFound { .. }, Some(search)) => {
            resolve_target(&request.target, search)
        }
        _ => preliminary_target,
    };
    let mut sections = Vec::new();
    let mut omissions = Vec::new();
    let mut query_count = 1_u64.saturating_add(u64::from(search.is_some()));
    let mut knowledge = Vec::new();

    if let TaskContextTarget::Exact { node_id } = &target {
        let declaration = preliminary;
        let mut implementation = declaration_details(&declaration);
        if !declaration.files.iter().any(|file| file.source.is_some()) {
            omissions.push(omission(
                "verified_source",
                "no source bytes passed digest verification for the exact target",
            ));
        }
        sections.push(TaskContextSection {
            kind: TaskContextSectionKind::DeclarationSource,
            evidence: declaration,
        });

        let callers = engine.callers(CallRequest {
            symbol: node_id.clone(),
            include_heuristic: false,
            limits: request.limits.query.clone(),
        })?;
        query_count = query_count.saturating_add(1);
        sections.push(TaskContextSection {
            kind: TaskContextSectionKind::ExactCallers,
            evidence: callers,
        });

        let callees = engine.callees(CallRequest {
            symbol: node_id.clone(),
            include_heuristic: false,
            limits: request.limits.query.clone(),
        })?;
        query_count = query_count.saturating_add(1);
        sections.push(TaskContextSection {
            kind: TaskContextSectionKind::ExactCallees,
            evidence: callees,
        });

        let impact = engine.impact(ImpactRequest {
            symbol: node_id.clone(),
            include_heuristic: false,
            limits: request.limits.query.clone(),
        })?;
        query_count = query_count.saturating_add(1);
        merge_responses(&mut implementation, declaration_details(&impact));
        if implementation.nodes.is_empty() && implementation.edges.is_empty() {
            omissions.push(omission(
                "implementation_type",
                "no exact implementation or type relationship was present in the bounded evidence",
            ));
        } else {
            sections.push(TaskContextSection {
                kind: TaskContextSectionKind::ImplementationType,
                evidence: implementation,
            });
        }
        let tests = filtered_response(&impact, &[EdgeKind::Calls, EdgeKind::Tests], true);
        if tests.nodes.is_empty() {
            omissions.push(omission(
                "related_tests",
                "bounded exact graph evidence did not identify a related test",
            ));
        } else {
            sections.push(TaskContextSection {
                kind: TaskContextSectionKind::RelatedTests,
                evidence: tests,
            });
        }
        if matches!(
            request.intent,
            TaskContextIntent::Modify | TaskContextIntent::Debug | TaskContextIntent::Test
        ) {
            sections.push(TaskContextSection {
                kind: TaskContextSectionKind::TransitiveImpact,
                evidence: impact,
            });
        }

        let max_knowledge =
            usize::try_from(request.limits.max_knowledge_items).unwrap_or(usize::MAX);
        for doc in memory
            .iter()
            .filter(|doc| doc.source_nodes.iter().any(|id| id == node_id))
        {
            if knowledge.len() >= max_knowledge {
                omissions.push(omission(
                    "project_knowledge_truncated",
                    "matching project knowledge exceeded maxKnowledgeItems",
                ));
                break;
            }
            let mut source_nodes = doc.source_nodes.clone();
            source_nodes.sort();
            source_nodes.dedup();
            knowledge.push(TaskContextKnowledge {
                path: doc.path.clone(),
                date: doc.date.clone(),
                question: doc.question.clone(),
                outcome: doc.outcome.clone(),
                correction: doc.correction.clone(),
                source_nodes,
                provenance: "compass_reflect_memory".to_owned(),
            });
        }
        if knowledge.is_empty() {
            omissions.push(omission(
                "history_project_knowledge",
                "no bounded reflection memory referenced the exact target identity",
            ));
        }
    } else {
        let reason = match &target {
            TaskContextTarget::Ambiguous { .. } => {
                "target is ambiguous; no structural evidence was composed"
            }
            TaskContextTarget::NotFound { .. } => {
                "target has no exact identity; search candidates were not selected"
            }
            TaskContextTarget::Exact { .. } => {
                return Err(TaskContextError::InvalidResult(
                    "exact target unexpectedly reached unresolved composition".to_owned(),
                ));
            }
        };
        omissions.push(omission("target_resolution", reason));
    }

    knowledge.sort_by(|left, right| (&left.date, &left.path).cmp(&(&right.date, &right.path)));
    omissions.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.reason.cmp(&right.reason))
    });
    omissions.dedup();
    let framework = build_framework_context(
        &target,
        &sections,
        engine.graph_identity(),
        engine.build_generation_identity(),
    )?;
    let mut context = TaskContext {
        schema: TASK_CONTEXT_SCHEMA.to_owned(),
        intent: request.intent,
        requested_target: request.target.clone(),
        target,
        graph_identity: engine.graph_identity().to_owned(),
        build_generation_identity: engine.build_generation_identity().to_owned(),
        sections,
        framework: Some(framework),
        agent_knowledge: None,
        project_knowledge: knowledge,
        omissions,
        truncated: preliminary_truncated
            || search.as_ref().is_some_and(|response| response.truncated),
        work: TaskContextWork {
            schema: TASK_CONTEXT_PROFILE_SCHEMA.to_owned(),
            query_count,
            candidates_returned: search.as_ref().map_or(0, |response| {
                u64::try_from(response.results.len()).unwrap_or(u64::MAX)
            }),
            knowledge_items_read: u64::try_from(memory.len()).unwrap_or(u64::MAX),
            ..TaskContextWork::default()
        },
        result_digest: String::new(),
    };
    context.truncated |= context
        .sections
        .iter()
        .any(|section| section.evidence.truncated);
    update_work(&mut context);
    context.result_digest = task_context_digest(&context)?;
    update_response_bytes(&mut context)?;
    if context.work.response_bytes > request.limits.max_response_bytes {
        enforce_response_bound(&mut context, request.limits.max_response_bytes)?;
        context.result_digest = task_context_digest(&context)?;
        update_response_bytes(&mut context)?;
    }
    if context.work.response_bytes > request.limits.max_response_bytes {
        return Err(TaskContextError::ResponseLimit {
            limit: request.limits.max_response_bytes,
        });
    }
    context.validate()?;
    Ok(context)
}

/// Attach only Agent Assertions that are directly relevant to the already-resolved exact target.
/// Grounding metadata stays separate from structural confidence, and no source excerpt is copied.
pub fn attach_agent_knowledge(
    context: &mut TaskContext,
    effective: &EffectiveGraph,
    state_revision: &compass_agent_graph::OverlayRevisionId,
    state: &OverlayState,
    max_records: usize,
    max_response_bytes: u64,
) -> Result<(), TaskContextError> {
    if max_records == 0 || max_records > MAX_KNOWLEDGE_ITEMS as usize {
        return Err(TaskContextError::InvalidRequest(format!(
            "Agent knowledge record limit must be between 1 and {MAX_KNOWLEDGE_ITEMS}"
        )));
    }
    if max_response_bytes == 0 || max_response_bytes > MAX_TASK_CONTEXT_BYTES as u64 {
        return Err(TaskContextError::InvalidRequest(
            "Agent knowledge response limit is zero or exceeds the task-context ceiling".to_owned(),
        ));
    }
    if state.base_generation != effective.base_generation
        || state_revision != &effective.overlay_revision
    {
        return Err(TaskContextError::InvalidResult(
            "Effective Graph and overlay state identities disagree".to_owned(),
        ));
    }
    let TaskContextTarget::Exact { node_id } = &context.target else {
        return Err(TaskContextError::InvalidRequest(
            "Agent knowledge requires an exact task-context target".to_owned(),
        ));
    };
    if context.graph_identity != effective.effective_identity.as_str() {
        return Err(TaskContextError::InvalidResult(
            "task context is not bound to the selected Effective Graph identity".to_owned(),
        ));
    }
    let incident_agent_edges = effective
        .graph
        .links
        .iter()
        .filter(|edge| edge.source == *node_id || edge.target == *node_id)
        .map(|edge| edge.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut assertions = Vec::new();
    for fact in &effective.agent_facts {
        if fact.projected_id != *node_id
            && !incident_agent_edges.contains(fact.projected_id.as_str())
        {
            continue;
        }
        let assertion = state.assertions.get(&fact.assertion).ok_or_else(|| {
            TaskContextError::InvalidResult(
                "Effective Graph metadata references a missing active assertion".to_owned(),
            )
        })?;
        if assertion.certificate_digest != fact.certificate_digest {
            return Err(TaskContextError::InvalidResult(
                "Effective Graph assertion certificate digest does not match overlay state"
                    .to_owned(),
            ));
        }
        assertions.push(AgentKnowledgeAssertion {
            assertion_id: assertion.id.clone(),
            projected_id: fact.projected_id.clone(),
            owner: assertion.owner.clone(),
            version: assertion.version,
            grounding_status: "GROUNDED".to_owned(),
            structural_confidence: "inferred".to_owned(),
            certificate_digest: assertion.certificate_digest.clone(),
            summary: assertion.summary.clone(),
            citations: assertion.grounding.evidence.clone(),
        });
    }
    let mut challenges = Vec::new();
    for effective_challenge in &effective.challenges {
        if effective_challenge.target_id != *node_id {
            continue;
        }
        let challenge = state
            .challenges
            .get(&effective_challenge.challenge)
            .ok_or_else(|| {
                TaskContextError::InvalidResult(
                    "Effective Graph metadata references a missing active Challenge".to_owned(),
                )
            })?;
        if challenge.certificate_digest != effective_challenge.certificate_digest {
            return Err(TaskContextError::InvalidResult(
                "Effective Graph Challenge certificate digest does not match overlay state"
                    .to_owned(),
            ));
        }
        challenges.push(AgentKnowledgeChallenge {
            challenge_id: challenge.id.clone(),
            target_id: effective_challenge.target_id.clone(),
            effect: challenge.effect,
            masked: effective_challenge.masked,
            grounding_status: "GROUNDED".to_owned(),
            certificate_digest: challenge.certificate_digest.clone(),
            summary: challenge.summary.clone(),
            citations: challenge.grounding.evidence.clone(),
        });
    }
    assertions.sort_by(|left, right| left.assertion_id.cmp(&right.assertion_id));
    challenges.sort_by(|left, right| left.challenge_id.cmp(&right.challenge_id));
    let available = assertions.len().saturating_add(challenges.len());
    let mut omitted_records = available.saturating_sub(max_records);
    if assertions.len() > max_records {
        assertions.truncate(max_records);
        challenges.clear();
    } else {
        challenges.truncate(max_records.saturating_sub(assertions.len()));
    }
    context.agent_knowledge = Some(AgentKnowledgeSection {
        schema: "compass.agent-knowledge/1".to_owned(),
        effective_identity: effective.effective_identity.clone(),
        base_generation: effective.base_generation.clone(),
        overlay_revision: effective.overlay_revision.clone(),
        composition_profile: effective.composition_profile,
        assertions,
        challenges,
        omissions: effective.omissions.clone(),
        truncated: omitted_records > 0,
        omitted_records: omitted_records as u64,
    });
    while u64::try_from(canonical_bytes(context)?.len()).unwrap_or(u64::MAX) > max_response_bytes {
        let Some(agent) = context.agent_knowledge.as_mut() else {
            break;
        };
        if !agent.trim_one() {
            return Err(TaskContextError::ResponseLimit {
                limit: max_response_bytes,
            });
        }
        omitted_records = omitted_records.saturating_add(1);
        agent.truncated = true;
        agent.omitted_records = omitted_records as u64;
        context.truncated = true;
    }
    update_work(context);
    context.result_digest = task_context_digest(context)?;
    update_response_bytes(context)?;
    context.validate()
}

impl AgentKnowledgeSection {
    fn trim_one(&mut self) -> bool {
        let removed = self.challenges.pop().is_some() || self.assertions.pop().is_some();
        if removed {
            self.omitted_records = self.omitted_records.saturating_add(1);
            self.truncated = true;
        }
        removed
    }
}

fn validate_request(request: &TaskContextRequest) -> Result<(), TaskContextError> {
    if request.target.is_empty()
        || request.target.len() > MAX_TASK_TARGET_BYTES
        || request.target.chars().any(char::is_control)
    {
        return Err(TaskContextError::InvalidRequest(format!(
            "target must contain 1 to {MAX_TASK_TARGET_BYTES} non-control bytes"
        )));
    }
    if !request.limits.is_valid() {
        return Err(TaskContextError::InvalidRequest(
            "limits are zero or exceed the task-context ceilings".to_owned(),
        ));
    }
    if request.repository_root.is_empty()
        || request.repository_root.len() > MAX_REPOSITORY_ROOT_BYTES
        || request.repository_root.chars().any(char::is_control)
    {
        return Err(TaskContextError::InvalidRequest(format!(
            "repository root must contain 1 to {MAX_REPOSITORY_ROOT_BYTES} non-control bytes"
        )));
    }
    Ok(())
}

fn validate_memory(memory: &[MemoryDoc]) -> Result<(), TaskContextError> {
    if memory.len() > MAX_MEMORY_DOCS {
        return Err(TaskContextError::InvalidRequest(format!(
            "project knowledge contains more than {MAX_MEMORY_DOCS} documents"
        )));
    }
    let mut input_bytes = 0_usize;
    let mut source_nodes = 0_usize;
    for doc in memory {
        for value in [
            &doc.query_type,
            &doc.date,
            &doc.question,
            &doc.outcome,
            &doc.correction,
            &doc.contributor,
            &doc.path,
        ] {
            input_bytes = input_bytes.saturating_add(value.len());
        }
        source_nodes = source_nodes.saturating_add(doc.source_nodes.len());
        for node in &doc.source_nodes {
            input_bytes = input_bytes.saturating_add(node.len());
        }
        if input_bytes > MAX_MEMORY_INPUT_BYTES || source_nodes > MAX_MEMORY_SOURCE_NODES {
            return Err(TaskContextError::InvalidRequest(format!(
                "project knowledge exceeds {MAX_MEMORY_INPUT_BYTES} bytes or {MAX_MEMORY_SOURCE_NODES} source-node references"
            )));
        }
    }
    Ok(())
}

fn validate_framework_strings(values: &[String]) -> Result<(), TaskContextError> {
    if values.iter().any(|value| {
        value.is_empty()
            || value.len() > MAX_FRAMEWORK_TEXT_BYTES
            || value.chars().any(char::is_control)
    }) {
        return Err(TaskContextError::InvalidResult(
            "framework context contains an empty, oversized, or control-bearing value".to_owned(),
        ));
    }
    Ok(())
}

fn framework_pack_id(framework: &str) -> Option<&'static str> {
    let normalized = framework.trim().to_ascii_lowercase();
    let mapped = match normalized.as_str() {
        "react" | "react-dom" => "react-ui",
        "next" | "nextjs" => "nextjs-routes",
        "react-router" | "react-router-dom" => "react-router-routes",
        "remix" => "remix-routes",
        "tanstack" | "tanstack-router" => "tanstack-router",
        "tanstack-start" => "tanstack-start",
        "vite" => "vite-config",
        "django" => "django-python",
        "django-rest-framework" | "drf" => "django-rest-framework-python",
        "fastapi" => "fastapi-python",
        "flask" => "flask-python",
        "pydantic" => "pydantic-python",
        "sqlalchemy" => "sqlalchemy-python",
        "celery" => "celery-python",
        "starlette" => "starlette-python",
        _ => return None,
    };
    Some(mapped)
}

fn framework_capabilities(pack_id: &str) -> Vec<String> {
    let values = match pack_id {
        "react-ui" => ["ui", "renders", "hooks", "client_server_boundary"].as_slice(),
        "nextjs-routes" => ["routes", "stages", "client_server_boundary", "config"].as_slice(),
        "react-router-routes" => ["routes", "loaders", "actions", "stages"].as_slice(),
        "remix-routes" => ["routes", "loaders", "actions", "stages"].as_slice(),
        "tanstack-router" => ["routes", "loaders", "components", "stages"].as_slice(),
        "tanstack-start" => ["routes", "loaders", "actions", "server_functions"].as_slice(),
        "vite-config" => ["aliases", "plugins", "file_sets", "config"].as_slice(),
        "django-python" => [
            "routes",
            "includes",
            "models",
            "fields",
            "relationships",
            "signals",
            "stages",
        ]
        .as_slice(),
        "django-rest-framework-python" => [
            "routes",
            "routers",
            "viewsets",
            "actions",
            "serializers",
            "security",
            "dependencies",
        ]
        .as_slice(),
        "fastapi-python" => ["routes", "dependencies", "mounts", "stages"].as_slice(),
        "flask-python" => ["routes", "blueprints", "factories", "hooks", "stages"].as_slice(),
        "pydantic-python" => ["models", "schemas", "dependencies"].as_slice(),
        "sqlalchemy-python" => ["models", "fields", "relationships", "table_mappings"].as_slice(),
        "celery-python" => ["tasks", "queues", "canvas", "schedules"].as_slice(),
        "starlette-python" => ["routes", "mounts", "stages"].as_slice(),
        _ => ["framework_evidence"].as_slice(),
    };
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn framework_node_id(node: &QueryNode) -> Option<&str> {
    node.framework
        .as_deref()
        .filter(|value| !value.trim().is_empty())
}

fn is_framework_relation(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Renders
            | EdgeKind::RoutesTo
            | EdgeKind::Handles
            | EdgeKind::Registers
            | EdgeKind::Produces
            | EdgeKind::Consumes
            | EdgeKind::Publishes
            | EdgeKind::Subscribes
            | EdgeKind::DependsOn
            | EdgeKind::MapsTo
            | EdgeKind::Decorates
    )
}

fn is_config_relation(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Imports | EdgeKind::DependsOn | EdgeKind::Aliases
    )
}

fn evidence_sort_key(evidence: &QueryEvidence) -> String {
    let anchor = evidence.anchor.as_ref().map_or_else(String::new, |anchor| {
        format!(
            "{}:{:020}:{:020}",
            anchor.file, anchor.start_byte, anchor.end_byte
        )
    });
    format!(
        "{}|{:?}|{:?}|{:?}|{}|{}",
        evidence.extractor,
        evidence.origin,
        evidence.confidence,
        evidence.resolution,
        evidence.rule.as_deref().unwrap_or_default(),
        anchor
    )
}

fn sorted_provenance(mut values: Vec<QueryEvidence>) -> Vec<QueryEvidence> {
    values.sort_by_key(evidence_sort_key);
    values.dedup_by(|left, right| evidence_sort_key(left) == evidence_sort_key(right));
    values
}

fn build_framework_context(
    target: &TaskContextTarget,
    sections: &[TaskContextSection],
    graph_identity: &str,
    build_generation_identity: &str,
) -> Result<FrameworkContext, TaskContextError> {
    let mut nodes = BTreeMap::<String, QueryNode>::new();
    let mut edges = BTreeMap::<String, QueryEdge>::new();
    let mut truncated = false;
    for section in sections {
        truncated |= section.evidence.truncated;
        for node in &section.evidence.nodes {
            nodes.entry(node.id.clone()).or_insert_with(|| node.clone());
        }
        for edge in &section.evidence.edges {
            edges.entry(edge.id.clone()).or_insert_with(|| edge.clone());
        }
    }

    let focus_node_id = match target {
        TaskContextTarget::Exact { node_id } => Some(node_id.clone()),
        TaskContextTarget::Ambiguous { .. } | TaskContextTarget::NotFound { .. } => None,
    };
    let focus_id = focus_node_id.as_deref();

    let mut framework_names = BTreeSet::new();
    for node in nodes.values() {
        if let Some(framework) = framework_node_id(node) {
            framework_names.insert(framework.to_owned());
        }
    }
    if edges.values().any(|edge| edge.kind == EdgeKind::Renders) {
        framework_names.insert("react".to_owned());
    }

    let mut packs = Vec::new();
    let mut unsupported = Vec::new();
    let mut incomplete = Vec::new();
    for framework in framework_names {
        let Some(pack_id) = framework_pack_id(&framework).or_else(|| {
            compass_languages::framework_pack_semantics_version(&framework)
                .map(|_| framework.as_str())
        }) else {
            unsupported.push(FrameworkCapabilityStatus {
                framework,
                capability: "framework_pack".to_owned(),
                reason: "the graph names a framework without a registered pack".to_owned(),
            });
            continue;
        };
        let Some(version) = compass_languages::framework_pack_semantics_version(pack_id) else {
            unsupported.push(FrameworkCapabilityStatus {
                framework,
                capability: "framework_pack".to_owned(),
                reason: "the graph names a pack whose semantics version is unavailable".to_owned(),
            });
            continue;
        };
        let observed_nodes = nodes
            .values()
            .filter(|node| {
                framework_node_id(node)
                    .is_some_and(|value| framework_pack_id(value).unwrap_or(value) == pack_id)
            })
            .count();
        let observed_relations = edges
            .values()
            .filter(|edge| is_framework_relation(edge.kind))
            .filter(|edge| {
                nodes.get(&edge.source).is_some_and(|node| {
                    framework_node_id(node)
                        .is_some_and(|value| framework_pack_id(value).unwrap_or(value) == pack_id)
                }) || nodes.get(&edge.target).is_some_and(|node| {
                    framework_node_id(node)
                        .is_some_and(|value| framework_pack_id(value).unwrap_or(value) == pack_id)
                })
            })
            .count();
        let has_ambiguity = nodes.values().any(|node| {
            node.framework
                .as_deref()
                .is_some_and(|value| framework_pack_id(value).unwrap_or(value) == pack_id)
                && node.evidence.iter().any(|evidence| {
                    matches!(
                        evidence.confidence,
                        compass_model::provenance::EvidenceConfidence::Ambiguous
                    ) || matches!(evidence.resolution, ResolutionState::Ambiguous)
                })
        }) || edges.values().any(|edge| {
            is_framework_relation(edge.kind)
                && edge.evidence.iter().any(|evidence| {
                    matches!(
                        evidence.confidence,
                        compass_model::provenance::EvidenceConfidence::Ambiguous
                    ) || matches!(evidence.resolution, ResolutionState::Ambiguous)
                })
        });
        let qualification = if has_ambiguity {
            FrameworkQualificationState::Ambiguous
        } else if observed_nodes == 0 && observed_relations == 0 {
            FrameworkQualificationState::Unsupported
        } else if truncated {
            FrameworkQualificationState::Incomplete
        } else {
            // A checked-in task context is evidence from the current graph;
            // promotion to `qualified` remains the independent corpus gate's
            // responsibility.
            FrameworkQualificationState::Qualifying
        };
        packs.push(FrameworkPackContext {
            id: pack_id.to_owned(),
            version,
            qualification,
            capabilities: framework_capabilities(pack_id),
            observed_nodes: u32::try_from(observed_nodes).unwrap_or(u32::MAX),
            observed_relations: u32::try_from(observed_relations).unwrap_or(u32::MAX),
        });
    }

    let mut routes = Vec::new();
    let mut runtime_boundaries = Vec::new();
    for node in nodes.values() {
        if node.kind == NodeKind::Route
            && focus_id.is_none_or(|focus| {
                node.id == focus
                    || edges.values().any(|edge| {
                        is_framework_relation(edge.kind)
                            && (edge.source == focus && edge.target == node.id
                                || edge.target == focus && edge.source == node.id)
                    })
            })
            && let Some(NodeDetails::Route(details)) = node.details.as_ref()
        {
            routes.push(FrameworkRouteContext {
                node_id: node.id.clone(),
                framework: node
                    .framework
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
                operation: details.operation.clone(),
                path: details.path.clone(),
                declaring_scope: details.declaring_scope.clone(),
                resolution: details.resolution,
                stages: details
                    .stages
                    .iter()
                    .map(|stage| FrameworkStageContext {
                        stage: stage.stage,
                        position: stage.position,
                        reference: stage.reference.clone(),
                        resolution: stage.resolution,
                        source: stage.source_anchor.clone(),
                        target: stage.target.clone(),
                        provenance: sorted_provenance(node.evidence.clone()),
                    })
                    .collect(),
                provenance: sorted_provenance(node.evidence.clone()),
            });
        }
        let boundary_roles = node
            .roles
            .iter()
            .copied()
            .filter(|role| {
                matches!(
                    role,
                    NodeRole::ClientBoundary
                        | NodeRole::ClientComponent
                        | NodeRole::ServerComponent
                        | NodeRole::ServerFunction
                )
            })
            .collect::<Vec<_>>();
        if !boundary_roles.is_empty()
            && focus_id.is_none_or(|focus| {
                node.id == focus
                    || edges.values().any(|edge| {
                        (edge.source == focus && edge.target == node.id)
                            || (edge.target == focus && edge.source == node.id)
                    })
            })
        {
            runtime_boundaries.push(FrameworkBoundaryContext {
                node_id: node.id.clone(),
                framework: node
                    .framework
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
                roles: boundary_roles,
                source: node.source.clone(),
                provenance: sorted_provenance(node.evidence.clone()),
            });
        }
    }

    let mut relations = Vec::new();
    let mut rendered_by = Vec::new();
    let mut renders = Vec::new();
    let mut config_dependencies = Vec::new();
    let mut ambiguities = Vec::new();
    for edge in edges
        .values()
        .filter(|edge| is_framework_relation(edge.kind) || is_config_relation(edge.kind))
    {
        let touches_focus =
            focus_id.is_none_or(|focus| edge.source == focus || edge.target == focus);
        if !touches_focus {
            continue;
        }
        let provenance = sorted_provenance(edge.evidence.clone());
        let relation = FrameworkRelationContext {
            id: edge.id.clone(),
            relation: edge.kind,
            source: edge.source.clone(),
            target: edge.target.clone(),
            details: edge.details.clone(),
            relationship_site: edge.relationship_site.clone(),
            provenance: provenance.clone(),
        };
        if edge.kind == EdgeKind::Renders {
            if focus_id == Some(edge.target.as_str()) {
                rendered_by.push(relation.clone());
            }
            if focus_id == Some(edge.source.as_str()) {
                renders.push(relation.clone());
            }
        }
        if is_config_relation(edge.kind) {
            config_dependencies.push(relation.clone());
        }
        if is_framework_relation(edge.kind) {
            relations.push(relation);
        }
        for evidence in &provenance {
            if !matches!(evidence.resolution, ResolutionState::Exact)
                || !evidence.candidates.is_empty()
            {
                ambiguities.push(FrameworkAmbiguity {
                    kind: edge.kind.as_str().to_owned(),
                    reference: edge.id.clone(),
                    candidates: evidence
                        .candidates
                        .iter()
                        .map(|candidate| candidate.node_id.clone())
                        .collect(),
                });
            }
        }
    }

    if let TaskContextTarget::Ambiguous { candidates }
    | TaskContextTarget::NotFound { candidates } = target
    {
        ambiguities.push(FrameworkAmbiguity {
            kind: "target".to_owned(),
            reference: "requested_target".to_owned(),
            candidates: candidates
                .iter()
                .map(|candidate| candidate.node_id.clone())
                .collect(),
        });
    }
    for section in sections {
        if section.evidence.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == QueryDiagnosticCode::IncompleteCoverage
                || diagnostic.code == QueryDiagnosticCode::BoundedTruncation
        }) {
            incomplete.push(FrameworkCapabilityStatus {
                framework: "graph".to_owned(),
                capability: "coverage".to_owned(),
                reason: "the bounded query response reported incomplete coverage".to_owned(),
            });
        }
    }

    routes.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    runtime_boundaries.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    relations.sort_by(|left, right| left.id.cmp(&right.id));
    rendered_by.sort_by(|left, right| left.id.cmp(&right.id));
    renders.sort_by(|left, right| left.id.cmp(&right.id));
    config_dependencies.sort_by(|left, right| left.id.cmp(&right.id));
    ambiguities.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.reference.cmp(&right.reference))
    });
    ambiguities.dedup();

    let mut context = FrameworkContext {
        schema: FRAMEWORK_CONTEXT_SCHEMA.to_owned(),
        graph_identity: graph_identity.to_owned(),
        build_generation_identity: build_generation_identity.to_owned(),
        focus_node_id,
        packs,
        routes,
        relations,
        rendered_by,
        renders,
        config_dependencies,
        runtime_boundaries,
        unsupported,
        incomplete,
        ambiguities,
        truncated,
        record_limit: MAX_FRAMEWORK_CONTEXT_RECORDS as u32,
        byte_limit: MAX_FRAMEWORK_CONTEXT_BYTES as u64,
    };
    let record_count = framework_record_count(&context);
    if record_count > MAX_FRAMEWORK_CONTEXT_RECORDS {
        context.truncated = true;
        while framework_record_count(&context) > MAX_FRAMEWORK_CONTEXT_RECORDS && context.trim_one()
        {
        }
    }
    while canonical_bytes(&context)?.len() > MAX_FRAMEWORK_CONTEXT_BYTES && context.trim_one() {
        context.truncated = true;
    }
    context.validate()?;
    Ok(context)
}

fn framework_record_count(context: &FrameworkContext) -> usize {
    context.packs.len()
        + context.routes.len()
        + context.relations.len()
        + context.rendered_by.len()
        + context.renders.len()
        + context.config_dependencies.len()
        + context.runtime_boundaries.len()
        + context.unsupported.len()
        + context.incomplete.len()
        + context.ambiguities.len()
}

fn resolve_target(requested: &str, search: &CodeQueryResponse) -> TaskContextTarget {
    let exact_ids = search
        .nodes
        .iter()
        .filter(|node| {
            node.id == requested || node.name == requested || node.qualified_name == requested
        })
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    if search
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::AmbiguousMatch)
    {
        return TaskContextTarget::Ambiguous {
            candidates: search.results.clone(),
        };
    }
    if exact_ids.len() == 1
        && let Some(node_id) = exact_ids.iter().next().cloned()
    {
        return TaskContextTarget::Exact { node_id };
    }
    let candidates = search.results.clone();
    if exact_ids.len() > 1 {
        TaskContextTarget::Ambiguous { candidates }
    } else {
        TaskContextTarget::NotFound { candidates }
    }
}

fn filtered_response(
    source: &CodeQueryResponse,
    kinds: &[EdgeKind],
    tests_only: bool,
) -> CodeQueryResponse {
    let selected_nodes = source
        .nodes
        .iter()
        .filter(|node| tests_only && is_test_node(node))
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let edges = source
        .edges
        .iter()
        .filter(|edge| {
            (kinds.is_empty() || kinds.contains(&edge.kind))
                && (!tests_only
                    || selected_nodes.contains(&edge.source)
                    || selected_nodes.contains(&edge.target))
        })
        .cloned()
        .collect::<Vec<QueryEdge>>();
    let mut admitted = selected_nodes;
    for edge in &edges {
        admitted.insert(edge.source.clone());
        admitted.insert(edge.target.clone());
    }
    let nodes = source
        .nodes
        .iter()
        .filter(|node| admitted.contains(&node.id))
        .cloned()
        .collect::<Vec<QueryNode>>();
    let mut response = CodeQueryResponse::empty(CodeQueryOperation::Impact, source.limits.clone());
    response.nodes = nodes;
    response.edges = edges;
    copy_verified_files(source, &mut response);
    response.truncated = source.truncated;
    response
}

fn declaration_details(source: &CodeQueryResponse) -> CodeQueryResponse {
    const TYPE_RELATIONSHIPS: &[EdgeKind] = &[
        EdgeKind::Embeds,
        EdgeKind::Extends,
        EdgeKind::Implements,
        EdgeKind::MixesIn,
        EdgeKind::TypeOf,
        EdgeKind::Returns,
        EdgeKind::Instantiates,
        EdgeKind::Overrides,
    ];
    let mut response = filtered_response(source, TYPE_RELATIONSHIPS, false);
    response.operation = CodeQueryOperation::Explore;
    let mut admitted = response
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    for node in source.nodes.iter().filter(|node| {
        node.details.is_some()
            || matches!(
                node.kind,
                compass_model::code_graph::NodeKind::Class
                    | compass_model::code_graph::NodeKind::Interface
                    | compass_model::code_graph::NodeKind::Trait
                    | compass_model::code_graph::NodeKind::Struct
                    | compass_model::code_graph::NodeKind::Enum
                    | compass_model::code_graph::NodeKind::TypeAlias
            )
    }) {
        if admitted.insert(node.id.clone()) {
            response.nodes.push(node.clone());
        }
    }
    response.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    copy_verified_files(source, &mut response);
    response.truncated = source.truncated;
    response
}

fn copy_verified_files(source: &CodeQueryResponse, target: &mut CodeQueryResponse) {
    let paths = target
        .nodes
        .iter()
        .filter_map(|node| node.source.as_ref().map(|anchor| anchor.file.clone()))
        .collect::<BTreeSet<_>>();
    target.files = source
        .files
        .iter()
        .filter(|file| paths.contains(&file.path))
        .cloned()
        .collect();
}

fn merge_responses(target: &mut CodeQueryResponse, additional: CodeQueryResponse) {
    let mut node_ids = target
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    target.nodes.extend(
        additional
            .nodes
            .into_iter()
            .filter(|node| node_ids.insert(node.id.clone())),
    );
    let mut edge_ids = target
        .edges
        .iter()
        .map(|edge| edge.id.clone())
        .collect::<BTreeSet<_>>();
    target.edges.extend(
        additional
            .edges
            .into_iter()
            .filter(|edge| edge_ids.insert(edge.id.clone())),
    );
    let mut paths = target
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    target.files.extend(
        additional
            .files
            .into_iter()
            .filter(|file| paths.insert(file.path.clone())),
    );
    target.truncated |= additional.truncated;
    target.sort_stable();
}

fn is_test_node(node: &QueryNode) -> bool {
    let path = node
        .source
        .as_ref()
        .map(|source| source.file.to_ascii_lowercase())
        .unwrap_or_default();
    let name = node.name.to_ascii_lowercase();
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.starts_with("test/")
        || path.contains("/test/")
        || name.starts_with("test_")
        || name.ends_with("_test")
}

fn omission(category: &str, reason: &str) -> TaskContextOmission {
    TaskContextOmission {
        category: category.to_owned(),
        reason: reason.to_owned(),
    }
}

fn update_work(context: &mut TaskContext) {
    context.work.nodes_returned = context
        .sections
        .iter()
        .map(|section| u64::try_from(section.evidence.nodes.len()).unwrap_or(u64::MAX))
        .sum();
    context.work.edges_returned = context
        .sections
        .iter()
        .map(|section| u64::try_from(section.evidence.edges.len()).unwrap_or(u64::MAX))
        .sum();
    context.work.files_verified = context
        .sections
        .iter()
        .map(|section| {
            u64::try_from(
                section
                    .evidence
                    .files
                    .iter()
                    .filter(|file| file.source.is_some())
                    .count(),
            )
            .unwrap_or(u64::MAX)
        })
        .sum();
    context.work.source_bytes = context
        .sections
        .iter()
        .flat_map(|section| &section.evidence.files)
        .filter_map(|file| file.source.as_ref())
        .map(|source| u64::try_from(source.len()).unwrap_or(u64::MAX))
        .sum();
    context.work.framework_records = context
        .framework
        .as_ref()
        .map(|framework| u64::try_from(framework_record_count(framework)).unwrap_or(u64::MAX))
        .unwrap_or(0);
    context.work.framework_bytes = context
        .framework
        .as_ref()
        .and_then(|framework| canonical_bytes(framework).ok())
        .map(|bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    context.work.agent_knowledge_records = context
        .agent_knowledge
        .as_ref()
        .map(|agent| {
            u64::try_from(
                agent
                    .assertions
                    .len()
                    .saturating_add(agent.challenges.len()),
            )
            .unwrap_or(u64::MAX)
        })
        .unwrap_or(0);
    context.work.agent_knowledge_bytes = context
        .agent_knowledge
        .as_ref()
        .and_then(|agent| canonical_bytes(agent).ok())
        .map(|bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX))
        .unwrap_or(0);
}

fn update_response_bytes(context: &mut TaskContext) -> Result<(), TaskContextError> {
    for _ in 0..4 {
        let actual = u64::try_from(canonical_bytes(context)?.len()).unwrap_or(u64::MAX);
        if actual == context.work.response_bytes {
            return Ok(());
        }
        context.work.response_bytes = actual;
    }
    Err(TaskContextError::InvalidResult(
        "response byte accounting did not converge within four passes".to_owned(),
    ))
}

fn enforce_response_bound(context: &mut TaskContext, limit: u64) -> Result<(), TaskContextError> {
    while u64::try_from(canonical_bytes(context)?.len()).unwrap_or(u64::MAX) > limit {
        if let Some(agent) = context.agent_knowledge.as_mut()
            && agent.trim_one()
        {
            context.truncated = true;
            context.omissions.push(omission(
                "agent_knowledge_budget",
                "omitted lower-priority Agent Graph evidence after reaching maxResponseBytes",
            ));
            update_work(context);
        } else if let Some(framework) = context.framework.as_mut()
            && framework.trim_one()
        {
            context.truncated = true;
            context.omissions.push(omission(
                "framework_context_budget",
                "omitted lower-priority framework evidence after reaching maxResponseBytes",
            ));
            update_work(context);
        } else if let Some(section) = context.sections.pop() {
            context.truncated = true;
            context.omissions.push(omission(
                "response_budget",
                &format!("omitted {:?} after reaching maxResponseBytes", section.kind),
            ));
            context.omissions.sort_by(|left, right| {
                left.category
                    .cmp(&right.category)
                    .then_with(|| left.reason.cmp(&right.reason))
            });
            update_work(context);
        } else if context.project_knowledge.pop().is_some() {
            context.truncated = true;
            context.omissions.push(omission(
                "response_budget",
                "omitted project knowledge after reaching maxResponseBytes",
            ));
        } else {
            return Err(TaskContextError::ResponseLimit { limit });
        }
    }
    Ok(())
}

fn task_context_digest(context: &TaskContext) -> Result<String, TaskContextError> {
    let mut value = serde_json::to_value(context)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("resultDigest");
        if let Some(work) = object.get_mut("work").and_then(Value::as_object_mut) {
            work.remove("responseBytes");
        }
    }
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(canonical_value_bytes(value)?)
    ))
}

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    canonical_value_bytes(serde_json::to_value(value)?)
}

fn canonical_value_bytes(mut value: Value) -> Result<Vec<u8>, serde_json::Error> {
    sort_json(&mut value);
    serde_json::to_vec(&value)
}

fn sort_json(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let old = std::mem::take(object);
            let mut entries = old.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            for (key, mut child) in entries {
                sort_json(&mut child);
                object.insert(key, child);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(sort_json),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use compass_model::code_graph::NodeKind;
    use compass_model::query_contract::{
        CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, QueryDiagnostic,
        QueryDiagnosticCode, QueryNode, SearchHit,
    };

    use super::{TaskContextTarget, framework_capabilities, framework_pack_id, resolve_target};

    fn node(id: &str, name: &str, qualified_name: &str) -> QueryNode {
        QueryNode {
            id: id.to_owned(),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: name.to_owned(),
            qualified_name: qualified_name.to_owned(),
            language: Some("rust".to_owned()),
            framework: None,
            source: None,
            details: None,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn exact_identity_is_selected_but_fuzzy_candidate_is_not() {
        let mut response =
            CodeQueryResponse::empty(CodeQueryOperation::Search, CodeQueryLimits::default());
        response
            .nodes
            .push(node("node:parse", "parse", "Parser::parse"));
        response.results.push(SearchHit {
            node_id: "node:parse".to_owned(),
            score: 1.0,
            matched_fields: vec!["name".to_owned()],
        });
        assert_eq!(
            resolve_target("Parser::parse", &response),
            TaskContextTarget::Exact {
                node_id: "node:parse".to_owned()
            }
        );
        assert!(matches!(
            resolve_target("parser-ish", &response),
            TaskContextTarget::NotFound { .. }
        ));
    }

    #[test]
    fn python_frameworks_map_to_independent_versioned_packs() {
        assert_eq!(framework_pack_id("django"), Some("django-python"));
        assert_eq!(
            framework_pack_id("django-rest-framework"),
            Some("django-rest-framework-python")
        );
        assert_eq!(framework_pack_id("fastapi"), Some("fastapi-python"));
        assert_eq!(framework_pack_id("flask"), Some("flask-python"));
        assert_eq!(framework_pack_id("pydantic"), Some("pydantic-python"));
        assert_eq!(framework_pack_id("sqlalchemy"), Some("sqlalchemy-python"));
        assert_eq!(framework_pack_id("celery"), Some("celery-python"));
        assert_eq!(framework_pack_id("starlette"), Some("starlette-python"));
        assert!(framework_capabilities("fastapi-python").contains(&"mounts".to_owned()));
        assert!(framework_capabilities("pydantic-python").contains(&"models".to_owned()));
        assert!(framework_capabilities("sqlalchemy-python").contains(&"table_mappings".to_owned()));
        assert!(framework_capabilities("celery-python").contains(&"canvas".to_owned()));
        assert!(framework_capabilities("starlette-python").contains(&"routes".to_owned()));
        assert!(
            framework_capabilities("django-rest-framework-python")
                .contains(&"serializers".to_owned())
        );
        assert_eq!(framework_pack_id("python-web"), None);
    }

    #[test]
    fn ambiguous_resolution_preserves_candidates_without_selecting_first()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut response =
            CodeQueryResponse::empty(CodeQueryOperation::Search, CodeQueryLimits::default());
        response.nodes.extend([
            node("node:a", "parse", "a::parse"),
            node("node:b", "parse", "b::parse"),
        ]);
        response.results.extend([
            SearchHit {
                node_id: "node:a".to_owned(),
                score: 1.0,
                matched_fields: vec!["name".to_owned()],
            },
            SearchHit {
                node_id: "node:b".to_owned(),
                score: 1.0,
                matched_fields: vec!["name".to_owned()],
            },
        ]);
        response.diagnostics.push(QueryDiagnostic {
            code: QueryDiagnosticCode::AmbiguousMatch,
            message: "matched two nodes".to_owned(),
            node_id: None,
            path: None,
        });
        let TaskContextTarget::Ambiguous { candidates } = resolve_target("parse", &response) else {
            return Err("ambiguous target was unexpectedly selected".into());
        };
        assert_eq!(candidates.len(), 2);
        Ok(())
    }
}
