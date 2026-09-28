use std::path::PathBuf;
use std::time::{Duration, Instant};

use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryResponse, ExploreRequest, ImpactRequest,
    NodeTrailRequest, SearchRequest,
};
use compass_output::{
    AgentOperandRole, AgentQueryContext, AgentTextPageOptions, DEFAULT_AGENT_TEXT_PAGE_TOKENS,
    build_code_query_brief, build_code_query_view, decode_agent_text_page_cursor,
    render_code_query_text_page,
};
use compass_query::{
    EngineSelection, ExactSearchFilter, NaturalQueryIntent, NaturalQueryRequest, QueryError,
    QueryErrorKind, open_with_engine, open_with_verified_document, plan_natural_query,
};

use crate::{Outcome, SharedOutputFormat, parse_shared_output_format};

/// Default typed-query deadline.
const DEFAULT_CODE_QUERY_TIMEOUT_MS: u64 = 60_000;
/// Hard upper bound for `--timeout-ms` on typed queries.
const MAX_CODE_QUERY_TIMEOUT_MS: u64 = 600_000;

pub(crate) fn command(operation: &str, args: &[String]) -> Outcome {
    let (format, query_args) = match parse_shared_output_format(args, operation) {
        Ok(parsed) => parsed,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    if format != SharedOutputFormat::Text
        && query_args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "--cursor" | "--text-budget" | "--evidence" | "--result-envelope"
            ) || arg.starts_with("--cursor=")
                || arg.starts_with("--text-budget=")
        })
    {
        return Outcome::failure(
            "error: --cursor, --text-budget, --evidence, and --result-envelope are text-only and require --format text".to_owned(),
        );
    }
    let text_budget = match number(&query_args, "--text-budget", DEFAULT_AGENT_TEXT_PAGE_TOKENS) {
        Ok(budget) => budget,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    if text_budget == 0 {
        return Outcome::failure("error: --text-budget requires a positive integer".to_owned());
    }
    let text_cursor = option(&query_args, "--cursor").map(str::to_owned);
    if let Some(cursor) = text_cursor.as_deref()
        && let Err(error) = decode_agent_text_page_cursor(cursor)
    {
        return Outcome::failure(format!("error: {error}"));
    }
    let timeout = match timeout(&query_args) {
        Ok(timeout) => timeout,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let deadline = Instant::now() + timeout;
    let brief = query_args.iter().any(|argument| argument == "--brief");
    if brief && format != SharedOutputFormat::AgentJson {
        return Outcome::failure("error: --brief requires --format agent-json".to_owned());
    }
    let result = if format == SharedOutputFormat::Text {
        execute_paged(operation, &query_args, deadline)
    } else {
        execute(operation, &query_args, 1, deadline)
    };
    match result {
        Ok(execution) => {
            if format == SharedOutputFormat::Json {
                match serde_json::to_string_pretty(&execution.response) {
                    Ok(json) => Outcome::success(json),
                    Err(error) => Outcome::failure(format!("error: {error}")),
                }
            } else if format == SharedOutputFormat::AgentJson {
                let projected = if brief {
                    build_code_query_brief(&execution.response, execution.context)
                        .and_then(|view| serde_json::to_string(&view).map_err(Into::into))
                } else {
                    build_code_query_view(&execution.response, execution.context)
                        .and_then(|view| serde_json::to_string_pretty(&view).map_err(Into::into))
                };
                match projected {
                    Ok(json) => Outcome::success(json),
                    Err(error) => Outcome::failure(format!("error: {error}")),
                }
            } else if format == SharedOutputFormat::Text {
                match render_code_query_text_page(
                    &execution.response,
                    execution.context,
                    AgentTextPageOptions {
                        token_budget: text_budget,
                        cursor: text_cursor.as_deref(),
                    },
                )
                .map(|page| page.text)
                {
                    Ok(text) => Outcome::success(text),
                    Err(error) => Outcome::failure(format!("error: {error}")),
                }
            } else {
                Outcome::failure("error: unsupported output format".to_owned())
            }
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

struct QueryExecution {
    response: CodeQueryResponse,
    context: AgentQueryContext,
}

/// Upper bound for the page-widening loop.
///
/// One widening step keeps the ledger stable across pages without paying for
/// repeated full traversals on every page. A still-truncated response keeps the
/// explicit bound footer so the caller can raise the record limits.
const MAX_PAGE_WIDENING_SCALE: u32 = 4;

/// Execute a paged text query with a stable ledger.
///
/// Every page re-runs the same widening sequence so the entry ledger does not
/// change between pages: widen the record bounds until the response is
/// complete or the widening ceiling is reached.
fn execute_paged(
    operation: &str,
    args: &[String],
    deadline: Instant,
) -> Result<QueryExecution, String> {
    let mut scale = 1_u32;
    loop {
        let execution = execute(operation, args, scale, deadline)?;
        if !execution.response.truncated
            || scale >= MAX_PAGE_WIDENING_SCALE
            || args.iter().any(|arg| arg == "--exact")
        {
            return Ok(execution);
        }
        scale = scale.saturating_mul(4);
    }
}

fn execute(
    operation: &str,
    args: &[String],
    page_scale: u32,
    deadline: Instant,
) -> Result<QueryExecution, String> {
    let calls_only = args.iter().any(|arg| arg == "--calls-only");
    if args.iter().any(|arg| arg.starts_with("--calls-only=")) {
        return Err("--calls-only is a flag and does not accept a value".to_owned());
    }
    if calls_only && operation != "node" {
        return Err("--calls-only requires node".to_owned());
    }
    if args.iter().filter(|arg| *arg == "--calls-only").count() > 1 {
        return Err("--calls-only may be supplied only once".to_owned());
    }
    let exact = args.iter().any(|arg| arg == "--exact");
    let mut exact_filter = ExactSearchFilter::default();
    for name in ["--file", "--line", "--kind"] {
        let present = args
            .iter()
            .any(|arg| arg == name || arg.starts_with(&format!("{name}=")));
        if present && (!exact || operation != "search") {
            return Err(format!("{name} requires search --exact"));
        }
        if present {
            let value = option(args, name)
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("{name} requires a value"))?;
            match name {
                "--file" => exact_filter.source_file = Some(value.to_owned()),
                "--line" => {
                    exact_filter.start_line = Some(
                        value
                            .parse()
                            .map_err(|_| "--line requires a positive integer")?,
                    )
                }
                "--kind" => exact_filter.kind =
                    Some(serde_json::from_value(serde_json::json!(value)).map_err(
                        |_| "--kind requires a stored node kind such as function, class, or struct",
                    )?),
                _ => {}
            }
        }
    }
    if exact && operation != "search" {
        return Err("--exact requires search".to_owned());
    }
    let positional = positional(args);
    if calls_only && positional.len() != 2 {
        return Err("node --calls-only requires exactly SOURCE and TARGET".to_owned());
    }
    let graph_option = option(args, "--graph");
    let revision = option(args, "--at");
    if graph_option.is_some() && revision.is_some() {
        return Err("--graph and --at are mutually exclusive".to_owned());
    }
    if revision.is_some() {
        for current_only in ["--engine", "--program", "--cache"] {
            if option(args, current_only).is_some() {
                return Err(format!("{current_only} cannot be combined with --at"));
            }
        }
    }
    let output =
        PathBuf::from(std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned()));
    let requested_graph = graph_option.map_or_else(|| output.join("graph.json"), PathBuf::from);
    let engine = match option(args, "--engine") {
        Some("default") => EngineSelection::Default,
        Some("json") => EngineSelection::Json,
        Some("store") => EngineSelection::Store,
        Some(value) => {
            return Err(format!(
                "--engine must be default, json, or store (found {value})"
            ));
        }
        None => EngineSelection::Default,
    };
    let cache = option(args, "--cache")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            requested_graph
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("cache")
        });
    let program = option(args, "--program")
        .map(PathBuf::from)
        .map(resolve_snapshot_artifact)
        .transpose()?;
    let engine = if let Some(revision) = revision {
        let (realization, document) = super::history_commands::load_typed_graph_at(revision)?;
        let current = std::env::current_dir().map_err(|error| error.to_string())?;
        let history_cache = current
            .join(".compass")
            .join("cache")
            .join("history-query")
            .join(realization.to_string());
        let history_graph = current
            .join(".compass")
            .join("history-query")
            .join(realization.to_string())
            .join("graph.json");
        open_with_verified_document(
            document,
            realization.as_hex(),
            &history_graph,
            program.as_deref(),
            &history_cache,
        )
        .map_err(|error| error.to_string())?
    } else {
        let graph = if graph_option.is_some() {
            resolve_snapshot_artifact(requested_graph)?
        } else {
            compass_files::BuildGuard::resolve_artifact(&output, "graph.json")
                .map_err(|error| error.to_string())?
        };
        open_with_engine(&graph, program.as_deref(), &cache, engine)
            .map_err(|error| error.to_string())?
    }
    .with_deadline(deadline);
    let limits = limits(args, page_scale)?;
    let include_heuristic = args.iter().any(|arg| arg == "--include-heuristic");
    let (response, question, operands) = match operation {
        "ask" => {
            let question = required(&positional, 0, "ask <QUESTION>")?.to_owned();
            let plan = plan_natural_query(&question).map_err(query_error)?;
            // The renderer needs the same operands the query engine executes.
            // The full question is retained separately as request metadata.
            let operands = if plan.routes_to_typed_query() {
                plan.operands()
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let role = match plan.intent() {
                            NaturalQueryIntent::Callers
                            | NaturalQueryIntent::Callees
                            | NaturalQueryIntent::Impact => AgentOperandRole::Symbol,
                            NaturalQueryIntent::NodeTrail if index == 0 => AgentOperandRole::Source,
                            NaturalQueryIntent::NodeTrail => AgentOperandRole::Target,
                            NaturalQueryIntent::Search | NaturalQueryIntent::Fallback => {
                                AgentOperandRole::Query
                            }
                        };
                        (role, value.clone())
                    })
                    .collect()
            } else {
                vec![(AgentOperandRole::Query, question.clone())]
            };
            let response = engine
                .query_natural(NaturalQueryRequest {
                    question: question.clone(),
                    include_heuristic,
                    limits,
                })
                .map_err(query_error)?;
            (response, Some(question), operands)
        }
        "search" => {
            let query = required(&positional, 0, "search <QUERY>")?.to_owned();
            let request = SearchRequest {
                query: query.clone(),
                limits,
            };
            let response = if exact {
                engine.search_exact(request, exact_filter)
            } else {
                engine.search(request)
            }
            .map_err(query_error)?;
            (response, None, vec![(AgentOperandRole::Query, query)])
        }
        "callers" | "callees" | "impact" => {
            let symbol = required(&positional, 0, "<SYMBOL>")?.to_owned();
            let response = match operation {
                "callers" => engine.callers(CallRequest {
                    symbol: symbol.clone(),
                    include_heuristic,
                    limits,
                }),
                "callees" => engine.callees(CallRequest {
                    symbol: symbol.clone(),
                    include_heuristic,
                    limits,
                }),
                "impact" => engine.impact(ImpactRequest {
                    symbol: symbol.clone(),
                    include_heuristic,
                    limits,
                }),
                _ => unreachable!(),
            }
            .map_err(query_error)?;
            (response, None, vec![(AgentOperandRole::Symbol, symbol)])
        }
        "explore" => {
            let symbols = positional.clone();
            let response = engine
                .explore(ExploreRequest {
                    symbols: symbols.clone(),
                    root: option(args, "--root").unwrap_or_default().to_owned(),
                    include_heuristic,
                    limits,
                })
                .map_err(query_error)?;
            let mut operands = symbols
                .into_iter()
                .map(|symbol| (AgentOperandRole::Symbol, symbol))
                .collect::<Vec<_>>();
            if let Some(root) = option(args, "--root").filter(|root| !root.is_empty()) {
                operands.push((AgentOperandRole::Root, root.to_owned()));
            }
            (response, None, operands)
        }
        "node" => {
            let source = required(&positional, 0, "node <SOURCE> <TARGET>")?.to_owned();
            let target = required(&positional, 1, "node <SOURCE> <TARGET>")?.to_owned();
            let response = engine
                .node_trail(NodeTrailRequest {
                    calls_only,
                    source: source.clone(),
                    target: target.clone(),
                    include_heuristic,
                    limits,
                })
                .map_err(query_error)?;
            (
                response,
                calls_only.then(|| format!("call path from {source} to {target}")),
                vec![
                    (AgentOperandRole::Source, source),
                    (AgentOperandRole::Target, target),
                ],
            )
        }
        _ => unreachable!(),
    };
    let mut context = AgentQueryContext::new(
        response.operation.into(),
        engine.graph_identity().to_owned(),
        engine.build_generation_identity().to_owned(),
    );
    if let Some(question) = question {
        context = context.with_question(question);
    }
    for (role, value) in operands {
        context = context.with_operand(role, value);
    }
    Ok(QueryExecution { response, context })
}

