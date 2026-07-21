//! Native MCP service for Graphify-compatible Trail graph queries.

mod transport;

pub use transport::{HttpOptions, serve_http, serve_stdio};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorData, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use trail_core::LoadedGraph;
use trail_graph::{Communities, god_nodes, suggest_questions, surprising_connections};
use trail_model::{Graph, GraphDocument, NodeIndex};
use trail_prs::{
    ProcessRunner, SystemRunner, compute_pr_impact, detect_default_branch, fetch_pr_files,
    fetch_prs, fetch_worktrees, format_prs_text, parse_ci,
};
use trail_query::{
    TraversalMode, find_node, pick_scored_endpoint, query_graph_text, sanitize_label, score_nodes,
};

const SERVER_NAME: &str = "graphify";

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
}

impl GraphContext {
    fn load(path: &Path) -> Result<Self, String> {
        let loaded = LoadedGraph::load_directed(path).map_err(|error| error.to_string())?;
        let mut communities = BTreeMap::<usize, Vec<NodeIndex>>::new();
        for (index, node) in loaded.graph.nodes() {
            if let Some(community) = node
                .attributes
                .get("community")
                .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
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

#[derive(Debug)]
struct StoreInner {
    default_graph: PathBuf,
    cache: Mutex<HashMap<PathBuf, CacheEntry>>,
}

/// Hot-reloading, multi-project graph store shared by every MCP session.
#[derive(Clone, Debug)]
pub struct GraphStore {
    inner: Arc<StoreInner>,
}

impl GraphStore {
    #[must_use]
    pub fn new(default_graph: impl Into<PathBuf>) -> Self {
        Self {
            inner: Arc::new(StoreInner {
                default_graph: default_graph.into(),
                cache: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn resolve(&self, project_path: Option<&str>) -> PathBuf {
        project_path.map_or_else(
            || self.inner.default_graph.clone(),
            |project| {
                let output = std::env::var_os("GRAPHIFY_OUT")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("graphify-out"));
                Path::new(project).join(output).join("graph.json")
            },
        )
    }

    fn load(&self, project_path: Option<&str>) -> Result<Arc<GraphContext>, String> {
        let path = self.resolve(project_path);
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
pub struct GraphifyMcp {
    store: GraphStore,
}

impl GraphifyMcp {
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
        if !tool_specs().iter().any(|tool| tool.name == name) {
            return format!("Unknown tool: {name}");
        }
        let project_path = arguments
            .remove("project_path")
            .and_then(|value| value.as_str().map(str::to_owned));
        let context = match self.store.load(project_path.as_deref()) {
            Ok(context) => context,
            Err(error) => return format!("Error executing {name}: {error}"),
        };
        match invoke_tool(name, &arguments, &context) {
            Ok(output) => output,
            Err(error) => format!("Error executing {name}: {error}"),
        }
    }

    /// Read a graphify resource without a transport.
    pub fn read(&self, uri: &str) -> Result<String, String> {
        let context = self.store.load(None)?;
        read_resource_text(uri, &context)
    }
}

impl ServerHandler for GraphifyMcp {
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
        let output = self.invoke(&request.name, request.arguments.unwrap_or_default());
        Ok(CallToolResult::success(vec![ContentBlock::text(output)]))
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
            .read(&request.uri)
            .map_err(|error| ErrorData::invalid_params(error, None))?;
        let mime = if request.uri == "graphify://report" {
            "text/markdown"
        } else {
            "text/plain"
        };
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(text, request.uri).with_mime_type(mime),
        ]))
    }
}

fn tool_specs() -> Vec<Tool> {
    let project = json!({
        "type": "string",
        "description": "Absolute path to a project directory containing graphify-out/graph.json. Optional — defaults to the graph this server was started with."
    });
    let mut specs = vec![
        tool(
            "query_graph",
            "Search the knowledge graph using BFS or DFS. Returns relevant nodes and edges as text context.",
            json!({"type":"object","properties":{
                "question":{"type":"string","description":"Natural language question or keyword search"},
                "mode":{"type":"string","enum":["bfs","dfs"],"default":"bfs","description":"bfs=broad context, dfs=trace a specific path"},
                "depth":{"type":"integer","default":3,"description":"Traversal depth (1-6)"},
                "token_budget":{"type":"integer","default":2000,"description":"Max output tokens"},
                "context_filter":{"type":"array","items":{"type":"string"},"description":"Optional explicit edge-context filter, e.g. ['call', 'field']"}
            },"required":["question"]}),
        ),
        tool(
            "get_node",
            "Get full details for a specific node by label or ID.",
            json!({"type":"object","properties":{"label":{"type":"string","description":"Node label or ID to look up"}},"required":["label"]}),
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

fn tool(name: &'static str, description: &'static str, schema: Value) -> Tool {
    let object = schema.as_object().cloned().unwrap_or_default();
    Tool::new(name, description, object)
}

fn resource_specs() -> Vec<Resource> {
    [
        (
            "graphify://report",
            "Graph Report",
            "Full GRAPH_REPORT.md",
            "text/markdown",
        ),
        (
            "graphify://stats",
            "Graph Stats",
            "Node/edge/community counts and confidence breakdown",
            "text/plain",
        ),
        (
            "graphify://god-nodes",
            "God Nodes",
            "Top 10 most-connected nodes",
            "text/plain",
        ),
        (
            "graphify://surprises",
            "Surprising Connections",
            "Cross-community surprising connections",
            "text/plain",
        ),
        (
            "graphify://audit",
            "Confidence Audit",
            "EXTRACTED/INFERRED/AMBIGUOUS edge breakdown",
            "text/plain",
        ),
        (
            "graphify://questions",
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
        "query_graph" => tool_query_graph(arguments, context),
        "get_node" => tool_get_node(arguments, context),
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
    let disabled = std::env::var("GRAPHIFY_QUERY_LOG_DISABLE")
        .ok()
        .is_some_and(|value| truthy(&value));
    if disabled {
        return;
    }
    let path = std::env::var_os("GRAPHIFY_QUERY_LOG")
        .filter(|value| !value.is_empty())
        .map(|value| expand_home(&PathBuf::from(value)))
        .or_else(|| {
            std::env::var("GRAPHIFY_QUERY_LOG_ENABLE")
                .ok()
                .filter(|value| truthy(value))?;
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
            Some(PathBuf::from(home).join(".cache/graphify-queries.log"))
        });
    let Some(path) = path else {
        return;
    };
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
    if std::env::var("GRAPHIFY_QUERY_LOG_RESPONSES")
        .ok()
        .is_some_and(|value| truthy(&value))
        && let Some(object) = record.as_object_mut()
    {
        object.insert("response".to_owned(), Value::String(result.to_owned()));
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
        && serde_json::to_writer(&mut file, &record).is_ok()
    {
        let _ = file.write_all(b"\n");
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

fn tool_get_node(arguments: &Map<String, Value>, context: &GraphContext) -> Result<String, String> {
    let query = string_argument(arguments, "label")?.to_lowercase();
    let Some((index, node)) = context.graph.nodes().find(|(_, node)| {
        node.label().to_lowercase().contains(&query) || node.id.to_lowercase() == query
    }) else {
        return Ok(format!("No node matching '{query}' found."));
    };
    let community_name = node.string("community_name");
    let community = if community_name.is_empty() {
        node.string("community")
    } else {
        community_name
    };
    Ok(format!(
        "Node: {}\n  ID: {}\n  Source: {} {}\n  Type: {}\n  Community: {}\n  Degree: {}",
        sanitize_label(node.label()),
        sanitize_label(&node.id),
        sanitize_label(&node.string("source_file")),
        sanitize_label(&node.string("source_location")),
        sanitize_label(&node.string("file_type")),
        sanitize_label(&community),
        context.graph.degree(index)
    ))
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
    scores: &[trail_query::ScoredNode],
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

fn read_resource_text(uri: &str, context: &GraphContext) -> Result<String, String> {
    match uri {
        "graphify://report" => {
            let report = context
                .path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("GRAPH_REPORT.md");
            Ok(fs::read_to_string(report).unwrap_or_else(|_| {
                "GRAPH_REPORT.md not found. Run graphify extract first.".to_owned()
            }))
        }
        "graphify://stats" => Ok(tool_graph_stats(context)),
        "graphify://god-nodes" => tool_god_nodes(&Map::new(), context),
        "graphify://surprises" => {
            let document = context.document()?;
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
        "graphify://audit" => {
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
        "graphify://questions" => {
            let document = context.document()?;
            let labels_path = context
                .path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(".graphify_labels.json");
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
        _ => Err(format!("Unknown resource: {uri}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let server = GraphifyMcp::new(graph);
        assert_eq!(GraphifyMcp::tools().len(), 10);
        assert_eq!(GraphifyMcp::resources().len(), 6);
        let text = server.invoke("graph_stats", Map::new());
        assert_eq!(
            text,
            "Nodes: 2\nEdges: 1\nCommunities: 1\nEXTRACTED: 100%\nINFERRED: 0%\nAMBIGUOUS: 0%\n"
        );
        Ok(())
    }

    #[test]
    fn project_path_routes_and_default_does_not_leak() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let default = temp.path().join("default.json");
        sample(&default)?;
        let project = temp.path().join("project");
        fs::create_dir_all(project.join("graphify-out"))?;
        fs::write(
            project.join("graphify-out/graph.json"),
            r#"{"directed":true,"nodes":[{"id":"a"},{"id":"b"},{"id":"c"}],"links":[]}"#,
        )?;
        let server = GraphifyMcp::new(default);
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
    fn python_rounding_is_bankers_rounding() {
        assert_eq!(python_percent(1, 8), 12);
        assert_eq!(python_percent(1, 40), 2);
        assert_eq!(python_percent(3, 40), 8);
    }

    #[test]
    fn unknown_tool_does_not_require_a_default_graph() {
        let server = GraphifyMcp::new("missing.json");
        assert_eq!(
            server.invoke("not_a_tool", Map::new()),
            "Unknown tool: not_a_tool"
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
        let server = GraphifyMcp::new(graph);
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
            temp.path().join(".graphify_labels.json"),
            r#"{"0":"Core","1":"Docs"}"#,
        )?;
        let server = GraphifyMcp::new(&graph);

        let invoke = |name: &str, value: Value| {
            server.invoke(name, value.as_object().cloned().unwrap_or_default())
        };
        assert!(invoke("get_node", json!({"label":"alpha"})).contains("Node: Alpha"));
        assert!(invoke("get_node", json!({"label":"absent"})).contains("No node"));
        assert!(invoke("get_node", json!({})).contains("'label'"));
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

        for uri in [
            "graphify://report",
            "graphify://stats",
            "graphify://god-nodes",
            "graphify://surprises",
            "graphify://audit",
            "graphify://questions",
        ] {
            assert!(!server.read(uri)?.is_empty(), "{uri}");
        }
        assert!(server.read("graphify://unknown").is_err());
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

        let missing = GraphifyMcp::new(temp.path().join("missing.json"));
        assert!(
            missing
                .invoke("graph_stats", Map::new())
                .contains("not found")
        );
        assert!(missing.read("graphify://stats").is_err());

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
}
