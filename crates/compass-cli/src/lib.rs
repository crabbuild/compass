//! Command implementation for the native Compass CLI.

mod call_graph_commands;
mod capability_commands;
mod code_query_commands;
mod dedup_commands;
mod help;
mod history_batch;
mod history_build;
mod history_commands;
mod hook_commands;
pub mod ide_contract;
mod ingest_commands;
mod init_commands;
mod install_commands;
mod integration_commands;
mod label_commands;
mod program_commands;
mod provider_commands;
mod prs_commands;
mod query_commands;
mod result_commands;
mod review_commands;
mod semantic_commands;
mod semantic_diff_commands;
mod semantic_diff_render;
mod store_commands;
mod task_context_commands;
mod upgrade_commands;

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use compass_analysis::{
    CallGraphDirection, UniversalCallGraphRequest, UniversalCallGraphRoot,
    build_universal_call_graph,
};
use compass_core::{
    BuildFileProgress, BuildOptions, BuildPurpose, BuildResult, BuildTimings,
    ClusterExistingOptions, ExportInputs, GraphStorage, InferenceLevel, LoadedGraph, SemanticLayer,
    WatchBackend, WatchBuildReason, WatchOptions, WatchStatus, build_graph_with_layers,
    build_graph_with_layers_and_progress, build_graph_with_layers_and_tiebreaker,
    cluster_existing_graph, default_graph_path, diagnose_graph_file, diagnose_graph_quality,
    format_diagnostic_json, format_diagnostic_report, format_quality_json, format_quality_report,
    merge_graphs, watch_local_graph,
};
use compass_files::{
    BuildScope, DetectOptions, Detection, Manifest, ManifestKind, ProjectConfig, detect,
};
use compass_global::{GlobalPaths, global_add};
use compass_graph::god_nodes;
use compass_graphdb::{push_to_falkordb, push_to_neo4j};
use compass_model::GraphError;
use compass_model::query_contract::{
    CodeQueryLimits, DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest,
    DiscoveryQueryResponse, DiscoveryScope, DiscoveryScopeKind, DiscoveryTraversal, ImpactRequest,
};
use compass_output::{
    AffectedLensOptions, AgentOrientation, ArtifactLens, CallflowOptions, CallflowSection,
    CanvasOptions, HtmlOptions, ObsidianOptions, SourceNavigation, SvgOptions, TreeOptions,
    WikiOptions, WorkbenchCoverage, WorkbenchCoverageStatus, WorkbenchModel, WorkbenchView,
    WorkbenchViewContent, affected_lens_view_model, artifact_lens_view_model, callflow_view_model,
    export_obsidian, export_wiki, graph_artifact_identity, graph_community_view_model_document,
    graph_view_model_bundle_document, graph_view_model_document, node_filenames,
    render_orientation_json, validate_orientation_graph_identity, write_callflow_html,
    write_canvas, write_cypher, write_graphml, write_svg, write_tree_html,
    write_workbench_html_with_source_navigation,
};
use compass_prs::{ProcessRunner, SystemRunner};
use compass_query::{
    DEFAULT_AFFECTED_RELATIONS, DEFAULT_TEXT_TOKEN_BUDGET, DiscoveryTextPageOptions,
    TextPageOptions, TraversalMode, discovery_request_digest, format_affected, format_benchmark,
    open as open_code_query, open_with_verified_document, query_graph_text_page,
    render_discovery_text_page, render_explanation_page, render_shortest_path, run_benchmark,
};
use compass_semantic::{
    CachedCorpusExtractionOptions, CorpusExtractionOptions, detect_backend_with_custom,
    extract_builtin_corpus_cached, extract_custom_corpus_cached, load_custom_providers,
    resolve_builtin_backend, resolve_custom_backend,
};

pub use help::HelpStyle;
pub use init_commands::{run_init, run_init_jsonl};

static PROCESS_CANCELLED: AtomicBool = AtomicBool::new(false);
static SIGNAL_HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

pub(crate) fn process_cancellation() -> Result<&'static AtomicBool, String> {
    let installed = SIGNAL_HANDLER.get_or_init(|| {
        ctrlc::set_handler(|| PROCESS_CANCELLED.store(true, Ordering::Release))
            .map_err(|error| error.to_string())
    });
    installed.as_ref().map_err(Clone::clone)?;
    PROCESS_CANCELLED.store(false, Ordering::Release);
    Ok(&PROCESS_CANCELLED)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Frontend {
    Compass,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildOperation {
    Init,
    Extract,
    Update,
}

impl BuildOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Extract => "extract",
            Self::Update => "update",
        }
    }

    fn extracts_semantics(self) -> bool {
        self == Self::Extract
    }
}

#[derive(Debug)]
pub struct Outcome {
    pub code: u8,
    pub stdout: String,
    pub stderr: String,
    pub stdout_trailing_newline: bool,
    pub stderr_trailing_newline: bool,
    html_output: Option<PathBuf>,
}

pub(crate) fn resolve_output_artifact(output: &Path, name: &str) -> Result<PathBuf, String> {
    compass_files::BuildGuard::resolve_artifact(output, name).map_err(|error| error.to_string())
}

impl Outcome {
    #[must_use]
    pub fn from_command_output(code: u8, stdout: String, stderr: String) -> Self {
        Self {
            code,
            stdout,
            stderr,
            stdout_trailing_newline: true,
            stderr_trailing_newline: true,
            html_output: None,
        }
    }

    fn success(stdout: String) -> Self {
        Self {
            code: 0,
            stdout,
            stderr: String::new(),
            stdout_trailing_newline: true,
            stderr_trailing_newline: true,
            html_output: None,
        }
    }

    fn success_exact(stdout: String) -> Self {
        Self {
            code: 0,
            stdout,
            stderr: String::new(),
            stdout_trailing_newline: false,
            stderr_trailing_newline: true,
            html_output: None,
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
            html_output: None,
        }
    }

    fn with_html_output(mut self, path: impl Into<PathBuf>) -> Self {
        self.html_output = Some(absolutize(path.into()));
        self
    }

    #[must_use]
    pub fn html_output(&self) -> Option<&Path> {
        self.html_output.as_deref()
    }
}

/// Write a completed command outcome without losing short writes or output failures.
///
/// Returns the command's exit code when both streams are written successfully, or `1` when
/// either stream fails. A stdout failure is reported to stderr when that stream remains usable.
pub fn write_outcome(outcome: &Outcome, stdout: &mut impl Write, stderr: &mut impl Write) -> u8 {
    if let Err(error) = write_output(stdout, &outcome.stdout, outcome.stdout_trailing_newline) {
        let _diagnostic = writeln!(stderr, "error: failed to write stdout: {error}");
        return 1;
    }
    if write_output(stderr, &outcome.stderr, outcome.stderr_trailing_newline).is_err() {
        return 1;
    }
    outcome.code
}

/// Ask before opening a successfully generated HTML page.
///
/// The prompt is deliberately disabled unless both input and prompt output are terminals, so
/// scripts, CI jobs, pipes, and redirected commands never block or launch a browser.
pub fn prompt_to_open_html(
    outcome: &Outcome,
    input: &mut impl BufRead,
    prompt_output: &mut impl Write,
    input_is_terminal: bool,
    prompt_is_terminal: bool,
) -> Result<bool, String> {
    prompt_to_open_html_with(
        outcome,
        input,
        prompt_output,
        input_is_terminal,
        prompt_is_terminal,
        open_html,
    )
}

fn prompt_to_open_html_with(
    outcome: &Outcome,
    input: &mut impl BufRead,
    prompt_output: &mut impl Write,
    input_is_terminal: bool,
    prompt_is_terminal: bool,
    mut opener: impl FnMut(&Path) -> Result<(), String>,
) -> Result<bool, String> {
    let Some(path) = outcome.html_output() else {
        return Ok(false);
    };
    if outcome.code != 0 || !input_is_terminal || !prompt_is_terminal {
        return Ok(false);
    }
    if !path.is_file() {
        return Err(format!(
            "generated HTML page no longer exists: {}",
            path.display()
        ));
    }

    write!(
        prompt_output,
        "Open {} in your browser? [y/N] ",
        path.display()
    )
    .map_err(|error| format!("could not write browser prompt: {error}"))?;
    prompt_output
        .flush()
        .map_err(|error| format!("could not flush browser prompt: {error}"))?;

    let mut answer = String::new();
    input
        .read_line(&mut answer)
        .map_err(|error| format!("could not read browser confirmation: {error}"))?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Ok(false);
    }

    opener(path)?;
    Ok(true)
}

fn open_html(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err("opening a browser is not supported on this platform".to_owned());

    command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = command
        .status()
        .map_err(|error| format!("could not launch the default browser: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the browser launcher exited with status {status}"))
    }
}

fn write_output<W: Write + ?Sized>(
    stream: &mut W,
    output: &str,
    trailing_newline: bool,
) -> std::io::Result<()> {
    if output.is_empty() {
        return Ok(());
    }
    stream.write_all(output.as_bytes())?;
    if trailing_newline {
        stream.write_all(b"\n")?;
    }
    Ok(())
}

#[must_use]
pub fn run(frontend: Frontend, arguments: impl IntoIterator<Item = OsString>) -> Outcome {
    let mut args = arguments
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let mut os_args = args.iter().map(OsString::from).collect::<Vec<_>>();
    let events = match ide_contract::take_jsonl_events(&mut os_args) {
        Ok(enabled) => {
            args = os_args
                .into_iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            enabled
        }
        Err(error) => return Outcome::failure_with_code(error, 2),
    };
    if let Some(outcome) = help::request(&args, HelpStyle::Plain) {
        return outcome;
    }
    if let Some("--version" | "-V") = args.first().map(String::as_str) {
        return Outcome::success(format!("compass {}", env!("CARGO_PKG_VERSION")));
    }
    let Some(command) = args.first().cloned() else {
        return Outcome::failure("error: missing command".to_owned());
    };
    args.remove(0);
    let operation = if command == "history" {
        args.first().map_or_else(
            || command.clone(),
            |subcommand| format!("{command}_{subcommand}"),
        )
    } else {
        command.clone()
    };
    let outcome = match command.as_str() {
        "history" => history_commands::command(frontend, &args),
        "call-graph" => call_graph_commands::command(frontend, &args),
        "capabilities" => capability_commands::command(frontend, &args),
        "ask" => code_query_commands::command("ask", &args),
        "search" => code_query_commands::command("search", &args),
        "callers" => code_query_commands::command("callers", &args),
        "callees" => code_query_commands::command("callees", &args),
        "impact" => code_query_commands::command("impact", &args),
        "explore" => code_query_commands::command("explore", &args),
        "node" => code_query_commands::command("node", &args),
        "context" => task_context_commands::command(&args),
        "history-worker" => history_commands::command_worker(frontend, &args),
        "diff" => semantic_diff_commands::command(frontend, &args),
        "query" => query_commands::command_query(frontend, &args),
        "program" => program_commands::command(frontend, &args),
        "path" => command_path(frontend, &args),
        "explain" => command_explain(frontend, &args),
        "affected" => command_affected(&args),
        "export" => command_export(frontend, &args),
        "benchmark" => command_benchmark(&args),
        "merge-graphs" => command_merge_graphs(&args),
        "cache-check" => semantic_commands::command_cache_check(frontend, &args),
        "merge-chunks" => semantic_commands::command_merge_chunks(frontend, &args),
        "merge-semantic" => semantic_commands::command_merge_semantic(frontend, &args),
        "provider" => provider_commands::command_provider(frontend, &args),
        "store" => store_commands::command(&args),
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
        "review" => review_commands::command(&args),
        "hook" => hook_commands::command_hook(frontend, &args),
        "hook-spawn" => hook_commands::command_hook_spawn(frontend, &args),
        "hook-refresh" => command_hook_refresh(frontend, &args),
        "install" => install_commands::command_install(frontend, &args),
        "uninstall" => install_commands::command_uninstall(frontend, &args),
        "upgrade" => upgrade_commands::command_upgrade(&args),
        platform if install_commands::is_direct_command(platform) => {
            install_commands::command_platform(frontend, platform, &args)
        }
        "tree" => command_tree(frontend, &args),
        "cluster-only" => command_cluster_only(frontend, &args),
        "diagnose" => command_diagnose(frontend, &args),
        "update" => command_build(frontend, &args, BuildOperation::Update),
        "extract" => command_build(frontend, &args, BuildOperation::Extract),
        "init" => Outcome::failure(
            "error: init requires terminal input and must be run from the compass binary"
                .to_owned(),
        ),
        "watch" => Outcome::failure(
            "error: watch is a streaming command and must be run from the compass binary"
                .to_owned(),
        ),
        "serve" => Outcome::failure(
            "error: serve is a long-lived command and must be run from the compass binary"
                .to_owned(),
        ),
        "--version" | "-V" | "-v" | "version" => {
            Outcome::success(format!("compass {}", env!("CARGO_PKG_VERSION")))
        }
        _ => Outcome::failure(help::unknown_command(&command)),
    };
    let outcome = help::append_usage_hint(outcome, &command, &args);
    if events {
        ide_contract::progress_outcome(&operation, outcome)
    } else {
        outcome
    }
}

pub fn run_watch_jsonl(
    arguments: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    let operation_id = format!("watch-{}", std::process::id());
    let mut writer = ide_contract::ProgressWriter::new(stdout);
    if writer
        .write(&ide_contract::ProgressEvent {
            schema: ide_contract::PROGRESS_SCHEMA,
            operation_id: &operation_id,
            operation: "watch",
            state: ide_contract::ProgressState::Started,
            phase: "watching",
            current: None,
            total: None,
            message: "Compass watch started",
            terminal: false,
        })
        .is_err()
    {
        return 1;
    }
    let code = run_watch_with_frontend(
        Frontend::Compass,
        arguments,
        &mut std::io::sink(),
        stderr,
        false,
    );
    let cancelled = PROCESS_CANCELLED.load(Ordering::Acquire);
    let terminal = ide_contract::ProgressEvent {
        schema: ide_contract::PROGRESS_SCHEMA,
        operation_id: &operation_id,
        operation: "watch",
        state: if cancelled {
            ide_contract::ProgressState::Cancelled
        } else if code == 0 {
            ide_contract::ProgressState::Succeeded
        } else {
            ide_contract::ProgressState::Failed
        },
        phase: if cancelled {
            "cancelled"
        } else if code == 0 {
            "complete"
        } else {
            "failed"
        },
        current: None,
        total: None,
        message: if cancelled {
            "Compass watch stopped"
        } else if code == 0 {
            "Compass watch completed"
        } else {
            "Compass watch failed"
        },
        terminal: true,
    };
    if writer.write(&terminal).is_err() {
        1
    } else {
        code
    }
}

#[must_use]
pub fn compass_help_request(arguments: &[OsString], style: HelpStyle) -> Option<Outcome> {
    help::request_os(arguments, style)
}

