//! Native MCP service for Compass-compatible Compass graph queries.

mod code_query;
mod transport;

pub use transport::{HttpOptions, serve_http, serve_stdio};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use std::time::{Duration, Instant};

use compass_core::LoadedGraph;
use compass_graph::{Communities, god_nodes, suggest_questions, surprising_connections};
use compass_model::code_graph::GraphDocument as CodeGraphDocument;
use compass_model::query_contract::{
    MAX_DISCOVERY_CANDIDATES, MAX_DISCOVERY_DEPTH, MAX_DISCOVERY_EDGES,
    MAX_DISCOVERY_EXPANDED_RELATIONSHIPS, MAX_DISCOVERY_FILTER_BYTES, MAX_DISCOVERY_FILTERS,
    MAX_DISCOVERY_NODES, MAX_DISCOVERY_QUESTION_BYTES, MAX_DISCOVERY_RESPONSE_BYTES,
    MAX_DISCOVERY_SEEDS, MAX_DISCOVERY_TIMEOUT_MS,
};
use compass_model::{Graph, GraphDocument, NodeIndex};
use compass_output::{
    AgentOrientation, ORIENTATION_JSON_MAX_BYTES, render_agent_report_markdown,
    render_orientation_json, validate_orientation_graph_identity,
};
use compass_prs::{
    ChangeRequestSource, LocalGitChangeRequestSource, ProcessRunner, SystemRunner,
    compute_pr_impact, detect_default_branch, detect_repository_identity, fetch_pr_files,
    fetch_prs, fetch_worktrees, format_prs_text, parse_ci,
};
use compass_query::{
    TraversalMode, find_node, pick_scored_endpoint, query_graph_text, sanitize_label, score_nodes,
};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorData, Implementation,
    ListResourcesResult, ListToolsResult, Meta, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const SERVER_NAME: &str = "compass";
const MAX_QUERY_LOG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_QUERY_LOG_RECORD_BYTES: usize = 128 * 1024;
const MAX_LOGGED_QUESTION_BYTES: usize = 4_096;
const MAX_MCP_STRUCTURED_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_MCP_RESOURCE_BYTES: usize = ORIENTATION_JSON_MAX_BYTES;
const MCP_TOOL_RESULT_SCHEMA: &str = "compass.mcp.tool-result/1";
const MCP_TRANSPORT_TRUNCATION_SCHEMA: &str = "compass.mcp.transport-truncation/1";

#[derive(Debug)]
enum InvocationError {
    InvalidParams(String),
    Internal(String),
    TransportLimit {
        required_bytes: usize,
        limit_bytes: usize,
        omitted_bytes: usize,
    },
}

impl InvocationError {
    fn protocol_error(self) -> ErrorData {
        match self {
            Self::InvalidParams(message) => ErrorData::invalid_params(message, None),
            Self::Internal(message) => ErrorData::internal_error(message, None),
            Self::TransportLimit {
                required_bytes,
                limit_bytes,
                omitted_bytes,
            } => ErrorData::internal_error(
                "MCP transport bound would truncate a semantic result".to_owned(),
                Some(json!({
                    "schema": MCP_TRANSPORT_TRUNCATION_SCHEMA,
                    "truncated": true,
                    "requiredBytes": required_bytes,
                    "limitBytes": limit_bytes,
                    "omittedBytes": omitted_bytes,
                })),
            ),
        }
    }
}

impl std::fmt::Display for InvocationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParams(message) | Self::Internal(message) => formatter.write_str(message),
            Self::TransportLimit {
                required_bytes,
                limit_bytes,
                omitted_bytes,
            } => write!(
                formatter,
                "MCP response requires {required_bytes} bytes; transport limit is {limit_bytes} bytes ({omitted_bytes} omitted)"
            ),
        }
    }
}

impl From<String> for InvocationError {
    fn from(message: String) -> Self {
        Self::InvalidParams(message)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileKey {
    modified: Option<SystemTime>,
    size: u64,
}

#[derive(Debug)]
struct GraphContext {
    path: PathBuf,
    graph: Graph,
    overlay: HashMap<String, Map<String, Value>>,
    communities: BTreeMap<usize, Vec<NodeIndex>>,
    typed_query_supported: bool,
    typed_document: Option<CodeGraphDocument>,
    typed_graph_identity: Option<String>,
}

impl GraphContext {
    fn load(path: &Path) -> Result<Self, String> {
        let loaded = LoadedGraph::load_directed(path).map_err(|error| error.to_string())?;
        let (typed_document, typed_graph_identity) =
            match CodeGraphDocument::load_with_artifact_digest(path) {
                Ok((document, identity)) => (Some(document), Some(identity)),
                Err(_) => (None, None),
            };
        let typed_query_supported = typed_document.is_some();
        let mut communities = BTreeMap::<usize, Vec<NodeIndex>>::new();
        for (index, node) in loaded.graph.nodes() {
            if let Some(community) = node
                .unsigned("community")
                .or_else(|| node.string("community").parse().ok())
                .and_then(|value| usize::try_from(value).ok())
            {
                communities.entry(community).or_default().push(index);
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            graph: loaded.graph,
            overlay: loaded.overlay,
            communities,
            typed_query_supported,
            typed_document,
            typed_graph_identity,
        })
    }

    fn document(&self) -> Result<GraphDocument, String> {
        GraphDocument::load(&self.path).map_err(|error| error.to_string())
    }

    fn community_ids(&self) -> Communities {
        self.communities
            .iter()
            .map(|(community, nodes)| {
                (
                    *community,
                    nodes
                        .iter()
                        .map(|index| self.graph.node(*index).id.clone())
                        .collect(),
                )
            })
            .collect()
    }
}

#[derive(Debug)]
struct CacheEntry {
    key: FileKey,
    context: Arc<GraphContext>,
}

struct StoreInner {
    default_graph: PathBuf,
    cache: Mutex<HashMap<PathBuf, CacheEntry>>,
    typed_queries: compass_query::QueryEngineCache,
}

/// Hot-reloading, multi-project graph store shared by every MCP session.
#[derive(Clone)]
pub struct GraphStore {
    inner: Arc<StoreInner>,
}

impl std::fmt::Debug for GraphStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GraphStore")
            .field("default_graph", &self.inner.default_graph)
            .field("typed_query_engines", &self.inner.typed_queries.len())
            .finish()
    }
}

impl GraphStore {
    #[must_use]
    pub fn new(default_graph: impl Into<PathBuf>) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                default_graph: default_graph.into(),
                cache: Mutex::new(HashMap::new()),
                typed_queries: compass_query::QueryEngineCache::default(),
            }),
        }
    }

    fn resolve(&self, project_path: Option<&str>) -> Result<PathBuf, String> {
        project_path.map_or_else(
            || {
                compass_files::BuildGuard::resolve_requested_artifact(&self.inner.default_graph)
                    .map_err(|error| error.to_string())
            },
            |project| {
                let output = std::env::var_os("COMPASS_OUT")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("compass-out"));
                let output = Path::new(project).join(output);
                compass_files::BuildGuard::resolve_requested_artifact(&output.join("graph.json"))
                    .map_err(|error| error.to_string())
            },
        )
    }

    fn source_root(&self, project_path: Option<&str>) -> Result<PathBuf, String> {
        project_path.map_or_else(
            || std::env::current_dir().map_err(|error| error.to_string()),
            |project| Ok(PathBuf::from(project)),
        )
    }

    fn load(&self, project_path: Option<&str>) -> Result<Arc<GraphContext>, String> {
        let path = self.resolve(project_path)?;
        let metadata =
            fs::metadata(&path).map_err(|_| format!("graph.json not found: {}", path.display()))?;
        let key = FileKey {
            modified: metadata.modified().ok(),
            size: metadata.len(),
        };
        if let Some(context) = self
            .inner
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&path)
            .filter(|entry| entry.key == key)
            .map(|entry| Arc::clone(&entry.context))
        {
            return Ok(context);
        }
        let context = Arc::new(GraphContext::load(&path)?);
        self.inner
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                path,
                CacheEntry {
                    key,
                    context: Arc::clone(&context),
                },
            );
        Ok(context)
    }
}

/// One MCP service instance. Clones share the hot-reload cache.
#[derive(Clone, Debug)]
pub struct CompassMcp {
    store: GraphStore,
}

impl CompassMcp {
    #[must_use]
    pub fn new(graph_path: impl Into<PathBuf>) -> Self {
        Self {
            store: GraphStore::new(graph_path),
        }
    }

    #[must_use]
    pub fn tools() -> Vec<Tool> {
        tool_specs()
    }

    #[must_use]
    pub fn resources() -> Vec<Resource> {
        resource_specs()
    }

    /// Invoke a graph tool without a transport, primarily for compatibility tests.
    #[must_use]
    pub fn invoke(&self, name: &str, mut arguments: Map<String, Value>) -> String {
        self.invoke_result(name, &mut arguments)
            .map(|result| {
                result
                    .structured_content
                    .map_or(result.text, |value| value.to_string())
            })
            .unwrap_or_else(|error| format!("Error executing {name}: {error}"))
    }

    /// Read a compass resource without a transport.
    pub fn read(&self, uri: &str) -> Result<String, String> {
        self.read_result(uri).map_err(|error| error.to_string())
    }

    fn read_result(&self, uri: &str) -> Result<String, InvocationError> {
        let context = self.store.load(None).map_err(InvocationError::Internal)?;
        let text = read_resource_text(uri, &context)?;
        if text.len() > MAX_MCP_RESOURCE_BYTES {
            return Err(InvocationError::TransportLimit {
                required_bytes: text.len(),
                limit_bytes: MAX_MCP_RESOURCE_BYTES,
                omitted_bytes: text.len().saturating_sub(MAX_MCP_RESOURCE_BYTES),
            });
        }
        Ok(text)
    }
}

impl ServerHandler for CompassMcp {
    fn get_info(&self) -> ServerInfo {
        let mut capabilities = ServerCapabilities::builder()
            .enable_experimental()
            .enable_tools()
            .enable_resources()
            .build();
        if let Some(resources) = capabilities.resources.as_mut() {
            resources.subscribe = Some(false);
            resources.list_changed = Some(false);
        }
        if let Some(tools) = capabilities.tools.as_mut() {
            tools.list_changed = Some(false);
        }
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tool_specs()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut arguments = request.arguments.unwrap_or_default();
        let result = self
            .invoke_result(&request.name, &mut arguments)
            .map_err(InvocationError::protocol_error)?;
        let mut response = result.structured_content.map_or_else(
            || CallToolResult::success(Vec::new()),
            CallToolResult::structured,
        );
        response.content = vec![ContentBlock::text(result.text)];
        Ok(response)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(resource_specs()))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let text = self
            .read_result(&request.uri)
            .map_err(InvocationError::protocol_error)?;
        let mime = match request.uri.as_str() {
            "compass://report" => "text/markdown",
            "compass://orientation" => "application/json",
            _ => "text/plain",
        };
        let required_bytes = text.len();
        let transport = Meta(Map::from_iter([(
            "transportTruncation".to_owned(),
            json!({
                "schema": MCP_TRANSPORT_TRUNCATION_SCHEMA,
                "truncated": false,
                "requiredBytes": required_bytes,
                "limitBytes": MAX_MCP_RESOURCE_BYTES,
                "omittedBytes": 0,
            }),
        )]));
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, request.uri)
                .with_mime_type(mime)
                .with_meta(transport),
        ]))
    }
}

