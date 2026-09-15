use std::collections::BTreeMap;
use std::path::PathBuf;

use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryResponse, ExploreRequest, ImpactRequest,
    NodeTrailRequest, SearchRequest,
};
use compass_query::{
    EngineSelection, NaturalQueryRequest, open_with_engine, open_with_verified_document,
};

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

fn render_text(response: &CodeQueryResponse) -> String {
    let no_match = response.diagnostics.iter().find(|diagnostic| {
        diagnostic.code == compass_model::query_contract::QueryDiagnosticCode::NoMatch
    });
    let mut lines = Vec::new();
    if let Some(diagnostic) = no_match {
        lines.push("match_confidence: none".to_owned());
        lines.push(diagnostic.message.clone());
    } else if !response.nodes.is_empty() {
        lines.push("match_confidence: exact".to_owned());
    }
    lines.push(format!(
        "{:?}: {} node(s), {} edge(s), {} path(s)",
        response.operation,
        response.nodes.len(),
        response.edges.len(),
        response.paths.len()
    ));
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
    let node_labels = response
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.qualified_name.as_str()))
        .collect::<BTreeMap<_, _>>();
    let edges = response
        .edges
        .iter()
        .map(|edge| (edge.id.as_str(), edge))
        .collect::<BTreeMap<_, _>>();
    for (index, path) in response.paths.iter().enumerate() {
        if let Some(target) = path.node_ids.last() {
            lines.push(format!("Target resolved: {target}"));
        }
        let mut segments = path
            .node_ids
            .first()
            .map(|node| {
                node_labels
                    .get(node.as_str())
                    .copied()
                    .unwrap_or(node.as_str())
                    .to_owned()
            })
            .into_iter()
            .collect::<Vec<_>>();
        for (edge_id, nodes) in path.edge_ids.iter().zip(path.node_ids.windows(2)) {
            let Some(edge) = edges.get(edge_id.as_str()) else {
                continue;
            };
            let right = node_labels
                .get(nodes[1].as_str())
                .copied()
                .unwrap_or(nodes[1].as_str());
            if edge.source == nodes[0] && edge.target == nodes[1] {
                segments.push(format!("--{}--> {right}", edge.kind.as_str()));
            } else {
                segments.push(format!("<--{}-- {right}", edge.kind.as_str()));
            }
        }
        lines.push(format!(
            "{} path (weighted, {} hops): {}",
            if index == 0 { "Best" } else { "Alternative" },
            path.edge_ids.len(),
            segments.join(" ")
        ));
    }
    lines.extend(
        response
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code != compass_model::query_contract::QueryDiagnosticCode::NoMatch
            })
            .map(|diagnostic| format!("! {:?}: {}", diagnostic.code, diagnostic.message)),
    );
    lines.join("\n")
}