/// Parse and run the long-lived native MCP server.
pub fn run_mcp(arguments: &[OsString], stdout: &mut impl Write, stderr: &mut impl Write) -> u8 {
    let args = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let options = match parse_mcp_options(&args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            let _result = writeln!(stdout, "{}", mcp_help());
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
        runtime.block_on(compass_mcp::serve_http(compass_mcp::HttpOptions {
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
        runtime.block_on(compass_mcp::serve_stdio(options.graph_path))
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

fn parse_mcp_options(args: &[String]) -> Result<Option<McpOptions>, String> {
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
    let mut api_key = std::env::var("COMPASS_API_KEY").ok();
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
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned()))
                .join("graph.json")
        });
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

fn mcp_help() -> String {
    "Usage: compass serve [GRAPH_PATH] [--graph PATH] [--transport stdio|http] [--host HOST] [--port PORT] [--api-key KEY] [--path PATH] [--json-response] [--stateless] [--session-timeout SECONDS]".to_owned()
}

/// Run Compass's long-lived native watcher, streaming status as changes arrive.
///
/// Signal registration lives at this process boundary rather than in
/// `compass-core`, so embedders can provide their own cancellation mechanism.
pub fn run_watch(arguments: &[OsString], stdout: &mut impl Write, stderr: &mut impl Write) -> u8 {
    run_watch_with_frontend(Frontend::Compass, arguments, stdout, stderr, true)
}

/// Run Compass's watcher with terminal-aware status rendering.
pub fn run_watch_with_terminal(
    arguments: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    output_is_terminal: bool,
) -> u8 {
    run_watch_with_frontend(
        Frontend::Compass,
        arguments,
        stdout,
        stderr,
        output_is_terminal,
    )
}

fn run_watch_with_frontend(
    frontend: Frontend,
    arguments: &[OsString],
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    output_is_terminal: bool,
) -> u8 {
    let args = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let options = match parse_watch_options(&args) {
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
    let stop = match process_cancellation() {
        Ok(stop) => stop,
        Err(error) => {
            let _result = writeln!(stderr, "error: could not install Ctrl+C handler: {error}");
            return 1;
        }
    };
    let result = watch_local_graph(&options, stop, |status| {
        write_watch_status_mode(frontend, status, stdout, stderr, output_is_terminal);
    });
    match result {
        Ok(()) => 0,
        Err(error) => {
            let _result = writeln!(stderr, "error: {error}");
            1
        }
    }
}

#[cfg(test)]
fn write_watch_status(status: WatchStatus, stdout: &mut impl Write, stderr: &mut impl Write) {
    write_watch_status_mode(Frontend::Compass, status, stdout, stderr, true);
}

fn write_watch_status_mode(
    frontend: Frontend,
    status: WatchStatus,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    output_is_terminal: bool,
) {
    let timestamp = (!output_is_terminal && frontend == Frontend::Compass)
        .then(watch_timestamp)
        .flatten();
    macro_rules! native_line {
        ($writer:expr, $($argument:tt)*) => {
            if let Some(timestamp) = timestamp.as_deref() {
                writeln!($writer, "[{timestamp}] {}", format_args!($($argument)*))
            } else {
                writeln!($writer, $($argument)*)
            }
        };
    }
    match status {
        WatchStatus::Starting {
            root,
            includes,
            excludes,
            output,
        } => {
            if frontend == Frontend::Compass {
                let scope = if includes == 0 {
                    format!("all eligible files, {excludes} exclude")
                } else {
                    format!("{includes} include, {excludes} exclude")
                };
                let _result = native_line!(
                    stdout,
                    "[compass watch] Starting {} (scope: {scope}; output: {})",
                    root.display(),
                    output.display()
                );
                let _result = stdout.flush();
            }
        }
        WatchStatus::Backend {
            backend,
            fallback_error,
            poll_interval,
        } => {
            if frontend == Frontend::Compass {
                match (backend, fallback_error) {
                    (WatchBackend::Native, _) => {
                        let _result = native_line!(
                            stdout,
                            "[compass watch] Native filesystem events active."
                        );
                    }
                    (WatchBackend::Polling, Some(error)) => {
                        let _result = native_line!(
                            stderr,
                            "[compass watch] Native watcher unavailable; polling every {}s: {error}",
                            poll_interval.as_secs_f64()
                        );
                    }
                    (WatchBackend::Polling, None) => {
                        let _result = native_line!(
                            stdout,
                            "[compass watch] Polling every {}s.",
                            poll_interval.as_secs_f64()
                        );
                    }
                }
                let _result = stdout.flush();
                let _result = stderr.flush();
            }
        }
        WatchStatus::Watching { root, debounce } => {
            let _result = native_line!(
                stdout,
                "[compass watch] Watching {} - press Ctrl+C to stop",
                root.display()
            );
            let _result = native_line!(
                stdout,
                "[compass watch] Deterministic changes rebuild locally; semantic media changes set needs_update."
            );
            let _result = native_line!(
                stdout,
                "[compass watch] Adaptive debounce: {}s",
                debounce.as_secs_f64()
            );
            let _result = stdout.flush();
        }
        WatchStatus::Synchronizing => {
            if frontend == Frontend::Compass {
                let _result = native_line!(stdout, "[compass watch] Synchronizing current graph…");
                let _result = stdout.flush();
            }
        }
        WatchStatus::Settling {
            paths,
            quiet_window,
            maximum_window,
        } => {
            if frontend == Frontend::Compass {
                let _result = native_line!(
                    stdout,
                    "[compass watch] {paths} path(s) changed; settling for {}s (maximum {}s)…",
                    quiet_window.as_secs_f64(),
                    maximum_window.as_secs_f64()
                );
                let _result = stdout.flush();
            }
        }
        WatchStatus::Building { reason } => {
            if frontend == Frontend::Compass {
                let _result = native_line!(
                    stdout,
                    "[compass watch] Building ({})…",
                    watch_reason(reason)
                );
                let _result = stdout.flush();
            }
        }
        WatchStatus::Batch {
            paths,
            deterministic,
            semantic,
        } => {
            let _result = native_line!(
                stdout,
                "[compass watch] {} file(s) changed ({deterministic} deterministic, {semantic} semantic)",
                paths.len()
            );
            let _result = stdout.flush();
        }
        WatchStatus::Rebuilt(result) => {
            let _result = native_line!(
                stdout,
                "[compass watch] Rebuilt: {} nodes, {} edges, {} communities ({} extracted, {} cached)",
                result.nodes,
                result.edges,
                result.communities,
                result.files_extracted,
                result.files_cached
            );
            let _result = native_line!(
                stdout,
                "[compass watch] {}",
                format_program_analysis(&result)
            );
            let _result = native_line!(
                stdout,
                "[compass watch] graph artifacts updated in {}",
                compass_files::BuildGuard::output_container_for_artifact(
                    &result.output_dir.join("graph.json")
                )
                .display()
            );
            if result.partial_graph {
                let _result = native_line!(
                    stderr,
                    "[compass watch] warning: partial graph published after omitting {} nodes and {} edges; {} identity collisions quarantined",
                    result.omitted_nodes,
                    result.omitted_edges,
                    result.identity_collisions
                );
            }
            let _result = stdout.flush();
            let _result = stderr.flush();
        }
        WatchStatus::UpToDate { reason } => {
            if frontend == Frontend::Compass {
                let _result = native_line!(
                    stdout,
                    "[compass watch] Up to date after {}.",
                    watch_reason(reason)
                );
                let _result = stdout.flush();
            }
        }
        WatchStatus::FollowUpQueued { paths } => {
            if frontend == Frontend::Compass {
                let _result = native_line!(
                    stdout,
                    "[compass watch] {paths} path(s) arrived during build; one follow-up queued."
                );
                let _result = stdout.flush();
            }
        }
        WatchStatus::RetryScheduled {
            delay,
            error,
            repeated,
        } => {
            if frontend == Frontend::Compass {
                let suffix = if repeated > 1 {
                    format!(" (same failure {repeated} times)")
                } else {
                    String::new()
                };
                let _result = native_line!(
                    stderr,
                    "[compass watch] Build failed; retrying in {}s: {error}{suffix}",
                    delay.as_secs_f64()
                );
                let _result = stderr.flush();
            }
        }
        WatchStatus::SemanticUpdateRequired { flag } => {
            let _result = native_line!(
                stdout,
                "[compass watch] Semantic media changed; update required. Flag written to {}",
                flag.display()
            );
            let _result = stdout.flush();
        }
        WatchStatus::EventError(error) => {
            let _result = native_line!(stderr, "[compass watch] Filesystem event error: {error}");
            let _result = stderr.flush();
        }
        WatchStatus::RebuildError(error) => {
            let _result = native_line!(stderr, "[compass watch] Rebuild failed: {error}");
            let _result = stderr.flush();
        }
        WatchStatus::Finishing => {
            if frontend == Frontend::Compass {
                let _result =
                    native_line!(stdout, "[compass watch] Finishing the active atomic build…");
                let _result = stdout.flush();
            }
        }
        WatchStatus::Stopped => {
            let _result = native_line!(stdout, "[compass watch] Stopped.");
            let _result = stdout.flush();
        }
    }
}

fn watch_reason(reason: WatchBuildReason) -> &'static str {
    match reason {
        WatchBuildReason::Initial => "initial synchronization",
        WatchBuildReason::Changes => "filesystem changes",
        WatchBuildReason::Retry => "retry",
        WatchBuildReason::Reconciliation => "periodic reconciliation",
    }
}

fn watch_timestamp() -> Option<String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

fn parse_watch_options(args: &[String]) -> Result<Option<WatchOptions>, String> {
    let mut root = None;
    let mut output_root = None;
    let mut debounce = Duration::from_millis(150);
    let mut no_cluster = false;
    let mut no_viz = false;
    let mut no_program = false;
    let mut program_requested = false;
    let mut gitignore = true;
    let mut excludes = Vec::new();
    let mut program_artifacts = Vec::new();
    let mut graph_storage = GraphStorage::default();
    let mut inference_level = InferenceLevel::default();
    let mut force_polling = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Ok(None),
            "--no-cluster" => no_cluster = true,
            "--no-viz" => no_viz = true,
            "--no-program" => no_program = true,
            "--program" => program_requested = true,
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
            "--store" if index + 1 < args.len() => {
                graph_storage = parse_graph_storage(&args[index + 1])?;
                index += 1;
            }
            value if value.starts_with("--store=") => {
                graph_storage = parse_graph_storage(&value[8..])?;
            }
            "--store" => return Err("error: --store requires json or sqlite".to_owned()),
            "--inference-level" if index + 1 < args.len() => {
                inference_level = parse_inference_level(&args[index + 1])?;
                index += 1;
            }
            value if value.starts_with("--inference-level=") => {
                inference_level = parse_inference_level(&value[18..])?;
            }
            "--inference-level" => {
                return Err(
                    "error: --inference-level requires low, medium, high, or max".to_owned(),
                );
            }
            "--exclude" if index + 1 < args.len() => {
                excludes.push(args[index + 1].clone());
                index += 1;
            }
            value if value.starts_with("--exclude=") => excludes.push(value[10..].to_owned()),
            "--program-artifact" if index + 1 < args.len() => {
                program_artifacts.push(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            "--program-artifact" => {
                return Err("error: --program-artifact requires a path".to_owned());
            }
            value if value.starts_with("--program-artifact=") => {
                let path = &value[19..];
                if path.is_empty() {
                    return Err("error: --program-artifact requires a path".to_owned());
                }
                program_artifacts.push(PathBuf::from(path));
            }
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
    if no_program && program_requested {
        return Err("error: --program conflicts with --no-program".to_owned());
    }
    if no_program && !program_artifacts.is_empty() {
        return Err("error: --no-program conflicts with --program-artifact".to_owned());
    }
    let mut options = WatchOptions::new(root.unwrap_or_else(|| PathBuf::from(".")));
    options.build.scope = ProjectConfig::load(&options.build.root)
        .map_err(|error| format!("error: {error}"))?
        .map_or_else(BuildScope::default, |config| config.build);
    options.debounce = debounce;
    options.force_polling = force_polling;
    options.build.output_root = output_root;
    options.build.no_cluster = no_cluster;
    options.build.no_viz = no_viz;
    options.build.graph_storage = graph_storage;
    options.build.inference_level = inference_level;
    options.build.gitignore = gitignore;
    options.build.extra_excludes = excludes;
    options.build.program_analysis = program_requested || !program_artifacts.is_empty();
    options.build.program_artifacts = program_artifacts;
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
    let Some(command) = args.first().map(String::as_str) else {
        return Outcome::failure(
            "Usage: compass diagnose <multigraph|quality> [OPTIONS]".to_owned(),
        );
    };
    if command == "quality" {
        let mut graph_path = default_graph_path();
        let mut json_output = false;
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
                value => {
                    return Outcome::failure(format!(
                        "error: unknown diagnose quality option {value}"
                    ));
                }
            }
            index += 1;
        }
        let graph_path = match compass_files::BuildGuard::resolve_requested_artifact(&graph_path) {
            Ok(path) => path,
            Err(error) => return Outcome::failure(format!("error: {error}")),
        };
        return match diagnose_graph_quality(&graph_path) {
            Ok(summary) if json_output => {
                match serde_json::to_string_pretty(&format_quality_json(&summary)) {
                    Ok(output) => Outcome::success(output),
                    Err(error) => Outcome::failure(format!("error: {error}")),
                }
            }
            Ok(summary) => Outcome::success(format_quality_report(&summary)),
            Err(error) => Outcome::failure(format!("error: {error}")),
        };
    }
    if command != "multigraph" {
        return Outcome::failure(
            "Usage: compass diagnose <multigraph|quality> [OPTIONS]".to_owned(),
        );
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
    let _ = frontend;
    let graph_path = match compass_files::BuildGuard::resolve_requested_artifact(&graph_path) {
        Ok(path) => path,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
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

fn command_cluster_only(_frontend: Frontend, args: &[String]) -> Outcome {
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
                    return Outcome::failure("error: --graph requires a value".to_owned());
                };
                graph_override = Some(PathBuf::from(value));
                index += 1;
            }
            "--no-viz" => no_viz = true,
            "--no-label" => no_label = true,
            "--timing" => timing = true,
            "--resolution" => {
                let Some(argument) = args.get(index + 1) else {
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
            value if value.starts_with("--min-community-size=") => {
                let Ok(parsed) = value[21..].parse::<usize>() else {
                    return Outcome::failure(
                        "error: --min-community-size requires an integer".to_owned(),
                    );
                };
                min_community_size = parsed;
            }
            "-h" | "--help" => {
                return Outcome::success("Usage: compass cluster-only [PATH] [--graph PATH] [--no-viz] [--no-label] [--resolution N] [--exclude-hubs N] [--min-community-size=N]".to_owned());
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
            value => {
                return Outcome::failure(format!("error: unexpected path: {value}"));
            }
        }
        index += 1;
    }
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let requested_graph = graph_override
        .clone()
        .unwrap_or_else(|| root.join(&output_name).join("graph.json"));
    let graph_path = match compass_files::BuildGuard::resolve_requested_artifact(&requested_graph) {
        Ok(path) => path,
        Err(error) => {
            return Outcome::failure(format!("error: could not resolve graph: {error}"));
        }
    };
    if !graph_path.exists() {
        return Outcome::failure(format!(
            "error: no graph found at {} — run `compass extract {}` first",
            graph_path.display(),
            root.display()
        ));
    }
    let output_dir = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let public_output_dir = compass_files::BuildGuard::output_container_for_artifact(&graph_path);
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
        Ok(result) => {
            let mut outcome = Outcome::success(format!(
                "Compass clustered {} nodes and {} edges into {} communities ({} labels reused).\nWritten to: {}",
                result.nodes,
                result.edges,
                result.communities,
                result.labels_reused,
                public_output_dir.display()
            ));
            if timing {
                outcome.stderr = format!(
                    "[compass timing] load: {:.1}s\n[compass timing] cluster: {:.1}s\n[compass timing] analyze: {:.1}s\n[compass timing] label: {:.1}s\n[compass timing] report: {:.1}s\n[compass timing] export: {:.1}s\n[compass timing] total: {:.1}s",
                    result.timings.load.as_secs_f64(),
                    result.timings.cluster.as_secs_f64(),
                    result.timings.analyze.as_secs_f64(),
                    result.timings.label.as_secs_f64(),
                    result.timings.report.as_secs_f64(),
                    result.timings.export.as_secs_f64(),
                    result.timings.total.as_secs_f64()
                );
            }
            outcome
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn command_tree(frontend: Frontend, args: &[String]) -> Outcome {
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
    graph_path = match compass_files::BuildGuard::resolve_requested_artifact(&graph_path) {
        Ok(path) => path,
        Err(error) => {
            return Outcome::failure(format!("error: could not resolve graph: {error}"));
        }
    };
    if !graph_path.is_file() {
        return Outcome::failure(format!(
            "error: graph.json not found at {}",
            graph_path.display()
        ));
    }
    if let Some((size, cap)) = compass_model::GraphDocument::size_cap_exceeded(&graph_path) {
        return Outcome::failure(format!(
            "error: graph file {} is {} bytes, exceeds {}-byte cap\n(set COMPASS_MAX_GRAPH_BYTES=<bytes> or COMPASS_MAX_GRAPH_BYTES=<N>GB to raise the limit)",
            graph_path.display(),
            grouped_decimal(size),
            grouped_decimal(cap)
        ));
    }
    let document = match compass_model::GraphDocument::load_for_recluster(&graph_path) {
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
    Outcome::success(format!("wrote {} ({size:.1} KB)", output_path.display()))
        .with_html_output(output_path)
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
    let _ = frontend;
    "Usage: compass tree [--graph PATH] [--output HTML]\n  --graph PATH         path to graph.json (default compass-out/graph.json)\n  --output HTML        output path (default compass-out/GRAPH_TREE.html)\n  --root PATH          filesystem root (default: longest common dir of all source_files)\n  --max-children N     cap visible children per node (default 200)\n  --top-k-edges N      pre-compute top-K outbound edges per symbol (default 12)\n  --label NAME         project label shown in the page header\n\nWhen run interactively, Compass asks before opening generated HTML in your browser."
        .to_owned()
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
            "Usage: compass merge-graphs <graph1.json> <graph2.json> [...] [--out merged.json]"
                .to_owned(),
        );
    }
    let mut resolved_paths = Vec::with_capacity(paths.len());
    for path in paths {
        let path = match compass_files::BuildGuard::resolve_requested_artifact(&path) {
            Ok(path) => path,
            Err(error) => {
                return Outcome::failure(format!("error: could not resolve graph: {error}"));
            }
        };
        if !path.exists() {
            return Outcome::failure(format!("error: not found: {}", path.display()));
        }
        resolved_paths.push(path);
    }
    match merge_graphs(&resolved_paths, &output) {
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
    let requested = args.first().map_or_else(default_graph_path, PathBuf::from);
    let graph_path = match compass_files::BuildGuard::resolve_requested_artifact(&requested) {
        Ok(path) => path,
        Err(error) => {
            return Outcome::failure(format!("error: could not resolve graph: {error}"));
        }
    };
    let document = match compass_model::GraphDocument::load(&graph_path) {
        Ok(document) => document,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let corpus_words = fs::read(".compass_detect.json")
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.get("total_words").and_then(serde_json::Value::as_u64))
        .and_then(|value| usize::try_from(value).ok());
    Outcome::success(format_benchmark(
        &run_benchmark(&document, corpus_words, None),
        true,
    ))
}

fn command_build(frontend: Frontend, args: &[String], operation: BuildOperation) -> Outcome {
    command_build_with_validation(frontend, args, operation, None, None, None)
}

pub(crate) fn command_build_with_precomputed_detection(
    frontend: Frontend,
    args: &[String],
    operation: BuildOperation,
    detection: Detection,
    started: Instant,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
) -> Outcome {
    command_build_with_validation(
        frontend,
        args,
        operation,
        progress,
        Some(detection),
        Some(started),
    )
}

fn command_build_with_validation(
    frontend: Frontend,
    args: &[String],
    operation: BuildOperation,
    file_progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
    precomputed_detection: Option<Detection>,
    operation_started: Option<Instant>,
) -> Outcome {
    let started = operation_started.unwrap_or_else(Instant::now);
    let mut outcome = command_build_with_validation_inner(
        frontend,
        args,
        operation,
        file_progress,
        precomputed_detection,
        started,
    );
    if outcome.code == 0 && outcome.stdout.starts_with("Usage:") {
        return outcome;
    }
    let elapsed = started.elapsed();
    let line = if outcome.code == 0 {
        format!(
            "Compass {} completed in {:.2}s wall time.",
            operation.label(),
            elapsed.as_secs_f64()
        )
    } else {
        format!(
            "Compass {} failed after {:.2}s wall time.",
            operation.label(),
            elapsed.as_secs_f64()
        )
    };
    let target = if outcome.code == 0 {
        &mut outcome.stdout
    } else {
        &mut outcome.stderr
    };
    if !target.is_empty() {
        target.push('\n');
    }
    target.push_str(&line);
    outcome
}

fn command_build_with_validation_inner(
    frontend: Frontend,
    args: &[String],
    operation: BuildOperation,
    file_progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
    precomputed_detection: Option<Detection>,
    started: Instant,
) -> Outcome {
    let extract = operation.extracts_semantics();
    let mut root = None;
    let mut output_root = None;
    let mut force = environment_truthy("COMPASS_FORCE");
    let mut reuse_cache_on_force = false;
    let mut no_cluster = false;
    let mut no_viz = false;
    let mut no_program = false;
    let mut program_requested = false;
    let mut graph_storage = GraphStorage::default();
    let mut inference_level = InferenceLevel::default();
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
    let mut max_source_bytes = None;
    let mut api_timeout = None;
    let mut allow_partial = false;
    let mut timing = false;
    let mut dedup_llm = false;
    let mut excludes = Vec::new();
    let mut program_artifacts = Vec::new();
    let mut resolution = 1.0;
    let mut exclude_hubs = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--force" => force = true,
            "--reuse-cache-on-force" => reuse_cache_on_force = true,
            "--no-cluster" => no_cluster = true,
            "--no-viz" => no_viz = true,
            "--no-program" => no_program = true,
            "--program" => program_requested = true,
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
                    return extract_parse_failure(
                        frontend,
                        format!(
                            "error: unknown --mode '{}'. Available: deep",
                            args[index + 1]
                        ),
                    );
                }
                deep_mode = true;
                index += 1;
            }
            value if extract && value.starts_with("--mode=") => {
                if &value[7..] != "deep" {
                    return extract_parse_failure(
                        frontend,
                        format!("error: unknown --mode '{}'. Available: deep", &value[7..]),
                    );
                }
                deep_mode = true;
            }
            "--token-budget" if extract && index + 1 < args.len() => {
                token_budget = match parse_positive_usize(&args[index + 1], "--token-budget") {
                    Ok(value) => Some(value),
                    Err(error) => return extract_parse_failure(frontend, error),
                };
                index += 1;
            }
            value if extract && value.starts_with("--token-budget=") => {
                token_budget = match parse_positive_usize(&value[15..], "--token-budget") {
                    Ok(value) => Some(value),
                    Err(error) => return extract_parse_failure(frontend, error),
                };
            }
            "--max-concurrency" if extract && index + 1 < args.len() => {
                max_concurrency = match parse_positive_usize(&args[index + 1], "--max-concurrency")
                {
                    Ok(value) => Some(value),
                    Err(error) => return extract_parse_failure(frontend, error),
                };
                index += 1;
            }
            value if extract && value.starts_with("--max-concurrency=") => {
                max_concurrency = match parse_positive_usize(&value[18..], "--max-concurrency") {
                    Ok(value) => Some(value),
                    Err(error) => return extract_parse_failure(frontend, error),
                };
            }
            "--api-timeout" if extract && index + 1 < args.len() => {
                api_timeout = match parse_positive_f64(&args[index + 1], "--api-timeout") {
                    Ok(value) => Some(value),
                    Err(error) => return extract_parse_failure(frontend, error),
                };
                index += 1;
            }
            value if extract && value.starts_with("--api-timeout=") => {
                api_timeout = match parse_positive_f64(&value[14..], "--api-timeout") {
                    Ok(value) => Some(value),
                    Err(error) => return extract_parse_failure(frontend, error),
                };
            }
            "--out" if index + 1 < args.len() => {
                output_root = Some(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            value if value.starts_with("--out=") => {
                output_root = Some(PathBuf::from(&value[6..]));
            }
            "--store" if index + 1 < args.len() => {
                graph_storage = match parse_graph_storage(&args[index + 1]) {
                    Ok(value) => value,
                    Err(error) => return extract_parse_failure(frontend, error),
                };
                index += 1;
            }
            value if value.starts_with("--store=") => {
                graph_storage = match parse_graph_storage(&value[8..]) {
                    Ok(value) => value,
                    Err(error) => return extract_parse_failure(frontend, error),
                };
            }
            "--store" => {
                return extract_parse_failure(
                    frontend,
                    "error: --store requires json or sqlite".to_owned(),
                );
            }
            "--inference-level" if index + 1 < args.len() => {
                inference_level = match parse_inference_level(&args[index + 1]) {
                    Ok(value) => value,
                    Err(error) => return extract_parse_failure(frontend, error),
                };
                index += 1;
            }
            value if value.starts_with("--inference-level=") => {
                inference_level = match parse_inference_level(&value[18..]) {
                    Ok(value) => value,
                    Err(error) => return extract_parse_failure(frontend, error),
                };
            }
            "--inference-level" => {
                return extract_parse_failure(
                    frontend,
                    "error: --inference-level requires low, medium, high, or max".to_owned(),
                );
            }
            "--exclude" if index + 1 < args.len() => {
                excludes.push(args[index + 1].clone());
                index += 1;
            }
            value if value.starts_with("--exclude=") => excludes.push(value[10..].to_owned()),
            "--program-artifact" if index + 1 < args.len() => {
                program_artifacts.push(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            "--program-artifact" => {
                return Outcome::failure("error: --program-artifact requires a path".to_owned());
            }
            value if value.starts_with("--program-artifact=") => {
                let path = &value[19..];
                if path.is_empty() {
                    return Outcome::failure(
                        "error: --program-artifact requires a path".to_owned(),
                    );
                }
                program_artifacts.push(PathBuf::from(path));
            }
            "--resolution" if index + 1 < args.len() => {
                resolution = match parse_positive_f64(&args[index + 1], "--resolution") {
                    Ok(value) => value,
                    Err(error) => return extract_parse_failure(frontend, error),
                };
                index += 1;
            }
            value if value.starts_with("--resolution=") => {
                resolution = match parse_positive_f64(&value[13..], "--resolution") {
                    Ok(value) => value,
                    Err(error) => return extract_parse_failure(frontend, error),
                };
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
                    Err(error) => return extract_parse_failure(frontend, error),
                };
                index += 1;
            }
            value if extract && value.starts_with("--max-workers=") => {
                max_workers = match parse_positive_usize(&value[14..], "--max-workers") {
                    Ok(value) => Some(value),
                    Err(error) => return extract_parse_failure(frontend, error),
                };
            }
            "--max-source-bytes" if index + 1 < args.len() => {
                max_source_bytes = match parse_positive_u64(&args[index + 1], "--max-source-bytes")
                {
                    Ok(value) => Some(value),
                    Err(error) => return extract_parse_failure(frontend, error),
                };
                index += 1;
            }
            value if value.starts_with("--max-source-bytes=") => {
                max_source_bytes = match parse_positive_u64(&value[19..], "--max-source-bytes") {
                    Ok(value) => Some(value),
                    Err(error) => return extract_parse_failure(frontend, error),
                };
            }
            "--max-source-bytes" => {
                return Outcome::failure(
                    "error: --max-source-bytes requires a positive integer".to_owned(),
                );
            }
            "--timing" => timing = true,
            "--dedup-llm" if extract => dedup_llm = true,
            "-h" | "--help" => {
                return Outcome::success(if extract {
                    extract_help()
                } else {
                    "Usage: compass update [path] [--program] [--program-artifact PATH] [--no-program] [--store json|sqlite] [--inference-level low|medium|high|max] [--max-source-bytes N] [--no-cluster] [--force] [--no-viz] [--timing]".to_owned()
                });
            }
            value if value.starts_with('-') => {
                return Outcome::failure(format!("error: unknown graph build option: {value}"));
            }
            value if root.is_none() => {
                root = Some(PathBuf::from(value));
            }
            value => {
                return Outcome::failure(format!(
                    "error: graph build accepts one path, unexpected: {value}"
                ));
            }
        }
        index += 1;
    }
    let has_explicit_root = root.is_some();
    if no_program && program_requested {
        return Outcome::failure("error: --program conflicts with --no-program".to_owned());
    }
    if no_program && !program_artifacts.is_empty() {
        return Outcome::failure(
            "error: --no-program conflicts with --program-artifact".to_owned(),
        );
    }
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
    let mut options = BuildOptions::new(&root);
    options.scope = match ProjectConfig::load(&root) {
        Ok(Some(config)) => config.build,
        Ok(None) => BuildScope::default(),
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    options.scan_filesystem = has_explicit_root || !extract;
    options.output_root = output_root;
    options.cache_root = std::env::var_os("COMPASS_HISTORY_CACHE_ROOT")
        .map(PathBuf::from)
        .filter(|_| environment_truthy("COMPASS_HISTORY_BUILD"));
    options.force = force;
    options.reuse_cache_on_force = reuse_cache_on_force;
    options.no_cluster = no_cluster;
    options.no_viz = no_viz;
    options.graph_storage = graph_storage;
    options.inference_level = inference_level;
    options.gitignore = gitignore;
    if environment_truthy("COMPASS_HISTORY_BUILD") {
        options.ignore_policy = compass_files::IgnorePolicy::HistoricalCommit;
    }
    options.extra_excludes = excludes;
    options.resolution = resolution;
    options.exclude_hubs = exclude_hubs;
    options.code_only = code_only;
    options.purpose = if extract {
        BuildPurpose::Extract
    } else {
        BuildPurpose::Update
    };
    options.google_workspace =
        google_workspace || compass_google_workspace::google_workspace_enabled(None);
    // Structural extraction is the fast, complete graph contract. Program IR
    // is an explicit opt-in because it adds a second analysis/output workload
    // that only some scenarios need. Keep --no-program accepted as a
    // compatibility spelling for callers that already select that profile.
    options.program_analysis = program_requested || !program_artifacts.is_empty();
    options.program_artifacts = program_artifacts;
    options.precomputed_detection = precomputed_detection;
    apply_max_workers_override(&mut options, max_workers);
    if let Some(max_source_bytes) = max_source_bytes {
        options.max_source_bytes = max_source_bytes;
    }
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let output_container = options
        .output_root
        .as_deref()
        .map(absolute_cli_path)
        .unwrap_or_else(|| root.clone())
        .join(output_name);
    let extract_incremental = extract
        && !force
        && compass_files::BuildGuard::resolve_artifact(&output_container, "graph.json")
            .is_ok_and(|path| path.is_file());
    let mut dedup_environment = std::env::vars().collect::<HashMap<_, _>>();
    if let Some(timeout) = api_timeout {
        dedup_environment.insert("COMPASS_API_TIMEOUT".to_owned(), timeout.to_string());
    }
    let mut dedup_tiebreaker = if dedup_llm {
        let global_providers = home_directory()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".compass")
            .join("providers.json");
        let local_providers = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".compass")
            .join("providers.json");
        match dedup_commands::DedupLlmTiebreaker::prepare(
            backend.as_deref(),
            model.as_deref(),
            dedup_environment,
            &global_providers,
            &local_providers,
            environment_truthy("COMPASS_ALLOW_LOCAL_PROVIDERS"),
            executable_on_path("claude"),
        ) {
            Ok(tiebreaker) => Some(tiebreaker),
            Err(error) if error.starts_with("no LLM API key found") => {
                let semantic_count = if code_only {
                    0
                } else {
                    pending_semantic_count(&options, extract_incremental)
                };
                let message = format!("error: {}", no_llm_api_key_message(semantic_count, true));
                return Outcome::failure(message);
            }
            Err(error) => {
                let message = format!("error: {error}");
                return Outcome::failure(message);
            }
        }
    } else {
        None
    };
    let postgres_graph = if let Some(dsn) = postgres_dsn.as_deref() {
        match compass_postgres::introspect_postgres(Some(dsn)) {
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
        match compass_cargo::introspect_cargo(&root) {
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
    let empty_extract_layer = SemanticLayer {
        fragment: serde_json::json!({
            "nodes": [],
            "edges": [],
            "hyperedges": [],
            "input_tokens": 0,
            "output_tokens": 0,
            "failed_chunks": 0,
        }),
        refreshed_files: Vec::new(),
        partial_files: Vec::new(),
        allow_partial,
    };
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
                .map(|tiebreaker| tiebreaker as &mut dyn compass_graph::EntityTiebreaker),
        )
    } else if extract && !auxiliary_fragments.is_empty() {
        build_graph_with_optional_tiebreaker(
            &options,
            Some(&empty_extract_layer),
            &auxiliary_fragments,
            dedup_tiebreaker
                .as_mut()
                .map(|tiebreaker| tiebreaker as &mut dyn compass_graph::EntityTiebreaker),
            None,
        )
        .map(|result| (result, Vec::new(), Duration::ZERO))
        .map_err(|error| error.to_string())
    } else {
        build_graph_with_optional_tiebreaker(
            &options,
            extract.then_some(&empty_extract_layer),
            &[],
            dedup_tiebreaker
                .as_mut()
                .map(|tiebreaker| tiebreaker as &mut dyn compass_graph::EntityTiebreaker),
            file_progress,
        )
        .map(|result| (result, Vec::new(), Duration::ZERO))
        .map_err(|error| error.to_string())
    };
    match built {
        Ok((result, mut notes, _semantic_elapsed)) => {
            if let Some(tiebreaker) = dedup_tiebreaker.as_mut() {
                notes.extend(tiebreaker.take_warnings());
            }
            let mut global_warning = None;
            if let Some((nodes, edges)) = postgres_counts {
                notes.push(format!(
                    "[compass extract] PostgreSQL: {nodes} nodes, {edges} edges"
                ));
            }
            if let Some((nodes, edges)) = cargo_counts {
                notes.push(format!(
                    "[compass extract] Cargo: {nodes} nodes, {edges} edges"
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
                        "[compass global] '{tag}' unchanged since last add - skipped."
                    )),
                    Ok(merged) => notes.push(format!(
                        "[compass global] '{tag}' merged into global graph (+{} nodes, -{} pruned).",
                        merged.nodes_added, merged.nodes_removed
                    )),
                    Err(error) => {
                        global_warning = Some(format!(
                            "[compass global] warning: failed to merge into global graph: {error}"
                        ));
                    }
                }
            }
            let mode = if no_cluster {
                "without clustering"
            } else {
                "with clustering"
            };
            let mut output = format!(
                "Compass indexed {} files ({} extracted, {} cached): {} nodes, {} edges, {} communities {mode}.\nWritten to: {}",
                result.files_considered,
                result.files_extracted,
                result.files_cached,
                result.nodes,
                result.edges,
                result.communities,
                compass_files::BuildGuard::output_container_for_artifact(
                    &result.output_dir.join("graph.json")
                )
                .display()
            );
            output.push('\n');
            output.push_str(&format_program_analysis(&result));
            if !notes.is_empty() {
                output.push('\n');
                output.push_str(&notes.join("\n"));
            }
            let mut outcome = Outcome::success(output);
            apply_build_quality_outcome(&result, &mut outcome);
            if let Some(warning) = global_warning {
                if !outcome.stderr.is_empty() {
                    outcome.stderr.push('\n');
                }
                outcome.stderr.push_str(&warning);
            }
            if timing {
                if !outcome.stderr.is_empty() {
                    outcome.stderr.push('\n');
                }
                outcome
                    .stderr
                    .push_str(&format_build_timings(started.elapsed(), &result.timings));
            }
            outcome
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn apply_max_workers_override(options: &mut BuildOptions, max_workers: Option<usize>) {
    if let Some(max_workers) = max_workers {
        options.max_workers = Some(max_workers);
    }
}

fn parse_graph_storage(value: &str) -> Result<GraphStorage, String> {
    match value {
        "json" => Ok(GraphStorage::Json),
        "sqlite" => Ok(GraphStorage::Sqlite),
        _ => Err(format!(
            "error: --store must be json or sqlite (found {value})"
        )),
    }
}

pub(crate) fn parse_inference_level(value: &str) -> Result<InferenceLevel, String> {
    match value {
        "low" => Ok(InferenceLevel::Low),
        "medium" => Ok(InferenceLevel::Medium),
        "high" => Ok(InferenceLevel::High),
        "max" => Ok(InferenceLevel::Max),
        _ => Err(format!(
            "error: --inference-level must be low, medium, high, or max (found {value})"
        )),
    }
}

fn format_build_timings(elapsed: Duration, timings: &BuildTimings) -> String {
    let stages = [
        ("detect", timings.detect),
        ("deterministic extract", timings.deterministic_extract),
        ("graph assembly", timings.graph_assembly),
        ("program analysis", timings.program_analysis),
        ("publish", timings.publish),
    ];
    let mut lines = stages
        .into_iter()
        .map(|(stage, duration)| {
            format!("[compass timing] {stage}: {:.1}s", duration.as_secs_f64())
        })
        .collect::<Vec<_>>();
    if timings.store_new_objects > 0
        || timings.store_reused_objects > 0
        || timings.store_write_transactions > 0
        || timings.store_gc_deleted_entries > 0
    {
        lines.push(format!(
            "[compass timing] store: new_objects={} reused_objects={} write_transactions={} bytes_written={} gc_deleted_entries={}",
            timings.store_new_objects,
            timings.store_reused_objects,
            timings.store_write_transactions,
            timings.store_bytes_written,
            timings.store_gc_deleted_entries,
        ));
    }
    lines.push(format!(
        "[compass timing] total: {:.1}s",
        elapsed.as_secs_f64()
    ));
    lines.join("\n")
}

fn pending_semantic_count(options: &BuildOptions, incremental: bool) -> usize {
    let root = fs::canonicalize(&options.root).unwrap_or_else(|_| options.root.clone());
    let output_root = options
        .output_root
        .as_deref()
        .map(resolve_cli_path)
        .unwrap_or_else(|| root.clone());
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let detect_options = DetectOptions {
        scan_filesystem: options.scan_filesystem,
        gitignore: options.gitignore,
        ignore_policy: options.ignore_policy,
        extra_excludes: options.extra_excludes.clone(),
        scope: options.scope.clone(),
        output_name: output_name.clone(),
        cache_root: Some(output_root.clone()),
        google_workspace: options.google_workspace,
        ..DetectOptions::default()
    };
    let files = if incremental {
        Manifest::incremental(
            &root,
            &output_root.join(output_name).join("manifest.json"),
            &detect_options,
            ManifestKind::Semantic,
        )
        .ok()
        .map(|result| result.new_files)
    } else {
        detect(&root, &detect_options)
            .ok()
            .map(|result| result.files)
    };
    files.map_or(0, |files| {
        ["document", "paper", "image"]
            .into_iter()
            .filter_map(|kind| files.get(kind))
            .map(Vec::len)
            .sum()
    })
}

fn no_llm_api_key_message(semantic_count: usize, dedup_llm: bool) -> String {
    let mut reasons = Vec::new();
    if semantic_count > 0 {
        reasons.push(format!(
            "{semantic_count} doc/paper/image file(s) need semantic extraction"
        ));
    }
    if dedup_llm {
        reasons.push("--dedup-llm was passed".to_owned());
    }
    let hint = if semantic_count > 0 {
        " Or pass --code-only to index just the code (local AST, no key) and skip the non-code files."
    } else {
        ""
    };
    format!(
        "no LLM API key found ({}). Set GEMINI_API_KEY or GOOGLE_API_KEY (gemini), MOONSHOT_API_KEY (kimi), ANTHROPIC_API_KEY (claude), OPENAI_API_KEY (openai), DEEPSEEK_API_KEY (deepseek), or pass --backend. A code-only corpus needs no key.{hint}",
        reasons.join("; ")
    )
}

fn command_hook_refresh(frontend: Frontend, args: &[String]) -> Outcome {
    let launch_root = args
        .iter()
        .find(|argument| !argument.starts_with('-'))
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let marker = launch_root.join(&output_name).join("source-root.txt");
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
    let result = command_build_with_validation(
        frontend,
        &build_args,
        BuildOperation::Update,
        None,
        None,
        None,
    );
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
    tiebreaker: Option<&mut dyn compass_graph::EntityTiebreaker>,
) -> Result<(BuildResult, Vec<String>, Duration), String> {
    let semantic_started = Instant::now();
    let root = fs::canonicalize(&options.root)
        .map_err(|error| format!("could not resolve {}: {error}", options.root.display()))?;
    let output_root = options
        .output_root
        .as_deref()
        .map(absolute_cli_path)
        .unwrap_or_else(|| root.clone());
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let manifest_path = output_root.join(&output_name).join("manifest.json");
    let detect_options = DetectOptions {
        scan_filesystem: options.scan_filesystem,
        gitignore: options.gitignore,
        ignore_policy: options.ignore_policy,
        extra_excludes: options.extra_excludes.clone(),
        scope: options.scope.clone(),
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
            "[compass extract] deep mode: {} live semantic file(s)",
            semantic_files.len()
        ));
    }

    let mut environment = std::env::vars().collect::<HashMap<_, _>>();
    if let Some(timeout) = api_timeout {
        environment.insert("COMPASS_API_TIMEOUT".to_owned(), timeout.to_string());
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
    let history_cache_root = options.cache_root.as_deref();
    let cache_root =
        history_cache_root.or_else(|| (output_root != root).then_some(output_root.as_path()));

    let extracted = if semantic_files.is_empty() {
        None
    } else {
        let global_providers = home_directory()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".compass")
            .join("providers.json");
        let local_providers = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".compass")
            .join("providers.json");
        let custom = load_custom_providers(
            &global_providers,
            &local_providers,
            environment_truthy_from(&environment, "COMPASS_ALLOW_LOCAL_PROVIDERS"),
        );
        notes.extend(
            custom
                .warnings
                .iter()
                .map(|warning| format!("[compass extract] warning: {warning}")),
        );
        let selected = requested_backend
            .map(str::to_owned)
            .or_else(|| {
                detect_backend_with_custom(&custom.providers, &environment).map(str::to_owned)
            })
            .ok_or_else(|| no_llm_api_key_message(semantic_files.len(), false))?;
        let mut completed_chunks = 0_usize;
        let mut progress = |index: usize,
                            total: usize,
                            _units: &[compass_semantic::SemanticUnit],
                            _fragment: &serde_json::Value| {
            completed_chunks = completed_chunks.saturating_add(1);
            notes.push(format!(
                "[compass extract] chunk {}/{} done",
                index + 1,
                total
            ));
        };
        let result = if let Some(backend) = compass_semantic::builtin_backend(&selected) {
            let resolved = resolve_builtin_backend(&selected, &environment, requested_model)
                .map_err(|error| error.to_string())?;
            let keyless_local_ollama = selected == "ollama"
                && resolved
                    .base_url
                    .as_deref()
                    .is_some_and(provider_url_is_loopback);
            if !backend.api_key_variables.is_empty()
                && resolved.api_key().is_none()
                && !keyless_local_ollama
            {
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
            let mut available = compass_semantic::BUILTIN_BACKENDS
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
            "[compass extract] semantic cache: {} hit / {} miss",
            result.cache_hits, result.cache_misses
        ));
        notes.extend(
            result
                .provider_warnings
                .iter()
                .map(|warning| format!("[compass extract] provider warning: {warning}")),
        );
        notes.extend(
            result
                .cache_issues
                .iter()
                .map(|issue| format!("[compass extract] cache warning: {}", issue.message)),
        );
        notes.extend(result.failures.iter().map(|failure| {
            format!(
                "[compass extract] chunk {} failed: {}",
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
        None,
    )
    .map_err(|error| error.to_string())?;
    Ok((result, notes, semantic_elapsed))
}

fn build_graph_with_optional_tiebreaker(
    options: &BuildOptions,
    semantic: Option<&SemanticLayer>,
    supplemental: &[serde_json::Value],
    tiebreaker: Option<&mut dyn compass_graph::EntityTiebreaker>,
    progress: Option<&(dyn Fn(BuildFileProgress) + Sync)>,
) -> Result<BuildResult, compass_core::CoreError> {
    match tiebreaker {
        Some(tiebreaker) => {
            build_graph_with_layers_and_tiebreaker(options, semantic, supplemental, tiebreaker)
        }
        None => match progress {
            Some(progress) => {
                build_graph_with_layers_and_progress(options, semantic, supplemental, progress)
            }
            None => build_graph_with_layers(options, semantic, supplemental),
        },
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
    let parsed = value.parse::<isize>().map_err(|_| {
        format!(
            "error: {option} must be a positive integer (got {})",
            python_string_repr(value)
        )
    })?;
    usize::try_from(parsed)
        .ok()
        .filter(|parsed| *parsed > 0)
        .ok_or_else(|| format!("error: {option} must be > 0 (got {parsed})"))
}

fn parse_positive_u64(value: &str, option: &str) -> Result<u64, String> {
    let parsed = value.parse::<u64>().map_err(|_| {
        format!(
            "error: {option} must be a positive integer (got {})",
            python_string_repr(value)
        )
    })?;
    (parsed > 0)
        .then_some(parsed)
        .ok_or_else(|| format!("error: {option} must be > 0 (got {parsed})"))
}

fn parse_positive_f64(value: &str, option: &str) -> Result<f64, String> {
    let parsed = value.parse::<f64>().map_err(|_| {
        format!(
            "error: {option} must be a positive number (got {})",
            python_string_repr(value)
        )
    })?;
    if parsed <= 0.0 {
        return Err(format!(
            "error: {option} must be > 0 (got {})",
            python_float_repr(parsed)
        ));
    }
    Ok(parsed)
}

fn python_float_repr(value: f64) -> String {
    if value.is_nan() {
        "nan".to_owned()
    } else if value == f64::INFINITY {
        "inf".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-inf".to_owned()
    } else if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

fn provider_url_is_loopback(value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    parsed.host().is_some_and(|host| match host {
        url::Host::Domain(host) => host.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(address) => address.is_loopback(),
        url::Host::Ipv6(address) => address.is_loopback(),
    })
}

fn python_string_repr(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn extract_parse_failure(frontend: Frontend, error: String) -> Outcome {
    let _ = frontend;
    Outcome::failure(error)
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

fn resolve_cli_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    let absolute = absolute_cli_path(path);
    let Some(parent) = absolute.parent() else {
        return absolute;
    };
    fs::canonicalize(parent).map_or(absolute.clone(), |resolved| {
        absolute
            .file_name()
            .map_or(resolved.clone(), |name| resolved.join(name))
    })
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
    "Usage: compass extract [PATH] [--program] [--program-artifact PATH] [--no-program] [--store json|sqlite] [--inference-level low|medium|high|max] [--code-only] [--cargo] [--google-workspace] [--postgres DSN] [--backend NAME] [--model MODEL] [--mode deep] [--token-budget N] [--max-concurrency N] [--max-workers N] [--max-source-bytes N] [--api-timeout SECONDS] [--allow-partial] [--dedup-llm] [--timing] [--out DIR] [--no-cluster] [--force] [--no-viz] [--no-gitignore] [--exclude PATTERN] [--resolution N] [--exclude-hubs N]".to_owned()
}

fn saved_graph_root() -> Option<PathBuf> {
    let path = default_graph_path().parent()?.join("source-root.txt");
    let root = fs::read_to_string(path).ok()?;
    let root = root.trim();
    (!root.is_empty()).then(|| PathBuf::from(root))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExportViewRequest {
    Code,
    Architecture,
    Call(String),
    Impact(String),
    Affected(String),
    History { base: String, target: String },
    Artifact(ArtifactLens),
}

struct ExportViewOptions<'a> {
    direction: CallGraphDirection,
    depth: u32,
    max_nodes: usize,
    max_edges: usize,
    include_heuristic: bool,
    relations: &'a [String],
    program_path: Option<&'a Path>,
}

fn parse_export_view(value: &str) -> Result<ExportViewRequest, String> {
    if value == "code" {
        Ok(ExportViewRequest::Code)
    } else if value == "architecture" {
        Ok(ExportViewRequest::Architecture)
    } else if let Some(root) = value.strip_prefix("call:").filter(|root| !root.is_empty()) {
        Ok(ExportViewRequest::Call(root.to_owned()))
    } else if let Some(root) = value
        .strip_prefix("impact:")
        .filter(|root| !root.is_empty())
    {
        Ok(ExportViewRequest::Impact(root.to_owned()))
    } else if let Some(root) = value
        .strip_prefix("affected:")
        .filter(|root| !root.is_empty())
    {
        Ok(ExportViewRequest::Affected(root.to_owned()))
    } else if let Some(range) = value.strip_prefix("history:") {
        parse_history_view(range)
    } else if let Some(lens) = value.strip_prefix("artifact:") {
        parse_artifact_lens(lens).map(ExportViewRequest::Artifact)
    } else {
        Err(format!(
            "invalid view '{value}'; expected code, architecture, call:SYMBOL, impact:SYMBOL, affected:NODE, history:OLD..NEW, or artifact:LENS"
        ))
    }
}

fn parse_history_view(value: &str) -> Result<ExportViewRequest, String> {
    let (base, target) = value
        .split_once("..")
        .filter(|(base, target)| !base.is_empty() && !target.is_empty())
        .ok_or_else(|| "history view must use OLD..NEW".to_owned())?;
    Ok(ExportViewRequest::History {
        base: base.to_owned(),
        target: target.to_owned(),
    })
}

fn parse_artifact_lens(value: &str) -> Result<ArtifactLens, String> {
    match value {
        "dependencies" | "dependency" => Ok(ArtifactLens::Dependencies),
        "routes" | "route" => Ok(ArtifactLens::Routes),
        "data" => Ok(ArtifactLens::Data),
        "messaging" | "messages" | "jobs" => Ok(ArtifactLens::Messaging),
        "tests" | "test" => Ok(ArtifactLens::Tests),
        "provenance" | "aliases" => Ok(ArtifactLens::Provenance),
        _ => Err(format!(
            "unknown artifact lens '{value}'; expected dependencies, routes, data, messaging, tests, or provenance"
        )),
    }
}

fn parse_call_direction(value: &str) -> Result<CallGraphDirection, String> {
    match value {
        "callers" => Ok(CallGraphDirection::Callers),
        "callees" => Ok(CallGraphDirection::Callees),
        "both" => Ok(CallGraphDirection::Both),
        _ => Err("--direction must be callers, callees, or both".to_owned()),
    }
}

fn validate_export_options(
    format: &str,
    seen: &std::collections::BTreeSet<&str>,
    views: &[ExportViewRequest],
) -> Result<(), String> {
    let allowed: &[&str] = match format {
        "html" => &[
            "--graph",
            "--labels",
            "--node-limit",
            "--no-viz",
            "--output",
            "--view",
            "--direction",
            "--depth",
            "--max-nodes",
            "--max-edges",
            "--relation",
            "--include-heuristic",
            "--program",
        ],
        "json" | "viewer-json" => &[
            "--graph",
            "--labels",
            "--node-limit",
            "--community",
            "--view",
            "--direction",
            "--depth",
            "--max-nodes",
            "--max-edges",
            "--relation",
            "--include-heuristic",
            "--program",
        ],
        "workbench-json" => &[
            "--graph",
            "--labels",
            "--node-limit",
            "--view",
            "--direction",
            "--depth",
            "--max-nodes",
            "--max-edges",
            "--relation",
            "--include-heuristic",
            "--program",
        ],
        "callflow-html" => &[
            "--graph",
            "--labels",
            "--report",
            "--sections",
            "--output",
            "--lang",
            "--max-sections",
            "--diagram-scale",
            "--max-diagram-nodes",
            "--max-diagram-edges",
        ],
        "callflow-json" => &[
            "--graph",
            "--labels",
            "--report",
            "--sections",
            "--lang",
            "--max-sections",
            "--diagram-scale",
            "--max-diagram-nodes",
            "--max-diagram-edges",
        ],
        "obsidian" => &["--graph", "--labels", "--dir"],
        "wiki" | "svg" => &["--graph", "--labels"],
        "graphml" => &["--graph"],
        "neo4j" | "falkordb" => &["--graph", "--push", "--user", "--password"],
        _ => &[],
    };
    if let Some(option) = seen.iter().find(|option| !allowed.contains(option)) {
        return Err(format!("{option} is not valid with export {format}"));
    }
    if seen.contains("--community") && !views.is_empty() {
        return Err("--community cannot be combined with workbench views".to_owned());
    }
    if seen.contains("--direction")
        && !views
            .iter()
            .any(|view| matches!(view, ExportViewRequest::Call(_)))
    {
        return Err("--direction requires a call graph view".to_owned());
    }
    if seen.contains("--include-heuristic")
        && !views
            .iter()
            .any(|view| matches!(view, ExportViewRequest::Impact(_)))
    {
        return Err("--include-heuristic requires an impact graph view".to_owned());
    }
    if seen.contains("--relation")
        && !views
            .iter()
            .any(|view| matches!(view, ExportViewRequest::Affected(_)))
    {
        return Err("--relation requires an affected graph view".to_owned());
    }
    if seen.contains("--program")
        && !views
            .iter()
            .any(|view| matches!(view, ExportViewRequest::Call(_)))
    {
        return Err("--program requires a call graph view".to_owned());
    }
    if seen.contains("--depth")
        && !views.iter().any(|view| {
            matches!(
                view,
                ExportViewRequest::Call(_)
                    | ExportViewRequest::Impact(_)
                    | ExportViewRequest::Affected(_)
            )
        })
    {
        return Err("--depth requires a call, impact, or affected graph view".to_owned());
    }
    if (seen.contains("--max-nodes") || seen.contains("--max-edges")) && views.is_empty() {
        return Err("--max-nodes and --max-edges require a requested view".to_owned());
    }
    Ok(())
}

fn command_export(frontend: Frontend, args: &[String]) -> Outcome {
    let Some(format) = args.first().map(String::as_str) else {
        return Outcome::failure(export_help());
    };
    if format == "orientation-json" {
        return command_export_orientation_json(&args[1..]);
    }
    if !matches!(
        format,
        "html"
            | "json"
            | "viewer-json"
            | "workbench-json"
            | "callflow-html"
            | "callflow-json"
            | "obsidian"
            | "wiki"
            | "svg"
            | "graphml"
            | "neo4j"
            | "falkordb"
    ) {
        return Outcome::failure(export_help());
    }
    if args.len() == 2 && matches!(args[1].as_str(), "-h" | "--help") {
        return Outcome::success(match format {
            "html" | "workbench-json" => export_workbench_help(format),
            "json" | "viewer-json" => export_json_help(),
            "callflow-html" | "callflow-json" => callflow_help(),
            _ => export_help(),
        });
    }
    let mut graph_path = default_graph_path();
    let mut graph_explicit = false;
    let mut labels_path = default_graph_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("labels.json");
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
    let mut community = None;
    let mut no_viz = false;
    let mut obsidian_dir = default_graph_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("obsidian");
    let mut push_uri = None;
    let mut view_requests = Vec::new();
    let mut view_depth = 3_u32;
    let mut view_max_nodes = 500_usize;
    let mut view_max_edges = 1_000_usize;
    let mut view_direction = CallGraphDirection::Both;
    let mut include_heuristic = false;
    let mut view_relations = Vec::new();
    let mut program_path = None;
    let mut seen_options = std::collections::BTreeSet::new();
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
                seen_options.insert("--graph");
                let Some(value) = next() else {
                    return Outcome::failure("error: --graph requires a path".to_owned());
                };
                graph_path = PathBuf::from(value);
                graph_explicit = true;
                index += 2;
            }
            "--labels" => {
                seen_options.insert("--labels");
                let Some(value) = next() else {
                    return Outcome::failure("error: --labels requires a path".to_owned());
                };
                labels_path = PathBuf::from(value);
                labels_explicit = true;
                index += 2;
            }
            "--report" => {
                seen_options.insert("--report");
                let Some(value) = next() else {
                    return Outcome::failure("error: --report requires a path".to_owned());
                };
                report_path = PathBuf::from(value);
                report_explicit = true;
                index += 2;
            }
            "--sections" => {
                seen_options.insert("--sections");
                let Some(value) = next() else {
                    return Outcome::failure("error: --sections requires a path".to_owned());
                };
                sections_path = Some(PathBuf::from(value));
                index += 2;
            }
            "--output" => {
                seen_options.insert("--output");
                let Some(value) = next() else {
                    return Outcome::failure("error: --output requires a path".to_owned());
                };
                output_path = Some(absolutize(PathBuf::from(value)));
                index += 2;
            }
            "--dir" => {
                seen_options.insert("--dir");
                let Some(value) = next() else {
                    return Outcome::failure("error: --dir requires a path".to_owned());
                };
                obsidian_dir = PathBuf::from(value);
                index += 2;
            }
            "--push" => {
                seen_options.insert("--push");
                let Some(value) = next() else {
                    return Outcome::failure("error: --push requires a URI".to_owned());
                };
                push_uri = Some(value);
                index += 2;
            }
            "--user" => {
                seen_options.insert("--user");
                let Some(value) = next() else {
                    return Outcome::failure("error: --user requires a value".to_owned());
                };
                push_user = value;
                index += 2;
            }
            "--password" => {
                seen_options.insert("--password");
                let Some(value) = next() else {
                    return Outcome::failure("error: --password requires a value".to_owned());
                };
                push_password = Some(value);
                index += 2;
            }
            "--lang" => {
                seen_options.insert("--lang");
                let Some(value) = next() else {
                    return Outcome::failure("error: --lang requires a value".to_owned());
                };
                language = value;
                index += 2;
            }
            "--max-sections" => {
                seen_options.insert("--max-sections");
                let Some(value) = parse_usize(next(), "--max-sections") else {
                    return Outcome::failure("error: --max-sections must be an integer".to_owned());
                };
                max_sections = value;
                index += 2;
            }
            "--max-diagram-nodes" => {
                seen_options.insert("--max-diagram-nodes");
                let Some(value) = parse_usize(next(), "--max-diagram-nodes") else {
                    return Outcome::failure(
                        "error: --max-diagram-nodes must be an integer".to_owned(),
                    );
                };
                max_diagram_nodes = value;
                index += 2;
            }
            "--max-diagram-edges" => {
                seen_options.insert("--max-diagram-edges");
                let Some(value) = parse_usize(next(), "--max-diagram-edges") else {
                    return Outcome::failure(
                        "error: --max-diagram-edges must be an integer".to_owned(),
                    );
                };
                max_diagram_edges = value;
                index += 2;
            }
            "--node-limit" => {
                seen_options.insert("--node-limit");
                let Some(value) = next().and_then(|value| value.parse::<isize>().ok()) else {
                    return Outcome::failure("error: --node-limit must be an integer".to_owned());
                };
                node_limit = value;
                index += 2;
            }
            "--community" if matches!(format, "json" | "viewer-json") => {
                seen_options.insert("--community");
                let Some(value) = next().and_then(|value| value.parse::<usize>().ok()) else {
                    return Outcome::failure(
                        "error: --community must be a non-negative integer".to_owned(),
                    );
                };
                if community.is_some() {
                    return Outcome::failure("error: duplicate --community".to_owned());
                }
                community = Some(value);
                index += 2;
            }
            value
                if matches!(format, "json" | "viewer-json")
                    && value.starts_with("--community=") =>
            {
                seen_options.insert("--community");
                let Some(value) = value
                    .strip_prefix("--community=")
                    .and_then(|value| value.parse::<usize>().ok())
                else {
                    return Outcome::failure(
                        "error: --community must be a non-negative integer".to_owned(),
                    );
                };
                if community.is_some() {
                    return Outcome::failure("error: duplicate --community".to_owned());
                }
                community = Some(value);
                index += 1;
            }
            "--community" => {
                return Outcome::failure(
                    "error: --community is only valid with export json".to_owned(),
                );
            }
            value if value.starts_with("--community=") => {
                return Outcome::failure(
                    "error: --community is only valid with export json".to_owned(),
                );
            }
            "--diagram-scale" => {
                seen_options.insert("--diagram-scale");
                let Some(value) = next().and_then(|value| value.parse::<f64>().ok()) else {
                    return Outcome::failure("error: --diagram-scale must be a number".to_owned());
                };
                diagram_scale = value;
                index += 2;
            }
            "--no-viz" => {
                seen_options.insert("--no-viz");
                no_viz = true;
                index += 1;
            }
            "--view" => {
                let Some(value) = next() else {
                    return Outcome::failure(
                        "error: --view requires a view specification".to_owned(),
                    );
                };
                match parse_export_view(&value) {
                    Ok(view) => view_requests.push(view),
                    Err(error) => return Outcome::failure(format!("error: {error}")),
                }
                seen_options.insert("--view");
                index += 2;
            }
            "--code-graph" => {
                view_requests.push(ExportViewRequest::Code);
                seen_options.insert("--view");
                index += 1;
            }
            "--architecture-graph" => {
                view_requests.push(ExportViewRequest::Architecture);
                seen_options.insert("--view");
                index += 1;
            }
            "--call-graph" => {
                let Some(value) = next().filter(|value| !value.is_empty()) else {
                    return Outcome::failure("error: --call-graph requires a symbol".to_owned());
                };
                view_requests.push(ExportViewRequest::Call(value));
                seen_options.insert("--view");
                index += 2;
            }
            "--impact-graph" => {
                let Some(value) = next().filter(|value| !value.is_empty()) else {
                    return Outcome::failure("error: --impact-graph requires a symbol".to_owned());
                };
                view_requests.push(ExportViewRequest::Impact(value));
                seen_options.insert("--view");
                index += 2;
            }
            "--affected-graph" => {
                let Some(value) = next().filter(|value| !value.is_empty()) else {
                    return Outcome::failure(
                        "error: --affected-graph requires a node or label".to_owned(),
                    );
                };
                view_requests.push(ExportViewRequest::Affected(value));
                seen_options.insert("--view");
                index += 2;
            }
            "--history-graph" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --history-graph requires OLD..NEW".to_owned());
                };
                match parse_history_view(&value) {
                    Ok(view) => view_requests.push(view),
                    Err(error) => return Outcome::failure(format!("error: {error}")),
                }
                seen_options.insert("--view");
                index += 2;
            }
            "--artifact-lens" => {
                let Some(value) = next() else {
                    return Outcome::failure(
                        "error: --artifact-lens requires a lens name".to_owned(),
                    );
                };
                match parse_artifact_lens(&value) {
                    Ok(lens) => view_requests.push(ExportViewRequest::Artifact(lens)),
                    Err(error) => return Outcome::failure(format!("error: {error}")),
                }
                seen_options.insert("--view");
                index += 2;
            }
            "--direction" => {
                let Some(value) = next() else {
                    return Outcome::failure(
                        "error: --direction requires callers, callees, or both".to_owned(),
                    );
                };
                match parse_call_direction(&value) {
                    Ok(direction) => view_direction = direction,
                    Err(error) => return Outcome::failure(format!("error: {error}")),
                }
                seen_options.insert("--direction");
                index += 2;
            }
            "--depth" => {
                let Some(value) = next()
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| *value > 0)
                else {
                    return Outcome::failure(
                        "error: --depth must be a positive integer".to_owned(),
                    );
                };
                view_depth = value;
                seen_options.insert("--depth");
                index += 2;
            }
            "--max-nodes" => {
                let Some(value) = next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| *value > 0)
                else {
                    return Outcome::failure(
                        "error: --max-nodes must be a positive integer".to_owned(),
                    );
                };
                view_max_nodes = value;
                seen_options.insert("--max-nodes");
                index += 2;
            }
            "--max-edges" => {
                let Some(value) = next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| *value > 0)
                else {
                    return Outcome::failure(
                        "error: --max-edges must be a positive integer".to_owned(),
                    );
                };
                view_max_edges = value;
                seen_options.insert("--max-edges");
                index += 2;
            }
            "--relation" => {
                let Some(value) = next().filter(|value| !value.is_empty()) else {
                    return Outcome::failure("error: --relation requires a value".to_owned());
                };
                view_relations.push(value);
                seen_options.insert("--relation");
                index += 2;
            }
            "--include-heuristic" => {
                include_heuristic = true;
                seen_options.insert("--include-heuristic");
                index += 1;
            }
            "--program" => {
                let Some(value) = next() else {
                    return Outcome::failure("error: --program requires a path".to_owned());
                };
                program_path = Some(PathBuf::from(value));
                seen_options.insert("--program");
                index += 2;
            }
            "-h" | "--help" if matches!(format, "callflow-html" | "callflow-json") => {
                return Outcome::success(callflow_help());
            }
            "-h" | "--help" if matches!(format, "json" | "viewer-json") => {
                return Outcome::success(export_json_help());
            }
            value
                if matches!(format, "callflow-html" | "callflow-json")
                    && !value.starts_with('-')
                    && !graph_explicit =>
            {
                let candidate = PathBuf::from(value);
                graph_path = if candidate.file_name().and_then(|name| name.to_str())
                    == Some("graph.json")
                    || candidate.extension().and_then(|value| value.to_str()) == Some("json")
                {
                    candidate
                } else if candidate.join("graph.json").exists() {
                    candidate.join("graph.json")
                } else {
                    candidate.join("compass-out/graph.json")
                };
                graph_explicit = true;
                index += 1;
            }
            value if value.starts_with("--view=") => {
                match parse_export_view(&value[7..]) {
                    Ok(view) => view_requests.push(view),
                    Err(error) => return Outcome::failure(format!("error: {error}")),
                }
                seen_options.insert("--view");
                index += 1;
            }
            value if value.starts_with("--call-graph=") => {
                let symbol = &value[13..];
                if symbol.is_empty() {
                    return Outcome::failure("error: --call-graph requires a symbol".to_owned());
                }
                view_requests.push(ExportViewRequest::Call(symbol.to_owned()));
                seen_options.insert("--view");
                index += 1;
            }
            value if value.starts_with("--impact-graph=") => {
                let symbol = &value[15..];
                if symbol.is_empty() {
                    return Outcome::failure("error: --impact-graph requires a symbol".to_owned());
                }
                view_requests.push(ExportViewRequest::Impact(symbol.to_owned()));
                seen_options.insert("--view");
                index += 1;
            }
            value if value.starts_with("--affected-graph=") => {
                let root = &value[17..];
                if root.is_empty() {
                    return Outcome::failure(
                        "error: --affected-graph requires a node or label".to_owned(),
                    );
                }
                view_requests.push(ExportViewRequest::Affected(root.to_owned()));
                seen_options.insert("--view");
                index += 1;
            }
            value if value.starts_with("--history-graph=") => {
                match parse_history_view(&value[16..]) {
                    Ok(view) => view_requests.push(view),
                    Err(error) => return Outcome::failure(format!("error: {error}")),
                }
                seen_options.insert("--view");
                index += 1;
            }
            value if value.starts_with("--artifact-lens=") => {
                match parse_artifact_lens(&value[16..]) {
                    Ok(lens) => view_requests.push(ExportViewRequest::Artifact(lens)),
                    Err(error) => return Outcome::failure(format!("error: {error}")),
                }
                seen_options.insert("--view");
                index += 1;
            }
            value => {
                return Outcome::failure(format!(
                    "error: unexpected export {format} argument {value}"
                ));
            }
        }
    }
    if let Err(error) = validate_export_options(format, &seen_options, &view_requests) {
        return Outcome::failure(format!("error: {error}"));
    }
    graph_path = match compass_files::BuildGuard::resolve_requested_artifact(&graph_path) {
        Ok(path) => path,
        Err(error) => {
            return Outcome::failure(format!("error: could not resolve graph: {error}"));
        }
    };
    if graph_explicit {
        let output_dir = graph_path.parent().unwrap_or_else(|| Path::new("."));
        if !labels_explicit {
            labels_path = output_dir.join("labels.json");
        }
        if !report_explicit {
            report_path = output_dir.join("GRAPH_REPORT.md");
        }
    }
    if (matches!(format, "json" | "viewer-json" | "workbench-json")
        || (format == "html" && !no_viz))
        && node_limit < 1
    {
        return Outcome::failure("error: --node-limit must be a positive integer".to_owned());
    }
    let mut inputs = match ExportInputs::load(&graph_path) {
        Ok(inputs) => inputs,
        Err(GraphError::NotFound(_)) => {
            return Outcome::failure(format!(
                "error: graph not found: {}. Run /compass <path> first.",
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
    let requested_workbench = !view_requests.is_empty() || format == "workbench-json";
    let result = match format {
        "html" => {
            let path = output_path
                .clone()
                .unwrap_or_else(|| output_dir.join("graph.html"));
            if no_viz {
                if path.exists()
                    && let Err(error) = fs::remove_file(&path)
                {
                    return Outcome::failure(format!(
                        "error: could not remove {}: {error}",
                        path.display()
                    ));
                }
                Ok(ExportOutput::text(
                    "--no-viz: skipped graph.html".to_owned(),
                ))
            } else {
                build_export_workbench(
                    &inputs,
                    &graph_path,
                    node_limit,
                    &view_requests,
                    &ExportViewOptions {
                        direction: view_direction,
                        depth: view_depth,
                        max_nodes: view_max_nodes,
                        max_edges: view_max_edges,
                        include_heuristic,
                        relations: &view_relations,
                        program_path: program_path.as_deref(),
                    },
                )
                .and_then(|model| {
                    let source_navigation = export_source_navigation(&inputs, &graph_path);
                    export_workbench_html(&model, source_navigation.as_ref(), path)
                })
            }
        }
        "json" | "viewer-json" => {
            if requested_workbench {
                build_export_workbench(
                    &inputs,
                    &graph_path,
                    node_limit,
                    &view_requests,
                    &ExportViewOptions {
                        direction: view_direction,
                        depth: view_depth,
                        max_nodes: view_max_nodes,
                        max_edges: view_max_edges,
                        include_heuristic,
                        relations: &view_relations,
                        program_path: program_path.as_deref(),
                    },
                )
                .and_then(|model| serde_json::to_string(&model).map_err(|error| error.to_string()))
                .map(ExportOutput::text)
            } else {
                let options = HtmlOptions {
                    community_labels: (!inputs.labels.is_empty()).then_some(&inputs.labels),
                    member_counts: None,
                    node_limit: Some(node_limit),
                    learning_overlay: None,
                };
                let model = if let Some(community) = community {
                    graph_community_view_model_document(
                        &inputs.document,
                        &inputs.communities,
                        &graph_path,
                        &options,
                        community,
                    )
                    .map_err(|error| error.to_string())
                } else {
                    graph_view_model_document(
                        &inputs.document,
                        &inputs.communities,
                        &graph_path,
                        &options,
                    )
                    .map_err(|error| error.to_string())
                    .and_then(|model| {
                        model.ok_or_else(|| "graph has no renderable community overview".to_owned())
                    })
                };
                model
                    .and_then(|model| {
                        serde_json::to_string(&model).map_err(|error| error.to_string())
                    })
                    .map(ExportOutput::text)
            }
        }
        "workbench-json" => build_export_workbench(
            &inputs,
            &graph_path,
            node_limit,
            &view_requests,
            &ExportViewOptions {
                direction: view_direction,
                depth: view_depth,
                max_nodes: view_max_nodes,
                max_edges: view_max_edges,
                include_heuristic,
                relations: &view_relations,
                program_path: program_path.as_deref(),
            },
        )
        .and_then(|model| serde_json::to_string(&model).map_err(|error| error.to_string()))
        .map(ExportOutput::text),
        "callflow-html" => export_callflow(
            &inputs,
            &graph_path,
            output_path.clone(),
            sections_path.as_deref(),
            &language,
            max_sections,
            diagram_scale,
            max_diagram_nodes,
            max_diagram_edges,
        ),
        "callflow-json" => export_callflow_json(
            &inputs,
            &graph_path,
            sections_path.as_deref(),
            &language,
            max_sections,
            diagram_scale,
            max_diagram_nodes,
            max_diagram_edges,
        )
        .map(ExportOutput::text),
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
        .map(ExportOutput::text)
        .map_err(|error| error.to_string()),
        "graphml" => write_graphml(
            &inputs.document,
            &inputs.communities,
            output_dir.join("graph.graphml"),
        )
        .map(|()| "graph.graphml written - open in Gephi, yEd, or any GraphML tool".to_owned())
        .map(ExportOutput::text)
        .map_err(|error| error.to_string()),
        "neo4j" => export_neo4j(
            &inputs,
            output_dir,
            push_uri.as_deref(),
            &push_user,
            push_password.as_deref(),
        )
        .map(ExportOutput::text),
        "falkordb" => export_falkordb(
            &inputs,
            output_dir,
            push_uri.as_deref(),
            &push_user,
            push_password.as_deref(),
        )
        .map(ExportOutput::text),
        "obsidian" => export_obsidian_cli(&inputs, &obsidian_dir).map(ExportOutput::text),
        "wiki" => export_wiki_cli(&inputs, output_dir).map(ExportOutput::text),
        _ => Err("unsupported export format".to_owned()),
    };
    match result {
        Ok(mut output) => {
            if format == "html" && output_path.is_none() {
                let artifact_directory = graph_path.parent().unwrap_or_else(|| Path::new("."));
                let output_container =
                    compass_files::BuildGuard::output_container_for_artifact(&graph_path);
                if output_container != artifact_directory
                    && let Err(error) = compass_files::BuildGuard::publish_root_artifacts(
                        &output_container,
                        &["graph.html"],
                        true,
                    )
                {
                    return Outcome::failure(format!(
                        "error: could not publish graph.html at the output root: {error}"
                    ));
                }
                if output.html_output.is_some() {
                    output.html_output = Some(output_container.join("graph.html"));
                }
            }
            let outcome = Outcome::success(output.message);
            match (frontend, output.html_output) {
                (Frontend::Compass, Some(path)) => outcome.with_html_output(path),
                _ => outcome,
            }
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn build_export_workbench(
    inputs: &ExportInputs,
    graph_path: &Path,
    node_limit: isize,
    requested_views: &[ExportViewRequest],
    options: &ExportViewOptions<'_>,
) -> Result<WorkbenchModel, String> {
    let default_views = [ExportViewRequest::Code];
    let requests = if requested_views.is_empty() {
        &default_views[..]
    } else {
        requested_views
    };
    let title = export_project_title(inputs, graph_path);
    let graph_identity = graph_artifact_identity(graph_path).map_err(|error| error.to_string())?;
    let labels = (!inputs.labels.is_empty()).then_some(&inputs.labels);
    let html_options = HtmlOptions {
        community_labels: labels,
        member_counts: None,
        node_limit: Some(node_limit),
        learning_overlay: None,
    };
    let analysis = options
        .program_path
        .map(program_commands::load_program)
        .transpose()?;
    let mut ids = std::collections::BTreeMap::<String, usize>::new();
    let mut views = Vec::with_capacity(requests.len());
    for request in requests {
        let (base_id, view) = match request {
            ExportViewRequest::Code => {
                let bundle = graph_view_model_bundle_document(
                    &inputs.document,
                    &inputs.communities,
                    graph_path,
                    &html_options,
                )
                .map_err(|error| error.to_string())?;
                let coverage = if bundle.truncated {
                    WorkbenchCoverage::bounded(
                        bundle.overview.stats.nodes,
                        bundle.overview.stats.edges,
                        true,
                    )
                } else {
                    WorkbenchCoverage::graph(&bundle.overview)
                };
                (
                    "code".to_owned(),
                    WorkbenchView {
                        id: String::new(),
                        title: "Code graph".to_owned(),
                        description: "Repository structure, ownership, and relationships"
                            .to_owned(),
                        coverage,
                        content: WorkbenchViewContent::Code {
                            model: bundle.overview,
                            community_details: bundle.community_details,
                        },
                    },
                )
            }
            ExportViewRequest::Architecture => {
                let model = architecture_view_model(inputs, graph_path, &title)?;
                let coverage = WorkbenchCoverage::bounded(
                    model.statistics.nodes,
                    model.statistics.edges,
                    false,
                );
                (
                    "architecture".to_owned(),
                    WorkbenchView {
                        id: String::new(),
                        title: "Architecture".to_owned(),
                        description: "Subsystems, scopes, and cross-section calls".to_owned(),
                        coverage,
                        content: WorkbenchViewContent::Architecture { model },
                    },
                )
            }
            ExportViewRequest::Call(root) => {
                let graph = build_universal_call_graph(
                    &inputs.document,
                    analysis.as_ref(),
                    &UniversalCallGraphRequest {
                        root: UniversalCallGraphRoot::Symbol {
                            symbol: root.clone(),
                        },
                        direction: options.direction,
                        depth: options.depth,
                        max_nodes: options.max_nodes,
                        max_edges: options.max_edges,
                    },
                )
                .map_err(|error| error.to_string())?;
                let coverage = WorkbenchCoverage {
                    status: if graph.truncated || graph.coverage.partial {
                        WorkbenchCoverageStatus::Partial
                    } else {
                        WorkbenchCoverageStatus::Complete
                    },
                    truncated: graph.truncated,
                    nodes: graph.nodes.len(),
                    edges: graph.edges.len(),
                    limitations: graph.coverage.limitations.clone(),
                };
                (
                    format!("call-{}", safe_output_name(root)),
                    WorkbenchView {
                        id: String::new(),
                        title: format!("Calls · {root}"),
                        description: "Caller and callee evidence around one symbol".to_owned(),
                        coverage,
                        content: WorkbenchViewContent::Call {
                            root: root.clone(),
                            graph,
                        },
                    },
                )
            }
            ExportViewRequest::Impact(root) => {
                let cache = graph_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("cache");
                let engine =
                    open_code_query(graph_path, None, &cache).map_err(|error| error.to_string())?;
                let max_nodes = u32::try_from(options.max_nodes)
                    .map_err(|_| "--max-nodes exceeds the impact query limit".to_owned())?;
                let max_edges = u32::try_from(options.max_edges)
                    .map_err(|_| "--max-edges exceeds the impact query limit".to_owned())?;
                let result = engine
                    .impact(ImpactRequest {
                        symbol: root.clone(),
                        include_heuristic: options.include_heuristic,
                        limits: CodeQueryLimits {
                            max_depth: options.depth,
                            max_nodes,
                            max_edges,
                            ..CodeQueryLimits::default()
                        },
                    })
                    .map_err(|error| error.to_string())?;
                let coverage = WorkbenchCoverage::bounded(
                    result.nodes.len(),
                    result.edges.len(),
                    result.truncated,
                );
                (
                    format!("impact-{}", safe_output_name(root)),
                    WorkbenchView {
                        id: String::new(),
                        title: format!("Impact · {root}"),
                        description: "Inbound code paths that can be affected by a change"
                            .to_owned(),
                        coverage,
                        content: WorkbenchViewContent::Impact {
                            root: root.clone(),
                            result,
                        },
                    },
                )
            }
            ExportViewRequest::Affected(root) => {
                let relations = if options.relations.is_empty() {
                    DEFAULT_AFFECTED_RELATIONS
                        .iter()
                        .map(|relation| (*relation).to_owned())
                        .collect::<Vec<_>>()
                } else {
                    options.relations.to_vec()
                };
                let projection = affected_lens_view_model(
                    &inputs.document,
                    &inputs.communities,
                    labels,
                    root,
                    AffectedLensOptions {
                        relations: &relations,
                        depth: usize::try_from(options.depth).unwrap_or(usize::MAX),
                        max_nodes: options.max_nodes,
                        max_edges: options.max_edges,
                    },
                )
                .map_err(|error| error.to_string())?;
                let coverage = WorkbenchCoverage::bounded(
                    projection.model.stats.nodes,
                    projection.model.stats.edges,
                    projection.truncated,
                );
                (
                    format!("affected-{}", safe_output_name(root)),
                    WorkbenchView {
                        id: String::new(),
                        title: format!("Affected · {root}"),
                        description: "Reverse dependency expansion from the selected node"
                            .to_owned(),
                        coverage,
                        content: WorkbenchViewContent::Affected {
                            root: root.clone(),
                            relations,
                            depth: usize::try_from(options.depth).unwrap_or(usize::MAX),
                            model: projection.model,
                        },
                    },
                )
            }
            ExportViewRequest::History { base, target } => {
                let (base_revision, _, before) =
                    history_commands::load_history_view_model_at(base, node_limit)?;
                let (target_revision, _, after) =
                    history_commands::load_history_view_model_at(target, node_limit)?;
                let summarized = before.stats.aggregated || after.stats.aggregated;
                let coverage = WorkbenchCoverage {
                    status: if summarized {
                        WorkbenchCoverageStatus::Summary
                    } else {
                        WorkbenchCoverageStatus::Complete
                    },
                    truncated: false,
                    nodes: before.stats.nodes.saturating_add(after.stats.nodes),
                    edges: before.stats.edges.saturating_add(after.stats.edges),
                    limitations: if summarized {
                        vec!["At least one historical graph is aggregated by community.".to_owned()]
                    } else {
                        Vec::new()
                    },
                };
                (
                    format!(
                        "history-{}-{}",
                        safe_output_name(base),
                        safe_output_name(target)
                    ),
                    WorkbenchView {
                        id: String::new(),
                        title: format!("History · {base}..{target}"),
                        description: "Structural comparison between immutable realizations"
                            .to_owned(),
                        coverage,
                        content: WorkbenchViewContent::History {
                            base_revision,
                            target_revision,
                            before,
                            after,
                        },
                    },
                )
            }
            ExportViewRequest::Artifact(lens) => {
                let projection = artifact_lens_view_model(
                    &inputs.document,
                    &inputs.communities,
                    labels,
                    *lens,
                    options.max_nodes,
                    options.max_edges,
                );
                let relations = lens.relations();
                let coverage = WorkbenchCoverage::bounded(
                    projection.model.stats.nodes,
                    projection.model.stats.edges,
                    projection.truncated,
                );
                (
                    format!("artifact-{}", lens.key()),
                    WorkbenchView {
                        id: String::new(),
                        title: lens.label().to_owned(),
                        description: format!(
                            "Artifact lens over {} relationship kinds",
                            relations.len()
                        ),
                        coverage,
                        content: WorkbenchViewContent::Artifact {
                            lens: *lens,
                            relations,
                            model: projection.model,
                        },
                    },
                )
            }
        };
        let count = ids.entry(base_id.clone()).or_default();
        *count += 1;
        let id = if *count == 1 {
            base_id
        } else {
            format!("{base_id}-{count}")
        };
        views.push(WorkbenchView { id, ..view });
    }
    Ok(WorkbenchModel::new(title, graph_identity, views))
}

fn export_project_title(inputs: &ExportInputs, graph_path: &Path) -> String {
    inputs
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
        .unwrap_or_else(|| "Compass graph".to_owned())
}

fn architecture_view_model(
    inputs: &ExportInputs,
    graph_path: &Path,
    title: &str,
) -> Result<compass_output::CallflowViewModel, String> {
    callflow_view_model(
        &inputs.document,
        &inputs.communities,
        &CallflowOptions {
            community_labels: (!inputs.labels.is_empty()).then_some(&inputs.labels),
            report: &inputs.report,
            project_name: title,
            ..CallflowOptions::default()
        },
    )
    .map_err(|error| {
        format!(
            "could not build architecture view for {}: {error}",
            graph_path.display()
        )
    })
}

fn export_workbench_html(
    model: &WorkbenchModel,
    source_navigation: Option<&SourceNavigation>,
    path: PathBuf,
) -> Result<ExportOutput, String> {
    write_workbench_html_with_source_navigation(model, source_navigation, &path)
        .map_err(|error| error.to_string())?;
    Ok(ExportOutput::html(
        format!(
            "{} written - open in any browser, no server needed",
            path.display()
        ),
        path,
    ))
}

fn export_source_navigation(inputs: &ExportInputs, graph_path: &Path) -> Option<SourceNavigation> {
    const GIT_SOURCE_LINK_TIMEOUT: Duration = Duration::from_secs(2);
    let revision = inputs
        .document
        .graph
        .get("build")
        .and_then(serde_json::Value::as_object)
        .and_then(|build| build.get("sourceCommit"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            inputs
                .document
                .extras
                .get("built_at_commit")
                .and_then(serde_json::Value::as_str)
        })?;
    let directory = graph_path.parent()?.to_str()?;
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let root = SystemRunner
        .run(
            "git",
            &[
                "-C".to_owned(),
                directory.to_owned(),
                "rev-parse".to_owned(),
                "--show-toplevel".to_owned(),
            ],
            GIT_SOURCE_LINK_TIMEOUT,
        )
        .ok()?;
    if root.code != 0 {
        return None;
    }
    let root = root.stdout.trim();
    if root.is_empty()
        || root
            .chars()
            .any(|character| matches!(character, '\0' | '\n' | '\r'))
    {
        return None;
    }
    let commit_object = format!("{revision}^{{commit}}");
    let commit = SystemRunner
        .run(
            "git",
            &[
                "-C".to_owned(),
                root.to_owned(),
                "cat-file".to_owned(),
                "-e".to_owned(),
                commit_object,
            ],
            GIT_SOURCE_LINK_TIMEOUT,
        )
        .ok()?;
    if commit.code != 0 {
        return None;
    }
    let remote = SystemRunner
        .run(
            "git",
            &[
                "-C".to_owned(),
                root.to_owned(),
                "remote".to_owned(),
                "get-url".to_owned(),
                "origin".to_owned(),
            ],
            GIT_SOURCE_LINK_TIMEOUT,
        )
        .ok()?;
    if remote.code != 0 {
        return None;
    }
    SourceNavigation::from_git_remote(remote.stdout.trim(), revision)
}

#[allow(clippy::too_many_arguments)]
fn export_callflow_json(
    inputs: &ExportInputs,
    graph_path: &Path,
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
    let model = callflow_view_model(
        &inputs.document,
        &inputs.communities,
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
    serde_json::to_string(&model).map_err(|error| error.to_string())
}

struct ExportOutput {
    message: String,
    html_output: Option<PathBuf>,
}

impl ExportOutput {
    fn text(message: String) -> Self {
        Self {
            message,
            html_output: None,
        }
    }

    fn html(message: String, path: PathBuf) -> Self {
        Self {
            message,
            html_output: Some(path),
        }
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
            "compass",
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
            "cypher.txt written ({}) - statements are OpenCypher. FalkorDB's GRAPH.QUERY runs one statement at a time (no bulk script import), so load a graph with: compass export falkordb --push falkordb://localhost:6379",
            path.display()
        ))
    }
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
) -> Result<ExportOutput, String> {
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
    Ok(ExportOutput::html(
        format!(
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
        ),
        path,
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
            "analysis.json is missing or empty — refusing to export wiki to prevent data loss.\nRun `compass extract .` (or `compass cluster-only .`) to regenerate community data first."
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
    "Usage: compass export <format>\n  orientation-json [--graph PATH]\n  html      [--graph PATH] [--output HTML] [VIEW ...]\n  json      [--graph PATH] [--node-limit N] [--community ID] [VIEW ...]\n  workbench-json [--graph PATH] [VIEW ...]\n  callflow-html [GRAPH|DIR] [--graph PATH] [--labels PATH] [--report PATH] [--sections PATH] [--output HTML]\n  callflow-json [GRAPH|DIR] [--graph PATH] [--labels PATH] [--report PATH] [--sections PATH]\n  obsidian  [--graph PATH] [--labels PATH] [--dir PATH]\n  wiki      [--graph PATH] [--labels PATH]\n  svg       [--graph PATH] [--labels PATH]\n  graphml   [--graph PATH]\n  neo4j     [--graph PATH] [--push URI] [--user U] [--password P]\n  falkordb  [--graph PATH] [--push URI] [--user U] [--password P]\n\nVIEW may be repeated: --code-graph, --architecture-graph, --call-graph SYMBOL, --impact-graph SYMBOL, --affected-graph NODE, --history-graph OLD..NEW, --artifact-lens LENS, or --view SPEC.".to_owned()
}

fn export_workbench_help(format: &str) -> String {
    format!(
        "Usage: compass export {format} [--graph PATH] [--labels PATH] [--node-limit N] [--output HTML] [VIEW ...]\n\nViews are emitted in command order into one navigable workbench:\n  --code-graph\n  --architecture-graph\n  --call-graph SYMBOL\n  --impact-graph SYMBOL\n  --affected-graph NODE\n  --history-graph OLD..NEW\n  --artifact-lens dependencies|routes|data|messaging|tests|provenance\n  --view code|architecture|call:SYMBOL|impact:SYMBOL|affected:NODE|history:OLD..NEW|artifact:LENS\n\nView options:\n  --direction callers|callees|both\n  --depth N\n  --max-nodes N\n  --max-edges N\n  --relation RELATION (repeatable; affected views)\n  --include-heuristic (impact views)\n  --program PATH (Program IR enrichment for call views)"
    )
}

fn command_export_orientation_json(args: &[String]) -> Outcome {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return Outcome::success(
            "Usage: compass export orientation-json [--graph PATH]\n\nEmit the versioned Agent Orientation that was atomically published with the selected graph generation."
                .to_owned(),
        );
    }
    let mut requested_graph = default_graph_path();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--graph" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --graph requires a path".to_owned());
                };
                requested_graph = PathBuf::from(value);
                index += 2;
            }
            value if value.starts_with("--graph=") => {
                requested_graph = PathBuf::from(&value[8..]);
                index += 1;
            }
            value => {
                return Outcome::failure(format!(
                    "error: unexpected orientation-json export argument {value}"
                ));
            }
        }
    }
    let graph_path = match compass_files::BuildGuard::resolve_requested_artifact(&requested_graph) {
        Ok(path) => path,
        Err(error) => return Outcome::failure(format!("error: could not resolve graph: {error}")),
    };
    let (graph, graph_digest) =
        match compass_model::code_graph::GraphDocument::load_with_artifact_digest(&graph_path) {
            Ok(loaded) => loaded,
            Err(error) => {
                return Outcome::failure(format!("error: could not load selected graph: {error}"));
            }
        };
    let orientation_path = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("orientation.json");
    const MAX_ORIENTATION_JSON_BYTES: u64 = 1024 * 1024;
    let orientation_json =
        match hook_commands::read_text_bounded(&orientation_path, MAX_ORIENTATION_JSON_BYTES) {
            Ok(orientation_json) => orientation_json,
            Err(error) => {
                return Outcome::failure(format!(
                    "error: coherent orientation artifact is unavailable for {}: {error}",
                    graph_path.display()
                ));
            }
        };
    let orientation = match serde_json::from_str::<AgentOrientation>(&orientation_json) {
        Ok(orientation) => orientation,
        Err(error) => {
            return Outcome::failure(format!("error: invalid orientation artifact: {error}"));
        }
    };
    let graph_identity = format!("sha256:{graph_digest}");
    if let Err(error) = validate_orientation_graph_identity(&orientation, &graph, &graph_identity) {
        return Outcome::failure(format!("error: {error}"));
    }
    match render_orientation_json(&orientation) {
        Ok(json) => Outcome::success(json),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn export_json_help() -> String {
    "Usage: compass export json [--graph PATH] [--labels PATH] [--node-limit N] [--community ID] [VIEW ...]\n  With no VIEW, emit the compatible compass.viewer.graph/1 model.\n  With any VIEW, emit compass.viewer.workbench/1.\n  --community ID exports one complete community and cannot be combined with VIEW.".to_owned()
}

fn callflow_help() -> String {
    "Usage: compass export callflow-html [GRAPH|DIR] [--graph PATH] [--labels PATH]\n  --report PATH          path to GRAPH_REPORT.md\n  --sections PATH        JSON section definitions\n  --output HTML          output path (default compass-out/<project>-callflow.html)\n  --lang LANG            auto, zh-CN, en, etc. (default auto)\n  --max-sections N       maximum auto-derived sections (default 15)\n  --diagram-scale N      Mermaid diagram scale (default 1.0)\n  --max-diagram-nodes N  representative nodes per section (default 18)\n  --max-diagram-edges N  representative edges per section (default 24)".to_owned()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GraphSelection {
    File(PathBuf),
    Commit(String),
}

pub(crate) fn command_natural_query(frontend: Frontend, args: &[String]) -> Outcome {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return Outcome::success(query_help(frontend));
    }
    let (selection, args) = match parse_graph_selection(args) {
        Ok(parsed) => parsed,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let Some(question) = args.first() else {
        return Outcome::failure(query_help(frontend));
    };
    let mut contexts = Vec::new();
    let mut budget = DEFAULT_TEXT_TOKEN_BUDGET;
    let mut page = 1_usize;
    let mode = TraversalMode::Bfs;
    let mut legacy_requested = false;
    let mut discovery_requested = false;
    let mut discovery_text_budget = DEFAULT_TEXT_TOKEN_BUDGET;
    let mut discovery_cursor = None::<String>;
    let mut discovery_text_pagination_requested = false;
    let mut discovery_direction = DiscoveryDirection::Auto;
    let mut discovery_scope = Vec::new();
    let mut discovery_traversal = DiscoveryTraversal::Bfs;
    let mut discovery_include_heuristic = false;
    let mut discovery_limits = DiscoveryLimits::default();
    let mut discovery_format = "text".to_owned();
    let mut discovery_result_envelope = false;
    let mut seen_discovery_options = HashSet::new();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--traverse" => {
                legacy_requested = true;
                index += 1;
            }
            "--dfs" => {
                discovery_traversal = DiscoveryTraversal::Dfs;
                discovery_requested = true;
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
                legacy_requested = true;
                index += 2;
            }
            "--context" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --context requires a value".to_owned());
                };
                contexts.push(value.clone());
                discovery_requested = true;
                index += 2;
            }
            "--page" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --page must be an integer".to_owned());
                };
                let Ok(value) = value.parse::<usize>() else {
                    return Outcome::failure("error: --page must be an integer".to_owned());
                };
                page = value;
                legacy_requested = true;
                index += 2;
            }
            "--text-budget" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --text-budget must be an integer".to_owned());
                };
                let Ok(value) = value.parse::<usize>() else {
                    return Outcome::failure("error: --text-budget must be an integer".to_owned());
                };
                discovery_text_budget = value;
                discovery_text_pagination_requested = true;
                discovery_requested = true;
                index += 2;
            }
            "--cursor" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --cursor requires a value".to_owned());
                };
                discovery_cursor = Some(value.clone());
                discovery_text_pagination_requested = true;
                discovery_requested = true;
                index += 2;
            }
            "--direction" | "--scope" | "--format" => {
                let name = args[index].as_str();
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure(format!("error: {name} requires a value"));
                };
                if name != "--scope" && !seen_discovery_options.insert(name.to_owned()) {
                    return Outcome::failure(format!("error: {name} must not be repeated"));
                }
                if let Err(error) = apply_discovery_option(
                    name,
                    value,
                    &mut discovery_direction,
                    &mut discovery_scope,
                    &mut discovery_format,
                ) {
                    return Outcome::failure(format!("error: {error}"));
                }
                discovery_requested = true;
                index += 2;
            }
            "--include-heuristic" => {
                if !seen_discovery_options.insert("--include-heuristic".to_owned()) {
                    return Outcome::failure(
                        "error: --include-heuristic must not be repeated".to_owned(),
                    );
                }
                discovery_include_heuristic = true;
                discovery_requested = true;
                index += 1;
            }
            "--result-envelope" => {
                if !seen_discovery_options.insert("--result-envelope".to_owned()) {
                    return Outcome::failure(
                        "error: --result-envelope must not be repeated".to_owned(),
                    );
                }
                discovery_result_envelope = true;
                discovery_requested = true;
                index += 1;
            }
            value if is_discovery_limit(value) => {
                if !seen_discovery_options.insert(value.to_owned()) {
                    return Outcome::failure(format!("error: {value} must not be repeated"));
                }
                let Some(raw) = args.get(index + 1) else {
                    return Outcome::failure(format!("error: {value} requires an integer"));
                };
                if let Err(error) = apply_discovery_limit(&mut discovery_limits, value, raw) {
                    return Outcome::failure(format!("error: {error}"));
                }
                discovery_requested = true;
                index += 2;
            }
            value if value.starts_with("--budget=") => {
                let Ok(value) = value[9..].parse::<usize>() else {
                    return Outcome::failure("error: --budget must be an integer".to_owned());
                };
                budget = value;
                legacy_requested = true;
                index += 1;
            }
            value if value.starts_with("--context=") => {
                contexts.push(value[10..].to_owned());
                discovery_requested = true;
                index += 1;
            }
            value if value.starts_with("--page=") => {
                let Ok(value) = value[7..].parse::<usize>() else {
                    return Outcome::failure("error: --page must be an integer".to_owned());
                };
                page = value;
                legacy_requested = true;
                index += 1;
            }
            value if value.starts_with("--text-budget=") => {
                let Ok(value) = value[14..].parse::<usize>() else {
                    return Outcome::failure("error: --text-budget must be an integer".to_owned());
                };
                discovery_text_budget = value;
                discovery_text_pagination_requested = true;
                discovery_requested = true;
                index += 1;
            }
            value if value.starts_with("--cursor=") => {
                discovery_cursor = Some(value[9..].to_owned());
                discovery_text_pagination_requested = true;
                discovery_requested = true;
                index += 1;
            }
            value
                if ["--direction=", "--scope=", "--format="]
                    .iter()
                    .any(|prefix| value.starts_with(prefix)) =>
            {
                let Some((name, option_value)) = value.split_once('=') else {
                    return Outcome::failure("error: invalid discovery option".to_owned());
                };
                if name != "--scope" && !seen_discovery_options.insert(name.to_owned()) {
                    return Outcome::failure(format!("error: {name} must not be repeated"));
                }
                if let Err(error) = apply_discovery_option(
                    name,
                    option_value,
                    &mut discovery_direction,
                    &mut discovery_scope,
                    &mut discovery_format,
                ) {
                    return Outcome::failure(format!("error: {error}"));
                }
                discovery_requested = true;
                index += 1;
            }
            value
                if value
                    .split_once('=')
                    .is_some_and(|(name, _)| is_discovery_limit(name)) =>
            {
                let Some((name, raw)) = value.split_once('=') else {
                    return Outcome::failure("error: invalid discovery limit".to_owned());
                };
                if !seen_discovery_options.insert(name.to_owned()) {
                    return Outcome::failure(format!("error: {name} must not be repeated"));
                }
                if let Err(error) = apply_discovery_limit(&mut discovery_limits, name, raw) {
                    return Outcome::failure(format!("error: {error}"));
                }
                discovery_requested = true;
                index += 1;
            }
            value => {
                return Outcome::failure(format!("error: unexpected query argument {value}"));
            }
        }
    }
    if legacy_requested && discovery_requested {
        return Outcome::failure(
            "error: legacy traversal controls cannot be combined with discovery controls"
                .to_owned(),
        );
    }
    if !legacy_requested {
        if discovery_format == "json" && discovery_text_pagination_requested {
            return Outcome::failure(
                "error: --cursor and --text-budget are text-only and cannot be used with --format json"
                    .to_owned(),
            );
        }
        if discovery_result_envelope && discovery_format != "json" {
            return Outcome::failure("error: --result-envelope requires --format json".to_owned());
        }
        let request = DiscoveryQueryRequest {
            question: question.clone(),
            direction: discovery_direction,
            relation_contexts: contexts,
            scope: discovery_scope,
            traversal: discovery_traversal,
            include_heuristic: discovery_include_heuristic,
            limits: discovery_limits,
        };
        let outcome = command_discovery_query(
            &selection,
            request,
            &discovery_format,
            discovery_text_budget,
            discovery_cursor.as_deref(),
            discovery_result_envelope,
        );
        if outcome.code == 0 {
            touch_selected_query_stamp(&selection);
        }
        return outcome;
    }
    if let Err(error) = validate_text_pagination(budget, page) {
        return Outcome::failure(format!("error: {error}"));
    }
    let loaded = match load_selection(frontend, &selection, false) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    let output = match query_graph_text_page(
        &loaded.graph,
        question,
        mode,
        2,
        TextPageOptions {
            token_budget: budget,
            page,
        },
        &contexts,
        &loaded.overlay,
    ) {
        Ok(output) => output,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    touch_selected_query_stamp(&selection);
    Outcome::success(output)
}

