//! Command compatibility layer for Trail's graph namespace.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use trail_core::{ExportInputs, LoadedGraph, default_graph_path};
use trail_graph::god_nodes;
use trail_model::GraphError;
use trail_output::{
    CallflowOptions, CallflowSection, CanvasOptions, HtmlOptions, ObsidianOptions, SvgOptions,
    WikiOptions, export_obsidian, export_wiki, node_filenames, write_callflow_html, write_canvas,
    write_graphml, write_html, write_svg,
};
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
        "export" => command_export(&args),
        "--help" | "-h" | "help" => Outcome::success(if frontend == Frontend::Trail {
            trail_help()
        } else {
            graphify_help()
        }),
        "--version" | "-V" => Outcome::success(format!("trail {}", env!("CARGO_PKG_VERSION"))),
        _ => Outcome::failure(format!("error: unknown graph command '{command}'")),
    }
}

fn command_export(args: &[String]) -> Outcome {
    let Some(format) = args.first().map(String::as_str) else {
        return Outcome::failure(export_help());
    };
    if !matches!(
        format,
        "html" | "callflow-html" | "obsidian" | "wiki" | "svg" | "graphml"
    ) {
        return Outcome::failure(export_help());
    }
    let mut graph_path = default_graph_path();
    let mut graph_explicit = false;
    let mut labels_path = default_graph_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".graphify_labels.json");
    let mut labels_explicit = false;
    let mut report_path = default_graph_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("GRAPH_REPORT.md");
    let mut report_explicit = false;
    let mut sections_path = None;
    let mut output_path = None;
    let mut language = "auto".to_owned();
    let mut max_sections = 15_usize;
    let mut diagram_scale = 1.0_f64;
    let mut max_diagram_nodes = 18_usize;
    let mut max_diagram_edges = 24_usize;
    let mut node_limit = 5000_isize;
    let mut no_viz = false;
    let mut obsidian_dir = default_graph_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("obsidian");
    let mut index = 1;
    while index < args.len() {
        let argument = args[index].as_str();
        let next = || args.get(index + 1).cloned();
        match argument {
            "--graph" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --graph requires a path".to_owned());
                };
                graph_path = PathBuf::from(value);
                graph_explicit = true;
                index += 2;
            }
            "--labels" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --labels requires a path".to_owned());
                };
                labels_path = PathBuf::from(value);
                labels_explicit = true;
                index += 2;
            }
            "--report" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --report requires a path".to_owned());
                };
                report_path = PathBuf::from(value);
                report_explicit = true;
                index += 2;
            }
            "--sections" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --sections requires a path".to_owned());
                };
                sections_path = Some(PathBuf::from(value));
                index += 2;
            }
            "--output" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --output requires a path".to_owned());
                };
                output_path = Some(absolutize(PathBuf::from(value)));
                index += 2;
            }
            "--dir" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --dir requires a path".to_owned());
                };
                obsidian_dir = PathBuf::from(value);
                index += 2;
            }
            "--lang" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --lang requires a value".to_owned());
                };
                language = value;
                index += 2;
            }
            "--max-sections" => {
                let Some(value) = parse_usize(next(), "--max-sections") else {
                    return Outcome::failure("error: --max-sections must be an integer".to_owned());
                };
                max_sections = value;
                index += 2;
            }
            "--max-diagram-nodes" => {
                let Some(value) = parse_usize(next(), "--max-diagram-nodes") else {
                    return Outcome::failure(
                        "error: --max-diagram-nodes must be an integer".to_owned(),
                    );
                };
                max_diagram_nodes = value;
                index += 2;
            }
            "--max-diagram-edges" => {
                let Some(value) = parse_usize(next(), "--max-diagram-edges") else {
                    return Outcome::failure(
                        "error: --max-diagram-edges must be an integer".to_owned(),
                    );
                };
                max_diagram_edges = value;
                index += 2;
            }
            "--node-limit" => {
                let Some(value) = next().and_then(|value| value.parse::<isize>().ok()) else {
                    return Outcome::failure("error: --node-limit must be an integer".to_owned());
                };
                node_limit = value;
                index += 2;
            }
            "--diagram-scale" => {
                let Some(value) = next().and_then(|value| value.parse::<f64>().ok()) else {
                    return Outcome::failure("error: --diagram-scale must be a number".to_owned());
                };
                diagram_scale = value;
                index += 2;
            }
            "--no-viz" => {
                no_viz = true;
                index += 1;
            }
            "-h" | "--help" if format == "callflow-html" => {
                return Outcome::success(callflow_help());
            }
            value if format == "callflow-html" && !value.starts_with('-') && !graph_explicit => {
                let candidate = PathBuf::from(value);
                graph_path = if candidate.file_name().and_then(|name| name.to_str())
                    == Some("graph.json")
                    || candidate.extension().and_then(|value| value.to_str()) == Some("json")
                {
                    candidate
                } else if candidate.join("graph.json").exists() {
                    candidate.join("graph.json")
                } else {
                    candidate.join("graphify-out/graph.json")
                };
                graph_explicit = true;
                index += 1;
            }
            _ => index += 1,
        }
    }
    if graph_explicit {
        let output_dir = graph_path.parent().unwrap_or_else(|| Path::new("."));
        if !labels_explicit {
            labels_path = output_dir.join(".graphify_labels.json");
        }
        if !report_explicit {
            report_path = output_dir.join("GRAPH_REPORT.md");
        }
    }
    let mut inputs = match ExportInputs::load(&graph_path) {
        Ok(inputs) => inputs,
        Err(GraphError::NotFound(_)) => {
            return Outcome::failure(format!(
                "error: graph not found: {}. Run /graphify <path> first.",
                graph_path.display()
            ));
        }
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    if labels_explicit {
        match load_usize_string_map(&labels_path) {
            Ok(labels) => inputs.labels = labels,
            Err(error) => return Outcome::failure(error),
        }
    }
    if report_explicit {
        inputs.report = fs::read_to_string(&report_path).unwrap_or_default();
    }
    let output_dir = graph_path.parent().unwrap_or_else(|| Path::new("."));
    let result = match format {
        "html" => export_html(&inputs, output_dir, no_viz, node_limit),
        "callflow-html" => export_callflow(
            &inputs,
            &graph_path,
            output_path,
            sections_path.as_deref(),
            &language,
            max_sections,
            diagram_scale,
            max_diagram_nodes,
            max_diagram_edges,
        ),
        "obsidian" => export_obsidian_cli(&inputs, &obsidian_dir),
        "wiki" => export_wiki_cli(&inputs, output_dir),
        "svg" => write_svg(
            &inputs.document,
            &inputs.communities,
            output_dir.join("graph.svg"),
            &SvgOptions {
                community_labels: (!inputs.labels.is_empty()).then_some(&inputs.labels),
                ..SvgOptions::default()
            },
        )
        .map(|()| "graph.svg written - embeds in Obsidian, Notion, GitHub READMEs".to_owned())
        .map_err(|error| error.to_string()),
        "graphml" => write_graphml(
            &inputs.document,
            &inputs.communities,
            output_dir.join("graph.graphml"),
        )
        .map(|()| "graph.graphml written - open in Gephi, yEd, or any GraphML tool".to_owned())
        .map_err(|error| error.to_string()),
        _ => Err("unsupported export format".to_owned()),
    };
    match result {
        Ok(output) => Outcome::success(output),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn export_html(
    inputs: &ExportInputs,
    output_dir: &Path,
    no_viz: bool,
    node_limit: isize,
) -> Result<String, String> {
    let path = output_dir.join("graph.html");
    if no_viz {
        if path.exists() {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
        return Ok("--no-viz: skipped graph.html".to_owned());
    }
    let result = write_html(
        &inputs.document,
        &inputs.communities,
        &path,
        &HtmlOptions {
            community_labels: (!inputs.labels.is_empty()).then_some(&inputs.labels),
            member_counts: None,
            node_limit: Some(node_limit),
            learning_overlay: None,
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(result.map_or_else(String::new, |_| {
        "graph.html written - open in any browser, no server needed".to_owned()
    }))
}

#[allow(clippy::too_many_arguments)]
fn export_callflow(
    inputs: &ExportInputs,
    graph_path: &Path,
    output_path: Option<PathBuf>,
    sections_path: Option<&Path>,
    language: &str,
    max_sections: usize,
    diagram_scale: f64,
    max_diagram_nodes: usize,
    max_diagram_edges: usize,
) -> Result<String, String> {
    let sections = sections_path.map(load_sections).transpose()?;
    let project = inputs
        .document
        .graph
        .get("project_name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            graph_path
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Project".to_owned());
    let path = output_path.unwrap_or_else(|| {
        graph_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{}-callflow.html", safe_output_name(&project)))
    });
    let result = write_callflow_html(
        &inputs.document,
        &inputs.communities,
        &path,
        &CallflowOptions {
            community_labels: (!inputs.labels.is_empty()).then_some(&inputs.labels),
            sections: sections.as_deref(),
            report: &inputs.report,
            project_name: &project,
            language,
            max_sections,
            diagram_scale,
            max_diagram_nodes,
            max_diagram_edges,
            ..CallflowOptions::default()
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "Loaded: {} nodes, {} edges, {} sections\nGraph: {}\nCall-flow HTML written: {}\n  Sections: {}  |  Mermaid diagrams: {}  |  Call tables: {}\n  Diagrams use Mermaid init directives plus interactive zoom/pan controls.\ncallflow HTML written - open in any browser: {}",
        inputs.document.nodes.len(),
        inputs.document.links.len(),
        result.loaded_sections,
        graph_path.display(),
        path.display(),
        result.rendered_sections,
        result.mermaid_diagrams,
        result.call_tables,
        path.display(),
    ))
}

fn export_obsidian_cli(inputs: &ExportInputs, output_dir: &Path) -> Result<String, String> {
    let result = export_obsidian(
        &inputs.document,
        &inputs.communities,
        output_dir,
        &ObsidianOptions {
            community_labels: (!inputs.labels.is_empty()).then_some(&inputs.labels),
            cohesion: (!inputs.cohesion.is_empty()).then_some(&inputs.cohesion),
        },
    )
    .map_err(|error| error.to_string())?;
    let filenames = node_filenames(&inputs.document);
    write_canvas(
        &inputs.document,
        &inputs.communities,
        output_dir.join("graph.canvas"),
        &CanvasOptions {
            community_labels: (!inputs.labels.is_empty()).then_some(&inputs.labels),
            node_filenames: Some(&filenames),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "Obsidian vault: {} notes in {}/\nCanvas: {}/graph.canvas\nOpen {}/ as a vault in Obsidian.",
        result.notes_written,
        output_dir.display(),
        output_dir.display(),
        output_dir.display()
    ))
}

fn export_wiki_cli(inputs: &ExportInputs, output_dir: &Path) -> Result<String, String> {
    if inputs.communities.is_empty() {
        return Err(
            ".graphify_analysis.json is missing or empty — refusing to export wiki to prevent data loss.\nRun `graphify extract .` (or `graphify cluster-only .`) to regenerate community data first."
                .to_owned(),
        );
    }
    let wiki_dir = output_dir.join("wiki");
    let computed_gods;
    let gods = if inputs.gods.is_empty() {
        computed_gods = god_nodes(&inputs.document, 10);
        computed_gods.as_slice()
    } else {
        inputs.gods.as_slice()
    };
    let result = export_wiki(
        &inputs.document,
        &inputs.communities,
        &wiki_dir,
        &WikiOptions {
            community_labels: (!inputs.labels.is_empty()).then_some(&inputs.labels),
            cohesion: (!inputs.cohesion.is_empty()).then_some(&inputs.cohesion),
            god_nodes: Some(gods),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "Wiki: {} articles written to {}/\n  {}/index.md  ->  agent entry point",
        result.articles_written,
        wiki_dir.display(),
        wiki_dir.display()
    ))
}

fn load_usize_string_map(path: &Path) -> Result<std::collections::BTreeMap<usize, String>, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("error reading {}: {error}", path.display()))?;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid JSON at {}: {error}", path.display()))?;
    for wrapper in ["labels", "communities"] {
        if let Some(inner) = value.get(wrapper).and_then(serde_json::Value::as_object) {
            value = serde_json::Value::Object(inner.clone());
        }
    }
    let Some(object) = value.as_object() else {
        return Ok(std::collections::BTreeMap::new());
    };
    Ok(object
        .iter()
        .filter_map(|(key, value)| {
            let key = key.parse().ok()?;
            let label = value.as_str().map(str::to_owned).or_else(|| {
                value.as_object().and_then(|object| {
                    ["label", "name", "title"]
                        .iter()
                        .find_map(|field| object.get(*field).and_then(serde_json::Value::as_str))
                        .map(str::to_owned)
                })
            })?;
            Some((key, label))
        })
        .collect())
}

fn load_sections(path: &Path) -> Result<Vec<CallflowSection>, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("error reading {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid JSON at {}: {error}", path.display()))?;
    let value = value.get("sections").cloned().unwrap_or(value);
    serde_json::from_value(value).map_err(|_| {
        format!(
            "ERROR: sections file must contain a JSON array: {}",
            path.display()
        )
    })
}

fn parse_usize(value: Option<String>, _name: &str) -> Option<usize> {
    value?.parse().ok()
}

fn absolutize(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir().map_or(path.clone(), |current| current.join(path))
    }
}

fn safe_output_name(value: &str) -> String {
    let output = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    if output.is_empty() {
        "project".to_owned()
    } else {
        output
    }
}

fn export_help() -> String {
    "Usage: graphify export <format>\n  html      [--graph PATH] [--labels PATH] [--node-limit N] [--no-viz]\n  callflow-html [GRAPH|DIR] [--graph PATH] [--labels PATH] [--report PATH] [--sections PATH] [--output HTML]\n  obsidian  [--graph PATH] [--labels PATH] [--dir PATH]\n  wiki      [--graph PATH] [--labels PATH]\n  svg       [--graph PATH] [--labels PATH]\n  graphml   [--graph PATH]".to_owned()
}

fn callflow_help() -> String {
    "Usage: graphify export callflow-html [GRAPH|DIR] [--graph PATH] [--labels PATH]\n  --report PATH          path to GRAPH_REPORT.md\n  --sections PATH        JSON section definitions\n  --output HTML          output path (default graphify-out/<project>-callflow.html)\n  --lang LANG            auto, zh-CN, en, etc. (default auto)\n  --max-sections N       maximum auto-derived sections (default 15)\n  --diagram-scale N      Mermaid diagram scale (default 1.0)\n  --max-diagram-nodes N  representative nodes per section (default 18)\n  --max-diagram-edges N  representative edges per section (default 24)".to_owned()
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
    "Usage: trail graph <command>\n\nCommands:\n  query\n  path\n  explain\n  affected\n  export"
        .to_owned()
}

fn graphify_help() -> String {
    "Usage: graphify <command>\n\nPorted commands:\n  query\n  path\n  explain\n  affected\n  export"
        .to_owned()
}