struct ToolInvocation {
    text: String,
    structured_content: Option<Value>,
}

impl CompassMcp {
    fn invoke_result(
        &self,
        name: &str,
        arguments: &mut Map<String, Value>,
    ) -> Result<ToolInvocation, InvocationError> {
        if !tool_specs().iter().any(|tool| tool.name == name) {
            return Err(InvocationError::InvalidParams(format!(
                "unknown tool: {name}"
            )));
        }
        if name == "query_graph" {
            code_query::validate_query_graph_arguments(arguments)?;
        }
        let project_path = match arguments.remove("project_path") {
            Some(Value::String(path)) => Some(path),
            Some(_) => {
                return Err(InvocationError::InvalidParams(
                    "project_path must be a string".to_owned(),
                ));
            }
            None => None,
        };
        if matches!(name, "review_pull_request" | "pr_readiness") {
            let root = self
                .store
                .source_root(project_path.as_deref())
                .map_err(InvocationError::Internal)?;
            return invoke_review_tool(arguments, &root, name == "pr_readiness");
        }
        if name == "task_context" {
            let graph_path = self
                .store
                .resolve(project_path.as_deref())
                .map_err(InvocationError::Internal)?;
            let root = self
                .store
                .source_root(project_path.as_deref())
                .map_err(InvocationError::Internal)?;
            let context = if compass_query::has_published_store(&graph_path) {
                None
            } else {
                Some(
                    self.store
                        .load(project_path.as_deref())
                        .map_err(InvocationError::Internal)?,
                )
            };
            return invoke_task_context(
                &self.store,
                arguments,
                &graph_path,
                &root,
                context.as_deref(),
            );
        }
        if name == "query_graph" && natural_discovery_requested(arguments) {
            let graph_path = self
                .store
                .resolve(project_path.as_deref())
                .map_err(InvocationError::Internal)?;
            if compass_query::has_published_store(&graph_path) {
                return invoke_discovery_tool(&self.store, arguments, &graph_path, None);
            }
        }
        let typed_query = matches!(
            name,
            "search_symbols"
                | "get_callers"
                | "get_callees"
                | "get_impact"
                | "explore_code"
                | "get_node"
        );
        if typed_query {
            let graph_path = self
                .store
                .resolve(project_path.as_deref())
                .map_err(InvocationError::Internal)?;
            if compass_query::has_published_store(&graph_path) {
                return invoke_typed_tool(&self.store, name, arguments, &graph_path, None);
            }
        }
        let context = self
            .store
            .load(project_path.as_deref())
            .map_err(InvocationError::Internal)?;
        if name == "query_graph" && should_route_natural_query(arguments, &context)? {
            return invoke_discovery_tool(&self.store, arguments, &context.path, Some(&context));
        }
        if typed_query {
            return invoke_typed_tool(&self.store, name, arguments, &context.path, Some(&context));
        }
        Ok(ToolInvocation {
            text: invoke_tool(name, arguments, &context).map_err(InvocationError::InvalidParams)?,
            structured_content: None,
        })
    }
}

fn invoke_typed_tool(
    store: &GraphStore,
    name: &str,
    arguments: &Map<String, Value>,
    graph_path: &Path,
    context: Option<&GraphContext>,
) -> Result<ToolInvocation, InvocationError> {
    let engine = cached_typed_engine(store, graph_path, context)?;
    let engine = engine
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let response = code_query::invoke_with_engine(name, arguments, &engine)?;
    let text = format!(
        "{:?}: {} nodes, {} edges, {} paths{}",
        response.operation,
        response.nodes.len(),
        response.edges.len(),
        response.paths.len(),
        if response.truncated {
            " (truncated)"
        } else {
            ""
        }
    );
    Ok(ToolInvocation {
        text,
        structured_content: Some(transport_envelope(
            serde_json::to_value(response)
                .map_err(|error| InvocationError::Internal(error.to_string()))?,
        )?),
    })
}

fn natural_discovery_requested(arguments: &Map<String, Value>) -> bool {
    !["mode", "depth", "token_budget", "context_filter"]
        .iter()
        .any(|name| arguments.contains_key(*name))
        && arguments
            .get("question")
            .and_then(Value::as_str)
            .is_some_and(|question| !question.is_empty())
}

fn invoke_discovery_tool(
    store: &GraphStore,
    arguments: &Map<String, Value>,
    graph_path: &Path,
    context: Option<&GraphContext>,
) -> Result<ToolInvocation, InvocationError> {
    let started = Instant::now();
    let engine = cached_typed_engine(store, graph_path, context)?;
    let engine = engine
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let response = code_query::invoke_discovery_with_engine(arguments, &engine)?;
    let text = format!(
        "Discovery: {} seeds, {} nodes, {} edges{}",
        response.seeds.len(),
        response.nodes.len(),
        response.edges.len(),
        if response.truncated {
            " (truncated)"
        } else {
            ""
        }
    );
    if let Some(question) = arguments.get("question").and_then(Value::as_str) {
        log_discovery_mcp_query(question, graph_path, &response, started.elapsed());
    }
    let semantic_result_digest = compass_query::discovery_response_digest(&response)
        .map_err(|error| InvocationError::Internal(error.to_string()))?;
    Ok(ToolInvocation {
        text,
        structured_content: Some(transport_envelope_with_digest(
            serde_json::to_value(response)
                .map_err(|error| InvocationError::Internal(error.to_string()))?,
            Some(&semantic_result_digest),
        )?),
    })
}

fn cached_typed_engine(
    store: &GraphStore,
    graph_path: &Path,
    context: Option<&GraphContext>,
) -> Result<compass_query::CachedQueryEngine, InvocationError> {
    if compass_query::has_published_store(graph_path) {
        return store
            .inner
            .typed_queries
            .open_published_store(graph_path)
            .map_err(|error| InvocationError::Internal(error.to_string()));
    }
    let context = context.ok_or_else(|| {
        InvocationError::Internal("typed JSON query context is unavailable".to_owned())
    })?;
    let document = context.typed_document.as_ref().ok_or_else(|| {
        InvocationError::InvalidParams(
            "discovery controls require a typed compass.graph/1 artifact".to_owned(),
        )
    })?;
    let identity = context.typed_graph_identity.as_deref().ok_or_else(|| {
        InvocationError::Internal("typed graph identity is unavailable".to_owned())
    })?;
    let cache_root = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cache");
    store
        .inner
        .typed_queries
        .open_verified_document(document, identity, graph_path, &cache_root)
        .map_err(|error| InvocationError::Internal(error.to_string()))
}

fn should_route_natural_query(
    arguments: &Map<String, Value>,
    context: &GraphContext,
) -> Result<bool, InvocationError> {
    let legacy = ["mode", "depth", "token_budget", "context_filter"]
        .iter()
        .any(|name| arguments.contains_key(*name));
    if legacy {
        return Ok(false);
    }
    let Some(_question) = arguments
        .get("question")
        .and_then(Value::as_str)
        .filter(|question| !question.is_empty())
    else {
        return Ok(false);
    };
    if !context.typed_query_supported && code_query::has_discovery_arguments(arguments) {
        return Err(InvocationError::InvalidParams(
            "discovery controls require a typed compass.graph/1 artifact".to_owned(),
        ));
    }
    if !context.typed_query_supported {
        return Ok(false);
    }
    Ok(true)
}

fn transport_envelope(result: Value) -> Result<Value, InvocationError> {
    transport_envelope_with_digest(result, None)
}

fn transport_envelope_with_digest(
    result: Value,
    semantic_result_digest: Option<&str>,
) -> Result<Value, InvocationError> {
    let mut envelope = json!({
        "schema": MCP_TOOL_RESULT_SCHEMA,
        "result": result,
        "transportTruncation": {
            "schema": MCP_TRANSPORT_TRUNCATION_SCHEMA,
            "truncated": false,
            "requiredBytes": 0,
            "limitBytes": MAX_MCP_STRUCTURED_RESPONSE_BYTES,
            "omittedBytes": 0,
        }
    });
    if let Some(digest) = semantic_result_digest {
        envelope["semanticResultDigest"] = json!(format!("sha256:{digest}"));
    }
    for _ in 0..8 {
        let required_bytes = serde_json::to_vec(&envelope)
            .map_err(|error| InvocationError::Internal(error.to_string()))?
            .len();
        if envelope["transportTruncation"]["requiredBytes"].as_u64()
            == u64::try_from(required_bytes).ok()
        {
            break;
        }
        envelope["transportTruncation"]["requiredBytes"] = json!(required_bytes);
    }
    let required_bytes = serde_json::to_vec(&envelope)
        .map_err(|error| InvocationError::Internal(error.to_string()))?
        .len();
    if required_bytes > MAX_MCP_STRUCTURED_RESPONSE_BYTES {
        return Err(InvocationError::TransportLimit {
            required_bytes,
            limit_bytes: MAX_MCP_STRUCTURED_RESPONSE_BYTES,
            omitted_bytes: required_bytes.saturating_sub(MAX_MCP_STRUCTURED_RESPONSE_BYTES),
        });
    }
    Ok(envelope)
}