fn apply_discovery_option(
    name: &str,
    value: &str,
    direction: &mut DiscoveryDirection,
    scope: &mut Vec<DiscoveryScope>,
    format: &mut String,
) -> Result<(), String> {
    match name {
        "--direction" => {
            *direction = match value {
                "auto" => DiscoveryDirection::Auto,
                "incoming" => DiscoveryDirection::Incoming,
                "outgoing" => DiscoveryDirection::Outgoing,
                "both" => DiscoveryDirection::Both,
                _ => {
                    return Err("--direction must be auto, incoming, outgoing, or both".to_owned());
                }
            };
        }
        "--scope" => scope.push(parse_discovery_scope(value)?),
        "--format" => {
            if !matches!(value, "text" | "json") {
                return Err("--format must be text or json for discovery queries".to_owned());
            }
            *format = value.to_owned();
        }
        _ => return Err(format!("unsupported discovery option {name}")),
    }
    Ok(())
}

fn parse_discovery_scope(value: &str) -> Result<DiscoveryScope, String> {
    let Some((kind, value)) = value.split_once(':') else {
        return Err(
            "--scope must use kind:value with kind community, source, package, or node".to_owned(),
        );
    };
    if value.is_empty() {
        return Err("--scope value must not be empty".to_owned());
    }
    let kind = match kind {
        "community" => DiscoveryScopeKind::Community,
        "source" => DiscoveryScopeKind::Source,
        "package" => DiscoveryScopeKind::Package,
        "node" => DiscoveryScopeKind::Node,
        _ => {
            return Err("--scope kind must be community, source, package, or node".to_owned());
        }
    };
    Ok(DiscoveryScope {
        kind,
        value: value.to_owned(),
    })
}