fn resolve_snapshot_artifact(path: PathBuf) -> Result<PathBuf, String> {
    compass_files::BuildGuard::resolve_requested_artifact(&path).map_err(|error| error.to_string())
}

/// Build the effective typed-query limits.
///
/// A continuation page widens the record bounds by its page number so the
/// requested slice is reachable while the returned page stays the same size.
/// `max_response_bytes` still bounds what the query may materialize.
fn limits(args: &[String], page_scale: u32) -> Result<CodeQueryLimits, String> {
    let defaults = CodeQueryLimits::default();
    let scale = page_scale.max(1);
    Ok(CodeQueryLimits {
        max_depth: number(args, "--max-depth", defaults.max_depth)?,
        max_nodes: number(args, "--max-nodes", defaults.max_nodes)?.saturating_mul(scale),
        max_edges: number(args, "--max-edges", defaults.max_edges)?.saturating_mul(scale),
        max_paths: number(args, "--max-paths", defaults.max_paths)?.saturating_mul(scale),
        max_candidates: number(args, "--max-candidates", defaults.max_candidates)?
            .saturating_mul(scale),
        max_source_bytes: number(args, "--max-source-bytes", defaults.max_source_bytes)?,
        max_response_bytes: number(args, "--max-response-bytes", defaults.max_response_bytes)?,
    })
}

