//! Command compatibility layer for Trail's graph namespace.

mod dedup_commands;
mod hook_commands;
mod ingest_commands;
mod install_commands;
mod integration_commands;
mod label_commands;
mod provider_commands;
mod prs_commands;
mod result_commands;
mod semantic_commands;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use trail_core::{
    BuildOptions, BuildPurpose, BuildResult, BuildTimings, ClusterExistingOptions, ExportInputs,
    LoadedGraph, SemanticLayer, WatchOptions, WatchStatus, build_graph_with_layers,
    build_graph_with_layers_and_tiebreaker, cluster_existing_graph, default_graph_path,
    diagnose_graph_file, format_diagnostic_json, format_diagnostic_report, merge_graphs,
    watch_local_graph,
};
use trail_files::{DetectOptions, Manifest, ManifestKind, write_bytes_atomic};
use trail_global::{GlobalPaths, global_add};
use trail_graph::god_nodes;
use trail_graphdb::{push_to_falkordb, push_to_neo4j};
use trail_model::GraphError;
use trail_output::{
    CallflowOptions, CallflowSection, CanvasOptions, HtmlOptions, ObsidianOptions, SvgOptions,
    TreeOptions, WikiOptions, export_obsidian, export_wiki, node_filenames, write_callflow_html,
    write_canvas, write_cypher, write_graphml, write_html, write_svg, write_tree_html,
};
use trail_query::{
    DEFAULT_AFFECTED_RELATIONS, TraversalMode, format_affected, format_benchmark, query_graph_text,
    render_explanation, render_shortest_path, run_benchmark,
};
use trail_semantic::{
    CachedCorpusExtractionOptions, CorpusExtractionOptions, detect_backend_with_custom,
    extract_builtin_corpus_cached, extract_custom_corpus_cached, load_custom_providers,
    resolve_builtin_backend, resolve_custom_backend,
};

const GRAPHIFY_COMPAT_VERSION: &str = "0.9.20";

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
    pub stdout_trailing_newline: bool,
    pub stderr_trailing_newline: bool,
}

impl Outcome {
    fn success(stdout: String) -> Self {
        Self {
            code: 0,
            stdout,
            stderr: String::new(),
            stdout_trailing_newline: true,
            stderr_trailing_newline: true,
        }
    }

    fn success_exact(stdout: String) -> Self {
        Self {
            code: 0,
            stdout,
            stderr: String::new(),
            stdout_trailing_newline: false,
            stderr_trailing_newline: true,
        }
    }

    fn failure(stderr: String) -> Self {
        Self::failure_with_code(stderr, 1)
    }

    fn failure_with_code(stderr: String, code: u8) -> Self {
        Self {
            code,
            stdout: String::new(),
            stderr,
            stdout_trailing_newline: true,
            stderr_trailing_newline: true,
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
        match args.first().map(String::as_str) {
            Some("--help" | "-h" | "help") => return Outcome::success(trail_help()),
            Some("--version" | "-V") => {
                return Outcome::success(format!("trail {}", env!("CARGO_PKG_VERSION")));
            }
            _ => {}
        }
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
    if frontend == Frontend::Trail
        && args
            .iter()
            .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        return Outcome::success(trail_command_help(&command));
    }
    match command.as_str() {
        "query" => command_query(&args),
        "path" => command_path(&args),
        "explain" => command_explain(&args),
        "affected" => command_affected(&args),
        "export" => command_export(&args),
        "benchmark" => command_benchmark(&args),
        "merge-graphs" => command_merge_graphs(&args),
        "cache-check" => semantic_commands::command_cache_check(frontend, &args),
        "merge-chunks" => semantic_commands::command_merge_chunks(frontend, &args),
        "merge-semantic" => semantic_commands::command_merge_semantic(frontend, &args),
        "provider" => provider_commands::command_provider(frontend, &args),
        "save-result" => result_commands::command_save_result(frontend, &args),
        "reflect" => result_commands::command_reflect(frontend, &args),
        "check-update" => integration_commands::command_check_update(frontend, &args),
        "hook-check" => integration_commands::command_hook_check(frontend, &args),
        "hook-guard" => integration_commands::command_hook_guard(frontend, &args),
        "merge-driver" => integration_commands::command_merge_driver(frontend, &args),
        "global" => integration_commands::command_global(frontend, &args),
        "clone" => integration_commands::command_clone(frontend, &args),
        "add" => ingest_commands::command_add(frontend, &args),
        "label" => label_commands::command_label(frontend, &args),
        "prs" => prs_commands::command_prs(frontend, &args),
        "hook" => hook_commands::command_hook(frontend, &args),
        "hook-spawn" => hook_commands::command_hook_spawn(frontend, &args),
        "hook-refresh" => command_hook_refresh(frontend, &args),
        "install" => install_commands::command_install(frontend, &args),
        "uninstall" => install_commands::command_uninstall(frontend, &args),
        platform if install_commands::is_direct_command(platform) => {
            install_commands::command_platform(frontend, platform, &args)
        }
        "tree" => command_tree(frontend, &args),
        "cluster-only" => command_cluster_only(frontend, &args),
        "diagnose" => command_diagnose(frontend, &args),
        "update" => command_build(frontend, &args, false),
        "extract" if frontend == Frontend::Trail => command_build(frontend, &args, true),
        "watch" if frontend == Frontend::Trail => Outcome::failure(
            "error: watch is a streaming command and must be run from the trail binary".to_owned(),
        ),
        "serve" if frontend == Frontend::Trail => Outcome::failure(
            "error: serve is a long-lived command and must be run from the trail binary".to_owned(),
        ),
        "--help" | "-h" | "help" => Outcome::success(if frontend == Frontend::Trail {
            trail_help()
        } else {
            graphify_help()
        }),
        "--version" | "-V" => Outcome::success(match frontend {
            Frontend::Trail => format!("trail {}", env!("CARGO_PKG_VERSION")),
            Frontend::Graphify => format!("graphify {GRAPHIFY_COMPAT_VERSION}"),
        }),
        _ if frontend == Frontend::Graphify => Outcome::failure(format!(
            "error: unknown command '{command}'\nRun 'graphify --help' for usage."
        )),
        _ => Outcome::failure(format!("error: unknown graph command '{command}'")),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpFrontend {
    Trail,
    Graphify,
}

/// Parse and run the long-lived native MCP server.
pub fn run_mcp(
    frontend: McpFrontend,
    arguments: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    let args = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let options = match parse_mcp_options(frontend, &args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            let _result = writeln!(stdout, "{}", mcp_help(frontend));
            return 0;
        }
        Err(error) => {
            let _result = writeln!(stderr, "{error}");
            return 2;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _result = writeln!(stderr, "error: could not start async runtime: {error}");
            return 1;
        }
    };
    let result = if options.transport == "http" {
        runtime.block_on(trail_mcp::serve_http(trail_mcp::HttpOptions {
            graph_path: options.graph_path,
            host: options.host,
            port: options.port,
            api_key: options.api_key,
            path: options.path,
            json_response: options.json_response,
            stateless: options.stateless,
            session_timeout: options.session_timeout,
        }))
    } else {
        runtime.block_on(trail_mcp::serve_stdio(options.graph_path))
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            let _result = writeln!(stderr, "error: {error}");
            1
        }
    }
}

#[derive(Debug)]
struct McpOptions {
    graph_path: PathBuf,
    transport: String,
    host: String,
    port: u16,
    api_key: Option<String>,
    path: String,
    json_response: bool,
    stateless: bool,
    session_timeout: Option<Duration>,
}

fn parse_mcp_options(frontend: McpFrontend, args: &[String]) -> Result<Option<McpOptions>, String> {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return Ok(None);
    }
    let mut positional = None;
    let mut graph_flag = None;
    let mut transport = "stdio".to_owned();
    let mut host = "127.0.0.1".to_owned();
    let mut port = 8080_u16;
    let mut api_key = std::env::var("GRAPHIFY_API_KEY").ok();
    let mut path = "/mcp".to_owned();
    let mut json_response = false;
    let mut stateless = false;
    let mut session_timeout = Some(Duration::from_secs(3600));
    let mut index = 0_usize;
    while index < args.len() {
        let value = &args[index];
        match value.as_str() {
            "--graph" => graph_flag = Some(mcp_value(args, &mut index, "--graph")?.into()),
            "--transport" => {
                transport = mcp_value(args, &mut index, "--transport")?.to_owned();
                if !matches!(transport.as_str(), "stdio" | "http") {
                    return Err(format!(
                        "error: argument --transport: invalid choice: '{transport}' (choose from 'stdio', 'http')"
                    ));
                }
            }
            "--host" => host = mcp_value(args, &mut index, "--host")?.to_owned(),
            "--port" => {
                let raw = mcp_value(args, &mut index, "--port")?;
                port = raw
                    .parse::<u16>()
                    .map_err(|_| format!("error: argument --port: invalid int value: '{raw}'"))?;
            }
            "--api-key" => api_key = Some(mcp_value(args, &mut index, "--api-key")?.to_owned()),
            "--path" => path = mcp_value(args, &mut index, "--path")?.to_owned(),
            "--json-response" => json_response = true,
            "--stateless" => stateless = true,
            "--session-timeout" => {
                let raw = mcp_value(args, &mut index, "--session-timeout")?;
                session_timeout = parse_session_timeout(raw)?;
            }
            _ if value.starts_with("--graph=") => {
                graph_flag = Some(PathBuf::from(&value[8..]));
            }
            _ if value.starts_with("--transport=") => {
                transport = value[12..].to_owned();
                if !matches!(transport.as_str(), "stdio" | "http") {
                    return Err(format!(
                        "error: argument --transport: invalid choice: '{transport}' (choose from 'stdio', 'http')"
                    ));
                }
            }
            _ if value.starts_with("--host=") => host = value[7..].to_owned(),
            _ if value.starts_with("--port=") => {
                let raw = &value[7..];
                port = raw
                    .parse::<u16>()
                    .map_err(|_| format!("error: argument --port: invalid int value: '{raw}'"))?;
            }
            _ if value.starts_with("--api-key=") => api_key = Some(value[10..].to_owned()),
            _ if value.starts_with("--path=") => path = value[7..].to_owned(),
            _ if value.starts_with("--session-timeout=") => {
                let raw = &value[18..];
                session_timeout = parse_session_timeout(raw)?;
            }
            _ if value.starts_with('-') => {
                return Err(format!("error: unrecognized arguments: {value}"));
            }
            _ if positional.is_none() => positional = Some(PathBuf::from(value)),
            _ => return Err(format!("error: unrecognized arguments: {value}")),
        }
        index += 1;
    }
    let graph_path = graph_flag
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| positional.filter(|path| !path.as_os_str().is_empty()))
        .unwrap_or_else(default_graph_path);
    let _ = frontend;
    Ok(Some(McpOptions {
        graph_path,
        transport,
        host,
        port,
        api_key,
        path,
        json_response,
        stateless,
        session_timeout,
    }))
}