fn is_discovery_limit(name: &str) -> bool {
    matches!(
        name,
        "--max-depth"
            | "--max-seeds"
            | "--max-candidates"
            | "--max-nodes"
            | "--max-edges"
            | "--max-expanded-relationships"
            | "--max-response-bytes"
            | "--timeout-ms"
    )
}

fn apply_discovery_limit(
    limits: &mut DiscoveryLimits,
    name: &str,
    raw: &str,
) -> Result<(), String> {
    let value = raw
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{name} requires a positive integer"))?;
    let as_u32 =
        || u32::try_from(value).map_err(|_| format!("{name} requires a positive 32-bit integer"));
    match name {
        "--max-depth" => limits.max_depth = as_u32()?,
        "--max-seeds" => limits.max_seeds = as_u32()?,
        "--max-candidates" => limits.max_candidates = as_u32()?,
        "--max-nodes" => limits.max_nodes = as_u32()?,
        "--max-edges" => limits.max_edges = as_u32()?,
        "--max-expanded-relationships" => limits.max_expanded_relationships = value,
        "--max-response-bytes" => limits.max_response_bytes = value,
        "--timeout-ms" => limits.timeout_ms = value,
        _ => return Err(format!("unsupported discovery limit {name}")),
    }
    Ok(())
}

