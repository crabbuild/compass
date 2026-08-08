use std::path::PathBuf;

use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryResponse, ExploreRequest, ImpactRequest,
    NodeTrailRequest, SearchRequest,
};
use compass_query::{EngineSelection, NaturalQueryRequest, open_with_engine};

use crate::Outcome;

pub(crate) fn command(operation: &str, args: &[String]) -> Outcome {
    match execute(operation, args) {
        Ok(response) => {
            let format = option(args, "--format").unwrap_or("text");
            if format == "json" {
                match serde_json::to_string_pretty(&response) {
                    Ok(json) => Outcome::success(json),
                    Err(error) => Outcome::failure(format!("error: {error}")),
                }
            } else if format == "text" {
                Outcome::success(render_text(&response))
            } else {
                Outcome::failure("error: --format must be json or text".to_owned())
            }
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn execute(operation: &str, args: &[String]) -> Result<CodeQueryResponse, String> {
    let positional = positional(args);
    let graph_option = option(args, "--graph");
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
    let graph = if graph_option.is_some() {
        resolve_snapshot_artifact(requested_graph)?
    } else {
        compass_files::BuildGuard::resolve_artifact(&output, "graph.json")
            .map_err(|error| error.to_string())?
    };
    let program = option(args, "--program")
        .map(PathBuf::from)
        .map(resolve_snapshot_artifact)
        .transpose()?;
    let engine = open_with_engine(&graph, program.as_deref(), &cache, engine)
        .map_err(|error| error.to_string())?;
    let limits = limits(args)?;
    match operation {
        "ask" => engine.query_natural(NaturalQueryRequest {
            question: required(&positional, 0, "ask <QUESTION>")?.to_owned(),
            include_heuristic: args.iter().any(|arg| arg == "--include-heuristic"),
            limits,
        }),
        "search" => engine.search(SearchRequest {
            query: required(&positional, 0, "search <QUERY>")?.to_owned(),
            limits,
        }),
        "callers" => engine.callers(CallRequest {
            symbol: required(&positional, 0, "callers <SYMBOL>")?.to_owned(),
            include_heuristic: args.iter().any(|arg| arg == "--include-heuristic"),
            limits,
        }),
        "callees" => engine.callees(CallRequest {
            symbol: required(&positional, 0, "callees <SYMBOL>")?.to_owned(),
            include_heuristic: args.iter().any(|arg| arg == "--include-heuristic"),
            limits,
        }),
        "impact" => engine.impact(ImpactRequest {
            symbol: required(&positional, 0, "impact <SYMBOL>")?.to_owned(),
            include_heuristic: args.iter().any(|arg| arg == "--include-heuristic"),
            limits,
        }),
        "explore" => engine.explore(ExploreRequest {
            symbols: positional,
            root: option(args, "--root").unwrap_or_default().to_owned(),
            include_heuristic: args.iter().any(|arg| arg == "--include-heuristic"),
            limits,
        }),
        "node" => engine.node_trail(NodeTrailRequest {
            source: required(&positional, 0, "node <SOURCE> <TARGET>")?.to_owned(),
            target: required(&positional, 1, "node <SOURCE> <TARGET>")?.to_owned(),
            include_heuristic: args.iter().any(|arg| arg == "--include-heuristic"),
            limits,
        }),
        _ => unreachable!(),
    }
    .map_err(|error| error.to_string())
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

fn render_text(response: &CodeQueryResponse) -> String {
    let mut lines = vec![format!(
        "{:?}: {} node(s), {} edge(s), {} path(s)",
        response.operation,
        response.nodes.len(),
        response.edges.len(),
        response.paths.len()
    )];
    lines.extend(response.nodes.iter().map(|node| {
        format!(
            "{} [{}] {}",
            node.qualified_name,
            node.kind.as_str(),
            node.source
                .as_ref()
                .map(|source| format!("{}:{}", source.file, source.start_line))
                .unwrap_or_default()
        )
    }));
    lines.extend(
        response
            .diagnostics
            .iter()
            .map(|diagnostic| format!("! {:?}: {}", diagnostic.code, diagnostic.message)),
    );
    lines.join("\n")
}