fn mcp_value<'a>(args: &'a [String], index: &mut usize, option: &str) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| format!("error: argument {option}: expected one argument"))
}

fn parse_session_timeout(raw: &str) -> Result<Option<Duration>, String> {
    let seconds = raw
        .parse::<f64>()
        .map_err(|_| format!("error: argument --session-timeout: invalid float value: '{raw}'"))?;
    if !seconds.is_finite() {
        return Err("error: --session-timeout must be finite".to_owned());
    }
    if seconds <= 0.0 {
        return Ok(None);
    }
    Duration::try_from_secs_f64(seconds)
        .map(Some)
        .map_err(|_| "error: --session-timeout is out of range".to_owned())
}

fn mcp_help(frontend: McpFrontend) -> String {
    let command = if frontend == McpFrontend::Trail {
        "trail graph serve"
    } else {
        "graphify-mcp"
    };
    format!(
        "Usage: {command} [GRAPH_PATH] [--graph PATH] [--transport stdio|http] [--host HOST] [--port PORT] [--api-key KEY] [--path PATH] [--json-response] [--stateless] [--session-timeout SECONDS]"
    )
}

/// Run Trail's long-lived native watcher, streaming status as changes arrive.
///
/// Signal registration lives at this process boundary rather than in
/// `trail-core`, so embedders can provide their own cancellation mechanism.
pub fn run_watch(arguments: &[OsString], stdout: &mut impl Write, stderr: &mut impl Write) -> u8 {
    run_watch_with_frontend(Frontend::Trail, arguments, stdout, stderr)
}

/// Run the frozen Graphify watch frontend without requiring Python or watchdog.
pub fn run_graphify_watch(
    arguments: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    run_watch_with_frontend(Frontend::Graphify, arguments, stdout, stderr)
}

fn run_watch_with_frontend(
    frontend: Frontend,
    arguments: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    let args = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let options = match parse_watch_options(frontend, &args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            let _result = writeln!(stdout, "{}", watch_help());
            return 0;
        }
        Err(error) => {
            let _result = writeln!(stderr, "{error}");
            return 1;
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let signal_stop = Arc::clone(&stop);
    if let Err(error) = ctrlc::set_handler(move || signal_stop.store(true, Ordering::Release)) {
        let _result = writeln!(stderr, "error: could not install Ctrl+C handler: {error}");
        return 1;
    }
    let result = watch_local_graph(&options, &stop, |status| match status {
        WatchStatus::Watching { root, debounce } => {
            if frontend == Frontend::Graphify {
                let _result = writeln!(
                    stdout,
                    "[graphify watch] Watching {} - press Ctrl+C to stop",
                    root.display()
                );
                let _result = writeln!(
                    stdout,
                    "[graphify watch] Code changes rebuild graph automatically. Doc/image changes require /graphify --update."
                );
                let _result = writeln!(
                    stdout,
                    "[graphify watch] Debounce: {:.1}s",
                    debounce.as_secs_f64()
                );
                let _result = stdout.flush();
                return;
            }
            let _result = writeln!(
                stdout,
                "[trail graph watch] Watching {} - press Ctrl+C to stop",
                root.display()
            );
            let _result = writeln!(
                stdout,
                "[trail graph watch] Deterministic changes rebuild locally; semantic media changes set needs_update."
            );
            let _result = writeln!(
                stdout,
                "[trail graph watch] Debounce: {}s",
                debounce.as_secs_f64()
            );
            let _result = stdout.flush();
        }
        WatchStatus::Batch {
            paths,
            deterministic,
            semantic,
        } => {
            if frontend == Frontend::Graphify {
                let _result =
                    writeln!(stdout, "\n[graphify watch] {} file(s) changed", paths.len());
                let _result = stdout.flush();
                return;
            }
            let _result = writeln!(
                stdout,
                "\n[trail graph watch] {} file(s) changed ({deterministic} deterministic, {semantic} semantic)",
                paths.len()
            );
            let _result = stdout.flush();
        }
        WatchStatus::Rebuilt(result) => {
            if frontend == Frontend::Graphify {
                if result.outputs_changed {
                    let _result = writeln!(
                        stdout,
                        "[graphify watch] Rebuilt: {} nodes, {} edges, {} communities",
                        result.nodes, result.edges, result.communities
                    );
                    let html = if result.html_written {
                        ", graph.html"
                    } else {
                        ""
                    };
                    let _result = writeln!(
                        stdout,
                        "[graphify watch] graph.json{html} and GRAPH_REPORT.md updated in {}",
                        result.output_dir.display()
                    );
                } else {
                    let _result = writeln!(
                        stdout,
                        "[graphify watch] No code-graph topology changes detected; outputs left untouched."
                    );
                }
                let _result = stdout.flush();
                return;
            }
            let _result = writeln!(
                stdout,
                "[trail graph watch] Rebuilt: {} nodes, {} edges, {} communities ({} extracted, {} cached)",
                result.nodes,
                result.edges,
                result.communities,
                result.files_extracted,
                result.files_cached
            );
            let _result = writeln!(
                stdout,
                "[trail graph watch] graph artifacts updated in {}",
                result.output_dir.display()
            );
            let _result = stdout.flush();
        }
        WatchStatus::SemanticUpdateRequired { flag } => {
            if frontend == Frontend::Graphify {
                let watch_root = flag
                    .parent()
                    .and_then(Path::parent)
                    .unwrap_or_else(|| Path::new("."));
                let _result = writeln!(
                    stdout,
                    "\n[graphify watch] New or changed files detected in {}",
                    watch_root.display()
                );
                let _result = writeln!(
                    stdout,
                    "[graphify watch] Non-code files changed - semantic re-extraction requires LLM."
                );
                let _result = writeln!(
                    stdout,
                    "[graphify watch] Run `/graphify --update` in Claude Code to update the graph."
                );
                let _result = writeln!(
                    stdout,
                    "[graphify watch] Flag written to {}",
                    flag.display()
                );
                let _result = stdout.flush();
                return;
            }
            let _result = writeln!(
                stdout,
                "[trail graph watch] Semantic media changed; update required. Flag written to {}",
                flag.display()
            );
            let _result = stdout.flush();
        }
        WatchStatus::EventError(error) => {
            if frontend == Frontend::Graphify {
                let _result = writeln!(stdout, "[graphify watch] Filesystem event error: {error}");
                let _result = stdout.flush();
                return;
            }
            let _result = writeln!(
                stderr,
                "[trail graph watch] Filesystem event error: {error}"
            );
            let _result = stderr.flush();
        }
        WatchStatus::RebuildError(error) => {
            if frontend == Frontend::Graphify {
                let _result = writeln!(stdout, "[graphify watch] Rebuild failed: {error}");
                let _result = stdout.flush();
            } else {
                let _result = writeln!(stderr, "[trail graph watch] Rebuild failed: {error}");
                let _result = stderr.flush();
            }
        }
        WatchStatus::Stopped => {
            let label = if frontend == Frontend::Graphify {
                "graphify watch"
            } else {
                "trail graph watch"
            };
            let _result = writeln!(stdout, "\n[{label}] Stopped.");
            let _result = stdout.flush();
        }
    });
    match result {
        Ok(()) => 0,
        Err(error) => {
            let _result = writeln!(stderr, "error: {error}");
            1
        }
    }
}

fn parse_watch_options(
    frontend: Frontend,
    args: &[String],
) -> Result<Option<WatchOptions>, String> {
    if frontend == Frontend::Graphify {
        let root = args
            .first()
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        if !root.exists() {
            return Err(format!("error: path not found: {}", root.display()));
        }
        let mut options = WatchOptions::new(root);
        options.graphify_compatibility = true;
        options.force_polling = cfg!(target_os = "macos");
        return Ok(Some(options));
    }
    let mut root = None;
    let mut output_root = None;
    let mut debounce = Duration::from_secs(3);
    let mut no_cluster = false;
    let mut no_viz = false;
    let mut gitignore = true;
    let mut excludes = Vec::new();
    let mut force_polling = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Ok(None),
            "--no-cluster" => no_cluster = true,
            "--no-viz" => no_viz = true,
            "--no-gitignore" => gitignore = false,
            "--poll" => force_polling = true,
            "--debounce" if index + 1 < args.len() => {
                debounce = parse_positive_seconds(&args[index + 1], "--debounce")?;
                index += 1;
            }
            value if value.starts_with("--debounce=") => {
                debounce = parse_positive_seconds(&value[11..], "--debounce")?;
            }
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
            value if value.starts_with('-') => {
                return Err(format!("error: unknown watch option: {value}"));
            }
            value if root.is_none() => root = Some(PathBuf::from(value)),
            value => {
                return Err(format!(
                    "error: watch accepts one path, unexpected: {value}"
                ));
            }
        }
        index += 1;
    }
    let mut options = WatchOptions::new(root.unwrap_or_else(|| PathBuf::from(".")));
    options.debounce = debounce;
    options.force_polling = force_polling;
    options.build.output_root = output_root;
    options.build.no_cluster = no_cluster;
    options.build.no_viz = no_viz;
    options.build.gitignore = gitignore;
    options.build.extra_excludes = excludes;
    Ok(Some(options))
}

