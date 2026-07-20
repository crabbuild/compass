//! Command compatibility layer for Trail's graph namespace.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use trail_core::{LoadedGraph, default_graph_path};
use trail_model::GraphError;
use trail_query::{
    DEFAULT_AFFECTED_RELATIONS, TraversalMode, format_affected, query_graph_text,
    render_explanation, render_shortest_path,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Frontend {
    Trail,
    Graphify,
}

#[derive(Debug)]
pub struct Outcome {
    pub code: u8,
    pub stdout: String,
    pub stderr: String,
}

impl Outcome {
    fn success(stdout: String) -> Self {
        Self {
            code: 0,
            stdout,
            stderr: String::new(),
        }
    }

    fn failure(stderr: String) -> Self {
        Self {
            code: 1,
            stdout: String::new(),
            stderr,
        }
    }
}

#[must_use]
pub fn run(frontend: Frontend, arguments: impl IntoIterator<Item = OsString>) -> Outcome {
    let mut args = arguments
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    if frontend == Frontend::Trail {
        if args.first().map(String::as_str) != Some("graph") {
            return Outcome::failure(trail_help());
        }
        args.remove(0);
    }
    let Some(command) = args.first().cloned() else {
        return Outcome::success(if frontend == Frontend::Trail {
            trail_help()
        } else {
            graphify_help()
        });
    };
    args.remove(0);
    match command.as_str() {
        "query" => command_query(&args),
        "path" => command_path(&args),
        "explain" => command_explain(&args),
        "affected" => command_affected(&args),
        "--help" | "-h" | "help" => Outcome::success(if frontend == Frontend::Trail {
            trail_help()
        } else {
            graphify_help()
        }),
        "--version" | "-V" => Outcome::success(format!("trail {}", env!("CARGO_PKG_VERSION"))),
        _ => Outcome::failure(format!("error: unknown graph command '{command}'")),
    }
}

fn command_query(args: &[String]) -> Outcome {
    let Some(question) = args.first() else {
        return Outcome::failure(
            "Usage: graphify query \"<question>\" [--dfs] [--context C] [--budget N] [--graph path]"
                .to_owned(),
        );
    };
    let mut graph_path = default_graph_path();
    let mut contexts = Vec::new();
    let mut budget = 2000_usize;
    let mut mode = TraversalMode::Bfs;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--dfs" => {
                mode = TraversalMode::Dfs;
                index += 1;
            }
            "--budget" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --budget must be an integer".to_owned());
                };
                let Ok(value) = value.parse::<usize>() else {
                    return Outcome::failure("error: --budget must be an integer".to_owned());
                };
                budget = value;
                index += 2;
            }
            "--context" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --context requires a value".to_owned());
                };
                contexts.push(value.clone());
                index += 2;
            }
            "--graph" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --graph requires a path".to_owned());
                };
                graph_path = PathBuf::from(value);
                index += 2;
            }
            value if value.starts_with("--budget=") => {
                let Ok(value) = value[9..].parse::<usize>() else {
                    return Outcome::failure("error: --budget must be an integer".to_owned());
                };
                budget = value;
                index += 1;
            }
            value if value.starts_with("--context=") => {
                contexts.push(value[10..].to_owned());
                index += 1;
            }
            value if value.starts_with("--graph=") => {
                graph_path = PathBuf::from(&value[8..]);
                index += 1;
            }
            _ => index += 1,
        }
    }
    let loaded = match load(&graph_path, false) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    Outcome::success(query_graph_text(
        &loaded.graph,
        question,
        mode,
        2,
        budget,
        &contexts,
        &loaded.overlay,
    ))
}

fn command_path(args: &[String]) -> Outcome {
    if args.len() < 2 {
        return Outcome::failure(
            "Usage: graphify path \"<source>\" \"<target>\" [--graph path]".to_owned(),
        );
    }
    let graph_path = parse_graph_path(&args[2..]);
    let loaded = match load(&graph_path, true) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    match render_shortest_path(&loaded.graph, &args[0], &args[1]) {
        Ok(output) => Outcome::success(output),
        Err(error) => Outcome::failure(error),
    }
}

fn command_explain(args: &[String]) -> Outcome {
    let Some(label) = args.first() else {
        return Outcome::failure("Usage: graphify explain \"<node>\" [--graph path]".to_owned());
    };
    let graph_path = parse_graph_path(&args[1..]);
    let loaded = match load(&graph_path, true) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    Outcome::success(render_explanation(&loaded.graph, label, &loaded.overlay))
}

fn command_affected(args: &[String]) -> Outcome {
    let Some(query) = args.first() else {
        return Outcome::failure(
            "Usage: graphify affected \"<node-or-label>\" [--relation R] [--depth N] [--graph path]"
                .to_owned(),
        );
    };
    let mut graph_path = default_graph_path();
    let mut relations = Vec::new();
    let mut depth = 2_usize;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--graph" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --graph requires a path".to_owned());
                };
                graph_path = PathBuf::from(value);
                index += 2;
            }
            "--depth" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --depth must be an integer".to_owned());
                };
                let Ok(value) = value.parse::<usize>() else {
                    return Outcome::failure("error: --depth must be an integer".to_owned());
                };
                depth = value;
                index += 2;
            }
            "--relation" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --relation requires a value".to_owned());
                };
                relations.push(value.clone());
                index += 2;
            }
            value if value.starts_with("--graph=") => {
                graph_path = PathBuf::from(&value[8..]);
                index += 1;
            }
            value if value.starts_with("--depth=") => {
                let Ok(value) = value[8..].parse::<usize>() else {
                    return Outcome::failure("error: --depth must be an integer".to_owned());
                };
                depth = value;
                index += 1;
            }
            value if value.starts_with("--relation=") => {
                relations.push(value[11..].to_owned());
                index += 1;
            }
            _ => index += 1,
        }
    }
    if relations.is_empty() {
        relations = DEFAULT_AFFECTED_RELATIONS
            .iter()
            .map(|relation| (*relation).to_owned())
            .collect();
    }
    let loaded = match load(&graph_path, true) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    Outcome::success(format_affected(&loaded.graph, query, &relations, depth))
}

fn parse_graph_path(args: &[String]) -> PathBuf {
    let mut path = default_graph_path();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--graph" {
            if let Some(value) = args.get(index + 1) {
                path = PathBuf::from(value);
            }
            index += 2;
        } else if let Some(value) = args[index].strip_prefix("--graph=") {
            path = PathBuf::from(value);
            index += 1;
        } else {
            index += 1;
        }
    }
    path
}

fn load(path: &Path, force_directed: bool) -> Result<LoadedGraph, Outcome> {
    let result = if force_directed {
        LoadedGraph::load_directed(path)
    } else {
        LoadedGraph::load(path)
    };
    result.map_err(|error| match error {
        GraphError::NotFound(path) => {
            Outcome::failure(format!("error: graph file not found: {}", path.display()))
        }
        GraphError::InvalidExtension(_) => {
            Outcome::failure("error: graph file must be a .json file".to_owned())
        }
        other => Outcome::failure(format!("error: could not load graph: {other}")),
    })
}

fn trail_help() -> String {
    "Usage: trail graph <command>\n\nCommands:\n  query\n  path\n  explain\n  affected".to_owned()
}

fn graphify_help() -> String {
    "Usage: graphify <command>\n\nPorted commands:\n  query\n  path\n  explain\n  affected"
        .to_owned()
}
