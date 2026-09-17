use std::path::PathBuf;

use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryResponse, ExploreRequest, ImpactRequest,
    NodeTrailRequest, SearchRequest,
};
use compass_output::{
    AgentOperandRole, AgentQueryContext, build_code_query_view, render_agent_query_text,
};
use compass_query::{
    EngineSelection, NaturalQueryRequest, open_with_engine, open_with_verified_document,
};

use crate::Outcome;

pub(crate) fn command(operation: &str, args: &[String]) -> Outcome {
    let format = option(args, "--format").unwrap_or("text");
    if format == "agent-json"
        && args.iter().any(|arg| {
            matches!(
                arg.as_str(),
                "--cursor" | "--text-budget" | "--evidence" | "--result-envelope"
            ) || arg.starts_with("--cursor=")
                || arg.starts_with("--text-budget=")
        })
    {
        return Outcome::failure(
            "error: --cursor, --text-budget, --evidence, and --result-envelope are text-only and cannot be used with --format agent-json".to_owned(),
        );
    }
    match execute(operation, args) {
        Ok(execution) => {
            if format == "json" {
                match serde_json::to_string_pretty(&execution.response) {
                    Ok(json) => Outcome::success(json),
                    Err(error) => Outcome::failure(format!("error: {error}")),
                }
            } else if format == "agent-json" {
                match build_code_query_view(&execution.response, execution.context)
                    .and_then(|view| serde_json::to_string_pretty(&view).map_err(Into::into))
                {
                    Ok(json) => Outcome::success(json),
                    Err(error) => Outcome::failure(format!("error: {error}")),
                }
            } else if format == "text" {
                match build_code_query_view(&execution.response, execution.context)
                    .and_then(|view| render_agent_query_text(&view))
                {
                    Ok(text) => Outcome::success(text),
                    Err(error) => Outcome::failure(format!("error: {error}")),
                }
            } else {
                Outcome::failure("error: --format must be json, agent-json, or text".to_owned())
            }
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

struct QueryExecution {
    response: CodeQueryResponse,
    context: AgentQueryContext,
}

fn execute(operation: &str, args: &[String]) -> Result<QueryExecution, String> {
    let positional = positional(args);
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
    };
    let limits = limits(args)?;
    let include_heuristic = args.iter().any(|arg| arg == "--include-heuristic");
    let (response, question, operands) = match operation {
        "ask" => {
            let question = required(&positional, 0, "ask <QUESTION>")?.to_owned();
            let response = engine
                .query_natural(NaturalQueryRequest {
                    question: question.clone(),
                    include_heuristic,
                    limits,
                })
                .map_err(|error| error.to_string())?;
            (
                response,
                Some(question.clone()),
                vec![(AgentOperandRole::Query, question)],
            )
        }
        "search" => {
            let query = required(&positional, 0, "search <QUERY>")?.to_owned();
            let response = engine
                .search(SearchRequest {
                    query: query.clone(),
                    limits,
                })
                .map_err(|error| error.to_string())?;
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
            .map_err(|error| error.to_string())?;
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
                .map_err(|error| error.to_string())?;
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
                    source: source.clone(),
                    target: target.clone(),
                    include_heuristic,
                    limits,
                })
                .map_err(|error| error.to_string())?;
            (
                response,
                None,
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

fn limits(args: &[String]) -> Result<CodeQueryLimits, String> {
    let defaults = CodeQueryLimits::default();
    Ok(CodeQueryLimits {
        max_depth: number(args, "--max-depth", defaults.max_depth)?,
        max_nodes: number(args, "--max-nodes", defaults.max_nodes)?,
        max_edges: number(args, "--max-edges", defaults.max_edges)?,
        max_paths: number(args, "--max-paths", defaults.max_paths)?,
        max_candidates: number(args, "--max-candidates", defaults.max_candidates)?,
        max_source_bytes: number(args, "--max-source-bytes", defaults.max_source_bytes)?,
        max_response_bytes: number(args, "--max-response-bytes", defaults.max_response_bytes)?,
    })
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