fn parse_positive_seconds(value: &str, option: &str) -> Result<Duration, String> {
    let seconds = value
        .parse::<f64>()
        .map_err(|_| format!("error: {option} requires a positive number"))?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(format!("error: {option} must be > 0"));
    }
    Ok(Duration::from_secs_f64(seconds))
}

fn command_diagnose(frontend: Frontend, args: &[String]) -> Outcome {
    if args.first().map(String::as_str) != Some("multigraph") {
        let prefix = match frontend {
            Frontend::Trail => "trail graph",
            Frontend::Graphify => "graphify",
        };
        return Outcome::failure(format!(
            "Usage: {prefix} diagnose multigraph [--graph path] [--json] [--max-examples N] [--directed] [--undirected] [--extract-path path]"
        ));
    }
    let mut graph_path = default_graph_path();
    let mut max_examples = 5_usize;
    let mut directed = None;
    let mut json_output = false;
    let mut extract_path = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--graph" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Outcome::failure("error: --graph requires a path".to_owned());
                };
                graph_path = PathBuf::from(value);
            }
            "--json" => json_output = true,
            "--max-examples" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Outcome::failure(
                        "error: --max-examples requires an integer".to_owned(),
                    );
                };
                let Ok(value) = value.parse::<isize>() else {
                    return Outcome::failure(
                        "error: --max-examples requires an integer".to_owned(),
                    );
                };
                let Ok(value) = usize::try_from(value) else {
                    return Outcome::failure("error: --max-examples must be >= 0".to_owned());
                };
                max_examples = value;
            }
            "--directed" if directed != Some(false) => directed = Some(true),
            "--undirected" if directed != Some(true) => directed = Some(false),
            "--directed" | "--undirected" => {
                return Outcome::failure(
                    "error: --directed and --undirected are mutually exclusive".to_owned(),
                );
            }
            "--extract-path" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Outcome::failure("error: --extract-path requires a path".to_owned());
                };
                extract_path = Some(PathBuf::from(value));
            }
            value => return Outcome::failure(format!("error: unknown diagnose option {value}")),
        }
        index += 1;
    }
    if frontend == Frontend::Graphify && extract_path.is_none() {
        let source_checkout = PathBuf::from("graphify/extract.py");
        if source_checkout.is_file() {
            extract_path = source_checkout.canonicalize().ok();
        }
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

fn command_cluster_only(frontend: Frontend, args: &[String]) -> Outcome {
    let mut root = PathBuf::from(".");
    let mut root_set = false;
    let mut graph_override = None;
    let mut no_viz = false;
    let mut no_label = false;
    let mut timing = false;
    let mut resolution = 1.0;
    let mut exclude_hubs = None;
    let mut min_community_size = 3_usize;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--graph" => {
                let Some(value) = args.get(index + 1) else {
                    if frontend == Frontend::Graphify {
                        index += 1;
                        continue;
                    }
                    return Outcome::failure("error: --graph requires a value".to_owned());
                };
                graph_override = Some(PathBuf::from(value));
                index += 1;
            }
            "--no-viz" => no_viz = true,
            "--no-label" => no_label = true,
            "--missing-only" => {}
            "--timing" => timing = true,
            "--resolution" => {
                let Some(argument) = args.get(index + 1) else {
                    if frontend == Frontend::Graphify {
                        index += 1;
                        continue;
                    }
                    return Outcome::failure("error: --resolution requires a value".to_owned());
                };
                let Ok(value) = argument.parse::<f64>() else {
                    return Outcome::failure("error: --resolution requires a number".to_owned());
                };
                resolution = value;
                index += 1;
            }
            value if value.starts_with("--resolution=") => {
                let Ok(parsed) = value[13..].parse::<f64>() else {
                    return Outcome::failure("error: --resolution requires a number".to_owned());
                };
                resolution = parsed;
            }
            "--exclude-hubs" => {
                let Some(argument) = args.get(index + 1) else {
                    if frontend == Frontend::Graphify {
                        index += 1;
                        continue;
                    }
                    return Outcome::failure("error: --exclude-hubs requires a value".to_owned());
                };
                let Ok(value) = argument.parse::<f64>() else {
                    return Outcome::failure("error: --exclude-hubs requires a number".to_owned());
                };
                exclude_hubs = Some(value);
                index += 1;
            }
            value if value.starts_with("--exclude-hubs=") => {
                let Ok(parsed) = value[15..].parse::<f64>() else {
                    return Outcome::failure("error: --exclude-hubs requires a number".to_owned());
                };
                exclude_hubs = Some(parsed);
            }
            "--backend" | "--model" | "--max-concurrency" | "--batch-size"
                if index + 1 < args.len() =>
            {
                index += 1;
            }
            value
                if value.starts_with("--backend=")
                    || value.starts_with("--model=")
                    || value.starts_with("--max-concurrency=")
                    || value.starts_with("--batch-size=") => {}
            value if value.starts_with("--min-community-size=") => {
                let Ok(parsed) = value[21..].parse::<usize>() else {
                    return Outcome::failure(
                        "error: --min-community-size requires an integer".to_owned(),
                    );
                };
                min_community_size = parsed;
            }
            "-h" | "--help" if frontend == Frontend::Trail => {
                return Outcome::success("Usage: trail graph cluster-only [PATH] [--graph PATH] [--no-viz] [--no-label] [--resolution N] [--exclude-hubs N] [--min-community-size=N]".to_owned());
            }
            value if value.starts_with('-') && frontend == Frontend::Trail => {
                return Outcome::failure(format!(
                    "error: unsupported native cluster-only option: {value}"
                ));
            }
            value if value.starts_with('-') => {}
            value if !root_set => {
                root = PathBuf::from(value);
                root_set = true;
            }
            value if frontend == Frontend::Trail => {
                return Outcome::failure(format!("error: unexpected path: {value}"));
            }
            _ => {}
        }
        index += 1;
    }
    let output_name = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let graph_path = graph_override
        .clone()
        .unwrap_or_else(|| root.join(&output_name).join("graph.json"));
    if !graph_path.exists() {
        return Outcome::failure(match frontend {
            Frontend::Graphify => format!(
                "error: no graph found at {} — run /graphify first",
                graph_path.display()
            ),
            Frontend::Trail => format!(
                "error: no graph found at {} — run `trail graph extract {}` first",
                graph_path.display(),
                root.display()
            ),
        });
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
    if frontend == Frontend::Graphify
        && !no_label
        && !output_dir.join(".graphify_labels.json").is_file()
    {
        return label_commands::command_label(frontend, args);
    }
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
        Ok(result) if frontend == Frontend::Graphify => {
            let mut warnings = result.load_warning.clone().into_iter().collect::<Vec<_>>();
            if timing {
                warnings.extend([
                    format!(
                        "[graphify timing] load: {:.1}s",
                        result.timings.load.as_secs_f64()
                    ),
                    format!(
                        "[graphify timing] cluster: {:.1}s",
                        result.timings.cluster.as_secs_f64()
                    ),
                    format!(
                        "[graphify timing] analyze: {:.1}s",
                        result.timings.analyze.as_secs_f64()
                    ),
                    format!(
                        "[graphify timing] label: {:.1}s",
                        result.timings.label.as_secs_f64()
                    ),
                    format!(
                        "[graphify timing] report: {:.1}s",
                        result.timings.report.as_secs_f64()
                    ),
                ]);
            }
            if let Some(warning) = result.backup_warning.clone() {
                warnings.push(warning);
            }
            if timing {
                warnings.extend([
                    format!(
                        "[graphify timing] export: {:.1}s",
                        result.timings.export.as_secs_f64()
                    ),
                    format!(
                        "[graphify timing] total: {:.1}s",
                        result.timings.total.as_secs_f64()
                    ),
                ]);
            }
            let done = if no_viz {
                format!(
                    "Done - {} communities. GRAPH_REPORT.md and graph.json updated (--no-viz; graph.html removed).",
                    result.communities
                )
            } else if result.html_written {
                format!(
                    "Done - {} communities. GRAPH_REPORT.md, graph.json and graph.html updated.",
                    result.communities
                )
            } else {
                format!(
                    "Done - {} communities. GRAPH_REPORT.md and graph.json updated.",
                    result.communities
                )
            };
            let backup = result
                .backup_message
                .as_deref()
                .map(|message| format!("{message}\n"))
                .unwrap_or_default();
            Outcome {
                code: 0,
                stdout: format!(
                    "Loading existing graph...\nGraph: {} nodes, {} edges\nRe-clustering...\n{backup}{done}",
                    result.nodes, result.edges
                ),
                stderr: warnings.join("\n"),
                stdout_trailing_newline: true,
                stderr_trailing_newline: true,
            }
        }
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

fn command_tree(frontend: Frontend, args: &[String]) -> Outcome {
    if frontend == Frontend::Graphify
        && args
            .iter()
            .any(|argument| matches!(argument.as_str(), "-h" | "--help" | "-?"))
    {
        return Outcome::success("Run 'graphify --help' for full usage.".to_owned());
    }
    let mut graph_path = default_graph_path();
    let mut output_path = None;
    let mut root = None;
    let mut max_children = 200_isize;
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
                let Ok(value) = args[index + 1].parse::<isize>() else {
                    return Outcome::failure(
                        "error: --max-children requires an integer".to_owned(),
                    );
                };
                max_children = value;
                index += 1;
            }
            "--top-k-edges" if index + 1 < args.len() => {
                if args[index + 1].parse::<isize>().is_err() {
                    return Outcome::failure("error: --top-k-edges requires an integer".to_owned());
                }
                index += 1;
            }
            "--label" if index + 1 < args.len() => {
                label = Some(args[index + 1].clone());
                index += 1;
            }
            "-h" | "--help" => return Outcome::success(tree_help(frontend)),
            _ => {}
        }
        index += 1;
    }
    if !graph_path.is_file() {
        return Outcome::failure(format!(
            "error: graph.json not found at {}",
            graph_path.display()
        ));
    }
    if let Some((size, cap)) = trail_model::GraphDocument::size_cap_exceeded(&graph_path) {
        return Outcome::failure(format!(
            "error: graph file {} is {} bytes, exceeds {}-byte cap\n(set GRAPHIFY_MAX_GRAPH_BYTES=<bytes> or GRAPHIFY_MAX_GRAPH_BYTES=<N>GB to raise the limit)",
            graph_path.display(),
            grouped_decimal(size),
            grouped_decimal(cap)
        ));
    }
    let document = match trail_model::GraphDocument::load_for_recluster_compatibility(&graph_path) {
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

fn grouped_decimal(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push('_');
        }
        output.push(character);
    }
    output
}