fn tool_specs() -> Vec<Tool> {
    let project = json!({
        "type": "string",
        "description": "Project directory containing compass-out/graph.json. Optional — defaults to the graph this server was started with."
    });
    let mut specs = vec![
        tool(
            "search_symbols",
            "Search Compass code symbols with the trusted FTS5 index.",
            code_query::schema(&["query"]),
        ),
        tool(
            "get_callers",
            "Return one-hop callers and route bindings for a symbol.",
            code_query::schema(&["symbol"]),
        ),
        tool(
            "get_callees",
            "Return one-hop callees for a symbol.",
            code_query::schema(&["symbol"]),
        ),
        tool(
            "get_impact",
            "Return the bounded transitive impact radius for a symbol.",
            code_query::schema(&["symbol"]),
        ),
        tool(
            "explore_code",
            "Return related symbols, connecting paths, and verified source grouped by file.",
            code_query::schema(&["symbols"]),
        ),
        tool(
            "get_node",
            "Return the trusted evidence trail between two code-graph nodes.",
            code_query::schema(&["source", "target"]),
        ),
        tool(
            "query_graph",
            "Run bounded structured discovery for natural-language questions; explicit legacy traversal fields preserve compatibility text context.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "question":{"type":"string","minLength":1,"maxLength":MAX_DISCOVERY_QUESTION_BYTES,"description":"Natural language question or keyword search"},
                "mode":{"type":"string","enum":["bfs","dfs"],"description":"Explicit legacy mode; selects compatibility traversal"},
                "depth":{"type":"integer","minimum":0,"maximum":6,"description":"Explicit legacy traversal depth (1-6)"},
                "token_budget":{"type":"integer","minimum":0,"description":"Explicit legacy text token budget"},
                "context_filter":{"type":"array","maxItems":MAX_DISCOVERY_FILTERS,"items":{"type":"string","maxLength":MAX_DISCOVERY_FILTER_BYTES},"description":"Optional explicit edge-context filter, e.g. ['call', 'field']"},
                "direction":{"type":"string","enum":["auto","incoming","outgoing","both"],"description":"Discovery edge direction; omitted uses bounded inference"},
                "relation_contexts":{"type":"array","maxItems":MAX_DISCOVERY_FILTERS,"items":{"type":"string","minLength":1,"maxLength":MAX_DISCOVERY_FILTER_BYTES},"description":"Canonical discovery relationship contexts"},
                "scope":{"type":"array","maxItems":MAX_DISCOVERY_FILTERS,"items":{"type":"object","additionalProperties":false,"properties":{"kind":{"type":"string","enum":["community","source","package","node"]},"value":{"type":"string","minLength":1,"maxLength":MAX_DISCOVERY_FILTER_BYTES}},"required":["kind","value"]},"description":"Repeatable OR discovery scopes"},
                "traversal":{"type":"string","enum":["bfs","dfs"],"description":"Bounded discovery traversal order; omitted uses bfs"},
                "include_heuristic":{"type":"boolean"},
                "max_depth":{"type":"integer","minimum":1,"maximum":MAX_DISCOVERY_DEPTH},
                "max_seeds":{"type":"integer","minimum":1,"maximum":MAX_DISCOVERY_SEEDS},
                "max_candidates":{"type":"integer","minimum":1,"maximum":MAX_DISCOVERY_CANDIDATES},
                "max_nodes":{"type":"integer","minimum":1,"maximum":MAX_DISCOVERY_NODES},
                "max_edges":{"type":"integer","minimum":1,"maximum":MAX_DISCOVERY_EDGES},
                "max_expanded_relationships":{"type":"integer","minimum":1,"maximum":MAX_DISCOVERY_EXPANDED_RELATIONSHIPS},
                "max_response_bytes":{"type":"integer","minimum":1,"maximum":MAX_DISCOVERY_RESPONSE_BYTES},
                "timeout_ms":{"type":"integer","minimum":1,"maximum":MAX_DISCOVERY_TIMEOUT_MS}
            },"required":["question"]}),
        ),
        tool(
            "get_neighbors",
            "Get all direct neighbors of a node with edge details.",
            json!({"type":"object","properties":{"label":{"type":"string"},"relation_filter":{"type":"string","description":"Optional: filter by relation type"}},"required":["label"]}),
        ),
        tool(
            "get_community",
            "Get all nodes in a community by community ID.",
            json!({"type":"object","properties":{"community_id":{"type":"integer","description":"Community ID (0-indexed by size)"}},"required":["community_id"]}),
        ),
        tool(
            "god_nodes",
            "Return the most connected nodes - the core abstractions of the knowledge graph.",
            json!({"type":"object","properties":{"top_n":{"type":"integer","default":10}}}),
        ),
        tool(
            "graph_stats",
            "Return summary statistics: node count, edge count, communities, confidence breakdown.",
            json!({"type":"object","properties":{}}),
        ),
        tool(
            "shortest_path",
            "Find the shortest path between two concepts in the knowledge graph.",
            json!({"type":"object","properties":{"source":{"type":"string","description":"Source concept label or keyword"},"target":{"type":"string","description":"Target concept label or keyword"},"max_hops":{"type":"integer","default":8,"description":"Maximum hops to consider"}},"required":["source","target"]}),
        ),
        tool(
            "list_prs",
            "List open GitHub PRs with CI status, review state, and graph impact (which communities each PR touches, blast radius). Use this before starting work to check if a PR already covers the area you're about to change.",
            json!({"type":"object","properties":{"base":{"type":"string","description":"Base branch to filter PRs by (auto-detected if omitted)"},"repo":{"type":"string","description":"GitHub repo (owner/repo). Defaults to current repo."}}}),
        ),
        tool(
            "get_pr_impact",
            "Get detailed graph impact for a specific PR: which files it changes, which knowledge-graph communities are affected, and how many nodes are touched. Use this to assess merge risk or check for overlap with your current work.",
            json!({"type":"object","properties":{"pr_number":{"type":"integer","description":"PR number to analyse"},"repo":{"type":"string","description":"GitHub repo (owner/repo). Defaults to current repo."}},"required":["pr_number"]}),
        ),
        tool(
            "triage_prs",
            "Return all actionable open PRs (correct base, not stale) with full graph impact data so you can reason about review priority, merge order, and conflict risk. Call this when the user asks 'what PRs should I review?' or 'what's ready to merge?'",
            json!({"type":"object","properties":{"base":{"type":"string","description":"Base branch to filter PRs by (auto-detected if omitted)"},"repo":{"type":"string","description":"GitHub repo (owner/repo). Defaults to current repo."}}}),
        ),
        tool(
            "review_pull_request",
            "Analyze exact local target and pull-request revisions with the canonical Compass PR Intelligence report. Git objects and matching history realizations must already exist; the tool never fetches or executes source.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "base":{"type":"string","minLength":1,"maxLength":4096,"description":"Exact local target revision"},
                "head":{"type":"string","minLength":1,"maxLength":4096,"description":"Exact local pull-request head revision"},
                "fingerprint":{"type":"string","pattern":"^[0-9a-f]{64}$","description":"Optional extraction fingerprint required at both revisions"}
            },"required":["base","head"]}),
        ),
        tool(
            "pr_readiness",
            "Compose the additive compass.pr-readiness/1 envelope for exact local revisions. It references the unchanged canonical PR report digest and uses only bounded local evidence.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "base":{"type":"string","minLength":1,"maxLength":4096},
                "head":{"type":"string","minLength":1,"maxLength":4096},
                "fingerprint":{"type":"string","pattern":"^[0-9a-f]{64}$"}
            },"required":["base","head"]}),
        ),
        tool(
            "task_context",
            "Compose a bounded, verified compass.task-context/1 packet after exact target resolution. Ambiguous fuzzy candidates are never selected.",
            json!({"type":"object","additionalProperties":false,"properties":{
                "intent":{"type":"string","enum":["explain","modify","debug","test"]},
                "target":{"type":"string","minLength":1,"maxLength":16384},
                "max_depth":{"type":"integer","minimum":1},
                "max_nodes":{"type":"integer","minimum":1},
                "max_edges":{"type":"integer","minimum":1},
                "max_paths":{"type":"integer","minimum":1},
                "max_candidates":{"type":"integer","minimum":1},
                "max_source_bytes":{"type":"integer","minimum":1},
                "max_response_bytes":{"type":"integer","minimum":1,"maximum":67108864},
                "max_knowledge_items":{"type":"integer","minimum":1,"maximum":100}
            },"required":["intent","target"]}),
        ),
    ];
    for spec in &mut specs {
        Arc::make_mut(&mut spec.input_schema)
            .entry("properties".to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(properties) = Arc::make_mut(&mut spec.input_schema)
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            properties.insert("project_path".to_owned(), project.clone());
        }
    }
    specs
}

fn invoke_review_tool(
    arguments: &Map<String, Value>,
    root: &Path,
    readiness: bool,
) -> Result<ToolInvocation, InvocationError> {
    for name in arguments.keys() {
        if !matches!(name.as_str(), "base" | "head" | "fingerprint") {
            let tool = if readiness {
                "pr_readiness"
            } else {
                "review_pull_request"
            };
            return Err(InvocationError::InvalidParams(format!(
                "unknown {tool} argument {name:?}"
            )));
        }
    }
    let base = string_argument(arguments, "base")?;
    let head = string_argument(arguments, "head")?;
    for (name, value) in [("base", base), ("head", head)] {
        if value.is_empty() || value.len() > 4_096 || value.chars().any(char::is_control) {
            return Err(InvocationError::InvalidParams(format!(
                "{name} must contain 1 to 4096 non-control characters"
            )));
        }
    }
    let fingerprint = match arguments.get("fingerprint") {
        Some(Value::String(value)) => {
            value
                .parse::<compass_history::ExtractionFingerprint>()
                .map_err(|_| {
                    InvocationError::InvalidParams(
                        "fingerprint must be a lowercase SHA-256 digest".to_owned(),
                    )
                })?;
            Some(value.as_str())
        }
        Some(_) => {
            return Err(InvocationError::InvalidParams(
                "fingerprint must be a string".to_owned(),
            ));
        }
        None => None,
    };
    let root_text = root.as_os_str().to_string_lossy();
    if root_text.is_empty()
        || root_text.len() > 32_768
        || root_text.contains('\u{fffd}')
        || root_text.chars().any(char::is_control)
    {
        return Err(InvocationError::InvalidParams(
            "project_path must contain 1 to 32768 non-control characters".to_owned(),
        ));
    }
    let repository = compass_history::Repository::discover(root)
        .map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    let identity = detect_repository_identity(&SystemRunner, repository.root())
        .map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    let request =
        LocalGitChangeRequestSource::new(&SystemRunner, repository.root(), identity, base, head)
            .capture()
            .map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    let old = repository
        .resolve(&request.revisions.target_head)
        .map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    let comparison_revision = request
        .revisions
        .merge_result
        .object_id()
        .unwrap_or(&request.revisions.pull_request_head);
    let new = repository
        .resolve(comparison_revision)
        .map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    let history = compass_history::HistoryStore::open_existing(&repository)
        .map_err(|error| InvocationError::Internal(error.to_string()))?
        .ok_or_else(|| {
            InvocationError::InvalidParams(
                "no immutable graph history exists; materialize both exact revisions with `compass history build` before invoking review_pull_request"
                    .to_owned(),
            )
        })?;
    let old_version = select_review_realization(&history, &old, fingerprint)?;
    let new_version = select_review_realization(&history, &new, fingerprint)?;
    if readiness {
        let bundle = compass_core::review_change_request_ready_exact(
            &repository,
            &history,
            &request,
            &old_version,
            &new_version,
            compass_pr_intelligence::Completeness::LocalExact,
            &SystemRunner,
        )
        .map_err(|error| InvocationError::Internal(error.to_string()))?;
        readiness_tool_invocation(bundle.readiness)
    } else {
        let report = compass_core::review_change_request_exact(
            &repository,
            &history,
            &request,
            &old_version,
            &new_version,
            compass_pr_intelligence::Completeness::LocalExact,
        )
        .map_err(|error| InvocationError::Internal(error.to_string()))?;
        review_tool_invocation(report)
    }
}