fn command_discovery_query(
    selection: &GraphSelection,
    request: DiscoveryQueryRequest,
    format: &str,
    text_budget: usize,
    cursor: Option<&str>,
    result_envelope: bool,
) -> Outcome {
    let include_heuristic = request.include_heuristic;
    let execution = match discovery_query(selection, request) {
        Ok(response) => response,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    if format == "json" {
        let output = if result_envelope {
            let envelope = match compass_query::discovery_result_envelope(execution.response) {
                Ok(envelope) => envelope,
                Err(error) => return Outcome::failure(format!("error: {error}")),
            };
            serde_json::to_string_pretty(&envelope)
        } else {
            serde_json::to_string_pretty(&execution.response)
        };
        match output {
            Ok(output) => Outcome::success(output),
            Err(error) => Outcome::failure(format!("error: {error}")),
        }
    } else {
        let request_digest = match discovery_request_digest(&execution.response, include_heuristic)
        {
            Ok(digest) => digest,
            Err(error) => return Outcome::failure(format!("error: {error}")),
        };
        match render_discovery_text_page(
            &execution.response,
            DiscoveryTextPageOptions {
                token_budget: text_budget,
                cursor,
                request_digest: &request_digest,
                graph_identity: &execution.graph_identity,
                graph_digest: &execution.graph_digest,
            },
        ) {
            Ok(page) => Outcome::success(page.text),
            Err(error) => Outcome::failure(format!("error: {error}")),
        }
    }
}

struct DiscoveryExecution {
    response: DiscoveryQueryResponse,
    graph_identity: String,
    graph_digest: String,
}

fn discovery_query(
    selection: &GraphSelection,
    request: DiscoveryQueryRequest,
) -> Result<DiscoveryExecution, String> {
    match selection {
        GraphSelection::File(path) => {
            let graph = compass_files::BuildGuard::resolve_requested_artifact(path)
                .map_err(|error| error.to_string())?;
            let cache = graph
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("cache");
            let engine =
                open_code_query(&graph, None, &cache).map_err(|error| error.to_string())?;
            let graph_identity = engine.build_generation_identity().to_owned();
            let graph_digest = engine.graph_identity().to_owned();
            let response = engine
                .discover(request)
                .map_err(|error| error.to_string())?;
            Ok(DiscoveryExecution {
                response,
                graph_identity,
                graph_digest,
            })
        }
        GraphSelection::Commit(revision) => {
            let (realization, document) = history_commands::load_typed_graph_at(revision)?;
            let current = std::env::current_dir().map_err(|error| error.to_string())?;
            let cache = current
                .join(".compass")
                .join("cache")
                .join("history-query")
                .join(realization.to_string());
            let graph_path = current
                .join(".compass")
                .join("history-query")
                .join(realization.to_string())
                .join("graph.json");
            let engine = open_with_verified_document(
                document,
                realization.as_hex(),
                &graph_path,
                None,
                &cache,
            )
            .map_err(|error| error.to_string())?;
            let graph_identity = engine.build_generation_identity().to_owned();
            let graph_digest = engine.graph_identity().to_owned();
            let response = engine
                .discover(request)
                .map_err(|error| error.to_string())?;
            Ok(DiscoveryExecution {
                response,
                graph_identity,
                graph_digest,
            })
        }
    }
}

fn command_path(frontend: Frontend, args: &[String]) -> Outcome {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return Outcome::success(path_help(frontend));
    }
    let (selection, args) = match parse_graph_selection(args) {
        Ok(parsed) => parsed,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    if args.len() != 2 {
        return Outcome::failure(path_help(frontend));
    }
    let loaded = match load_selection(frontend, &selection, true) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    match render_shortest_path(&loaded.graph, &args[0], &args[1]) {
        Ok(output) => {
            touch_selected_query_stamp(&selection);
            Outcome::success(output)
        }
        Err(error) => Outcome::failure(error),
    }
}

fn command_explain(frontend: Frontend, args: &[String]) -> Outcome {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return Outcome::success(explain_help(frontend));
    }
    let (selection, args) = match parse_graph_selection(args) {
        Ok(parsed) => parsed,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let Some(label) = args.first() else {
        return Outcome::failure(explain_help(frontend));
    };
    let mut budget = DEFAULT_TEXT_TOKEN_BUDGET;
    let mut page = 1_usize;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
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
            "--page" => {
                let Some(value) = args.get(index + 1) else {
                    return Outcome::failure("error: --page must be an integer".to_owned());
                };
                let Ok(value) = value.parse::<usize>() else {
                    return Outcome::failure("error: --page must be an integer".to_owned());
                };
                page = value;
                index += 2;
            }
            value if value.starts_with("--budget=") => {
                let Ok(value) = value[9..].parse::<usize>() else {
                    return Outcome::failure("error: --budget must be an integer".to_owned());
                };
                budget = value;
                index += 1;
            }
            value if value.starts_with("--page=") => {
                let Ok(value) = value[7..].parse::<usize>() else {
                    return Outcome::failure("error: --page must be an integer".to_owned());
                };
                page = value;
                index += 1;
            }
            value => {
                return Outcome::failure(format!("error: unexpected explain argument {value}"));
            }
        }
    }
    if let Err(error) = validate_text_pagination(budget, page) {
        return Outcome::failure(format!("error: {error}"));
    }
    let loaded = match load_selection(frontend, &selection, true) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    let output = match render_explanation_page(&loaded.graph, label, budget, page, &loaded.overlay)
    {
        Ok(output) => output,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    touch_selected_query_stamp(&selection);
    Outcome::success(output)
}