fn tree_help(frontend: Frontend) -> String {
    let prefix = match frontend {
        Frontend::Trail => "trail graph",
        Frontend::Graphify => "graphify",
    };
    format!(
        "Usage: {prefix} tree [--graph PATH] [--output HTML]\n  --graph PATH         path to graph.json (default graphify-out/graph.json)\n  --output HTML        output path (default graphify-out/GRAPH_TREE.html)\n  --root PATH          filesystem root (default: longest common dir of all source_files)\n  --max-children N     cap visible children per node (default 200)\n  --top-k-edges N      pre-compute top-K outbound edges per symbol (default 12)\n  --label NAME         project label shown in the page header"
    )
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

fn validate_graphify_update_args(args: &[String]) -> Option<Outcome> {
    let mut path = None;
    for argument in args {
        if matches!(argument.as_str(), "--force" | "--no-cluster") {
            continue;
        }
        if argument.starts_with('-') {
            return Some(Outcome::failure_with_code(
                format!("error: unknown update option: {argument}"),
                2,
            ));
        }
        if path.is_some() {
            return Some(Outcome::failure_with_code(
                "error: update accepts at most one path argument".to_owned(),
                2,
            ));
        }
        path = Some(argument);
    }
    if let Some(path) = path
        && !Path::new(path).exists()
    {
        return Some(Outcome::failure(format!("error: path not found: {path}")));
    }
    None
}

fn format_graphify_update(result: &BuildResult, watch_path: &Path, no_cluster: bool) -> Outcome {
    let output_name = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let output_dir = if watch_path == Path::new(".") {
        PathBuf::from(&output_name)
    } else {
        watch_path.join(&output_name)
    };
    let mut lines = vec![format!(
        "Re-extracting code files in {} (no LLM needed)...",
        watch_path.display()
    )];
    if !result.empty_files.is_empty() {
        let examples = result
            .empty_files
            .iter()
            .take(5)
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect::<Vec<_>>();
        let remaining = result.empty_files.len().saturating_sub(examples.len());
        let suffix = if remaining == 0 {
            String::new()
        } else {
            format!(" (+{remaining} more)")
        };
        lines.push(format!(
            "  warning: {} source file(s) produced zero nodes and are absent from the graph: {}{suffix}. A re-run will retry them (empties are no longer cached); if it persists, please report the file(s) (#1666).",
            result.empty_files.len(),
            examples.join(", ")
        ));
    }
    if result.outputs_changed {
        if no_cluster {
            lines.push(format!(
                "[graphify watch] Rebuilt (no clustering): {} nodes, {} edges",
                result.nodes, result.edges
            ));
            lines.push(format!(
                "[graphify watch] graph.json updated in {}",
                output_dir.display()
            ));
        } else {
            let viz_limit = std::env::var("GRAPHIFY_VIZ_NODE_LIMIT")
                .ok()
                .and_then(|value| value.parse::<isize>().ok())
                .unwrap_or(5_000);
            if !result.html_written
                && isize::try_from(result.nodes).map_or(true, |nodes| nodes > viz_limit)
            {
                lines.push(format!(
                    "[graphify watch] Skipped graph.html: Graph has {} nodes - too large for HTML viz (limit: {viz_limit}). Use --no-viz, raise GRAPHIFY_VIZ_NODE_LIMIT, or reduce input size.",
                    result.nodes,
                ));
            }
            lines.push(format!(
                "[graphify watch] Rebuilt: {} nodes, {} edges, {} communities",
                result.nodes, result.edges, result.communities
            ));
            let artifacts = if result.html_written {
                "graph.json, graph.html and GRAPH_REPORT.md"
            } else {
                "graph.json and GRAPH_REPORT.md"
            };
            lines.push(format!(
                "[graphify watch] {artifacts} updated in {}",
                output_dir.display()
            ));
        }
    } else {
        lines.push(
            "[graphify watch] No code-graph topology changes detected; outputs left untouched."
                .to_owned(),
        );
    }
    lines.push(
        "Code graph updated. For doc/paper/image changes run /graphify --update in your AI assistant."
            .to_owned(),
    );
    if ![
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "MOONSHOT_API_KEY",
        "DEEPSEEK_API_KEY",
        "GRAPHIFY_NO_TIPS",
    ]
    .into_iter()
    .any(|key| std::env::var_os(key).is_some())
    {
        lines.push(
            "Tip: set GEMINI_API_KEY or GOOGLE_API_KEY to use Gemini for semantic extraction."
                .to_owned(),
        );
    }
    Outcome::success(lines.join("\n"))
}

fn command_build(frontend: Frontend, args: &[String], extract: bool) -> Outcome {
    if frontend == Frontend::Graphify
        && !extract
        && let Some(error) = validate_graphify_update_args(args)
    {
        return error;
    }
    let started = Instant::now();
    let mut root = None;
    let mut output_root = None;
    let mut force = environment_truthy("GRAPHIFY_FORCE");
    let mut no_cluster = false;
    let mut no_viz = false;
    let mut gitignore = true;
    let mut code_only = false;
    let mut cargo = false;
    let mut google_workspace = false;
    let mut global_merge = false;
    let mut global_repo_tag = None;
    let mut postgres_dsn = None;
    let mut backend = None;
    let mut model = None;
    let mut deep_mode = false;
    let mut token_budget = None;
    let mut max_concurrency = None;
    let mut max_workers = None;
    let mut api_timeout = None;
    let mut allow_partial = false;
    let mut timing = false;
    let mut dedup_llm = false;
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
            "--cargo" if extract => cargo = true,
            "--google-workspace" if extract => google_workspace = true,
            "--global" if extract => global_merge = true,
            "--as" if extract && index + 1 < args.len() => {
                global_repo_tag = Some(args[index + 1].clone());
                index += 1;
            }
            value if extract && value.starts_with("--as=") => {
                global_repo_tag = Some(value[5..].to_owned());
            }
            "--postgres" if extract && index + 1 < args.len() => {
                postgres_dsn = Some(args[index + 1].clone());
                index += 1;
            }
            value if extract && value.starts_with("--postgres=") => {
                postgres_dsn = Some(value[11..].to_owned());
            }
            "--allow-partial" if extract => allow_partial = true,
            "--backend" if extract && index + 1 < args.len() => {
                backend = Some(args[index + 1].clone());
                index += 1;
            }
            value if extract && value.starts_with("--backend=") => {
                backend = Some(value[10..].to_owned());
            }
            "--model" if extract && index + 1 < args.len() => {
                model = Some(args[index + 1].clone());
                index += 1;
            }
            value if extract && value.starts_with("--model=") => {
                model = Some(value[8..].to_owned());
            }
            "--mode" if extract && index + 1 < args.len() => {
                if args[index + 1] != "deep" {
                    return Outcome::failure(format!(
                        "error: unknown --mode '{}'. Available: deep",
                        args[index + 1]
                    ));
                }
                deep_mode = true;
                index += 1;
            }
            value if extract && value.starts_with("--mode=") => {
                if &value[7..] != "deep" {
                    return Outcome::failure(format!(
                        "error: unknown --mode '{}'. Available: deep",
                        &value[7..]
                    ));
                }
                deep_mode = true;
            }
            "--token-budget" if extract && index + 1 < args.len() => {
                token_budget = match parse_positive_usize(&args[index + 1], "--token-budget") {
                    Ok(value) => Some(value),
                    Err(error) => return Outcome::failure(error),
                };
                index += 1;
            }
            value if extract && value.starts_with("--token-budget=") => {
                token_budget = match parse_positive_usize(&value[15..], "--token-budget") {
                    Ok(value) => Some(value),
                    Err(error) => return Outcome::failure(error),
                };
            }
            "--max-concurrency" if extract && index + 1 < args.len() => {
                max_concurrency = match parse_positive_usize(&args[index + 1], "--max-concurrency")
                {
                    Ok(value) => Some(value),
                    Err(error) => return Outcome::failure(error),
                };
                index += 1;
            }
            value if extract && value.starts_with("--max-concurrency=") => {
                max_concurrency = match parse_positive_usize(&value[18..], "--max-concurrency") {
                    Ok(value) => Some(value),
                    Err(error) => return Outcome::failure(error),
                };
            }
            "--api-timeout" if extract && index + 1 < args.len() => {
                api_timeout = match parse_positive_f64(&args[index + 1], "--api-timeout") {
                    Ok(value) => Some(value),
                    Err(error) => return Outcome::failure(error),
                };
                index += 1;
            }
            value if extract && value.starts_with("--api-timeout=") => {
                api_timeout = match parse_positive_f64(&value[14..], "--api-timeout") {
                    Ok(value) => Some(value),
                    Err(error) => return Outcome::failure(error),
                };
            }
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
            value if value.starts_with("--resolution=") => {
                let Ok(parsed) = value[13..].parse::<f64>() else {
                    return Outcome::failure(
                        "error: --resolution must be a positive number".to_owned(),
                    );
                };
                if !parsed.is_finite() || parsed <= 0.0 {
                    return Outcome::failure("error: --resolution must be > 0".to_owned());
                }
                resolution = parsed;
            }
            "--exclude-hubs" if index + 1 < args.len() => {
                let Ok(value) = args[index + 1].parse::<f64>() else {
                    return Outcome::failure("error: --exclude-hubs must be a number".to_owned());
                };
                exclude_hubs = Some(value);
                index += 1;
            }
            value if value.starts_with("--exclude-hubs=") => {
                let Ok(parsed) = value[15..].parse::<f64>() else {
                    return Outcome::failure("error: --exclude-hubs must be a number".to_owned());
                };
                if !parsed.is_finite() {
                    return Outcome::failure(
                        "error: --exclude-hubs must be a finite number".to_owned(),
                    );
                }
                exclude_hubs = Some(parsed);
            }
            "--max-workers" if extract && index + 1 < args.len() => {
                max_workers = match parse_positive_usize(&args[index + 1], "--max-workers") {
                    Ok(value) => Some(value),
                    Err(error) => return Outcome::failure(error),
                };
                index += 1;
            }
            value if extract && value.starts_with("--max-workers=") => {
                max_workers = match parse_positive_usize(&value[14..], "--max-workers") {
                    Ok(value) => Some(value),
                    Err(error) => return Outcome::failure(error),
                };
            }
            "--timing" if extract => timing = true,
            "--dedup-llm" if extract => dedup_llm = true,
            "-h" | "--help" => {
                return Outcome::success(if extract {
                    extract_help()
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
    let has_explicit_root = root.is_some();
    if extract && !has_explicit_root && postgres_dsn.is_none() {
        return Outcome::failure(
            "error: must specify a path to scan or a --postgres DSN".to_owned(),
        );
    }
    let root = if extract && !has_explicit_root {
        PathBuf::from(".")
    } else {
        root.or_else(saved_graph_root)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    if frontend == Frontend::Graphify && !root.exists() {
        return Outcome::failure(format!("error: path not found: {}", root.display()));
    }
    let mut options = BuildOptions::new(&root);
    options.scan_filesystem = has_explicit_root || !extract;
    options.output_root = output_root;
    options.force = force;
    options.no_cluster = no_cluster;
    options.no_viz = no_viz;
    options.gitignore = gitignore;
    options.extra_excludes = excludes;
    options.resolution = resolution;
    options.exclude_hubs = exclude_hubs;
    options.purpose = if extract {
        BuildPurpose::Extract
    } else {
        BuildPurpose::Update
    };
    options.google_workspace =
        google_workspace || trail_google_workspace::google_workspace_enabled(None);
    options.max_workers = max_workers;
    let mut dedup_environment = std::env::vars().collect::<HashMap<_, _>>();
    if let Some(timeout) = api_timeout {
        dedup_environment.insert("GRAPHIFY_API_TIMEOUT".to_owned(), timeout.to_string());
    }
    let mut dedup_tiebreaker = if dedup_llm {
        let global_providers = home_directory()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".graphify")
            .join("providers.json");
        let local_providers = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".graphify")
            .join("providers.json");
        match dedup_commands::DedupLlmTiebreaker::prepare(
            backend.as_deref(),
            model.as_deref(),
            dedup_environment,
            &global_providers,
            &local_providers,
            environment_truthy("GRAPHIFY_ALLOW_LOCAL_PROVIDERS"),
            executable_on_path("claude"),
        ) {
            Ok(tiebreaker) => Some(tiebreaker),
            Err(error) if error == "no LLM backend selected" => {
                return Outcome::failure(
                    "error: no LLM API key found (--dedup-llm was passed). Set GEMINI_API_KEY or GOOGLE_API_KEY (gemini), MOONSHOT_API_KEY (kimi), ANTHROPIC_API_KEY (claude), OPENAI_API_KEY (openai), DEEPSEEK_API_KEY (deepseek), or pass --backend. A code-only corpus needs no key."
                        .to_owned(),
                );
            }
            Err(error) => return Outcome::failure(format!("error: {error}")),
        }
    } else {
        None
    };
    let compatibility_manifest = (frontend == Frontend::Graphify && !extract).then(|| {
        let output_name =
            std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
        let output_root = options.output_root.as_ref().unwrap_or(&root);
        let path = output_root.join(output_name).join("manifest.json");
        let existing = fs::read(&path).ok();
        (path, existing)
    });
    let postgres_graph = if let Some(dsn) = postgres_dsn.as_deref() {
        match trail_postgres::introspect_postgres(Some(dsn)) {
            Ok(graph) => Some(graph),
            Err(error) => return Outcome::failure(format!("error: {error}")),
        }
    } else {
        None
    };
    let postgres_counts = postgres_graph
        .as_ref()
        .map(|graph| (graph.node_count(), graph.edge_count()));
    let cargo_graph = if cargo {
        match trail_cargo::introspect_cargo(&root) {
            Ok(graph) => Some(graph),
            Err(error) => return Outcome::failure(format!("error: {error}")),
        }
    } else {
        None
    };
    let cargo_counts = cargo_graph
        .as_ref()
        .map(|graph| (graph.nodes.len(), graph.edges.len()));
    let mut auxiliary_fragments = Vec::new();
    if let Some(graph) = postgres_graph {
        auxiliary_fragments.push(graph.into_fragment());
    }
    if let Some(graph) = cargo_graph {
        auxiliary_fragments.push(graph.into_fragment());
    }
    let built = if extract && !code_only {
        build_semantic_graph(
            &options,
            backend.as_deref(),
            model.as_deref(),
            deep_mode,
            token_budget,
            max_concurrency,
            api_timeout,
            allow_partial,
            &auxiliary_fragments,
            dedup_tiebreaker
                .as_mut()
                .map(|tiebreaker| tiebreaker as &mut dyn trail_graph::EntityTiebreaker),
        )
    } else if extract && !auxiliary_fragments.is_empty() {
        build_graph_with_optional_tiebreaker(
            &options,
            None,
            &auxiliary_fragments,
            dedup_tiebreaker
                .as_mut()
                .map(|tiebreaker| tiebreaker as &mut dyn trail_graph::EntityTiebreaker),
        )
        .map(|result| (result, Vec::new(), Duration::ZERO))
        .map_err(|error| error.to_string())
    } else {
        build_graph_with_optional_tiebreaker(
            &options,
            None,
            &[],
            dedup_tiebreaker
                .as_mut()
                .map(|tiebreaker| tiebreaker as &mut dyn trail_graph::EntityTiebreaker),
        )
        .map(|result| (result, Vec::new(), Duration::ZERO))
        .map_err(|error| error.to_string())
    };
    match built {
        Ok((result, mut notes, semantic_elapsed)) => {
            if let Some(tiebreaker) = dedup_tiebreaker.as_mut() {
                notes.extend(tiebreaker.take_warnings());
            }
            if let Some((path, existing)) = compatibility_manifest {
                let restored = match existing {
                    Some(bytes) => {
                        write_bytes_atomic(&path, &bytes).map_err(|error| error.to_string())
                    }
                    None if path.exists() => {
                        fs::remove_file(&path).map_err(|error| error.to_string())
                    }
                    None => Ok(()),
                };
                if let Err(error) = restored {
                    return Outcome::failure(format!(
                        "error: could not restore legacy manifest state at {}: {error}",
                        path.display()
                    ));
                }
            }
            let mut global_warning = None;
            if let Some((nodes, edges)) = postgres_counts {
                notes.push(format!(
                    "[trail graph extract] PostgreSQL: {nodes} nodes, {edges} edges"
                ));
            }
            if let Some((nodes, edges)) = cargo_counts {
                notes.push(format!(
                    "[trail graph extract] Cargo: {nodes} nodes, {edges} edges"
                ));
            }
            if global_merge {
                let tag = global_repo_tag.clone().unwrap_or_else(|| {
                    root.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default()
                        .to_owned()
                });
                match GlobalPaths::discover().and_then(|paths| {
                    global_add(
                        &paths,
                        &result.output_dir.join("graph.json"),
                        &tag,
                        time::OffsetDateTime::now_utc(),
                    )
                }) {
                    Ok(merged) if merged.skipped => notes.push(format!(
                        "[graphify global] '{tag}' unchanged since last add - skipped."
                    )),
                    Ok(merged) => notes.push(format!(
                        "[graphify global] '{tag}' merged into global graph (+{} nodes, -{} pruned).",
                        merged.nodes_added, merged.nodes_removed
                    )),
                    Err(error) => {
                        global_warning = Some(format!(
                            "[graphify global] warning: failed to merge into global graph: {error}"
                        ));
                    }
                }
            }
            let mode = if no_cluster {
                "without clustering"
            } else {
                "with clustering"
            };
            if frontend == Frontend::Graphify && !extract {
                return format_graphify_update(&result, &root, no_cluster);
            }
            let mut output = format!(
                "Trail indexed {} files ({} extracted, {} cached): {} nodes, {} edges, {} communities {mode}.\nWritten to: {}",
                result.files_considered,
                result.files_extracted,
                result.files_cached,
                result.nodes,
                result.edges,
                result.communities,
                result.output_dir.display()
            );
            if !notes.is_empty() {
                output.push('\n');
                output.push_str(&notes.join("\n"));
            }
            let mut outcome = Outcome::success(output);
            if let Some(warning) = global_warning {
                outcome.stderr = warning;
            }
            if timing {
                if !outcome.stderr.is_empty() {
                    outcome.stderr.push('\n');
                }
                outcome.stderr.push_str(&format_extract_timings(
                    no_cluster,
                    started.elapsed(),
                    semantic_elapsed,
                    &result.timings,
                ));
            }
            outcome
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn format_extract_timings(
    no_cluster: bool,
    elapsed: Duration,
    semantic_elapsed: Duration,
    timings: &BuildTimings,
) -> String {
    let mut stages = vec![
        ("detect", timings.detect),
        ("AST extract", timings.ast_extract),
        ("semantic extract", semantic_elapsed),
    ];
    if no_cluster {
        stages.push(("write", timings.write));
    } else {
        stages.extend([
            ("build", timings.build),
            ("cluster", timings.cluster),
            ("analyze", timings.analyze),
            ("export", timings.export),
        ]);
    }
    let mut lines = stages
        .into_iter()
        .map(|(stage, duration)| {
            format!("[graphify timing] {stage}: {:.1}s", duration.as_secs_f64())
        })
        .collect::<Vec<_>>();
    lines.push(format!(
        "[graphify timing] total: {:.1}s",
        elapsed.as_secs_f64()
    ));
    lines.join("\n")
}

fn command_hook_refresh(frontend: Frontend, args: &[String]) -> Outcome {
    let launch_root = args
        .iter()
        .find(|argument| !argument.starts_with('-'))
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let output_name = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let marker = launch_root.join(&output_name).join(".graphify_root");
    let recorded_root = hook_commands::read_text_bounded(&marker, 16 * 1024)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && !value.contains('\0'));
    let build_args = recorded_root.map_or_else(
        || args.to_vec(),
        |recorded| {
            vec![
                recorded,
                "--out".to_owned(),
                launch_root.to_string_lossy().into_owned(),
            ]
        },
    );
    let result = command_build(frontend, &build_args, false);
    if result.code != 0 {
        return result;
    }
    let memory = launch_root.join(&output_name).join("memory");
    let has_memories = fs::read_dir(memory).is_ok_and(|entries| {
        entries
            .filter_map(Result::ok)
            .any(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("md"))
    });
    if has_memories {
        let _ = result_commands::command_reflect(frontend, &["--if-stale".to_owned()]);
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn build_semantic_graph(
    options: &BuildOptions,
    requested_backend: Option<&str>,
    requested_model: Option<&str>,
    deep_mode: bool,
    token_budget: Option<usize>,
    max_concurrency: Option<usize>,
    api_timeout: Option<f64>,
    allow_partial: bool,
    auxiliary_fragments: &[serde_json::Value],
    tiebreaker: Option<&mut dyn trail_graph::EntityTiebreaker>,
) -> Result<(BuildResult, Vec<String>, Duration), String> {
    let semantic_started = Instant::now();
    let root = fs::canonicalize(&options.root)
        .map_err(|error| format!("could not resolve {}: {error}", options.root.display()))?;
    let output_root = options
        .output_root
        .as_deref()
        .map(absolute_cli_path)
        .unwrap_or_else(|| root.clone());
    let output_name = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let manifest_path = output_root.join(&output_name).join("manifest.json");
    let detect_options = DetectOptions {
        scan_filesystem: options.scan_filesystem,
        gitignore: options.gitignore,
        extra_excludes: options.extra_excludes.clone(),
        output_name,
        ..DetectOptions::default()
    };
    let incremental = Manifest::incremental(
        &root,
        &manifest_path,
        &detect_options,
        ManifestKind::Semantic,
    )
    .map_err(|error| error.to_string())?;
    let live_semantic = semantic_files(&incremental.detection.files);
    let semantic_files = if options.force || deep_mode {
        live_semantic.clone()
    } else {
        semantic_files(&incremental.new_files)
    };
    let mut notes = Vec::new();
    if deep_mode {
        notes.push(format!(
            "[trail graph extract] deep mode: {} live semantic file(s)",
            semantic_files.len()
        ));
    }

    let mut environment = std::env::vars().collect::<HashMap<_, _>>();
    if let Some(timeout) = api_timeout {
        environment.insert("GRAPHIFY_API_TIMEOUT".to_owned(), timeout.to_string());
    }
    let mut extraction_options = CorpusExtractionOptions::default();
    if let Some(token_budget) = token_budget {
        extraction_options.token_budget = Some(token_budget);
    }
    if let Some(max_concurrency) = max_concurrency {
        extraction_options.max_concurrency = max_concurrency;
    }
    let cached_options = CachedCorpusExtractionOptions {
        extraction: extraction_options,
        deep_mode,
        force: options.force,
        cache_enabled: true,
        prune_live_files: Some(live_semantic),
    };
    let cache_root = (output_root != root).then_some(output_root.as_path());

    let extracted = if semantic_files.is_empty() {
        None
    } else {
        let global_providers = home_directory()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".graphify")
            .join("providers.json");
        let local_providers = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".graphify")
            .join("providers.json");
        let custom = load_custom_providers(
            &global_providers,
            &local_providers,
            environment_truthy_from(&environment, "GRAPHIFY_ALLOW_LOCAL_PROVIDERS"),
        );
        notes.extend(
            custom
                .warnings
                .iter()
                .map(|warning| format!("[trail graph extract] warning: {warning}")),
        );
        let selected = requested_backend
            .map(str::to_owned)
            .or_else(|| {
                detect_backend_with_custom(&custom.providers, &environment).map(str::to_owned)
            })
            .ok_or_else(|| {
                format!(
                    "no LLM API key found ({} doc/paper/image file(s) need semantic extraction). Set GEMINI_API_KEY or GOOGLE_API_KEY, MOONSHOT_API_KEY, ANTHROPIC_API_KEY, OPENAI_API_KEY, or DEEPSEEK_API_KEY; pass --backend; or use --code-only",
                    semantic_files.len()
                )
            })?;
        let mut completed_chunks = 0_usize;
        let mut progress = |index: usize,
                            total: usize,
                            _units: &[trail_semantic::SemanticUnit],
                            _fragment: &serde_json::Value| {
            completed_chunks = completed_chunks.saturating_add(1);
            notes.push(format!(
                "[trail graph extract] chunk {}/{} done",
                index + 1,
                total
            ));
        };
        let result = if let Some(backend) = trail_semantic::builtin_backend(&selected) {
            let resolved = resolve_builtin_backend(&selected, &environment, requested_model)
                .map_err(|error| error.to_string())?;
            if !backend.api_key_variables.is_empty() && resolved.api_key().is_none() {
                return Err(format!(
                    "backend '{selected}' requires {} to be set",
                    backend.api_key_variables.join(" or ")
                ));
            }
            if selected == "bedrock"
                && !["AWS_PROFILE", "AWS_REGION", "AWS_DEFAULT_REGION", "AWS_ACCESS_KEY_ID"]
                    .into_iter()
                    .any(|key| environment.get(key).is_some_and(|value| !value.is_empty()))
            {
                return Err(
                    "backend 'bedrock' requires AWS credentials or region configuration"
                        .to_owned(),
                );
            }
            if selected == "claude-cli" && !executable_on_path("claude") {
                return Err(
                    "backend 'claude-cli' requires the `claude` CLI on PATH (install Claude Code and authenticate once)"
                        .to_owned(),
                );
            }
            extract_builtin_corpus_cached(
                &semantic_files,
                &resolved,
                &root,
                cache_root,
                &cached_options,
                &environment,
                &mut progress,
            )
        } else if let Some(config) = custom.providers.get(&selected) {
            let resolved = resolve_custom_backend(
                &selected,
                config,
                &environment,
                requested_model,
                None,
            )
            .map_err(|error| error.to_string())?;
            extract_custom_corpus_cached(
                &semantic_files,
                &resolved,
                &root,
                cache_root,
                &cached_options,
                &environment,
                &mut progress,
            )
        } else {
            let mut available = trail_semantic::BUILTIN_BACKENDS
                .iter()
                .map(|backend| backend.name.to_owned())
                .chain(custom.providers.keys().cloned())
                .collect::<Vec<_>>();
            available.sort();
            return Err(format!(
                "unknown backend '{selected}'. Available: {}",
                available.join(", ")
            ));
        }
        .map_err(|error| error.to_string())?;
        if result.cache_misses > 0 && completed_chunks == 0 {
            return Err(format!(
                "all semantic chunks failed for backend '{selected}' ({} uncached file(s))",
                result.cache_misses
            ));
        }
        notes.push(format!(
            "[trail graph extract] semantic cache: {} hit / {} miss",
            result.cache_hits, result.cache_misses
        ));
        notes.extend(
            result
                .provider_warnings
                .iter()
                .map(|warning| format!("[trail graph extract] provider warning: {warning}")),
        );
        notes.extend(
            result
                .cache_issues
                .iter()
                .map(|issue| format!("[trail graph extract] cache warning: {}", issue.message)),
        );
        notes.extend(result.failures.iter().map(|failure| {
            format!(
                "[trail graph extract] chunk {} failed: {}",
                failure.index + 1,
                failure.message
            )
        }));
        Some(result)
    };

    let layer = SemanticLayer {
        fragment: extracted.as_ref().map_or_else(
            || {
                serde_json::json!({
                    "nodes": [],
                    "edges": [],
                    "hyperedges": [],
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "failed_chunks": 0,
                })
            },
            |result| result.fragment.clone(),
        ),
        refreshed_files: semantic_files,
        partial_files: extracted
            .as_ref()
            .map(|result| result.partial_files.clone())
            .unwrap_or_default(),
        allow_partial,
    };
    let semantic_elapsed = semantic_started.elapsed();
    let result = build_graph_with_optional_tiebreaker(
        options,
        Some(&layer),
        auxiliary_fragments,
        tiebreaker,
    )
    .map_err(|error| error.to_string())?;
    Ok((result, notes, semantic_elapsed))
}

fn build_graph_with_optional_tiebreaker(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[serde_json::Value],
    tiebreaker: Option<&mut dyn trail_graph::EntityTiebreaker>,
) -> Result<BuildResult, trail_core::CoreError> {
    match tiebreaker {
        Some(tiebreaker) => {
            build_graph_with_layers_and_tiebreaker(options, semantic, supplemental, tiebreaker)
        }
        None => build_graph_with_layers(options, semantic, supplemental),
    }
}

fn semantic_files(files: &std::collections::BTreeMap<String, Vec<String>>) -> Vec<PathBuf> {
    ["document", "paper", "image"]
        .into_iter()
        .filter_map(|kind| files.get(kind))
        .flatten()
        .map(PathBuf::from)
        .collect()
}

fn parse_positive_usize(value: &str, option: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("error: {option} must be a positive integer (got {value:?})"))
}

fn parse_positive_f64(value: &str, option: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| format!("error: {option} must be a positive number (got {value:?})"))
}

fn environment_truthy(key: &str) -> bool {
    std::env::var(key).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        )
    })
}