fn readiness_tool_invocation(
    readiness: compass_pr_intelligence::PullRequestReadiness,
) -> Result<ToolInvocation, InvocationError> {
    let text = format!(
        "PR readiness: {:?} tests, {:?} documentation drift, {} missing evidence records",
        readiness.facets.tests.state,
        readiness.facets.documentation_drift.state,
        readiness.missing_evidence.len()
    );
    let digest = readiness.readiness_digest.clone();
    Ok(ToolInvocation {
        text,
        structured_content: Some(transport_envelope_with_digest(
            serde_json::to_value(readiness)
                .map_err(|error| InvocationError::Internal(error.to_string()))?,
            Some(&digest),
        )?),
    })
}

fn review_tool_invocation(
    report: compass_pr_intelligence::PullRequestReport,
) -> Result<ToolInvocation, InvocationError> {
    let text = format!(
        "PR review: {:?} advisory risk, {} findings, {} gate results",
        report.advisory_risk.band,
        report.findings.len(),
        report.gates.len()
    );
    Ok(ToolInvocation {
        text,
        structured_content: Some(transport_envelope(
            serde_json::to_value(report)
                .map_err(|error| InvocationError::Internal(error.to_string()))?,
        )?),
    })
}

fn invoke_task_context(
    store: &GraphStore,
    arguments: &Map<String, Value>,
    graph_path: &Path,
    root: &Path,
    graph_context: Option<&GraphContext>,
) -> Result<ToolInvocation, InvocationError> {
    const ALLOWED: &[&str] = &[
        "intent",
        "target",
        "max_depth",
        "max_nodes",
        "max_edges",
        "max_paths",
        "max_candidates",
        "max_source_bytes",
        "max_response_bytes",
        "max_knowledge_items",
    ];
    if let Some(name) = arguments
        .keys()
        .find(|name| !ALLOWED.contains(&name.as_str()))
    {
        return Err(InvocationError::InvalidParams(format!(
            "unknown task_context argument {name:?}"
        )));
    }
    let intent = match string_argument(arguments, "intent")? {
        "explain" => compass_core::TaskContextIntent::Explain,
        "modify" => compass_core::TaskContextIntent::Modify,
        "debug" => compass_core::TaskContextIntent::Debug,
        "test" => compass_core::TaskContextIntent::Test,
        value => {
            return Err(InvocationError::InvalidParams(format!(
                "intent must be explain, modify, debug, or test (found {value})"
            )));
        }
    };
    let target = string_argument(arguments, "target")?.to_owned();
    if target.is_empty() || target.len() > 16_384 || target.chars().any(char::is_control) {
        return Err(InvocationError::InvalidParams(
            "target must contain 1 to 16384 non-control bytes".to_owned(),
        ));
    }
    let mut query_limits = code_query::limits(arguments).map_err(InvocationError::InvalidParams)?;
    let max_response_bytes = arguments
        .get("max_response_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(16 * 1024 * 1024);
    query_limits.max_response_bytes = query_limits
        .max_response_bytes
        .min(compass_model::query_contract::CodeQueryLimits::default().max_response_bytes);
    let max_knowledge_items = match arguments.get("max_knowledge_items") {
        Some(value) => value
            .as_u64()
            .ok_or_else(|| {
                InvocationError::InvalidParams(
                    "max_knowledge_items must be a positive 32-bit integer".to_owned(),
                )
            })
            .and_then(|value| {
                u32::try_from(value).map_err(|_| {
                    InvocationError::InvalidParams("max_knowledge_items exceeds u32".to_owned())
                })
            })?,
        None => 20_u32,
    };
    let limits = compass_core::TaskContextLimits {
        query: query_limits,
        max_knowledge_items,
        max_response_bytes,
    };
    if !limits.is_valid() {
        return Err(InvocationError::InvalidParams(
            "task-context limits are zero or exceed their ceilings".to_owned(),
        ));
    }
    let engine = cached_typed_engine(store, graph_path, graph_context)?;
    let engine = engine
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = compass_core::build_task_context(
        &engine,
        &compass_core::TaskContextRequest {
            intent,
            target,
            repository_root: root.to_string_lossy().into_owned(),
            limits,
        },
        &compass_reflect::load_memory_docs(
            &graph_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("memory"),
        ),
    )
    .map_err(|error| match error {
        compass_core::TaskContextError::InvalidRequest(message) => {
            InvocationError::InvalidParams(message)
        }
        other => InvocationError::Internal(other.to_string()),
    })?;
    let text = format!(
        "Task context: {:?}, {} sections{}",
        result.target,
        result.sections.len(),
        if result.truncated { " (truncated)" } else { "" }
    );
    let digest = result.result_digest.clone();
    Ok(ToolInvocation {
        text,
        structured_content: Some(transport_envelope_with_digest(
            serde_json::to_value(result)
                .map_err(|error| InvocationError::Internal(error.to_string()))?,
            Some(&digest),
        )?),
    })
}

fn select_review_realization(
    history: &compass_history::HistoryStore,
    commit: &compass_history::CommitId,
    fingerprint: Option<&str>,
) -> Result<compass_history::PublishedVersion, InvocationError> {
    if let Some(fingerprint) = fingerprint {
        let mut matches = history
            .list(Some(commit))
            .map_err(|error| InvocationError::Internal(error.to_string()))?
            .into_iter()
            .filter(|version| version.version.extraction_fingerprint == fingerprint)
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return matches.pop().ok_or_else(|| {
                InvocationError::Internal("matching realization disappeared".to_owned())
            });
        }
        return Err(InvocationError::InvalidParams(format!(
            "revision {commit} has {} realizations with extraction fingerprint {fingerprint}; expected exactly one",
            matches.len()
        )));
    }
    history
        .preferred(commit)
        .map_err(|error| InvocationError::Internal(error.to_string()))?
        .ok_or_else(|| {
            InvocationError::InvalidParams(format!(
                "revision {commit} has no preferred complete realization; run `compass history build {commit}`"
            ))
        })
}

fn tool(name: &'static str, description: &'static str, schema: Value) -> Tool {
    let object = schema.as_object().cloned().unwrap_or_default();
    Tool::new(name, description, object)
}

fn resource_specs() -> Vec<Resource> {
    [
        (
            "compass://orientation",
            "Agent Orientation",
            "Versioned bounded orientation from the selected graph generation",
            "application/json",
        ),
        (
            "compass://report",
            "Graph Report",
            "Bounded report rendered from orientation validated against the selected graph",
            "text/markdown",
        ),
        (
            "compass://stats",
            "Graph Stats",
            "Node/edge/community counts and confidence breakdown",
            "text/plain",
        ),
        (
            "compass://god-nodes",
            "God Nodes",
            "Top 10 most-connected nodes",
            "text/plain",
        ),
        (
            "compass://surprises",
            "Surprising Connections",
            "Cross-community surprising connections",
            "text/plain",
        ),
        (
            "compass://audit",
            "Confidence Audit",
            "EXTRACTED/INFERRED/AMBIGUOUS edge breakdown",
            "text/plain",
        ),
        (
            "compass://questions",
            "Suggested Questions",
            "Suggested questions for this codebase",
            "text/plain",
        ),
    ]
    .into_iter()
    .map(|(uri, name, description, mime)| {
        Resource::new(uri, name)
            .with_description(description)
            .with_mime_type(mime)
    })
    .collect()
}

fn invoke_tool(
    name: &str,
    arguments: &Map<String, Value>,
    context: &GraphContext,
) -> Result<String, String> {
    match name {
        "search_symbols" | "get_callers" | "get_callees" | "get_impact" | "explore_code"
        | "get_node" => Err("code query tool requires typed invocation".to_owned()),
        "query_graph" => tool_query_graph(arguments, context),
        "get_neighbors" => tool_get_neighbors(arguments, context),
        "get_community" => tool_get_community(arguments, context),
        "god_nodes" => tool_god_nodes(arguments, context),
        "graph_stats" => Ok(tool_graph_stats(context)),
        "shortest_path" => tool_shortest_path(arguments, context),
        "list_prs" => tool_list_prs(arguments),
        "get_pr_impact" => tool_get_pr_impact(arguments, context),
        "triage_prs" => tool_triage_prs(arguments, context),
        _ => Ok(format!("Unknown tool: {name}")),
    }
}

fn string_argument<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("'{name}'"))
}

fn integer_argument(arguments: &Map<String, Value>, name: &str, default: i64) -> i64 {
    arguments
        .get(name)
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(default)
}

fn optional_string<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn tool_query_graph(
    arguments: &Map<String, Value>,
    context: &GraphContext,
) -> Result<String, String> {
    let question = string_argument(arguments, "question")?;
    let mode = if optional_string(arguments, "mode") == Some("dfs") {
        TraversalMode::Dfs
    } else {
        TraversalMode::Bfs
    };
    let depth =
        usize::try_from(integer_argument(arguments, "depth", 3).clamp(0, 6)).unwrap_or_default();
    let budget = usize::try_from(integer_argument(arguments, "token_budget", 2000).max(0))
        .unwrap_or_default();
    let filters = arguments
        .get("context_filter")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let started = Instant::now();
    let result = query_graph_text(
        &context.graph,
        question,
        mode,
        depth,
        budget,
        &filters,
        &context.overlay,
    );
    log_mcp_query(
        question,
        &context.path,
        &result,
        mode,
        depth,
        budget,
        started.elapsed(),
    );
    Ok(result)
}

fn log_mcp_query(
    question: &str,
    corpus: &Path,
    result: &str,
    mode: TraversalMode,
    depth: usize,
    token_budget: usize,
    duration: Duration,
) {
    if question.len() > MAX_LOGGED_QUESTION_BYTES {
        return;
    }
    let words = result.split_whitespace().collect::<Vec<_>>();
    let nodes = words.windows(3).find_map(|window| {
        (matches!(window[1], "node" | "nodes") && window[2] == "found")
            .then(|| window[0].parse::<usize>().ok())
            .flatten()
    });
    let mode = match mode {
        TraversalMode::Bfs => "bfs",
        TraversalMode::Dfs => "dfs",
    };
    let mut record = json!({
        "schema": "compass.query-log/1",
        "ts": OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default(),
        "kind": "mcp_query",
        "question": question,
        "corpus": corpus.to_string_lossy(),
        "nodes_returned": nodes,
        "result_chars": result.chars().count(),
        "duration_ms": (duration.as_secs_f64() * 1000.0 * 1000.0).round() / 1000.0,
        "mode": mode,
        "depth": depth,
        "token_budget": token_budget,
    });
    if std::env::var("COMPASS_QUERY_LOG_RESPONSES")
        .ok()
        .is_some_and(|value| truthy(&value))
        && let Some(object) = record.as_object_mut()
    {
        object.insert("response".to_owned(), Value::String(result.to_owned()));
    }
    append_query_log(record);
}