fn validate_text_pagination(token_budget: usize, page: usize) -> Result<(), String> {
    if token_budget == 0 {
        return Err("token budget must be greater than zero".to_owned());
    }
    if page == 0 {
        return Err("page must be greater than zero".to_owned());
    }
    Ok(())
}

fn command_affected(args: &[String]) -> Outcome {
    let Some(query) = args.first() else {
        return Outcome::failure(
            "Usage: compass affected \"<node-or-label>\" [--relation R] [--depth N] [--graph path]"
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
    let loaded = match load_affected(&graph_path) {
        Ok(loaded) => loaded,
        Err(outcome) => return outcome,
    };
    Outcome::success(format_affected(&loaded.graph, query, &relations, depth))
}

pub(crate) fn parse_graph_selection(
    args: &[String],
) -> Result<(GraphSelection, Vec<String>), String> {
    let mut selection = None;
    let mut remaining = Vec::new();
    let mut options = true;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--" if options => options = false,
            "--graph" if options => {
                index += 1;
                let value = args.get(index).ok_or("--graph requires a path")?;
                if value.is_empty() || value.starts_with('-') {
                    return Err("--graph requires a path".to_owned());
                }
                set_graph_selection(&mut selection, GraphSelection::File(value.into()))?;
            }
            "--at" if options => {
                index += 1;
                let value = args.get(index).ok_or("--at requires a revision")?;
                if value.is_empty() || value.starts_with('-') {
                    return Err("--at requires a revision".to_owned());
                }
                set_graph_selection(&mut selection, GraphSelection::Commit(value.clone()))?;
            }
            value if options && value.starts_with("--graph=") => {
                let value = &value[8..];
                if value.is_empty() {
                    return Err("--graph requires a path".to_owned());
                }
                set_graph_selection(&mut selection, GraphSelection::File(value.into()))?;
            }
            value if options && value.starts_with("--at=") => {
                let value = &value[5..];
                if value.is_empty() {
                    return Err("--at requires a revision".to_owned());
                }
                set_graph_selection(&mut selection, GraphSelection::Commit(value.to_owned()))?;
            }
            value => remaining.push(value.to_owned()),
        }
        index += 1;
    }
    Ok((
        selection.unwrap_or_else(|| GraphSelection::File(default_graph_path())),
        remaining,
    ))
}

