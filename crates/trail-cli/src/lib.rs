//! Command compatibility layer for Trail's graph namespace.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use trail_core::{
    BuildOptions, ClusterExistingOptions, ExportInputs, LoadedGraph, build_local_graph,
    cluster_existing_graph, default_graph_path, diagnose_graph_file, format_diagnostic_json,
    format_diagnostic_report, merge_graphs,
};
use trail_graph::god_nodes;
use trail_model::GraphError;
use trail_output::{
    CallflowOptions, CallflowSection, CanvasOptions, HtmlOptions, ObsidianOptions, SvgOptions,
    TreeOptions, WikiOptions, export_obsidian, export_wiki, node_filenames, write_callflow_html,
    write_canvas, write_graphml, write_html, write_svg, write_tree_html,
};
use trail_query::{
    DEFAULT_AFFECTED_RELATIONS, TraversalMode, format_affected, format_benchmark, query_graph_text,
    render_explanation, render_shortest_path, run_benchmark,
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
        "benchmark" => command_benchmark(&args),
        "merge-graphs" => command_merge_graphs(&args),
        "tree" if frontend == Frontend::Trail => command_tree(&args),
        "cluster-only" if frontend == Frontend::Trail => command_cluster_only(&args),
        "diagnose" if frontend == Frontend::Trail => command_diagnose(&args),
        "update" if frontend == Frontend::Trail => command_build(&args, false),
        "extract" if frontend == Frontend::Trail => command_build(&args, true),
        "--help" | "-h" | "help" => Outcome::success(if frontend == Frontend::Trail {
            trail_help()
        } else {
            graphify_help()
        }),
        "--version" | "-V" => Outcome::success(format!("trail {}", env!("CARGO_PKG_VERSION"))),
        _ => Outcome::failure(format!("error: unknown graph command '{command}'")),
    }
}