fn log_discovery_mcp_query(
    question: &str,
    corpus: &Path,
    response: &compass_model::query_contract::DiscoveryQueryResponse,
    duration: Duration,
) {
    if question.len() > MAX_LOGGED_QUESTION_BYTES {
        return;
    }
    append_query_log(json!({
        "schema": "compass.query-log/1",
        "ts": OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default(),
        "kind": "mcp_query",
        "question": question,
        "corpus": corpus.to_string_lossy(),
        "nodes_returned": response.nodes.len(),
        "result_chars": 0,
        "duration_ms": (duration.as_secs_f64() * 1000.0 * 1000.0).round() / 1000.0,
        "operation": "discovery",
        "truncated": response.truncated,
    }));
}

fn query_log_path() -> Option<PathBuf> {
    let disabled = std::env::var("COMPASS_QUERY_LOG_DISABLE")
        .ok()
        .is_some_and(|value| truthy(&value));
    if disabled {
        return None;
    }
    std::env::var_os("COMPASS_QUERY_LOG")
        .filter(|value| !value.is_empty())
        .map(|value| expand_home(&PathBuf::from(value)))
        .or_else(|| {
            std::env::var("COMPASS_QUERY_LOG_ENABLE")
                .ok()
                .filter(|value| truthy(value))?;
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
            Some(PathBuf::from(home).join(".cache/compass-queries.log"))
        })
}

fn encode_query_log_record(mut record: Value) -> Option<Vec<u8>> {
    if let Some(object) = record.as_object_mut()
        && object
            .get("response")
            .and_then(Value::as_str)
            .is_some_and(|response| response.len() > MAX_QUERY_LOG_RECORD_BYTES / 2)
    {
        object.remove("response");
        object.insert("response_truncated".to_owned(), Value::Bool(true));
    }
    let mut encoded = serde_json::to_vec(&record).ok()?;
    if encoded.len() > MAX_QUERY_LOG_RECORD_BYTES
        && let Some(object) = record.as_object_mut()
        && object.remove("response").is_some()
    {
        object.insert("response_truncated".to_owned(), Value::Bool(true));
        encoded = serde_json::to_vec(&record).ok()?;
    }
    if encoded.len() >= MAX_QUERY_LOG_RECORD_BYTES {
        return None;
    }
    encoded.push(b'\n');
    Some(encoded)
}

fn append_query_log(record: Value) {
    let Some(path) = query_log_path() else {
        return;
    };
    let Some(encoded) = encode_query_log_record(record) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
        && file.metadata().ok().is_some_and(|metadata| {
            metadata.len()
                <= MAX_QUERY_LOG_BYTES
                    .saturating_sub(u64::try_from(encoded.len()).unwrap_or(u64::MAX))
        })
    {
        let _ = file.write_all(&encoded);
    }
}