/// Parse the bounded typed-query deadline.
fn timeout(args: &[String]) -> Result<Duration, String> {
    let value = number(args, "--timeout-ms", DEFAULT_CODE_QUERY_TIMEOUT_MS)?;
    if value == 0 || value > MAX_CODE_QUERY_TIMEOUT_MS {
        return Err(format!(
            "--timeout-ms must be between 1 and {MAX_CODE_QUERY_TIMEOUT_MS}"
        ));
    }
    Ok(Duration::from_millis(value))
}

/// Render a typed-query failure with an actionable timeout hint.
fn query_error(error: QueryError) -> String {
    if error.kind() == QueryErrorKind::Timeout {
        format!("{error}; raise --timeout-ms or lower --max-nodes/--max-edges")
    } else {
        error.to_string()
    }
}

fn number<T: std::str::FromStr + Copy>(
    args: &[String],
    name: &str,
    default: T,
) -> Result<T, String> {
    option(args, name).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| format!("{name} requires a positive integer"))
    })
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().enumerate().find_map(|(index, argument)| {
        if argument == name {
            args.get(index + 1).map(String::as_str)
        } else {
            argument
                .strip_prefix(name)
                .and_then(|value| value.strip_prefix('='))
        }
    })
}

fn positional(args: &[String]) -> Vec<String> {
    let value_options = [
        "--graph",
        "--at",
        "--program",
        "--cache",
        "--engine",
        "--format",
        "--root",
        "--max-depth",
        "--max-nodes",
        "--max-edges",
        "--max-paths",
        "--max-candidates",
        "--max-source-bytes",
        "--max-response-bytes",
        "--text-budget",
        "--cursor",
        "--timeout-ms",
        "--file",
        "--line",
        "--kind",
    ];
    let mut values = Vec::new();
    let mut skip = false;
    for argument in args {
        if skip {
            skip = false;
        } else if value_options.contains(&argument.as_str()) {
            skip = true;
        } else if !argument.starts_with("--") {
            values.push(argument.clone());
        }
    }
    values
}

fn required<'a>(values: &'a [String], index: usize, usage: &str) -> Result<&'a str, String> {
    values
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("usage: compass {usage} [OPTIONS]"))
}
