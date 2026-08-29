use std::fmt::Write as _;
use std::path::PathBuf;

use compass_core::{
    TaskContext, TaskContextIntent, TaskContextLimits, TaskContextRequest, TaskContextTarget,
    build_task_context,
};
use compass_model::query_contract::CodeQueryLimits;
use compass_query::{EngineSelection, open_with_engine};

use crate::Outcome;

pub(crate) fn command(args: &[String]) -> Outcome {
    let format = match option(args, "--format").unwrap_or("text") {
        value @ ("text" | "json") => value,
        value => {
            return Outcome::failure(format!(
                "error: --format must be text or json (found {value})"
            ));
        }
    };
    match execute(args) {
        Ok(context) => match format {
            "text" => Outcome::success(render_text(&context)),
            "json" => serde_json::to_string_pretty(&context)
                .map(Outcome::success)
                .unwrap_or_else(|error| Outcome::failure(format!("error: {error}"))),
            _ => Outcome::failure("error: unreachable task-context format".to_owned()),
        },
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn execute(args: &[String]) -> Result<TaskContext, String> {
    validate_options(args)?;
    let positional = positional(args);
    let intent = match required(&positional, 0)? {
        "explain" => TaskContextIntent::Explain,
        "modify" => TaskContextIntent::Modify,
        "debug" => TaskContextIntent::Debug,
        "test" => TaskContextIntent::Test,
        value => {
            return Err(format!(
                "intent must be explain, modify, debug, or test (found {value})"
            ));
        }
    };
    let target = required(&positional, 1)?.to_owned();
    if positional.len() > 2 {
        return Err(format!(
            "unexpected positional argument {:?}",
            positional[2]
        ));
    }
    let output =
        PathBuf::from(std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned()));
    let graph_option = option(args, "--graph");
    let requested_graph = graph_option.map_or_else(|| output.join("graph.json"), PathBuf::from);
    let graph = if graph_option.is_some() {
        compass_files::BuildGuard::resolve_requested_artifact(&requested_graph)
    } else {
        compass_files::BuildGuard::resolve_artifact(&output, "graph.json")
    }
    .map_err(|error| error.to_string())?;
    let program = option(args, "--program")
        .map(PathBuf::from)
        .map(|path| compass_files::BuildGuard::resolve_requested_artifact(&path))
        .transpose()
        .map_err(|error| error.to_string())?;
    let cache = option(args, "--cache")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            graph
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("cache")
        });
    let engine_selection = match option(args, "--engine") {
        None | Some("default") => EngineSelection::Default,
        Some("json") => EngineSelection::Json,
        Some("store") => EngineSelection::Store,
        Some(value) => {
            return Err(format!(
                "--engine must be default, json, or store (found {value})"
            ));
        }
    };
    let engine = open_with_engine(&graph, program.as_deref(), &cache, engine_selection)
        .map_err(|error| error.to_string())?;
    let repository_root = option(args, "--root")
        .map(str::to_owned)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let memory_dir = option(args, "--memory")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            graph
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("memory")
        });
    let defaults = CodeQueryLimits::default();
    let max_response_bytes = number(args, "--max-response-bytes", 16_u64 * 1024 * 1024)?;
    let limits = TaskContextLimits {
        query: CodeQueryLimits {
            max_depth: number(args, "--max-depth", defaults.max_depth)?,
            max_nodes: number(args, "--max-nodes", defaults.max_nodes)?,
            max_edges: number(args, "--max-edges", defaults.max_edges)?,
            max_paths: number(args, "--max-paths", defaults.max_paths)?,
            max_candidates: number(args, "--max-candidates", defaults.max_candidates)?,
            max_source_bytes: number(args, "--max-source-bytes", defaults.max_source_bytes)?,
            max_response_bytes: max_response_bytes.min(defaults.max_response_bytes),
        },
        max_knowledge_items: number(args, "--max-knowledge-items", 20_u32)?,
        max_response_bytes,
    };
    build_task_context(
        &engine,
        &TaskContextRequest {
            intent,
            target,
            repository_root,
            limits,
        },
        &compass_reflect::load_memory_docs(&memory_dir),
    )
    .map_err(|error| error.to_string())
}

fn render_text(context: &TaskContext) -> String {
    let mut output = String::new();
    let _ = writeln!(
        output,
        "Task context: {:?} {:?}",
        context.intent, context.target
    );
    let _ = writeln!(output, "Graph: {}", context.graph_identity);
    for section in &context.sections {
        let _ = writeln!(
            output,
            "- {:?}: {} nodes, {} edges, {} verified files{}",
            section.kind,
            section.evidence.nodes.len(),
            section.evidence.edges.len(),
            section
                .evidence
                .files
                .iter()
                .filter(|file| file.source.is_some())
                .count(),
            if section.evidence.truncated {
                " (truncated)"
            } else {
                ""
            }
        );
    }
    if let TaskContextTarget::Ambiguous { candidates }
    | TaskContextTarget::NotFound { candidates } = &context.target
    {
        for candidate in candidates {
            let _ = writeln!(
                output,
                "  candidate: {} ({:.3})",
                candidate.node_id, candidate.score
            );
        }
    }
    for knowledge in &context.project_knowledge {
        let _ = writeln!(
            output,
            "- knowledge {}: {}",
            knowledge.path, knowledge.outcome
        );
    }
    for omission in &context.omissions {
        let _ = writeln!(output, "! {}: {}", omission.category, omission.reason);
    }
    let _ = writeln!(
        output,
        "Work: {} queries, {} nodes, {} edges, {} source bytes",
        context.work.query_count,
        context.work.nodes_returned,
        context.work.edges_returned,
        context.work.source_bytes
    );
    let _ = write!(output, "Result digest: {}", context.result_digest);
    output
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

fn required(values: &[String], index: usize) -> Result<&str, String> {
    values.get(index).map(String::as_str).ok_or_else(|| {
        "usage: compass context <explain|modify|debug|test> <TARGET> [OPTIONS]".to_owned()
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
        "--memory",
        "--max-depth",
        "--max-nodes",
        "--max-edges",
        "--max-paths",
        "--max-candidates",
        "--max-source-bytes",
        "--max-response-bytes",
        "--max-knowledge-items",
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

fn validate_options(args: &[String]) -> Result<(), String> {
    const OPTIONS: &[&str] = &[
        "--graph",
        "--program",
        "--cache",
        "--engine",
        "--format",
        "--root",
        "--memory",
        "--max-depth",
        "--max-nodes",
        "--max-edges",
        "--max-paths",
        "--max-candidates",
        "--max-source-bytes",
        "--max-response-bytes",
        "--max-knowledge-items",
    ];
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument.starts_with("--") {
            let name = argument
                .split_once('=')
                .map_or(argument.as_str(), |pair| pair.0);
            if !OPTIONS.contains(&name) {
                return Err(format!("unknown option {name}"));
            }
            if !argument.contains('=') {
                index = index.saturating_add(1);
                if args.get(index).is_none_or(|value| value.starts_with("--")) {
                    return Err(format!("{name} requires a value"));
                }
            }
        }
        index = index.saturating_add(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_options;

    #[test]
    fn option_contract_is_strict_before_graph_access() {
        assert!(validate_options(&["--unknown".to_owned()]).is_err());
        assert!(validate_options(&["--graph".to_owned()]).is_err());
        assert!(
            validate_options(&[
                "explain".to_owned(),
                "node:id".to_owned(),
                "--format=json".to_owned(),
            ])
            .is_ok()
        );
    }
}
