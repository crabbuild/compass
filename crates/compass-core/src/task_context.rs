use std::collections::BTreeSet;

use compass_model::code_graph::EdgeKind;
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, ExploreRequest,
    ImpactRequest, QueryDiagnosticCode, QueryEdge, QueryNode, SearchHit, SearchRequest,
};
use compass_query::CodeQueryEngine;
use compass_reflect::MemoryDoc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const TASK_CONTEXT_SCHEMA: &str = "compass.task-context/1";
pub const TASK_CONTEXT_PROFILE_SCHEMA: &str = "compass.task-context-profile/1";
const MAX_TASK_TARGET_BYTES: usize = 16 * 1024;
const MAX_REPOSITORY_ROOT_BYTES: usize = 32 * 1024;
const MAX_KNOWLEDGE_ITEMS: u32 = 100;
const MAX_MEMORY_DOCS: usize = 10_000;
const MAX_MEMORY_SOURCE_NODES: usize = 100_000;
const MAX_MEMORY_INPUT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TASK_CONTEXT_BYTES: usize = 64 * 1024 * 1024;

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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskContextSection {
    pub kind: TaskContextSectionKind,
    pub evidence: CodeQueryResponse,
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
    pub response_bytes: u64,
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
    let mut context = TaskContext {
        schema: TASK_CONTEXT_SCHEMA.to_owned(),
        intent: request.intent,
        requested_target: request.target.clone(),
        target,
        graph_identity: engine.graph_identity().to_owned(),
        build_generation_identity: engine.build_generation_identity().to_owned(),
        sections,
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
        if let Some(section) = context.sections.pop() {
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

    use super::{TaskContextTarget, resolve_target};

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
    fn ambiguous_resolution_preserves_candidates_without_selecting_first() {
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
            assert!(false, "ambiguous target was unexpectedly selected");
            return;
        };
        assert_eq!(candidates.len(), 2);
    }
}