fn environment_truthy_from(environment: &HashMap<String, String>, key: &str) -> bool {
    environment.get(key).is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        )
    })
}

fn absolute_cli_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
    }
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn executable_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';')
            .map(str::to_owned)
            .collect::<Vec<_>>()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path).any(|directory| {
        extensions.iter().any(|extension| {
            directory
                .join(format!("{name}{extension}"))
                .metadata()
                .is_ok_and(|metadata| metadata.is_file())
        })
    })
}

fn extract_help() -> String {
    "Usage: trail graph extract [PATH] [--code-only] [--cargo] [--google-workspace] [--postgres DSN] [--backend NAME] [--model MODEL] [--mode deep] [--token-budget N] [--max-concurrency N] [--max-workers N] [--api-timeout SECONDS] [--allow-partial] [--dedup-llm] [--timing] [--out DIR] [--no-cluster] [--force] [--no-viz] [--no-gitignore] [--exclude PATTERN] [--resolution N] [--exclude-hubs N]".to_owned()
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
        "html" | "callflow-html" | "obsidian" | "wiki" | "svg" | "graphml" | "neo4j" | "falkordb"
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
    let mut push_uri = None;
    let mut push_user = "neo4j".to_owned();
    let mut push_password = if format == "falkordb" {
        std::env::var("FALKORDB_PASSWORD").ok()
    } else {
        std::env::var("NEO4J_PASSWORD").ok()
    }
    .filter(|value| !value.is_empty());
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
            "--push" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --push requires a URI".to_owned());
                };
                push_uri = Some(value);
                index += 2;
            }
            "--user" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --user requires a value".to_owned());
                };
                push_user = value;
                index += 2;
            }
            "--password" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --password requires a value".to_owned());
                };
                push_password = Some(value);
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
        "neo4j" => export_neo4j(
            &inputs,
            output_dir,
            push_uri.as_deref(),
            &push_user,
            push_password.as_deref(),
        ),
        "falkordb" => export_falkordb(
            &inputs,
            output_dir,
            push_uri.as_deref(),
            &push_user,
            push_password.as_deref(),
        ),
        _ => Err("unsupported export format".to_owned()),
    };
    match result {
        Ok(output) => Outcome::success(output),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn export_neo4j(
    inputs: &ExportInputs,
    output_dir: &Path,
    push_uri: Option<&str>,
    user: &str,
    password: Option<&str>,
) -> Result<String, String> {
    if let Some(uri) = push_uri {
        let password = password.ok_or_else(|| "--password required for --push".to_owned())?;
        let result = push_to_neo4j(
            &inputs.document,
            uri,
            user,
            password,
            Some(&inputs.communities),
        )
        .map_err(|error| error.to_string())?;
        Ok(format!(
            "Pushed to Neo4j: {} nodes, {} edges",
            result.nodes, result.edges
        ))
    } else {
        let path = output_dir.join("cypher.txt");
        write_cypher(&inputs.document, &path).map_err(|error| error.to_string())?;
        Ok(format!(
            "cypher.txt written - import with: cypher-shell < {}",
            path.display()
        ))
    }
}

fn export_falkordb(
    inputs: &ExportInputs,
    output_dir: &Path,
    push_uri: Option<&str>,
    user: &str,
    password: Option<&str>,
) -> Result<String, String> {
    if let Some(uri) = push_uri {
        let result = push_to_falkordb(
            &inputs.document,
            uri,
            Some(user),
            password,
            Some(&inputs.communities),
            "graphify",
        )
        .map_err(|error| error.to_string())?;
        Ok(format!(
            "Pushed to FalkorDB: {} nodes, {} edges",
            result.nodes, result.edges
        ))
    } else {
        let path = output_dir.join("cypher.txt");
        write_cypher(&inputs.document, &path).map_err(|error| error.to_string())?;
        Ok(format!(
            "cypher.txt written ({}) - statements are OpenCypher. FalkorDB's GRAPH.QUERY runs one statement at a time (no bulk script import), so load a graph with: graphify export falkordb --push falkordb://localhost:6379",
            path.display()
        ))
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
    "Usage: graphify export <format>\n  html      [--graph PATH] [--labels PATH] [--node-limit N] [--no-viz]\n  callflow-html [GRAPH|DIR] [--graph PATH] [--labels PATH] [--report PATH] [--sections PATH] [--output HTML]\n  obsidian  [--graph PATH] [--labels PATH] [--dir PATH]\n  wiki      [--graph PATH] [--labels PATH]\n  svg       [--graph PATH] [--labels PATH]\n  graphml   [--graph PATH]\n  neo4j     [--graph PATH] [--push URI] [--user U] [--password P]\n  falkordb  [--graph PATH] [--push URI] [--user U] [--password P]".to_owned()
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
    let output = query_graph_text(
        &loaded.graph,
        question,
        mode,
        2,
        budget,
        &contexts,
        &loaded.overlay,
    );
    integration_commands::touch_query_stamp(&graph_path);
    Outcome::success(output)
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
        Ok(output) => {
            integration_commands::touch_query_stamp(&graph_path);
            Outcome::success(output)
        }
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
    let output = render_explanation(&loaded.graph, label, &loaded.overlay);
    integration_commands::touch_query_stamp(&graph_path);
    Outcome::success(output)
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
    "Usage: trail graph <command>\n\nCommands:\n  update\n  extract\n  watch\n  serve\n  cluster-only\n  label\n  query\n  path\n  explain\n  affected\n  tree\n  export\n  benchmark\n  diagnose multigraph\n  merge-graphs\n  merge-driver\n  global\n  clone\n  add\n  prs\n  hook\n  install\n  uninstall\n  cache-check\n  merge-chunks\n  merge-semantic\n  provider\n  save-result\n  reflect\n  check-update\n  hook-check\n  hook-guard"
        .to_owned()
}

fn trail_command_help(command: &str) -> String {
    match command {
        "update" => "Usage: trail graph update [PATH] [--out DIR] [--no-cluster] [--force] [--no-viz] [--no-gitignore] [--exclude PATTERN] [--resolution N] [--exclude-hubs N]".to_owned(),
        "extract" => extract_help(),
        "watch" => watch_help(),
        "serve" => mcp_help(McpFrontend::Trail),
        "cluster-only" => "Usage: trail graph cluster-only [PATH] [--graph PATH] [--no-viz] [--no-label] [--resolution N] [--exclude-hubs N] [--min-community-size=N]".to_owned(),
        "label" => label_commands::label_help(Frontend::Trail),
        "prs" => prs_commands::prs_help(Frontend::Trail),
        "query" => "Usage: trail graph query \"<question>\" [--dfs] [--context VALUE] [--budget N] [--graph PATH]".to_owned(),
        "path" => "Usage: trail graph path \"<source>\" \"<target>\" [--graph PATH]".to_owned(),
        "explain" => "Usage: trail graph explain \"<node>\" [--graph PATH]".to_owned(),
        "affected" => "Usage: trail graph affected \"<node-or-label>\" [--relation R] [--depth N] [--graph PATH]".to_owned(),
        "tree" => tree_help(Frontend::Trail),
        "export" => export_help().replacen("graphify export", "trail graph export", 1),
        "benchmark" => "Usage: trail graph benchmark [GRAPH_JSON]".to_owned(),
        "diagnose" => "Usage: trail graph diagnose multigraph [--graph PATH] [--json] [--max-examples N] [--directed|--undirected] [--extract-path PATH]".to_owned(),
        "merge-graphs" => "Usage: trail graph merge-graphs <graph1.json> <graph2.json> [...] [--out merged.json]".to_owned(),
        "cache-check" => semantic_commands::cache_check_help(Frontend::Trail),
        "merge-chunks" => semantic_commands::merge_chunks_help(Frontend::Trail),
        "merge-semantic" => semantic_commands::merge_semantic_help(Frontend::Trail),
        "provider" => provider_commands::provider_help(Frontend::Trail),
        "save-result" => result_commands::save_result_help(Frontend::Trail),
        "reflect" => result_commands::reflect_help(Frontend::Trail),
        "check-update" => integration_commands::check_update_help(Frontend::Trail),
        "merge-driver" => integration_commands::merge_driver_help(Frontend::Trail),
        "global" => integration_commands::global_help(Frontend::Trail),
        "clone" => integration_commands::clone_help(Frontend::Trail),
        "add" => ingest_commands::add_help(Frontend::Trail),
        "hook" => hook_commands::hook_help(Frontend::Trail),
        "install" => install_commands::command_install(Frontend::Trail, &["--help".to_owned()]).stdout,
        "uninstall" => "Usage: trail graph uninstall [--project] [--purge] [--platform P|P]".to_owned(),
        _ => trail_help(),
    }
}

fn watch_help() -> String {
    "Usage: trail graph watch [PATH] [--debounce SECONDS] [--out DIR] [--no-cluster] [--no-viz] [--no-gitignore] [--exclude PATTERN] [--poll]"
        .to_owned()
}

fn graphify_help() -> String {
    "Usage: graphify <command>\n\nPorted commands:\n  install\n  uninstall\n  update\n  watch\n  cluster-only\n  query\n  path\n  explain\n  affected\n  tree\n  export\n  benchmark\n  diagnose multigraph\n  merge-graphs\n  merge-driver\n  global\n  clone\n  add\n  label\n  prs\n  hook\n  cache-check\n  merge-chunks\n  merge-semantic\n  provider\n  save-result\n  reflect\n  check-update\n  hook-check\n  hook-guard"
        .to_owned()
}

#[cfg(test)]
mod mcp_option_tests {
    use super::*;

    #[test]
    fn argparse_style_equals_options_are_supported() -> Result<(), String> {
        let args = [
            "--graph=custom.json",
            "--transport=http",
            "--host=0.0.0.0",
            "--port=9000",
            "--api-key=secret",
            "--path=/graph",
            "--session-timeout=12.5",
            "--json-response",
            "--stateless",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let options = parse_mcp_options(McpFrontend::Graphify, &args)?
            .ok_or_else(|| "options unexpectedly returned help".to_owned())?;
        assert_eq!(options.graph_path, PathBuf::from("custom.json"));
        assert_eq!(options.transport, "http");
        assert_eq!(options.host, "0.0.0.0");
        assert_eq!(options.port, 9000);
        assert_eq!(options.api_key.as_deref(), Some("secret"));
        assert_eq!(options.path, "/graph");
        assert_eq!(options.session_timeout, Some(Duration::from_secs_f64(12.5)));
        assert!(options.json_response);
        assert!(options.stateless);
        Ok(())
    }

    #[test]
    fn graph_flag_overrides_positional_like_python_argparse() -> Result<(), String> {
        let args = ["positional.json", "--graph=flag.json"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let options = parse_mcp_options(McpFrontend::Graphify, &args)?
            .ok_or_else(|| "options unexpectedly returned help".to_owned())?;
        assert_eq!(options.graph_path, PathBuf::from("flag.json"));
        Ok(())
    }

    #[test]
    fn oversized_session_timeout_is_an_error_not_a_panic() {
        assert_eq!(
            parse_session_timeout("1e300"),
            Err("error: --session-timeout is out of range".to_owned())
        );
    }

    #[test]
    fn graphify_version_reports_the_compatibility_baseline() {
        let outcome = run(Frontend::Graphify, [OsString::from("--version")]);
        assert_eq!(outcome.code, 0);
        assert_eq!(outcome.stdout, "graphify 0.9.20");
    }

    #[test]
    fn graphify_unknown_command_matches_the_legacy_diagnostic() {
        let outcome = run(Frontend::Graphify, [OsString::from("not-a-command")]);
        assert_eq!(outcome.code, 1);
        assert_eq!(
            outcome.stderr,
            "error: unknown command 'not-a-command'\nRun 'graphify --help' for usage."
        );
    }
}