fn set_graph_selection(
    selected: &mut Option<GraphSelection>,
    value: GraphSelection,
) -> Result<(), String> {
    let Some(existing) = selected else {
        *selected = Some(value);
        return Ok(());
    };
    let message = if std::mem::discriminant(existing) == std::mem::discriminant(&value) {
        "graph source selector may only be specified once"
    } else {
        "--graph and --at are mutually exclusive"
    };
    Err(message.to_owned())
}

pub(crate) fn load_selection(
    frontend: Frontend,
    selection: &GraphSelection,
    force_directed: bool,
) -> Result<LoadedGraph, Outcome> {
    match selection {
        GraphSelection::File(path) => load(path, force_directed),
        GraphSelection::Commit(revision) => {
            history_commands::load_graph_at(frontend, revision, force_directed)
                .map_err(|error| Outcome::failure(format!("error: {error}")))
        }
    }
}

pub(crate) fn load_indexed_selection(
    frontend: Frontend,
    selection: &GraphSelection,
) -> Result<LoadedGraph, Outcome> {
    match selection {
        GraphSelection::File(path) => {
            let path =
                compass_files::BuildGuard::resolve_requested_artifact(path).map_err(|error| {
                    Outcome::failure(format!("error: could not resolve graph: {error}"))
                })?;
            let graph = compass_model::Graph::load_directed(&path).map_err(graph_load_outcome)?;
            Ok(LoadedGraph {
                graph,
                overlay: HashMap::new(),
            })
        }
        GraphSelection::Commit(revision) => {
            history_commands::load_graph_at(frontend, revision, true)
                .map_err(|error| Outcome::failure(format!("error: {error}")))
        }
    }
}

fn touch_selected_query_stamp(selection: &GraphSelection) {
    if let GraphSelection::File(path) = selection {
        integration_commands::touch_query_stamp(path);
    }
}

fn query_help(frontend: Frontend) -> String {
    let prefix = frontend_name(frontend);
    let help = format!(
        "Usage: {prefix} query \"<question>\" [--direction auto|incoming|outgoing|both] [--scope KIND:VALUE] [--context VALUE] [--dfs] [--format text|json] [--graph PATH|--at REV]\n\nNatural discovery options (default for a typed graph):\n  --direction <VALUE>               Direction: auto, incoming, outgoing, or both [default: auto]\n  --scope <KIND:VALUE>              Repeatable OR scope; KIND is community, source, package, or node\n  --context <VALUE>                 Repeatable strict relationship-context filter\n  --dfs                             Use depth-first expansion [default: breadth-first]\n  --include-heuristic               Include heuristic evidence [default: excluded]\n  --format <text|json>              Discovery output [default: text]\n  --text-budget <N>                 Approximate tokens in one text page [default: 2000]\n  --cursor <TOKEN>                  Continue the same immutable semantic result (text only)\n  --max-depth <N>                   Traversal depth [default: 2; hard maximum: 8]\n  --max-seeds <N>                   Ranked seed count [default: 3; hard maximum: 3]\n  --max-candidates <N>              Ranked candidate count [default/hard maximum: 256]\n  --max-nodes <N>                   Returned node count [default/hard maximum: 500]\n  --max-edges <N>                   Returned edge count [default/hard maximum: 1000]\n  --max-expanded-relationships <N>  Examined relationships [default/hard maximum: 10000]\n  --max-response-bytes <N>          Serialized response bytes [default/hard maximum: 8388608]\n  --timeout-ms <N>                  Discovery deadline in milliseconds [default/hard maximum: 30000]\n\nLegacy traversal options:\n  --traverse                        Force legacy relevance traversal\n  --budget <N>                      Approximate tokens per page [default: 2000]\n  --page <N>                        Result page, starting at 1 [default: 1]\n\nGraph selection:\n  --graph <PATH>                    Read a graph JSON file\n  --at <REV>                        Resolve REV once to an immutable typed realization; conflicts with --graph\n\nCompassQL options:\n  --cql                             Use CompassQL mode\n  --timeout-ms <N>                  CompassQL execution timeout\n  --max-expanded-relationships <N>  CompassQL relationship expansion limit\n  Run `{prefix} help query` for all CompassQL controls and examples.\n\nDiscovery limits must be positive; values above a hard maximum are rejected rather than clamped. JSON rejects text pagination controls. Legacy --traverse, --budget, and --page cannot be mixed with discovery controls."
    );
    let help = help
        .replace(
            "default/hard maximum: 500",
            "default: 64; hard maximum: 500",
        )
        .replace(
            "default/hard maximum: 1000",
            "default: 128; hard maximum: 1000",
        );
    format!(
        "{help}\n  --result-envelope                 Wrap JSON with a query-owned semantic digest"
    )
}

