use std::path::PathBuf;

use compass_analysis::{
    CallGraphDirection, UniversalCallGraphRequest, UniversalCallGraphRoot,
    build_universal_call_graph,
};
use compass_model::GraphDocument;

use crate::{Frontend, Outcome};

pub(crate) fn command(frontend: Frontend, args: &[String]) -> Outcome {
    if frontend != Frontend::Compass {
        return Outcome::failure("error: call-graph is a Compass command".to_owned());
    }
    let options = match parse(args) {
        Ok(options) => options,
        Err(error) => return Outcome::failure_with_code(format!("error: {error}"), 2),
    };
    let graph = match GraphDocument::load(&options.graph) {
        Ok(graph) => graph,
        Err(error) => {
            return Outcome::failure_with_code(
                format!(
                    "error: could not load graph {}: {error}",
                    options.graph.display()
                ),
                3,
            );
        }
    };
    let analysis = match options.program {
        Some(path) => match crate::program_commands::load_program(&path) {
            Ok(analysis) => Some(analysis),
            Err(error) => return Outcome::failure_with_code(format!("error: {error}"), 3),
        },
        None => None,
    };
    match build_universal_call_graph(
        &graph,
        analysis.as_ref(),
        &UniversalCallGraphRequest {
            root: options.root,
            direction: options.direction,
            depth: options.depth,
            max_nodes: options.max_nodes,
            max_edges: options.max_edges,
        },
    ) {
        Ok(response) => match serde_json::to_string(&response) {
            Ok(json) => Outcome::success(json),
            Err(error) => Outcome::failure(format!("error: could not render call graph: {error}")),
        },
        Err(error) => Outcome::failure_with_code(format!("error: {error}"), 4),
    }
}

struct Options {
    root: UniversalCallGraphRoot,
    direction: CallGraphDirection,
    depth: u32,
    max_nodes: usize,
    max_edges: usize,
    graph: PathBuf,
    program: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut file = None;
    let mut byte = None;
    let mut line = None;
    let mut symbol = None;
    let mut direction = CallGraphDirection::Both;
    let mut depth = 2;
    let mut max_nodes = 250;
    let mut max_edges = 500;
    let mut graph = default_out_path("graph.json");
    let mut program = None;
    let mut format_json = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        let (name, inline) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(name, value)| {
                (name, Some(value))
            });
        let value = |index: &mut usize| -> Result<String, String> {
            if let Some(value) = inline {
                if value.is_empty() {
                    return Err(format!("{name} requires a value"));
                }
                return Ok(value.to_owned());
            }
            *index += 1;
            args.get(*index)
                .filter(|value| !value.is_empty())
                .cloned()
                .ok_or_else(|| format!("{name} requires a value"))
        };
        match name {
            "--file" => file = Some(value(&mut index)?),
            "--byte" => byte = Some(parse_u64(name, &value(&mut index)?)?),
            "--line" => {
                let parsed = parse_u64(name, &value(&mut index)?)?;
                if parsed == 0 {
                    return Err("--line must be a positive integer".to_owned());
                }
                line = Some(parsed);
            }
            "--symbol" => symbol = Some(value(&mut index)?),
            "--direction" => {
                direction = match value(&mut index)?.as_str() {
                    "callers" => CallGraphDirection::Callers,
                    "callees" => CallGraphDirection::Callees,
                    "both" => CallGraphDirection::Both,
                    _ => return Err("--direction must be callers, callees, or both".to_owned()),
                };
            }
            "--depth" => depth = positive_u32(name, &value(&mut index)?)?,
            "--max-nodes" => max_nodes = positive_usize(name, &value(&mut index)?)?,
            "--max-edges" => max_edges = positive_usize(name, &value(&mut index)?)?,
            "--graph" => graph = PathBuf::from(value(&mut index)?),
            "--program" => program = Some(PathBuf::from(value(&mut index)?)),
            "--format" => {
                if value(&mut index)? != "json" {
                    return Err("--format must be json".to_owned());
                }
                format_json = true;
            }
            _ => return Err(format!("unknown call-graph option {argument}")),
        }
        index += 1;
    }
    if !format_json {
        return Err("call-graph requires --format json".to_owned());
    }
    let source_parts = (file.is_some(), byte.is_some(), line.is_some());
    if symbol.is_some() && source_parts != (false, false, false) {
        return Err("provide either --symbol or --file/--byte/--line".to_owned());
    }
    let root = if let Some(symbol) = symbol {
        UniversalCallGraphRoot::Symbol { symbol }
    } else if let (Some(file), Some(byte), Some(line)) = (file, byte, line) {
        UniversalCallGraphRoot::SourcePosition { file, byte, line }
    } else {
        return Err("--file, --byte, and --line must be provided together".to_owned());
    };
    Ok(Options {
        root,
        direction,
        depth,
        max_nodes,
        max_edges,
        graph,
        program,
    })
}

fn default_out_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned()))
        .join(name)
}

fn parse_u64(name: &str, value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("{name} must be a non-negative integer"))
}

fn positive_u32(name: &str, value: &str) -> Result<u32, String> {
    match value.parse() {
        Ok(value) if value > 0 => Ok(value),
        _ => Err(format!("{name} must be a positive integer")),
    }
}

fn positive_usize(name: &str, value: &str) -> Result<usize, String> {
    match value.parse() {
        Ok(value) if value > 0 => Ok(value),
        _ => Err(format!("{name} must be a positive integer")),
    }
}