fn truthy(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

fn expand_home(path: &Path) -> PathBuf {
    let Some(value) = path.to_str() else {
        return path.to_path_buf();
    };
    let Some(suffix) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    else {
        return path.to_path_buf();
    };
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map_or_else(|| path.to_path_buf(), |home| home.join(suffix))
}

fn tool_get_neighbors(
    arguments: &Map<String, Value>,
    context: &GraphContext,
) -> Result<String, String> {
    let query = string_argument(arguments, "label")?.to_lowercase();
    let filter = optional_string(arguments, "relation_filter")
        .unwrap_or_default()
        .to_lowercase();
    let Some(&index) = find_node(&context.graph, &query).first() else {
        return Ok(format!("No node matching '{query}' found."));
    };
    let mut lines = vec![format!(
        "Neighbors of {}:",
        sanitize_label(context.graph.node(index).label())
    )];
    let mut outgoing = HashSet::new();
    for edge_index in context.graph.outgoing_edges(index) {
        let edge = context.graph.edge(edge_index);
        let Some(neighbor) = context.graph.node_index(&edge.target) else {
            continue;
        };
        if !outgoing.insert(neighbor) {
            continue;
        }
        let relation = edge.string("relation");
        if !filter.is_empty() && !relation.to_lowercase().contains(&filter) {
            continue;
        }
        lines.push(format!(
            "  --> {} [{}] [{}]",
            sanitize_label(context.graph.node(neighbor).label()),
            sanitize_label(&relation),
            sanitize_label(&edge.string("confidence"))
        ));
    }
    let mut incoming = HashSet::new();
    for edge_index in context.graph.incoming_edges(index) {
        let edge = context.graph.edge(edge_index);
        let Some(neighbor) = context.graph.node_index(&edge.source) else {
            continue;
        };
        if !incoming.insert(neighbor) {
            continue;
        }
        let relation = edge.string("relation");
        if !filter.is_empty() && !relation.to_lowercase().contains(&filter) {
            continue;
        }
        lines.push(format!(
            "  <-- {} [{}] [{}]",
            sanitize_label(context.graph.node(neighbor).label()),
            sanitize_label(&relation),
            sanitize_label(&edge.string("confidence"))
        ));
    }
    Ok(lines.join("\n"))
}

fn tool_get_community(
    arguments: &Map<String, Value>,
    context: &GraphContext,
) -> Result<String, String> {
    let raw = integer_argument(arguments, "community_id", -1);
    let Ok(community) = usize::try_from(raw) else {
        return Ok(format!("Community {raw} not found."));
    };
    let Some(nodes) = context
        .communities
        .get(&community)
        .filter(|nodes| !nodes.is_empty())
    else {
        return Ok(format!("Community {community} not found."));
    };
    let name = context.graph.node(nodes[0]).string("community_name");
    let base = format!("Community {community}");
    let clean = sanitize_label(&name);
    let header = if clean.is_empty() || clean == base {
        base
    } else {
        format!("{base} — {clean}")
    };
    let mut lines = vec![format!("{header} ({} nodes):", nodes.len())];
    for index in nodes {
        let node = context.graph.node(*index);
        lines.push(format!(
            "  {} [{}]",
            sanitize_label(node.label()),
            sanitize_label(&node.string("source_file"))
        ));
    }
    Ok(lines.join("\n"))
}

fn tool_god_nodes(
    arguments: &Map<String, Value>,
    context: &GraphContext,
) -> Result<String, String> {
    let top_n =
        usize::try_from(integer_argument(arguments, "top_n", 10).max(0)).unwrap_or_default();
    let nodes = god_nodes(&context.document()?, top_n);
    let mut lines = vec!["God nodes (most connected):".to_owned()];
    lines.extend(
        nodes.iter().enumerate().map(|(index, node)| {
            format!("  {}. {} - {} edges", index + 1, node.label, node.degree)
        }),
    );
    Ok(lines.join("\n"))
}

fn tool_graph_stats(context: &GraphContext) -> String {
    let mut extracted = 0_usize;
    let mut inferred = 0_usize;
    let mut ambiguous = 0_usize;
    for edge_index in 0..context.graph.edge_count() {
        match context.graph.edge(edge_index).string("confidence").as_str() {
            "INFERRED" => inferred += 1,
            "AMBIGUOUS" => ambiguous += 1,
            _ => extracted += 1,
        }
    }
    let total = context.graph.edge_count().max(1);
    format!(
        "Nodes: {}\nEdges: {}\nCommunities: {}\nEXTRACTED: {}%\nINFERRED: {}%\nAMBIGUOUS: {}%\n",
        context.graph.node_count(),
        context.graph.edge_count(),
        context.communities.len(),
        python_percent(extracted, total),
        python_percent(inferred, total),
        python_percent(ambiguous, total)
    )
}

fn python_percent(count: usize, total: usize) -> usize {
    let scaled = count.saturating_mul(100);
    let quotient = scaled / total;
    let remainder = scaled % total;
    match remainder.saturating_mul(2).cmp(&total) {
        std::cmp::Ordering::Less => quotient,
        std::cmp::Ordering::Greater => quotient + 1,
        std::cmp::Ordering::Equal if quotient % 2 == 1 => quotient + 1,
        std::cmp::Ordering::Equal => quotient,
    }
}

fn tool_shortest_path(
    arguments: &Map<String, Value>,
    context: &GraphContext,
) -> Result<String, String> {
    let source_query = string_argument(arguments, "source")?;
    let target_query = string_argument(arguments, "target")?;
    let source_scores = score_nodes(
        &context.graph,
        &source_query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>(),
        false,
    );
    let target_scores = score_nodes(
        &context.graph,
        &target_query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>(),
        false,
    );
    if source_scores.ranked.is_empty() {
        return Ok(format!("No node matching source '{source_query}' found."));
    }
    if target_scores.ranked.is_empty() {
        return Ok(format!("No node matching target '{target_query}' found."));
    }
    let source = pick_scored_endpoint(&context.graph, &source_scores.ranked, source_query);
    let target = pick_scored_endpoint(&context.graph, &target_scores.ranked, target_query);
    if source == target {
        return Ok(format!(
            "'{source_query}' and '{target_query}' both resolved to the same node '{}'. Use a more specific label or the exact node ID.",
            context.graph.node(source).id
        ));
    }
    let Some(path) = shortest_path(&context.graph, source, target) else {
        return Ok(format!(
            "No path found between '{}' and '{}'.",
            context.graph.node(source).label(),
            context.graph.node(target).label()
        ));
    };
    let hops = path.len().saturating_sub(1);
    let max_hops =
        usize::try_from(integer_argument(arguments, "max_hops", 8).max(0)).unwrap_or_default();
    if hops > max_hops {
        return Ok(format!(
            "Path exceeds max_hops={max_hops} ({hops} hops found)."
        ));
    }
    let mut warnings = Vec::new();
    ambiguity_warning("source", &source_scores.ranked, source, &mut warnings);
    ambiguity_warning("target", &target_scores.ranked, target, &mut warnings);
    let mut segments = vec![context.graph.node(path[0]).label().to_owned()];
    for pair in path.windows(2) {
        let left = pair[0];
        let right = pair[1];
        if let Some(edge_index) = context.graph.edge_between(left, right) {
            let edge = context.graph.edge(edge_index);
            let confidence = edge.string("confidence");
            let suffix = if confidence.is_empty() {
                String::new()
            } else {
                format!(" [{confidence}]")
            };
            segments.push(format!(
                "--{}{suffix}--> {}",
                edge.string("relation"),
                context.graph.node(right).label()
            ));
        } else if let Some(edge_index) = context.graph.edge_between(right, left) {
            let edge = context.graph.edge(edge_index);
            let confidence = edge.string("confidence");
            let suffix = if confidence.is_empty() {
                String::new()
            } else {
                format!(" [{confidence}]")
            };
            segments.push(format!(
                "<--{}{suffix}-- {}",
                edge.string("relation"),
                context.graph.node(right).label()
            ));
        }
    }
    let prefix = if warnings.is_empty() {
        String::new()
    } else {
        format!("{}\n", warnings.join("\n"))
    };
    Ok(format!(
        "{prefix}Shortest path ({hops} hops):\n  {}",
        segments.join(" ")
    ))
}

fn ambiguity_warning(
    name: &str,
    scores: &[compass_query::ScoredNode],
    chosen: NodeIndex,
    warnings: &mut Vec<String>,
) {
    if scores.len() < 2 || scores[0].node != chosen || scores[0].score <= 0.0 {
        return;
    }
    let top = scores[0].score;
    let runner = scores[1].score;
    if (top - runner) / top < 0.10 {
        warnings.push(format!(
            "warning: {name} match was ambiguous (top score {}, runner-up {})",
            format_score(top),
            format_score(runner)
        ));
    }
}

fn format_score(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn shortest_path(graph: &Graph, source: NodeIndex, target: NodeIndex) -> Option<Vec<NodeIndex>> {
    let mut queue = VecDeque::from([source]);
    let mut parent = HashMap::<NodeIndex, NodeIndex>::new();
    parent.insert(source, source);
    while let Some(node) = queue.pop_front() {
        if node == target {
            break;
        }
        for neighbor in graph.successors(node).chain(graph.predecessors(node)) {
            if let std::collections::hash_map::Entry::Vacant(entry) = parent.entry(neighbor) {
                entry.insert(node);
                queue.push_back(neighbor);
            }
        }
    }
    if !parent.contains_key(&target) {
        return None;
    }
    let mut path = vec![target];
    while path.last().copied() != Some(source) {
        let next = parent.get(path.last()?).copied()?;
        path.push(next);
    }
    path.reverse();
    Some(path)
}

fn tool_list_prs(arguments: &Map<String, Value>) -> Result<String, String> {
    let runner = SystemRunner;
    let repo = optional_string(arguments, "repo");
    let base = optional_string(arguments, "base")
        .map(str::to_owned)
        .unwrap_or_else(|| detect_default_branch(&runner, repo));
    let mut prs =
        fetch_prs(&runner, repo, Some(&base), None).map_err(|error| format!("Error: {error}"))?;
    let worktrees = fetch_worktrees(&runner);
    for pr in &mut prs {
        pr.worktree_path = worktrees.get(&pr.branch).cloned();
    }
    Ok(format_prs_text(&prs, &base, OffsetDateTime::now_utc()))
}

fn tool_get_pr_impact(
    arguments: &Map<String, Value>,
    context: &GraphContext,
) -> Result<String, String> {
    let number = u64::try_from(integer_argument(arguments, "pr_number", -1))
        .map_err(|_| "'pr_number'".to_owned())?;
    let repo = optional_string(arguments, "repo");
    let runner = SystemRunner;
    let mut command = vec![
        "pr".to_owned(),
        "view".to_owned(),
        number.to_string(),
        "--json".to_owned(),
        "title,headRefName,baseRefName,author,isDraft,reviewDecision,statusCheckRollup,updatedAt"
            .to_owned(),
    ];
    if let Some(repo) = repo {
        command.extend(["--repo".to_owned(), repo.to_owned()]);
    }
    let Ok(output) = runner.run("gh", &command, std::time::Duration::from_secs(30)) else {
        return Ok(format!("PR #{number} not found or gh not authenticated."));
    };
    if output.code != 0 {
        return Ok(format!("PR #{number} not found or gh not authenticated."));
    }
    let Ok(data) = serde_json::from_str::<Value>(&output.stdout) else {
        return Ok(format!("PR #{number} not found or gh not authenticated."));
    };
    let files = fetch_pr_files(&runner, number, repo);
    if files.is_empty() {
        return Ok(format!(
            "PR #{number}: no changed files found (may require gh auth)."
        ));
    }
    let document = context.document()?;
    let (communities, nodes) = compute_pr_impact(&files, &document);
    let ci = parse_ci(
        data.get("statusCheckRollup")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice),
    );
    let mut lines = vec![
        format!(
            "PR #{number}: {}",
            data.get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
        ),
        format!(
            "CI: {ci}  Review: {}",
            data.get("reviewDecision")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or("none")
        ),
        format!(
            "Base: {}  Author: {}",
            data.get("baseRefName")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            data.pointer("/author/login")
                .and_then(Value::as_str)
                .unwrap_or("?")
        ),
        format!(
            "\nGraph impact: {nodes} nodes across {} communities",
            communities.len()
        ),
        format!("Communities touched: {communities:?}"),
        format!("Files changed ({}):", files.len()),
    ];
    lines.extend(files.iter().take(20).map(|file| format!("  {file}")));
    if files.len() > 20 {
        lines.push(format!("  … and {} more", files.len() - 20));
    }
    Ok(lines.join("\n"))
}

fn tool_triage_prs(
    arguments: &Map<String, Value>,
    context: &GraphContext,
) -> Result<String, String> {
    let runner = SystemRunner;
    let repo = optional_string(arguments, "repo");
    let base = optional_string(arguments, "base")
        .map(str::to_owned)
        .unwrap_or_else(|| detect_default_branch(&runner, repo));
    let mut prs =
        fetch_prs(&runner, repo, Some(&base), None).map_err(|error| format!("Error: {error}"))?;
    let now = OffsetDateTime::now_utc();
    let worktrees = fetch_worktrees(&runner);
    for pr in &mut prs {
        pr.worktree_path = worktrees.get(&pr.branch).cloned();
    }
    let mut actionable = prs
        .into_iter()
        .filter(|pr| pr.base_branch == base && !matches!(pr.status(now), "WRONG-BASE" | "STALE"))
        .collect::<Vec<_>>();
    if actionable.is_empty() {
        return Ok(format!("No actionable PRs targeting {base}."));
    }
    let document = context.document()?;
    for pr in &mut actionable {
        let files = fetch_pr_files(&runner, pr.number, repo);
        if !files.is_empty() {
            (pr.communities_touched, pr.nodes_affected) = compute_pr_impact(&files, &document);
            pr.files_changed = files;
        }
    }
    actionable.sort_by_key(|pr| status_index(pr.status(now)));
    let header = format!(
        "Actionable PRs targeting {base}: {}\nRank these by review priority. Higher blast_radius = more graph communities affected = higher merge risk.\n",
        actionable.len()
    );
    let mut lines = vec![header];
    for pr in actionable {
        let impact = if pr.blast_radius().is_empty() {
            String::new()
        } else {
            format!("  blast_radius={}", pr.blast_radius())
        };
        let worktree = pr
            .worktree_path
            .as_ref()
            .map(|path| format!("  worktree={path}"))
            .unwrap_or_default();
        lines.push(format!(
            "PR #{} [{}] CI={} review={} age={}d author={}{}{}\n  title: {}",
            pr.number,
            pr.status(now),
            pr.ci_status,
            if pr.review_decision.is_empty() {
                "none"
            } else {
                &pr.review_decision
            },
            pr.days_old(now),
            pr.author,
            impact,
            worktree,
            pr.title
        ));
    }
    Ok(lines.join("\n\n"))
}

fn status_index(status: &str) -> usize {
    [
        "WRONG-BASE",
        "CI-FAIL",
        "CHANGES-REQ",
        "DRAFT",
        "STALE",
        "PENDING",
        "APPROVED",
        "READY",
    ]
    .iter()
    .position(|candidate| *candidate == status)
    .unwrap_or(99)
}

fn read_resource_text(uri: &str, context: &GraphContext) -> Result<String, InvocationError> {
    match uri {
        "compass://orientation" => {
            let orientation = validated_orientation(context)?;
            render_orientation_json(&orientation)
                .map_err(|error| InvocationError::InvalidParams(error.to_string()))
        }
        "compass://report" => render_agent_report_markdown(&validated_orientation(context)?, false)
            .map_err(|error| InvocationError::InvalidParams(error.to_string())),
        "compass://stats" => Ok(tool_graph_stats(context)),
        "compass://god-nodes" => {
            tool_god_nodes(&Map::new(), context).map_err(InvocationError::InvalidParams)
        }
        "compass://surprises" => {
            let document = context.document().map_err(InvocationError::InvalidParams)?;
            let surprises = surprising_connections(&document, &context.community_ids(), 10);
            if surprises.is_empty() {
                return Ok("No surprising connections found.".to_owned());
            }
            let mut lines = vec!["Surprising cross-community connections:".to_owned()];
            lines.extend(
                surprises.into_iter().map(|item| {
                    format!("  {} <-> {} [{}]", item.source, item.target, item.relation)
                }),
            );
            Ok(lines.join("\n"))
        }
        "compass://audit" => {
            let mut extracted = 0_usize;
            let mut inferred = 0_usize;
            let mut ambiguous = 0_usize;
            for edge_index in 0..context.graph.edge_count() {
                match context.graph.edge(edge_index).string("confidence").as_str() {
                    "INFERRED" => inferred += 1,
                    "AMBIGUOUS" => ambiguous += 1,
                    _ => extracted += 1,
                }
            }
            let total = context.graph.edge_count().max(1);
            Ok(format!(
                "Total edges: {total}\nEXTRACTED: {extracted} ({}%)\nINFERRED: {inferred} ({}%)\nAMBIGUOUS: {ambiguous} ({}%)\n",
                python_percent(extracted, total),
                python_percent(inferred, total),
                python_percent(ambiguous, total)
            ))
        }
        "compass://questions" => {
            let document = context.document().map_err(InvocationError::InvalidParams)?;
            let labels_path = context
                .path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("labels.json");
            let labels = fs::read(&labels_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<BTreeMap<usize, String>>(&bytes).ok())
                .unwrap_or_else(|| {
                    context
                        .communities
                        .keys()
                        .map(|community| (*community, format!("Community {community}")))
                        .collect()
                });
            let questions = suggest_questions(&document, &context.community_ids(), &labels, 10);
            if questions.is_empty() {
                return Ok("No suggested questions available.".to_owned());
            }
            let mut lines = vec!["Suggested questions:".to_owned()];
            lines.extend(
                questions
                    .into_iter()
                    .map(|item| format!("  - {}", item.question.unwrap_or_default())),
            );
            Ok(lines.join("\n"))
        }
        _ => Err(InvocationError::InvalidParams(format!(
            "Unknown resource: {uri}"
        ))),
    }
}

fn validated_orientation(context: &GraphContext) -> Result<AgentOrientation, InvocationError> {
    let (typed, graph_digest) =
        compass_model::code_graph::GraphDocument::load_with_artifact_digest(&context.path)
            .map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    let orientation_path = context
        .path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("orientation.json");
    let orientation_json = match read_bounded_resource(&orientation_path) {
        Ok(orientation_json) => orientation_json,
        Err(error @ InvocationError::TransportLimit { .. }) => return Err(error),
        Err(error) => {
            return Err(InvocationError::InvalidParams(format!(
                "coherent orientation artifact is unavailable for {}: {error}",
                context.path.display()
            )));
        }
    };
    let orientation =
        serde_json::from_str::<AgentOrientation>(&orientation_json).map_err(|error| {
            InvocationError::InvalidParams(format!("invalid orientation artifact: {error}"))
        })?;
    let graph_identity = format!("sha256:{graph_digest}");
    validate_orientation_graph_identity(&orientation, &typed, &graph_identity)
        .map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    Ok(orientation)
}

fn read_bounded_resource(path: &Path) -> Result<String, InvocationError> {
    let file =
        fs::File::open(path).map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    if !metadata.is_file() {
        return Err(InvocationError::InvalidParams(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    let required_bytes = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    if required_bytes > MAX_MCP_RESOURCE_BYTES {
        return Err(InvocationError::TransportLimit {
            required_bytes,
            limit_bytes: MAX_MCP_RESOURCE_BYTES,
            omitted_bytes: required_bytes.saturating_sub(MAX_MCP_RESOURCE_BYTES),
        });
    }
    let read_limit = u64::try_from(MAX_MCP_RESOURCE_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut bytes = Vec::with_capacity(required_bytes);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| InvocationError::InvalidParams(error.to_string()))?;
    if bytes.len() > MAX_MCP_RESOURCE_BYTES {
        return Err(InvocationError::TransportLimit {
            required_bytes: bytes.len(),
            limit_bytes: MAX_MCP_RESOURCE_BYTES,
            omitted_bytes: bytes.len().saturating_sub(MAX_MCP_RESOURCE_BYTES),
        });
    }
    String::from_utf8(bytes).map_err(|error| InvocationError::InvalidParams(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_errors_preserve_json_rpc_taxonomy() {
        assert_eq!(
            InvocationError::InvalidParams("bad input".to_owned())
                .protocol_error()
                .code,
            rmcp::model::ErrorCode::INVALID_PARAMS
        );
        assert_eq!(
            InvocationError::Internal("corrupt graph".to_owned())
                .protocol_error()
                .code,
            rmcp::model::ErrorCode::INTERNAL_ERROR
        );
        let oversize =
            transport_envelope(Value::String("x".repeat(MAX_MCP_STRUCTURED_RESPONSE_BYTES)));
        assert!(matches!(
            oversize,
            Err(InvocationError::TransportLimit { .. })
        ));
        if let Err(InvocationError::TransportLimit {
            required_bytes,
            limit_bytes,
            omitted_bytes,
        }) = oversize
        {
            assert!(required_bytes > limit_bytes);
            assert_eq!(limit_bytes, MAX_MCP_STRUCTURED_RESPONSE_BYTES);
            assert_eq!(omitted_bytes, required_bytes - limit_bytes);
        }
    }

    #[test]
    fn resource_reader_reports_typed_transport_oversize() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let resource = directory.path().join("orientation.json");
        fs::write(&resource, vec![b'x'; MAX_MCP_RESOURCE_BYTES + 1])?;

        let error = match read_bounded_resource(&resource) {
            Ok(_) => return Err("resource unexpectedly fit the transport limit".into()),
            Err(error) => error,
        };
        let InvocationError::TransportLimit {
            required_bytes,
            limit_bytes,
            omitted_bytes,
        } = error
        else {
            return Err("expected typed transport limit".into());
        };
        assert_eq!(required_bytes, MAX_MCP_RESOURCE_BYTES + 1);
        assert_eq!(limit_bytes, MAX_MCP_RESOURCE_BYTES);
        assert_eq!(omitted_bytes, 1);
        Ok(())
    }

    #[test]
    fn query_log_records_are_bounded_and_drop_oversized_responses()
    -> Result<(), Box<dyn std::error::Error>> {
        let encoded = encode_query_log_record(json!({
            "schema": "compass.query-log/1",
            "question": "who calls charge",
            "response": "x".repeat(MAX_QUERY_LOG_RECORD_BYTES),
        }))
        .ok_or("the response-free record must remain bounded")?;
        let record = serde_json::from_slice::<Value>(&encoded)?;

        assert_eq!(record["question"], "who calls charge");
        assert_eq!(record["response_truncated"], true);
        assert!(record.get("response").is_none());
        assert!(encoded.len() <= MAX_QUERY_LOG_RECORD_BYTES);
        Ok(())
    }

    fn sample(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        fs::write(
            path,
            r#"{"directed":true,"multigraph":false,"graph":{},"nodes":[{"id":"a","label":"Alpha","community":0},{"id":"b","label":"Beta","community":0}],"links":[{"source":"a","target":"b","relation":"calls","confidence":"EXTRACTED"}]}"#,
        )?;
        Ok(())
    }

    #[test]
    fn tool_and_resource_contract_is_complete() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let graph = temp.path().join("graph.json");
        sample(&graph)?;
        let server = CompassMcp::new(graph);
        let tools = CompassMcp::tools();
        assert_eq!(tools.len(), 18);
        for (name, required) in [
            ("task_context", json!(["intent", "target"])),
            ("pr_readiness", json!(["base", "head"])),
        ] {
            let spec = tools
                .iter()
                .find(|tool| tool.name == name)
                .ok_or("new tool is missing")?;
            assert_eq!(spec.input_schema["additionalProperties"], false);
            assert_eq!(spec.input_schema["required"], required);
        }
        assert_eq!(CompassMcp::resources().len(), 7);
        let text = server.invoke("graph_stats", Map::new());
        assert_eq!(
            text,
            "Nodes: 2\nEdges: 1\nCommunities: 1\nEXTRACTED: 100%\nINFERRED: 0%\nAMBIGUOUS: 0%\n"
        );
        Ok(())
    }

    #[test]
    fn review_tool_contract_is_strict_and_validates_before_repository_access()
    -> Result<(), Box<dyn std::error::Error>> {
        let tools = CompassMcp::tools();
        let review = tools
            .iter()
            .find(|tool| tool.name == "review_pull_request")
            .ok_or("review tool is missing")?;
        assert_eq!(review.input_schema["additionalProperties"], false);
        assert_eq!(review.input_schema["required"], json!(["base", "head"]));

        let mut arguments = Map::new();
        arguments.insert("base".to_owned(), Value::String("main".to_owned()));
        arguments.insert("head".to_owned(), Value::String("feature".to_owned()));
        arguments.insert("fingerprint".to_owned(), json!(42));
        let error = invoke_review_tool(&arguments, Path::new("."), false)
            .err()
            .ok_or("invalid fingerprint unexpectedly reached repository access")?;
        assert!(error.to_string().contains("fingerprint must be a string"));

        arguments.remove("fingerprint");
        arguments.insert("unexpected".to_owned(), Value::Bool(true));
        let error = invoke_review_tool(&arguments, Path::new("."), false)
            .err()
            .ok_or("unknown argument unexpectedly reached repository access")?;
        assert!(
            error
                .to_string()
                .contains("unknown review_pull_request argument")
        );
        Ok(())
    }

    #[test]
    fn review_tool_envelope_preserves_the_canonical_report_without_projection()
    -> Result<(), Box<dyn std::error::Error>> {
        use compass_pr_intelligence::{
            AdvisoryRisk, Completeness, GateResult, GateState, MergeOutcome, PullRequestReport,
            ReportIdentity, RepositoryIdentity, RevisionSet, RiskBand, report_digest,
        };

        let mut report = PullRequestReport {
            schema: compass_pr_intelligence::REPORT_SCHEMA.to_owned(),
            identity: ReportIdentity {
                repository: RepositoryIdentity {
                    forge: "github".to_owned(),
                    host: "github.com".to_owned(),
                    owner: "crabbuild".to_owned(),
                    name: "compass".to_owned(),
                },
                pull_request_number: Some(42),
                revisions: RevisionSet {
                    merge_base: "1".repeat(40),
                    pull_request_head: "2".repeat(40),
                    target_head: "3".repeat(40),
                    merge_result: MergeOutcome::Clean {
                        object_id: "4".repeat(40),
                    },
                },
                graph_schema: "networkx-node-link/v1".to_owned(),
                extractor_version: "extractor/1".to_owned(),
                configuration_digest: "5".repeat(64),
                policy_pack_digest: format!("sha256:{}", "6".repeat(64)),
                evidence_manifest_digest: format!("sha256:{}", "7".repeat(64)),
            },
            completeness: Completeness::LocalExact,
            findings: Vec::new(),
            risk_factors: Vec::new(),
            advisory_risk: AdvisoryRisk {
                rubric_version: 1,
                score: Some(0),
                band: RiskBand::Low,
                explanation: "Advisory only".to_owned(),
            },
            gates: vec![GateResult {
                id: "proven-contract-break".to_owned(),
                rule_version: 1,
                state: GateState::Pass,
                statement: "No exact break".to_owned(),
                finding_fingerprints: Vec::new(),
            }],
            omissions: Vec::new(),
            report_digest: format!("sha256:{}", "0".repeat(64)),
        };
        report.report_digest = report_digest(&report)?;
        report.validate()?;
        let expected = serde_json::to_value(&report)?;
        let invocation = review_tool_invocation(report).map_err(|error| error.to_string())?;
        let structured = invocation
            .structured_content
            .ok_or("review structured content is missing")?;
        assert_eq!(structured["result"], expected);
        assert_eq!(structured["transportTruncation"]["truncated"], false);
        Ok(())
    }

    #[test]
    fn project_path_routes_and_default_does_not_leak() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let default = temp.path().join("default.json");
        sample(&default)?;
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("compass-out"))?;
        fs::write(
            project.join("compass-out/graph.json"),
            r#"{"directed":true,"nodes":[{"id":"a"},{"id":"b"},{"id":"c"}],"links":[]}"#,
        )?;
        let server = CompassMcp::new(default);
        let mut args = Map::new();
        args.insert(
            "project_path".to_owned(),
            Value::String(project.to_string_lossy().into_owned()),
        );
        assert!(server.invoke("graph_stats", args).contains("Nodes: 3"));
        assert!(
            server
                .invoke("graph_stats", Map::new())
                .contains("Nodes: 2")
        );
        Ok(())
    }

    #[test]
    fn project_path_fails_closed_on_a_malformed_snapshot_pointer()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let default = temp.path().join("default.json");
        sample(&default)?;
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("compass-out"))?;
        sample(&project.join("compass-out/graph.json"))?;
        fs::write(project.join("compass-out/current-snapshot"), "../escape")?;
        let server = CompassMcp::new(default);
        let mut args = Map::new();
        args.insert(
            "project_path".to_owned(),
            Value::String(project.to_string_lossy().into_owned()),
        );
        let result = server.invoke("graph_stats", args);
        assert!(result.contains("snapshot"), "{result}");
        assert!(!result.contains("Nodes: 2"), "{result}");
        Ok(())
    }

    #[test]
    fn default_public_graph_tracks_snapshot_changes_and_fails_closed()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let output = temp.path().join("compass-out");
        fs::create_dir_all(&output)?;
        sample(&output.join("graph.json"))?;

        let first = compass_files::BuildGuard::begin(&output)?;
        fs::write(
            first.staging_directory().join("graph.json"),
            r#"{"directed":true,"nodes":[{"id":"snapshot-one"}],"links":[]}"#,
        )?;
        first.commit_with_artifacts(&["graph.json"])?;

        let server = CompassMcp::new(output.join("graph.json"));
        assert!(
            server
                .invoke("graph_stats", Map::new())
                .contains("Nodes: 1")
        );

        let second = compass_files::BuildGuard::begin(&output)?;
        fs::write(
            second.staging_directory().join("graph.json"),
            r#"{"directed":true,"nodes":[{"id":"a"},{"id":"b"},{"id":"c"}],"links":[]}"#,
        )?;
        second.commit_with_artifacts(&["graph.json"])?;
        assert!(
            server
                .invoke("graph_stats", Map::new())
                .contains("Nodes: 3")
        );

        fs::write(output.join("current-snapshot"), "../escape")?;
        let malformed = server.invoke("graph_stats", Map::new());
        assert!(malformed.contains("snapshot"), "{malformed}");
        assert!(!malformed.contains("Nodes: 2"), "{malformed}");
        Ok(())
    }

    #[test]
    fn python_rounding_is_bankers_rounding() {
        assert_eq!(python_percent(1, 8), 12);
        assert_eq!(python_percent(1, 40), 2);
        assert_eq!(python_percent(3, 40), 8);
    }

    #[test]
    fn unknown_tool_does_not_require_a_default_graph() {
        let server = CompassMcp::new("missing.json");
        assert_eq!(
            server.invoke("not_a_tool", Map::new()),
            "Error executing not_a_tool: unknown tool: not_a_tool"
        );
    }

    #[test]
    fn multigraph_neighbors_are_reported_once_like_networkx()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let graph = temp.path().join("graph.json");
        fs::write(
            &graph,
            r#"{"directed":true,"multigraph":true,"nodes":[{"id":"a","label":"Alpha"},{"id":"b","label":"Beta"}],"links":[{"source":"a","target":"b","key":"one","relation":"calls","confidence":"EXTRACTED"},{"source":"a","target":"b","key":"two","relation":"imports","confidence":"INFERRED"}]}"#,
        )?;
        let server = CompassMcp::new(graph);
        let mut arguments = Map::new();
        arguments.insert("label".to_owned(), Value::String("Alpha".to_owned()));
        let output = server.invoke("get_neighbors", arguments);
        assert_eq!(output.matches("--> Beta").count(), 1);
        assert!(output.contains("[calls] [EXTRACTED]"));
        Ok(())
    }

    #[test]
    fn every_local_tool_and_resource_handles_success_missing_and_filter_shapes()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let graph = temp.path().join("graph.json");
        fs::write(
            &graph,
            r#"{"directed":true,"multigraph":false,"graph":{},"nodes":[
{"id":"a","label":"Alpha","community":0,"community_name":"Core","file_type":"code","source_file":"a.rs","source_location":"L1"},
{"id":"b","label":"Beta","community":"0","community_name":"Core","file_type":"code","source_file":"b.rs"},
{"id":"c","label":"Gamma","community":1,"file_type":"document","source_file":"c.md"},
{"id":"d","label":"Delta","file_type":"code","source_file":"d.rs"}],
"links":[
{"source":"a","target":"b","relation":"calls","confidence":"EXTRACTED"},
{"source":"c","target":"b","relation":"documents","confidence":"INFERRED"},
{"source":"b","target":"d","relation":"uses","confidence":"AMBIGUOUS"}]}
"#,
        )?;
        fs::write(temp.path().join("GRAPH_REPORT.md"), "# Report\nBody\n")?;
        fs::write(
            temp.path().join("labels.json"),
            r#"{"0":"Core","1":"Docs"}"#,
        )?;
        let server = CompassMcp::new(&graph);

        let invoke = |name: &str, value: Value| {
            server.invoke(name, value.as_object().cloned().unwrap_or_default())
        };
        let get_node = invoke("get_node", json!({"source":"a","target":"b"}));
        assert!(
            get_node.contains("discovery controls require a typed compass.graph/1 artifact"),
            "{get_node}"
        );
        let neighbors = invoke("get_neighbors", json!({"label":"Beta"}));
        assert!(neighbors.contains("--> Delta"));
        assert!(neighbors.contains("<-- Alpha"));
        assert!(
            invoke(
                "get_neighbors",
                json!({"label":"Beta","relation_filter":"doc"})
            )
            .contains("Gamma")
        );
        assert!(invoke("get_neighbors", json!({"label":"none"})).contains("No node"));
        assert!(invoke("get_community", json!({"community_id":"0"})).contains("Core"));
        assert!(invoke("get_community", json!({"community_id":-1})).contains("not found"));
        assert!(invoke("get_community", json!({"community_id":99})).contains("not found"));
        assert!(invoke("god_nodes", json!({"top_n":"2"})).contains("God nodes"));
        assert!(invoke("graph_stats", json!({})).contains("INFERRED: 33%"));

        assert!(
            invoke("shortest_path", json!({"source":"none","target":"Beta"}))
                .contains("No node matching source")
        );
        assert!(
            invoke("shortest_path", json!({"source":"Alpha","target":"none"}))
                .contains("No node matching target")
        );
        assert!(
            invoke("shortest_path", json!({"source":"Alpha","target":"Alpha"}))
                .contains("same node")
        );
        assert!(
            invoke(
                "shortest_path",
                json!({"source":"Alpha","target":"Gamma","max_hops":0})
            )
            .contains("exceeds max_hops")
        );
        assert!(
            invoke("shortest_path", json!({"source":"Delta","target":"Alpha"}))
                .contains("Shortest path")
        );
        assert!(!invoke("query_graph", json!({"question":"Alpha","mode":"dfs","depth":99,"token_budget":-1,"context_filter":["calls",7]})).is_empty());
        assert!(invoke("query_graph", json!({})).contains("'question'"));
        assert!(invoke("get_pr_impact", json!({"pr_number":-1})).contains("'pr_number'"));

        assert!(server.read("compass://report").is_err());
        for uri in [
            "compass://stats",
            "compass://god-nodes",
            "compass://surprises",
            "compass://audit",
            "compass://questions",
        ] {
            assert!(!server.read(uri)?.is_empty(), "{uri}");
        }
        assert!(server.read("compass://unknown").is_err());
        Ok(())
    }

    #[test]
    fn graph_store_cache_reload_missing_graph_and_pure_helpers_are_total()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let graph = temp.path().join("graph.json");
        sample(&graph)?;
        let store = GraphStore::new(&graph);
        let first = store.load(None)?;
        let warm = store.load(None)?;
        assert!(Arc::ptr_eq(&first, &warm));
        fs::write(
            &graph,
            r#"{"directed":true,"nodes":[{"id":"only","label":"Only"}],"links":[],"padding":"changed-size"}"#,
        )?;
        let changed = store.load(None)?;
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(changed.graph.node_count(), 1);

        let missing = CompassMcp::new(temp.path().join("missing.json"));
        assert!(
            missing
                .invoke("graph_stats", Map::new())
                .contains("not found")
        );
        assert!(missing.read("compass://stats").is_err());

        assert_eq!(integer_argument(&Map::new(), "x", 7), 7);
        assert_eq!(
            integer_argument(
                &json!({"x":"8"}).as_object().cloned().unwrap_or_default(),
                "x",
                7
            ),
            8
        );
        assert_eq!(
            optional_string(
                &json!({"x":""}).as_object().cloned().unwrap_or_default(),
                "x"
            ),
            None
        );
        assert!(truthy("YES"));
        assert!(!truthy("off"));
        assert_eq!(
            expand_home(Path::new("plain/path")),
            PathBuf::from("plain/path")
        );
        assert_eq!(format_score(2.0), "2");
        assert_eq!(format_score(1.234_567_89), "1.234568");
        assert_eq!(python_percent(1, 3), 33);
        assert_eq!(python_percent(2, 3), 67);
        for (index, status) in [
            "WRONG-BASE",
            "CI-FAIL",
            "CHANGES-REQ",
            "DRAFT",
            "STALE",
            "PENDING",
            "APPROVED",
            "READY",
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(status_index(status), index);
        }
        assert_eq!(status_index("unknown"), 99);
        Ok(())
    }

    #[test]
    fn empty_graph_reverse_paths_ambiguity_and_document_failures_are_total()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let empty_path = temp.path().join("empty.json");
        fs::write(&empty_path, r#"{"directed":true,"nodes":[],"links":[]}"#)?;
        let empty = CompassMcp::new(&empty_path);
        assert_eq!(
            empty.read("compass://surprises")?,
            "No surprising connections found."
        );
        assert_eq!(
            empty.read("compass://questions")?,
            "Suggested questions:\n  - "
        );
        assert_eq!(
            empty.invoke("god_nodes", Map::new()),
            "God nodes (most connected):"
        );

        let graph_path = temp.path().join("paths.json");
        fs::write(
            &graph_path,
            r#"{"directed":true,"nodes":[{"id":"a","label":"Twin","community":1},{"id":"b","label":"Twin helper","community":1},{"id":"c","label":"Tail"}],"links":[{"source":"a","target":"b","relation":"","confidence":""},{"source":"b","target":"c","relation":"uses","confidence":"EXTRACTED"}]}"#,
        )?;
        let store = GraphStore::new(&graph_path);
        let context = store.load(None)?;
        assert!(
            tool_get_community(
                json!({"community_id":1})
                    .as_object()
                    .ok_or("community args")?,
                &context,
            )?
            .starts_with("Community 1 (2 nodes)")
        );
        assert_eq!(
            invoke_tool("not-real", &Map::new(), &context)?,
            "Unknown tool: not-real"
        );

        let a = context.graph.node_index("a").ok_or("node a")?;
        let b = context.graph.node_index("b").ok_or("node b")?;
        let c = context.graph.node_index("c").ok_or("node c")?;
        assert_eq!(shortest_path(&context.graph, a, a), Some(vec![a]));
        assert_eq!(shortest_path(&context.graph, c, a), Some(vec![c, b, a]));
        let mut warnings = Vec::new();
        ambiguity_warning(
            "source",
            &[
                compass_query::ScoredNode {
                    score: 10.0,
                    node: a,
                },
                compass_query::ScoredNode {
                    score: 9.5,
                    node: b,
                },
            ],
            a,
            &mut warnings,
        );
        assert_eq!(warnings.len(), 1);
        ambiguity_warning(
            "source",
            &[compass_query::ScoredNode {
                score: 0.0,
                node: a,
            }],
            a,
            &mut warnings,
        );
        assert_eq!(warnings.len(), 1);

        let reverse = tool_shortest_path(
            json!({"source":"Tail","target":"Twin"})
                .as_object()
                .ok_or("path args")?,
            &context,
        )?;
        assert!(reverse.contains("<--uses"));
        assert!(reverse.contains("<----"));
        assert!(expand_home(Path::new("~/compass-cache")).ends_with("compass-cache"));

        fs::write(&graph_path, "not json")?;
        assert!(context.document().is_err());
        assert!(tool_god_nodes(&Map::new(), &context).is_err());
        Ok(())
    }
}