fn command_diagnose(args: &[String]) -> Outcome {
    if args.first().map(String::as_str) != Some("multigraph") {
        return Outcome::failure("Usage: trail graph diagnose multigraph [--graph path] [--json] [--max-examples N] [--directed] [--undirected] [--extract-path path]".to_owned());
    }
    let mut graph_path = default_graph_path();
    let mut max_examples = 5_usize;
    let mut directed = None;
    let mut json_output = false;
    let mut extract_path = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--graph" if index + 1 < args.len() => {
                graph_path = PathBuf::from(&args[index + 1]);
                index += 1;
            }
            "--json" => json_output = true,
            "--max-examples" if index + 1 < args.len() => {
                let Ok(value) = args[index + 1].parse::<usize>() else {
                    return Outcome::failure(
                        "error: --max-examples requires a non-negative integer".to_owned(),
                    );
                };
                max_examples = value;
                index += 1;
            }
            "--directed" if directed != Some(false) => directed = Some(true),
            "--undirected" if directed != Some(true) => directed = Some(false),
            "--directed" | "--undirected" => {
                return Outcome::failure(
                    "error: --directed and --undirected are mutually exclusive".to_owned(),
                );
            }
            "--extract-path" if index + 1 < args.len() => {
                extract_path = Some(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            value => return Outcome::failure(format!("error: unknown diagnose option {value}")),
        }
        index += 1;
    }
    match diagnose_graph_file(&graph_path, directed, max_examples, extract_path.as_deref()) {
        Ok(summary) if json_output => {
            match serde_json::to_string_pretty(&format_diagnostic_json(&summary)) {
                Ok(output) => Outcome::success(output),
                Err(error) => Outcome::failure(format!("error: {error}")),
            }
        }
        Ok(summary) => Outcome::success(format_diagnostic_report(&summary)),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn command_cluster_only(args: &[String]) -> Outcome {
    let mut root = PathBuf::from(".");
    let mut root_set = false;
    let mut graph_override = None;
    let mut no_viz = false;
    let mut no_label = false;
    let mut resolution = 1.0;
    let mut exclude_hubs = None;
    let mut min_community_size = 3_usize;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--graph" if index + 1 < args.len() => {
                graph_override = Some(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            "--no-viz" => no_viz = true,
            "--no-label" => no_label = true,
            "--resolution" if index + 1 < args.len() => {
                let Ok(value) = args[index + 1].parse::<f64>() else {
                    return Outcome::failure("error: --resolution requires a number".to_owned());
                };
                resolution = value;
                index += 1;
            }
            "--exclude-hubs" if index + 1 < args.len() => {
                let Ok(value) = args[index + 1].parse::<f64>() else {
                    return Outcome::failure("error: --exclude-hubs requires a number".to_owned());
                };
                exclude_hubs = Some(value);
                index += 1;
            }
            value if value.starts_with("--min-community-size=") => {
                let Ok(parsed) = value[21..].parse::<usize>() else {
                    return Outcome::failure(
                        "error: --min-community-size requires an integer".to_owned(),
                    );
                };
                min_community_size = parsed;
            }
            "-h" | "--help" => {
                return Outcome::success("Usage: trail graph cluster-only [PATH] [--graph PATH] [--no-viz] [--no-label] [--resolution N] [--exclude-hubs N] [--min-community-size=N]".to_owned());
            }
            value if value.starts_with('-') => {
                return Outcome::failure(format!(
                    "error: unsupported native cluster-only option: {value}"
                ));
            }
            value if !root_set => {
                root = PathBuf::from(value);
                root_set = true;
            }
            value => return Outcome::failure(format!("error: unexpected path: {value}")),
        }
        index += 1;
    }
    let output_name = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let graph_path = graph_override
        .clone()
        .unwrap_or_else(|| root.join(&output_name).join("graph.json"));
    if !graph_path.exists() {
        return Outcome::failure(format!(
            "error: no graph found at {} — run `trail graph extract {} --code-only` first",
            graph_path.display(),
            root.display()
        ));
    }
    let output_dir = if graph_override.is_some()
        && graph_path.parent().and_then(Path::file_name) == Path::new(&output_name).file_name()
    {
        graph_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        root.join(&output_name)
    };
    match cluster_existing_graph(&ClusterExistingOptions {
        graph_path,
        output_dir: output_dir.clone(),
        root,
        no_viz,
        no_label,
        resolution,
        exclude_hubs,
        min_community_size,
    }) {
        Ok(result) => Outcome::success(format!(
            "Trail clustered {} nodes and {} edges into {} communities ({} labels reused).\nWritten to: {}",
            result.nodes,
            result.edges,
            result.communities,
            result.labels_reused,
            output_dir.display()
        )),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn command_tree(args: &[String]) -> Outcome {
    let mut graph_path = default_graph_path();
    let mut output_path = None;
    let mut root = None;
    let mut max_children = 200_usize;
    let mut label = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--graph" if index + 1 < args.len() => {
                graph_path = PathBuf::from(&args[index + 1]);
                index += 1;
            }
            "--output" if index + 1 < args.len() => {
                output_path = Some(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            "--root" if index + 1 < args.len() => {
                root = Some(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            "--max-children" if index + 1 < args.len() => {
                let Ok(value) = args[index + 1].parse::<usize>() else {
                    return Outcome::failure(
                        "error: --max-children requires an integer".to_owned(),
                    );
                };
                max_children = value;
                index += 1;
            }
            "--top-k-edges" if index + 1 < args.len() => {
                if args[index + 1].parse::<usize>().is_err() {
                    return Outcome::failure("error: --top-k-edges requires an integer".to_owned());
                }
                index += 1;
            }
            "--label" if index + 1 < args.len() => {
                label = Some(args[index + 1].clone());
                index += 1;
            }
            "-h" | "--help" => return Outcome::success(tree_help()),
            value => return Outcome::failure(format!("error: unknown tree option {value}")),
        }
        index += 1;
    }
    if !graph_path.is_file() {
        return Outcome::failure(format!(
            "error: graph.json not found at {}",
            graph_path.display()
        ));
    }
    let document = match trail_model::GraphDocument::load(&graph_path) {
        Ok(document) => document,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let output_path = output_path.unwrap_or_else(|| {
        graph_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("GRAPH_TREE.html")
    });
    if let Err(error) = write_tree_html(
        &document,
        &output_path,
        &TreeOptions {
            root: root.as_deref(),
            max_children,
            project_label: label.as_deref(),
            ..TreeOptions::default()
        },
    ) {
        return Outcome::failure(format!("error: {error}"));
    }
    let size = fs::metadata(&output_path)
        .map(|metadata| metadata.len() as f64 / 1024.0)
        .unwrap_or_default();
    let absolute =
        fs::canonicalize(&output_path).unwrap_or_else(|_| absolutize(output_path.clone()));
    Outcome::success(format!(
        "wrote {} ({size:.1} KB)\nopen with: xdg-open {}  (or file://{})",
        output_path.display(),
        output_path.display(),
        absolute.display()
    ))
}

fn tree_help() -> String {
    "Usage: trail graph tree [--graph PATH] [--output HTML]\n  --graph PATH         path to graph.json (default graphify-out/graph.json)\n  --output HTML        output path (default graphify-out/GRAPH_TREE.html)\n  --root PATH          filesystem root (default: longest common dir of all source_files)\n  --max-children N     cap visible children per node (default 200)\n  --top-k-edges N      accepted for Graphify compatibility (currently ignored there too)\n  --label NAME         project label shown in the page header".to_owned()
}

fn command_merge_graphs(args: &[String]) -> Outcome {
    let mut paths = Vec::new();
    let mut output = default_graph_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("merged-graph.json");
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--out" && index + 1 < args.len() {
            output = PathBuf::from(&args[index + 1]);
            index += 2;
        } else {
            paths.push(PathBuf::from(&args[index]));
            index += 1;
        }
    }
    if paths.len() < 2 {
        return Outcome::failure(
            "Usage: graphify merge-graphs <graph1.json> <graph2.json> [...] [--out merged.json]"
                .to_owned(),
        );
    }
    for path in &paths {
        if !path.exists() {
            return Outcome::failure(format!("error: not found: {}", path.display()));
        }
    }
    match merge_graphs(&paths, &output) {
        Ok(result) => {
            let mut lines = Vec::new();
            if result.naive_tags_collided {
                lines.push(format!(
                    "  note: repo dir names collide; using distinct tags: {}",
                    result.tags.join(", ")
                ));
            }
            lines.push(format!(
                "Merged {} graphs -> {} nodes, {} edges",
                result.graphs, result.nodes, result.edges
            ));
            lines.push(format!("Written to: {}", result.output_path.display()));
            Outcome::success(lines.join("\n"))
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn command_benchmark(args: &[String]) -> Outcome {
    let graph_path = args.first().map_or_else(default_graph_path, PathBuf::from);
    let document = match trail_model::GraphDocument::load(&graph_path) {
        Ok(document) => document,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let corpus_words = fs::read(".graphify_detect.json")
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.get("total_words").and_then(serde_json::Value::as_u64))
        .and_then(|value| usize::try_from(value).ok());
    Outcome::success(format_benchmark(
        &run_benchmark(&document, corpus_words, None),
        true,
    ))
}

fn command_build(args: &[String], extract: bool) -> Outcome {
    let mut root = None;
    let mut output_root = None;
    let mut force = false;
    let mut no_cluster = false;
    let mut no_viz = false;
    let mut gitignore = true;
    let mut code_only = false;
    let mut excludes = Vec::new();
    let mut resolution = 1.0;
    let mut exclude_hubs = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--force" => force = true,
            "--no-cluster" => no_cluster = true,
            "--no-viz" => no_viz = true,
            "--no-gitignore" => gitignore = false,
            "--code-only" => code_only = true,
            "--out" if index + 1 < args.len() => {
                output_root = Some(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            value if value.starts_with("--out=") => {
                output_root = Some(PathBuf::from(&value[6..]));
            }
            "--exclude" if index + 1 < args.len() => {
                excludes.push(args[index + 1].clone());
                index += 1;
            }
            value if value.starts_with("--exclude=") => excludes.push(value[10..].to_owned()),
            "--resolution" if index + 1 < args.len() => {
                let Ok(value) = args[index + 1].parse::<f64>() else {
                    return Outcome::failure(
                        "error: --resolution must be a positive number".to_owned(),
                    );
                };
                if value <= 0.0 {
                    return Outcome::failure("error: --resolution must be > 0".to_owned());
                }
                resolution = value;
                index += 1;
            }
            "--exclude-hubs" if index + 1 < args.len() => {
                let Ok(value) = args[index + 1].parse::<f64>() else {
                    return Outcome::failure("error: --exclude-hubs must be a number".to_owned());
                };
                exclude_hubs = Some(value);
                index += 1;
            }
            "-h" | "--help" => {
                return Outcome::success(if extract {
                    "Usage: trail graph extract <path> --code-only [--out DIR] [--no-cluster] [--force] [--no-gitignore] [--exclude PATTERN]".to_owned()
                } else {
                    "Usage: trail graph update [path] [--no-cluster] [--force] [--no-viz]"
                        .to_owned()
                });
            }
            value if value.starts_with('-') => {
                return Outcome::failure(format!("error: unknown graph build option: {value}"));
            }
            value if root.is_none() => root = Some(PathBuf::from(value)),
            value => {
                return Outcome::failure(format!(
                    "error: graph build accepts one path, unexpected: {value}"
                ));
            }
        }
        index += 1;
    }
    if extract && !code_only {
        return Outcome::failure(
            "error: native semantic extraction is not available yet; pass --code-only".to_owned(),
        );
    }
    let root = root
        .or_else(saved_graph_root)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut options = BuildOptions::new(&root);
    options.output_root = output_root;
    options.force = force;
    options.no_cluster = no_cluster;
    options.no_viz = no_viz;
    options.gitignore = gitignore;
    options.extra_excludes = excludes;
    options.resolution = resolution;
    options.exclude_hubs = exclude_hubs;
    match build_local_graph(&options) {
        Ok(result) => {
            let mode = if no_cluster {
                "without clustering"
            } else {
                "with clustering"
            };
            Outcome::success(format!(
                "Trail indexed {} files ({} extracted, {} cached): {} nodes, {} edges, {} communities {mode}.\nWritten to: {}",
                result.files_considered,
                result.files_extracted,
                result.files_cached,
                result.nodes,
                result.edges,
                result.communities,
                result.output_dir.display()
            ))
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn saved_graph_root() -> Option<PathBuf> {
    let path = default_graph_path().parent()?.join(".graphify_root");
    let root = fs::read_to_string(path).ok()?;
    let root = root.trim();
    (!root.is_empty()).then(|| PathBuf::from(root))
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
    "Usage: trail graph <command>\n\nCommands:\n  update\n  extract\n  cluster-only\n  query\n  path\n  explain\n  affected\n  tree\n  export\n  benchmark\n  diagnose multigraph\n  merge-graphs"
        .to_owned()
}

fn graphify_help() -> String {
    "Usage: graphify <command>\n\nPorted commands:\n  query\n  path\n  explain\n  affected\n  export\n  benchmark\n  merge-graphs"
        .to_owned()
}