fn path_help(frontend: Frontend) -> String {
    let prefix = frontend_name(frontend);
    format!("Usage: {prefix} path \"<source>\" \"<target>\" [--graph PATH|--at REV]")
}

fn explain_help(frontend: Frontend) -> String {
    let prefix = frontend_name(frontend);
    format!("Usage: {prefix} explain \"<node>\" [--budget N] [--page N] [--graph PATH|--at REV]")
}

fn frontend_name(frontend: Frontend) -> &'static str {
    let _ = frontend;
    "compass"
}

fn load(path: &Path, force_directed: bool) -> Result<LoadedGraph, Outcome> {
    let path = compass_files::BuildGuard::resolve_requested_artifact(path)
        .map_err(|error| Outcome::failure(format!("error: could not resolve graph: {error}")))?;
    let result = if force_directed {
        LoadedGraph::load_directed(&path)
    } else {
        LoadedGraph::load(&path)
    };
    result.map_err(graph_load_outcome)
}

fn load_affected(path: &Path) -> Result<LoadedGraph, Outcome> {
    let path = compass_files::BuildGuard::resolve_requested_artifact(path)
        .map_err(|error| Outcome::failure(format!("error: could not resolve graph: {error}")))?;
    LoadedGraph::load_for_affected(&path).map_err(graph_load_outcome)
}

fn graph_load_outcome(error: GraphError) -> Outcome {
    match error {
        GraphError::NotFound(path) => {
            Outcome::failure(format!("error: graph file not found: {}", path.display()))
        }
        GraphError::InvalidExtension(_) => {
            Outcome::failure("error: graph file must be a .json file".to_owned())
        }
        other => Outcome::failure(format!("error: could not load graph: {other}")),
    }
}

fn watch_help() -> String {
    "Usage: compass watch [PATH] [--program] [--program-artifact PATH] [--no-program] [--debounce SECONDS] [--store json|sqlite] [--inference-level low|medium|high|max] [--out DIR] [--no-cluster] [--no-viz] [--no-gitignore] [--exclude PATTERN] [--poll]"
        .to_owned()
}

fn format_program_analysis(result: &BuildResult) -> String {
    format!(
        "Program analysis: {} syntax analyzed, {} syntax reused, {} artifacts loaded, {} artifacts reused, {} artifact documents analyzed, {} artifact documents reused, {} modules, {} summaries, {} conflicts",
        result.program_syntax_analyzed,
        result.program_syntax_reused,
        result.program_artifacts_loaded,
        result.program_artifacts_reused,
        result.program_artifact_documents_analyzed,
        result.program_artifact_documents_reused,
        result.program_modules,
        result.program_summaries,
        result.program_conflicts
    )
}

fn format_partial_graph_warning(result: &BuildResult) -> Option<String> {
    result.partial_graph.then(|| {
        if result.resolution_degraded {
            format!(
                "error: Compass published a bounded partial graph because universal collection resolution omitted {} relationship candidates; declarations remain available. See graph diagnostic 'universal_resolution_partial'.",
                result.omitted_edges
            )
        } else {
            format!(
                "warning: Compass published a partial graph after omitting {} nodes and {} edges; {} identity collisions quarantined.",
                result.omitted_nodes, result.omitted_edges, result.identity_collisions
            )
        }
    })
}

fn apply_build_quality_outcome(result: &BuildResult, outcome: &mut Outcome) {
    if let Some(warning) = format_partial_graph_warning(result) {
        outcome.stderr = warning;
    }
    if result.resolution_degraded {
        outcome.code = 1;
    }
}

#[cfg(test)]
mod mcp_option_tests {
    use super::*;
    use std::io::Cursor;

    fn sample_build_result(outputs_changed: bool, html_written: bool) -> BuildResult {
        BuildResult {
            root: PathBuf::from("project"),
            output_dir: PathBuf::from("project/compass-out"),
            detection: compass_files::Detection {
                files: std::collections::BTreeMap::new(),
                total_files: 2,
                total_words: 0,
                needs_graph: true,
                warning: None,
                skipped_sensitive: Vec::new(),
                unclassified: Vec::new(),
                walk_errors: Vec::new(),
                ignored: Vec::new(),
                compassignore_patterns: 0,
                scan_root: "project".to_owned(),
                google_workspace_shortcuts: Vec::new(),
            },
            files_considered: 2,
            files_extracted: 1,
            files_cached: 1,
            empty_files: Vec::new(),
            nodes: 3,
            edges: 2,
            communities: 1,
            omitted_nodes: 0,
            omitted_edges: 0,
            identity_collisions: 0,
            partial_graph: false,
            resolution_degraded: false,
            html_written,
            outputs_changed,
            program_modules: 0,
            program_summaries: 0,
            program_syntax_analyzed: 0,
            program_syntax_reused: 0,
            program_artifacts_loaded: 0,
            program_artifacts_reused: 0,
            program_artifact_documents_analyzed: 0,
            program_artifact_documents_reused: 0,
            program_conflicts: 0,
            timings: BuildTimings::default(),
        }
    }

    #[test]
    fn partial_graph_warning_discloses_exact_omission_counts() {
        let mut result = sample_build_result(true, false);
        result.partial_graph = true;
        result.omitted_nodes = 3;
        result.omitted_edges = 5;
        result.identity_collisions = 2;

        assert_eq!(
            format_partial_graph_warning(&result).as_deref(),
            Some(
                "warning: Compass published a partial graph after omitting 3 nodes and 5 edges; 2 identity collisions quarantined."
            )
        );
    }

    #[test]
    fn degraded_resolution_is_a_non_success_with_a_machine_diagnostic_pointer() {
        let mut result = sample_build_result(true, false);
        result.partial_graph = true;
        result.resolution_degraded = true;
        result.omitted_edges = 7;
        let mut outcome = Outcome::success("graph published".to_owned());

        apply_build_quality_outcome(&result, &mut outcome);

        assert_eq!(outcome.code, 1);
        assert_eq!(
            outcome.stderr,
            "error: Compass published a bounded partial graph because universal collection resolution omitted 7 relationship candidates; declarations remain available. See graph diagnostic 'universal_resolution_partial'."
        );
    }

    #[test]
    fn html_open_confirmation_requires_explicit_yes() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let html = directory.path().join("report.html");
        fs::write(&html, "<!doctype html>")?;
        let outcome = Outcome::success("report written".to_owned()).with_html_output(html.clone());
        let mut input = Cursor::new(b"yes\n");
        let mut prompt = Vec::new();
        let mut opened = Vec::new();

        assert!(prompt_to_open_html_with(
            &outcome,
            &mut input,
            &mut prompt,
            true,
            true,
            |path| {
                opened.push(path.to_path_buf());
                Ok(())
            },
        )?);
        assert_eq!(opened, [absolutize(html)]);
        assert!(String::from_utf8(prompt)?.contains("Open "));
        Ok(())
    }

    #[test]
    fn html_open_confirmation_is_safe_for_default_and_noninteractive_runs()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let html = directory.path().join("report.html");
        fs::write(&html, "<!doctype html>")?;
        let outcome = Outcome::success("report written".to_owned()).with_html_output(html);
        let mut opened = false;
        let mut input = Cursor::new(b"\n");
        let mut prompt = Vec::new();
        assert!(!prompt_to_open_html_with(
            &outcome,
            &mut input,
            &mut prompt,
            true,
            true,
            |_| {
                opened = true;
                Ok(())
            },
        )?);
        assert!(!opened);

        let mut input = Cursor::new(b"yes\n");
        let mut prompt = Vec::new();
        assert!(!prompt_to_open_html_with(
            &outcome,
            &mut input,
            &mut prompt,
            false,
            false,
            |_| {
                opened = true;
                Ok(())
            },
        )?);
        assert!(!opened);
        assert!(prompt.is_empty());
        Ok(())
    }

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
        let options = parse_mcp_options(&args)?
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
        let options = parse_mcp_options(&args)?
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
    fn compass_version_reports_the_package_version() {
        let outcome = run(Frontend::Compass, [OsString::from("--version")]);
        assert_eq!(outcome.code, 0);
        assert_eq!(
            outcome.stdout,
            format!("compass {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn compass_unknown_command_matches_the_legacy_diagnostic() {
        let outcome = run(Frontend::Compass, [OsString::from("not-a-command")]);
        assert_eq!(outcome.code, 1);
        assert_eq!(
            outcome.stderr,
            "error: unknown command 'not-a-command'\nRun 'compass --help' for usage."
        );
    }

    #[test]
    fn watch_statuses_preserve_native_output_contract() -> Result<(), Box<dyn std::error::Error>> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        write_watch_status(
            WatchStatus::Starting {
                root: PathBuf::from("project"),
                includes: 1,
                excludes: 2,
                output: PathBuf::from("project/compass-out"),
            },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::Backend {
                backend: WatchBackend::Native,
                fallback_error: None,
                poll_interval: Duration::from_millis(500),
            },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(WatchStatus::Synchronizing, &mut stdout, &mut stderr);
        write_watch_status(
            WatchStatus::Watching {
                root: PathBuf::from("project"),
                debounce: Duration::from_millis(1500),
            },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::Settling {
                paths: 2,
                quiet_window: Duration::from_millis(150),
                maximum_window: Duration::from_millis(750),
            },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::Building {
                reason: WatchBuildReason::Changes,
            },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::Batch {
                paths: vec![PathBuf::from("src/lib.rs"), PathBuf::from("notes.md")],
                deterministic: 1,
                semantic: 1,
            },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::UpToDate {
                reason: WatchBuildReason::Reconciliation,
            },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::FollowUpQueued { paths: 1 },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::RetryScheduled {
                delay: Duration::from_secs(2),
                error: "temporarily blocked".to_owned(),
                repeated: 1,
            },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(WatchStatus::Finishing, &mut stdout, &mut stderr);
        write_watch_status(
            WatchStatus::Rebuilt(Box::new(sample_build_result(true, true))),
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::SemanticUpdateRequired {
                flag: PathBuf::from("project/compass-out/needs_update"),
            },
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::EventError("event failed".to_owned()),
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(
            WatchStatus::RebuildError("build failed".to_owned()),
            &mut stdout,
            &mut stderr,
        );
        write_watch_status(WatchStatus::Stopped, &mut stdout, &mut stderr);

        let stdout = String::from_utf8(stdout)?;
        let stderr = String::from_utf8(stderr)?;
        assert!(stdout.contains("Starting project (scope: 1 include, 2 exclude"));
        assert!(stdout.contains("Native filesystem events active"));
        assert!(stdout.contains("Synchronizing current graph"));
        assert!(stdout.contains("[compass watch] Watching project"));
        assert!(stdout.contains("settling for 0.15s (maximum 0.75s)"));
        assert!(stdout.contains("Building (filesystem changes)"));
        assert!(stdout.contains("2 file(s) changed (1 deterministic, 1 semantic)"));
        assert!(stdout.contains("3 nodes, 2 edges, 1 communities (1 extracted, 1 cached)"));
        assert!(stdout.contains("Up to date after periodic reconciliation"));
        assert!(stdout.contains("one follow-up queued"));
        assert!(stdout.contains("Finishing the active atomic build"));
        assert!(stdout.contains("Flag written to project/compass-out/needs_update"));
        assert!(stdout.contains("[compass watch] Stopped."));
        assert!(stderr.contains("Filesystem event error: event failed"));
        assert!(stderr.contains("Rebuild failed: build failed"));
        assert!(stderr.contains("retrying in 2s: temporarily blocked"));

        let mut logged = Vec::new();
        write_watch_status_mode(
            Frontend::Compass,
            WatchStatus::Synchronizing,
            &mut logged,
            &mut Vec::new(),
            false,
        );
        let logged = String::from_utf8(logged)?;
        assert!(logged.contains("Z] [compass watch] Synchronizing current graph"));
        Ok(())
    }

    #[test]
    fn parser_numeric_url_path_and_export_helpers_cover_boundary_values()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(grouped_decimal(1_234_567), "1_234_567");
        assert_eq!(python_float_repr(f64::NAN), "nan");
        assert_eq!(python_float_repr(f64::INFINITY), "inf");
        assert_eq!(python_float_repr(f64::NEG_INFINITY), "-inf");
        assert_eq!(python_float_repr(2.0), "2.0");
        assert_eq!(python_float_repr(2.5), "2.5");
        assert!(provider_url_is_loopback("http://localhost:11434"));
        assert!(provider_url_is_loopback("http://[::1]:11434"));
        assert!(!provider_url_is_loopback("not a url"));
        assert!(!provider_url_is_loopback("https://example.com"));
        assert_eq!(python_string_repr("a'b\\c"), "'a\\'b\\\\c'");
        assert_eq!(parse_positive_usize("2", "--count")?, 2);
        assert!(parse_positive_usize("0", "--count").is_err());
        assert!(parse_positive_usize("bad", "--count").is_err());
        assert_eq!(parse_positive_f64("0.5", "--rate")?, 0.5);
        assert!(parse_positive_f64("0", "--rate").is_err());
        assert_eq!(safe_output_name(" /Project: One/ "), "Project--One");
        assert_eq!(safe_output_name("///"), "project");
        assert_eq!(parse_usize(Some("7".to_owned()), "value"), Some(7));
        assert_eq!(parse_usize(Some("bad".to_owned()), "value"), None);
        assert_eq!(parse_usize(None, "value"), None);

        let directory = tempfile::tempdir()?;
        let map_path = directory.path().join("map.json");
        fs::write(
            &map_path,
            r#"{"labels":{"0":"Zero","1":{"name":"One"},"2":{"title":"Two"},"bad":"skip","3":7}}"#,
        )?;
        let labels = load_usize_string_map(&map_path)?;
        assert_eq!(labels.len(), 3);
        assert_eq!(labels.get(&1).map(String::as_str), Some("One"));
        fs::write(&map_path, "[]")?;
        assert!(load_usize_string_map(&map_path)?.is_empty());
        fs::write(&map_path, "not json")?;
        assert!(load_usize_string_map(&map_path).is_err());
        assert!(load_usize_string_map(&directory.path().join("missing")).is_err());

        let sections = directory.path().join("sections.json");
        fs::write(&sections, r#"{"sections":[]}"#)?;
        assert!(load_sections(&sections)?.is_empty());
        fs::write(&sections, "{}")?;
        assert!(load_sections(&sections).is_err());
        Ok(())
    }

    #[test]
    fn watch_tree_and_cluster_parsers_cover_value_forms() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let root = directory.path().to_string_lossy().into_owned();
        let args = vec![
            root.clone(),
            "--debounce".to_owned(),
            "0.25".to_owned(),
            "--out".to_owned(),
            "artifacts".to_owned(),
            "--inference-level".to_owned(),
            "high".to_owned(),
            "--exclude".to_owned(),
            "target".to_owned(),
            "--poll".to_owned(),
        ];
        let watch = parse_watch_options(&args)?.ok_or("unexpected help")?;
        assert_eq!(watch.debounce, Duration::from_millis(250));
        assert_eq!(watch.build.output_root, Some(PathBuf::from("artifacts")));
        assert_eq!(watch.build.inference_level, InferenceLevel::High);
        assert_eq!(watch.build.extra_excludes, ["target"]);
        assert!(watch.force_polling);
        assert!(watch.adaptive);

        let defaults = parse_watch_options(&[])?.ok_or("unexpected help")?;
        assert_eq!(defaults.debounce, Duration::from_millis(150));
        assert!(defaults.adaptive);
        assert!(!defaults.build.program_analysis);

        let program = parse_watch_options(&["--program".to_owned()])?.ok_or("unexpected help")?;
        assert!(program.build.program_analysis);

        assert_eq!(
            command_cluster_only(Frontend::Compass, &["--help".to_owned()]).code,
            0
        );
        for args in [
            vec!["--resolution".to_owned()],
            vec!["--exclude-hubs".to_owned()],
            vec!["--resolution".to_owned(), "bad".to_owned()],
            vec!["--exclude-hubs".to_owned(), "bad".to_owned()],
        ] {
            assert_eq!(command_cluster_only(Frontend::Compass, &args).code, 1);
        }
        assert_eq!(
            command_tree(
                Frontend::Compass,
                &["--max-children".to_owned(), "bad".to_owned()]
            )
            .code,
            1
        );
        assert_eq!(
            command_tree(
                Frontend::Compass,
                &["--top-k-edges".to_owned(), "bad".to_owned()]
            )
            .code,
            1
        );

        Ok(())
    }

    #[test]
    fn absent_worker_override_preserves_host_sized_default() {
        let mut options = BuildOptions::new(".");
        assert_eq!(options.max_workers, None);
        apply_max_workers_override(&mut options, None);
        assert_eq!(options.max_workers, None);
        apply_max_workers_override(&mut options, Some(4));
        assert_eq!(options.max_workers, Some(4));
    }

    #[test]
    fn source_size_limit_parser_rejects_zero_negative_and_invalid_values() {
        assert_eq!(
            parse_positive_u64("16777216", "--max-source-bytes"),
            Ok(16_777_216)
        );
        for invalid in ["0", "-1", "large"] {
            assert!(parse_positive_u64(invalid, "--max-source-bytes").is_err());
        }
        assert!(extract_help().contains("[--max-source-bytes N]"));
    }

    #[test]
    fn inference_level_parser_is_closed_and_defaults_to_low()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(InferenceLevel::default(), InferenceLevel::Low);
        assert_eq!(parse_inference_level("low")?, InferenceLevel::Low);
        assert_eq!(parse_inference_level("medium")?, InferenceLevel::Medium);
        assert_eq!(parse_inference_level("high")?, InferenceLevel::High);
        assert_eq!(parse_inference_level("max")?, InferenceLevel::Max);
        for invalid in ["", "none", "maximum", "HIGH"] {
            assert!(parse_inference_level(invalid).is_err());
        }
        assert!(watch_help().contains("--inference-level low|medium|high|max"));
        assert!(extract_help().contains("--inference-level low|medium|high|max"));
        Ok(())
    }
}
