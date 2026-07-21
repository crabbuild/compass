use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use regex::Regex;
use serde_json::{Map, Value};
use trail_files::write_text_atomic;
use trail_global::{GlobalError, GlobalPaths, global_add, global_list, global_remove};

use crate::{Frontend, Outcome};

const MERGE_MAX_BYTES: u64 = 50 * 1024 * 1024;
const MERGE_MAX_NODES: usize = 100_000;
const MANIFEST_MAX_BYTES: u64 = 2_000_000;
const SESSION_ID_MAX_CHARS: usize = 64;

const SEARCH_NUDGE_TEXT: &str = "MANDATORY: graphify-out/graph.json exists. You MUST run `graphify query \"<question>\"` before grepping raw files. Only grep after graphify has oriented you, or to modify/debug specific lines.";
const READ_NUDGE_TEXT: &str = "MANDATORY: graphify-out/graph.json exists. You MUST run graphify before reading source files. Use: `graphify query \"<question>\"` (scoped subgraph), `graphify explain \"<concept>\"`, or `graphify path \"<A>\" \"<B>\"`. Only read raw files after graphify has oriented you, or to modify/debug specific lines. This rule applies to subagents too — include it in every subagent prompt involving code exploration.";
const READ_STALE_TEXT: &str = "graphify-out/graph.json exists but may be STALE for this file (the file changed after the last build). Prefer `graphify query \"<question>\"` for orientation, and run `graphify update` to refresh the graph. Reading the file directly is fine.";
const READ_DENY_TEXT: &str = "graphify strict mode: this project has a fresh knowledge graph that covers this file. Run `graphify query \"<your question>\"` (or `graphify explain` / `graphify path`) FIRST to orient yourself, then re-issue this Read — it will be allowed. This block fires at most once per session; reading raw files to modify or debug specific lines is fine after one query. Apply the same rule in any subagent prompt that explores code.";
const GEMINI_NUDGE_TEXT: &str = "graphify: knowledge graph at graphify-out/. For focused questions, run `graphify query \"<question>\"` (scoped subgraph, usually much smaller than GRAPH_REPORT.md) instead of grepping raw files. Read GRAPH_REPORT.md only for broad architecture context.";
const SOURCE_EXTENSIONS: &[&str] = &[
    "py", "js", "cjs", "ts", "tsx", "jsx", "astro", "vue", "svelte", "go", "rs", "java", "rb", "c",
    "h", "cpp", "hpp", "cc", "cs", "kt", "swift", "php", "scala", "lua", "sh", "md", "rst", "txt",
    "mdx",
];

pub(super) fn command_hook_check(_frontend: Frontend, _args: &[String]) -> Outcome {
    Outcome::success(String::new())
}

pub(super) fn command_global(frontend: Frontend, args: &[String]) -> Outcome {
    let paths = match GlobalPaths::discover() {
        Ok(paths) => paths,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    match args.first().map(String::as_str).unwrap_or_default() {
        "add" => global_add_command(frontend, args, &paths),
        "remove" => global_remove_command(frontend, args, &paths),
        "list" => {
            let loaded = global_list(&paths);
            let repos = loaded
                .value
                .get("repos")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let stdout = if repos.is_empty() {
                "Global graph is empty. Use 'graphify global add' to add a project.".to_owned()
            } else {
                let mut lines = vec![format!("Global graph: {}", paths.graph.display())];
                for (tag, value) in repos {
                    let nodes = value
                        .get("node_count")
                        .map(python_string_value)
                        .unwrap_or_else(|| "?".to_owned());
                    let added = value.get("added_at").and_then(Value::as_str).unwrap_or("?");
                    lines.push(format!(
                        "  {tag}: {nodes} nodes, added {}",
                        added.get(..10).unwrap_or(added)
                    ));
                }
                lines.join("\n")
            };
            outcome_with_warnings(stdout, loaded.warnings)
        }
        "path" => Outcome::success(paths.graph.display().to_string()),
        _ => Outcome::failure(global_help(frontend)),
    }
}

pub(super) fn command_clone(frontend: Frontend, args: &[String]) -> Outcome {
    let Some(url) = args.first() else {
        return Outcome::failure(clone_help(frontend));
    };
    let mut branch = None;
    let mut destination = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--branch" if index + 1 < args.len() => {
                branch = Some(args[index + 1].clone());
                index += 2;
            }
            "--out" if index + 1 < args.len() => {
                destination = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            _ => index += 1,
        }
    }
    if branch
        .as_deref()
        .is_some_and(|branch| branch.starts_with('-'))
    {
        return Outcome::failure(format!(
            "error: invalid branch name: {}",
            python_repr(&branch.unwrap_or_default())
        ));
    }
    let normalized = url.trim_end_matches('/');
    let without_git = normalized.strip_suffix(".git").unwrap_or(normalized);
    let Some(captures) = github_url_pattern().captures(without_git) else {
        return Outcome::failure(format!("error: not a recognised GitHub URL: {without_git}"));
    };
    let owner = captures
        .get(1)
        .map(|value| value.as_str())
        .unwrap_or_default();
    let repo = captures
        .get(2)
        .map(|value| value.as_str())
        .unwrap_or_default();
    let destination = match destination {
        Some(destination) => destination,
        None => {
            let Ok(paths) = GlobalPaths::discover() else {
                return Outcome::failure(
                    "error: could not determine the user home directory".to_owned(),
                );
            };
            paths.directory.join("repos").join(owner).join(repo)
        }
    };
    let mut stdout = Vec::new();
    let mut stderr = String::new();
    if destination.exists() {
        stdout.push(format!(
            "Repo already cloned at {} - pulling latest...",
            destination.display()
        ));
        let mut command = Command::new("git");
        command.args(["-C", destination.to_string_lossy().as_ref(), "pull"]);
        if let Some(branch) = branch.as_deref() {
            command.args(["origin", "--", branch]);
        }
        match command.output() {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                stderr = format!(
                    "warning: git pull failed:\n{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(error) => stderr = format!("warning: git pull failed:\n{error}"),
        }
    } else {
        if let Some(parent) = destination.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            return Outcome::failure(format!("error: git clone failed:\n{error}"));
        }
        stdout.push(format!(
            "Cloning {without_git} -> {} ...",
            destination.display()
        ));
        let git_url = format!("{without_git}.git");
        let mut command = Command::new("git");
        command.args(["clone", "--depth", "1"]);
        if let Some(branch) = branch.as_deref() {
            command.args(["--branch", branch]);
        }
        command.arg("--").arg(git_url).arg(&destination);
        match command.output() {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                return Outcome {
                    code: 1,
                    stdout: stdout.join("\n"),
                    stderr: format!(
                        "error: git clone failed:\n{}",
                        String::from_utf8_lossy(&output.stderr)
                    ),
                    stdout_trailing_newline: true,
                    stderr_trailing_newline: true,
                };
            }
            Err(error) => {
                return Outcome {
                    code: 1,
                    stdout: stdout.join("\n"),
                    stderr: format!("error: git clone failed:\n{error}"),
                    stdout_trailing_newline: true,
                    stderr_trailing_newline: true,
                };
            }
        }
    }
    stdout.push(format!("Ready at: {}", destination.display()));
    stdout.push(destination.display().to_string());
    Outcome {
        code: 0,
        stdout: stdout.join("\n"),
        stderr,
        stdout_trailing_newline: true,
        stderr_trailing_newline: true,
    }
}

fn github_url_pattern() -> &'static Regex {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"github\.com[:/]([^/]+)/([^/]+?)$").unwrap_or_else(|_| std::process::abort())
    })
}

fn global_add_command(frontend: Frontend, args: &[String], paths: &GlobalPaths) -> Outcome {
    let mut source = None;
    let mut tag = None;
    let mut index = 1;
    while index < args.len() {
        if args[index] == "--as" && index + 1 < args.len() {
            tag = Some(args[index + 1].clone());
            index += 2;
        } else if source.is_none() {
            source = Some(PathBuf::from(&args[index]));
            index += 1;
        } else {
            index += 1;
        }
    }
    let Some(source) = source else {
        return Outcome::failure(global_add_help(frontend));
    };
    let tag = tag.unwrap_or_else(|| {
        source
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned()
    });
    match global_add(paths, &source, &tag, time::OffsetDateTime::now_utc()) {
        Ok(result) => {
            let stdout = if result.skipped {
                format!("'{tag}' unchanged since last add - global graph not modified.")
            } else {
                format!(
                    "Added '{tag}' to global graph: +{} nodes, -{} pruned. Global: {}",
                    result.nodes_added,
                    result.nodes_removed,
                    paths.graph.display()
                )
            };
            outcome_with_warnings(stdout, result.warnings)
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn global_remove_command(frontend: Frontend, args: &[String], paths: &GlobalPaths) -> Outcome {
    let tag = args.get(1).map(String::as_str).unwrap_or_default();
    if tag.is_empty() {
        return Outcome::failure(global_remove_help(frontend));
    }
    match global_remove(paths, tag) {
        Ok((removed, warnings)) => outcome_with_warnings(
            format!("Removed '{tag}' from global graph ({removed} nodes pruned)."),
            warnings,
        ),
        Err(GlobalError::UnknownRepo(_)) => {
            Outcome::failure(format!("error: \"repo '{tag}' not in global graph\""))
        }
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn outcome_with_warnings(stdout: String, warnings: Vec<String>) -> Outcome {
    Outcome {
        code: 0,
        stdout,
        stderr: warnings.join("\n"),
        stdout_trailing_newline: true,
        stderr_trailing_newline: true,
    }
}

fn python_string_value(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(value) => if *value { "True" } else { "False" }.to_owned(),
        Value::String(value) => value.clone(),
        value => value.to_string(),
    }
}

fn python_repr(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

pub(super) fn command_check_update(frontend: Frontend, args: &[String]) -> Outcome {
    let Some(path) = args.first() else {
        return Outcome::failure(check_update_help(frontend));
    };
    let root = absolute_path(PathBuf::from(path));
    let flag = root.join(output_root()).join("needs_update");
    if flag.exists() {
        Outcome::success(format!(
            "[graphify check-update] Pending non-code changes in {}.\n[graphify check-update] Run `/graphify --update` to apply semantic re-extraction.",
            root.display()
        ))
    } else {
        Outcome::success(String::new())
    }
}

pub(super) fn command_hook_guard(_frontend: Frontend, args: &[String]) -> Outcome {
    let kind = args.first().map_or("", String::as_str);
    if kind == "gemini" {
        let mut payload = Map::new();
        payload.insert("decision".to_owned(), Value::String("allow".to_owned()));
        if graph_path().is_file() {
            payload.insert(
                "additionalContext".to_owned(),
                Value::String(GEMINI_NUDGE_TEXT.to_owned()),
            );
        }
        return Outcome::success_exact(compact_json(Value::Object(payload)));
    }
    let mut input = Vec::new();
    if io::stdin()
        .take(MANIFEST_MAX_BYTES + 1)
        .read_to_end(&mut input)
        .is_err()
        || input.len() as u64 > MANIFEST_MAX_BYTES
    {
        return Outcome::success(String::new());
    }
    let Ok(document) = serde_json::from_slice::<Value>(&input) else {
        return Outcome::success(String::new());
    };
    let Some(root) = document.as_object() else {
        return Outcome::success(String::new());
    };
    let tool = root.get("tool_input").unwrap_or(&document).as_object();
    let Some(tool) = tool else {
        return Outcome::success(String::new());
    };
    let output = match kind {
        "search" => search_guard(tool),
        "read" => read_guard(root, tool, args.iter().skip(1).any(|arg| arg == "--strict")),
        _ => None,
    };
    Outcome::success(output.unwrap_or_default())
}

fn search_guard(tool: &Map<String, Value>) -> Option<String> {
    let command = python_string(tool.get("command"));
    let grep_tool = command.is_empty() && !python_string(tool.get("pattern")).is_empty();
    let bash_search = ["grep", "ripgrep", "rg ", "find ", "fd ", "ack ", "ag "]
        .iter()
        .any(|token| command.contains(token));
    ((grep_tool || bash_search) && graph_path().is_file())
        .then(|| pretool_payload("additionalContext", SEARCH_NUDGE_TEXT))
}

fn read_guard(
    root_document: &Map<String, Value>,
    tool: &Map<String, Value>,
    strict_flag: bool,
) -> Option<String> {
    let file_path = python_string(tool.get("file_path"));
    let pattern = python_string(tool.get("pattern"));
    let path = python_string(tool.get("path"));
    let values = [&file_path, &pattern, &path];
    let joined = values
        .iter()
        .map(|value| value.to_lowercase().replace('\\', "/"))
        .collect::<Vec<_>>()
        .join(" ");
    let output = output_root()
        .to_string_lossy()
        .to_lowercase()
        .replace('\\', "/");
    let output_name = Path::new(&output)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&output);
    if joined.contains("graphify-out/") || joined.contains(&format!("{output_name}/")) {
        return None;
    }
    let source_target = values.iter().any(|value| {
        let normalized = value.to_lowercase().replace('\\', "/");
        let tail = normalized.rsplit('/').next().unwrap_or_default();
        tail.rsplit_once('.')
            .is_some_and(|(_, extension)| SOURCE_EXTENSIONS.contains(&extension))
    });
    if !source_target {
        return None;
    }
    let project = project_root();
    let explicit = [&file_path, &path]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if !explicit.is_empty()
        && !explicit
            .iter()
            .any(|value| !Path::new(value).is_absolute() || is_within(Path::new(value), &project))
    {
        return None;
    }
    let graph = graph_path();
    let graph_modified = graph
        .metadata()
        .and_then(|metadata| metadata.modified())
        .ok()?;
    let file_stale = if file_path.is_empty() {
        false
    } else {
        fs::metadata(&file_path)
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified > graph_modified)
    };
    if file_stale || output_root().join("needs_update").exists() {
        return Some(pretool_payload("additionalContext", READ_STALE_TEXT));
    }
    let tool_name = python_string(root_document.get("tool_name"));
    let is_read = tool_name.is_empty() || tool_name == "Read";
    let session = python_string(root_document.get("session_id"));
    if strict_enabled(strict_flag)
        && is_read
        && !query_stamp_fresh()
        && target_is_indexed(&file_path, &project)
        && mark_session_denied(&session)
    {
        return Some(deny_payload());
    }
    Some(pretool_payload("additionalContext", READ_NUDGE_TEXT))
}

fn pretool_payload(field: &str, text: &str) -> String {
    let mut hook = Map::new();
    hook.insert(
        "hookEventName".to_owned(),
        Value::String("PreToolUse".to_owned()),
    );
    hook.insert(field.to_owned(), Value::String(text.to_owned()));
    let mut root = Map::new();
    root.insert("hookSpecificOutput".to_owned(), Value::Object(hook));
    compact_json(Value::Object(root))
}

fn deny_payload() -> String {
    let mut hook = Map::new();
    hook.insert(
        "hookEventName".to_owned(),
        Value::String("PreToolUse".to_owned()),
    );
    hook.insert(
        "permissionDecision".to_owned(),
        Value::String("deny".to_owned()),
    );
    hook.insert(
        "permissionDecisionReason".to_owned(),
        Value::String(READ_DENY_TEXT.to_owned()),
    );
    let mut root = Map::new();
    root.insert("hookSpecificOutput".to_owned(), Value::Object(hook));
    compact_json(Value::Object(root))
}

fn compact_json(value: Value) -> String {
    serde_json::to_string(&value).unwrap_or_default()
}

fn python_string(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Bool(value)) => if *value { "True" } else { "False" }.to_owned(),
        Some(Value::Number(value)) => value.to_string(),
        Some(value) => compact_json(value.clone()),
    }
}

fn strict_enabled(flag: bool) -> bool {
    match std::env::var("GRAPHIFY_HOOK_STRICT")
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => flag,
    }
}

fn query_stamp_fresh() -> bool {
    let ttl = std::env::var("GRAPHIFY_HOOK_STRICT_TTL")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1800.0);
    let Ok(modified) = output_root()
        .join("cache/last_query_stamp")
        .metadata()
        .and_then(|metadata| metadata.modified())
    else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map_or(true, |age| age.as_secs_f64() < ttl)
}

pub(super) fn touch_query_stamp(graph: &Path) {
    let Some(parent) = graph.parent() else {
        return;
    };
    let stamp = parent.join("cache/last_query_stamp");
    let seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64());
    let _ = write_text_atomic(stamp, &seconds.to_string());
}

fn mark_session_denied(session_id: &str) -> bool {
    let safe = session_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .take(SESSION_ID_MAX_CHARS)
        .collect::<String>();
    if safe.is_empty() {
        return false;
    }
    let directory = output_root().join("cache/hook_sessions");
    if fs::create_dir_all(&directory).is_err() {
        return false;
    }
    let marker = directory.join(format!("{safe}.denied"));
    match OpenOptions::new().write(true).create_new(true).open(marker) {
        Ok(_) => {
            remove_old_session_markers(&directory);
            true
        }
        Err(_) => false,
    }
}

fn remove_old_session_markers(directory: &Path) {
    let cutoff = Duration::from_secs(86_400);
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        if path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > cutoff)
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn target_is_indexed(file_path: &str, root: &Path) -> bool {
    if file_path.is_empty() {
        return true;
    }
    let manifest_path = output_root().join("manifest.json");
    let Ok(metadata) = manifest_path.metadata() else {
        return true;
    };
    if metadata.len() > MANIFEST_MAX_BYTES {
        return true;
    }
    let Ok(bytes) = fs::read(manifest_path) else {
        return true;
    };
    let Ok(Value::Object(manifest)) = serde_json::from_slice::<Value>(&bytes) else {
        return true;
    };
    if manifest.is_empty() {
        return true;
    }
    let target = Path::new(file_path);
    let absolute = target.to_string_lossy().replace('\\', "/");
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let relative = absolute_path(target.to_path_buf())
        .strip_prefix(root)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"));
    manifest.keys().any(|key| {
        let key = key.replace('\\', "/");
        key == absolute
            || (!name.is_empty() && (key == name || key.ends_with(&format!("/{name}"))))
            || relative
                .as_ref()
                .is_some_and(|relative| key == *relative || key.ends_with(&format!("/{relative}")))
    })
}

pub(super) fn command_merge_driver(frontend: Frontend, args: &[String]) -> Outcome {
    if args.len() < 3 {
        return Outcome::failure(merge_driver_help(frontend));
    }
    let current = Path::new(&args[1]);
    let other = Path::new(&args[2]);
    let current_graph = match load_merge_graph(current) {
        Ok(graph) => graph,
        Err(error) => {
            return Outcome::failure(format!(
                "[graphify merge-driver] error loading graphs: {error}"
            ));
        }
    };
    let other_graph = match load_merge_graph(other) {
        Ok(graph) => graph,
        Err(error) => {
            return Outcome::failure(format!(
                "[graphify merge-driver] error loading graphs: {error}"
            ));
        }
    };
    let merged = match compose_graphs(current_graph, other_graph) {
        Ok(graph) => graph,
        Err(error) => {
            return Outcome::failure(format!(
                "[graphify merge-driver] error loading graphs: {error}"
            ));
        }
    };
    let node_count = merged
        .get("nodes")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if node_count > MERGE_MAX_NODES {
        return Outcome::failure(format!(
            "[graphify merge-driver] merged graph has {node_count} nodes, exceeds {MERGE_MAX_NODES}-node cap; aborting merge."
        ));
    }
    let encoded = match python_pretty_json(&merged) {
        Ok(encoded) => encoded,
        Err(error) => {
            return Outcome::failure(format!(
                "[graphify merge-driver] error writing graph: {error}"
            ));
        }
    };
    match write_text_atomic(current, &encoded) {
        Ok(()) => Outcome::success(String::new()),
        Err(error) => Outcome::failure(format!(
            "[graphify merge-driver] error writing graph: {error}"
        )),
    }
}

fn load_merge_graph(path: &Path) -> Result<Map<String, Value>, String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("cannot stat {}: {error}", path.display()))?;
    if metadata.len() > MERGE_MAX_BYTES {
        return Err(format!(
            "graph.json {} is {} bytes, exceeds {MERGE_MAX_BYTES}-byte cap",
            path.display(),
            metadata.len()
        ));
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "graph document must be a JSON object".to_owned())
}

fn compose_graphs(
    mut current: Map<String, Value>,
    other: Map<String, Value>,
) -> Result<Value, String> {
    let directed = graph_flag(&current, "directed");
    let multigraph = graph_flag(&current, "multigraph");
    if directed != graph_flag(&other, "directed") {
        return Err("All graphs must be directed or undirected.".to_owned());
    }
    if multigraph != graph_flag(&other, "multigraph") {
        return Err("All graphs must be graphs or multigraphs.".to_owned());
    }
    let mut graph_attributes = current
        .remove("graph")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    merge_attributes(
        &mut graph_attributes,
        other
            .get("graph")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
    );
    let mut nodes = Vec::<Map<String, Value>>::new();
    let mut node_positions = HashMap::<String, usize>::new();
    for document in [&current, &other] {
        for node in graph_array(document, "nodes")? {
            let Some(object) = node.as_object() else {
                return Err("node entry must be an object".to_owned());
            };
            let Some(id) = object.get("id") else {
                return Err("node entry is missing id".to_owned());
            };
            insert_node(&mut nodes, &mut node_positions, id.clone(), object.clone());
        }
    }
    let edge_name = if current.contains_key("links") || other.contains_key("links") {
        "links"
    } else {
        "edges"
    };
    let mut edges = Vec::<Map<String, Value>>::new();
    let mut edge_positions = HashMap::<String, usize>::new();
    for document in [&current, &other] {
        let mut used_auto_keys = HashMap::<String, HashSet<String>>::new();
        let mut auto_keys = HashMap::<String, u64>::new();
        let source_edges = document
            .get(edge_name)
            .or_else(|| {
                document.get(if edge_name == "links" {
                    "edges"
                } else {
                    "links"
                })
            })
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for edge in source_edges {
            let Some(object) = edge.as_object() else {
                return Err("edge entry must be an object".to_owned());
            };
            let Some(source) = object.get("source") else {
                return Err("edge entry is missing source".to_owned());
            };
            let Some(target) = object.get("target") else {
                return Err("edge entry is missing target".to_owned());
            };
            ensure_implicit_node(&mut nodes, &mut node_positions, source);
            ensure_implicit_node(&mut nodes, &mut node_positions, target);
            let mut merged_edge = object.clone();
            let pair = edge_pair(source, target, directed);
            let key = if multigraph {
                let edge_key = if let Some(key) = object.get("key") {
                    used_auto_keys
                        .entry(pair.clone())
                        .or_default()
                        .insert(value_key(key));
                    key.clone()
                } else {
                    let used = used_auto_keys.entry(pair.clone()).or_default();
                    let next = auto_keys.entry(pair.clone()).or_default();
                    while used.contains(&value_key(&Value::from(*next))) {
                        *next += 1;
                    }
                    let key = Value::from(*next);
                    used.insert(value_key(&key));
                    *next += 1;
                    merged_edge.insert("key".to_owned(), key.clone());
                    key
                };
                format!("{pair}:{}", value_key(&edge_key))
            } else {
                pair
            };
            if let Some(position) = edge_positions.get(&key).copied() {
                merge_attributes(&mut edges[position], merged_edge);
            } else {
                edge_positions.insert(key, edges.len());
                edges.push(merged_edge);
            }
        }
    }
    let mut output = Map::new();
    output.insert("directed".to_owned(), Value::Bool(directed));
    output.insert("multigraph".to_owned(), Value::Bool(multigraph));
    output.insert("graph".to_owned(), Value::Object(graph_attributes));
    output.insert(
        "nodes".to_owned(),
        Value::Array(nodes.into_iter().map(networkx_node).collect()),
    );
    output.insert(
        "links".to_owned(),
        Value::Array(
            edges
                .into_iter()
                .map(|edge| networkx_edge(edge, multigraph))
                .collect(),
        ),
    );
    Ok(Value::Object(output))
}

fn insert_node(
    nodes: &mut Vec<Map<String, Value>>,
    positions: &mut HashMap<String, usize>,
    id: Value,
    attributes: Map<String, Value>,
) {
    let key = value_key(&id);
    if let Some(position) = positions.get(&key).copied() {
        merge_attributes(&mut nodes[position], attributes);
    } else {
        positions.insert(key, nodes.len());
        nodes.push(attributes);
    }
}

fn merge_attributes(target: &mut Map<String, Value>, incoming: Map<String, Value>) {
    for (key, value) in incoming {
        if let Some(existing) = target.get_mut(&key) {
            *existing = value;
        } else {
            target.insert(key, value);
        }
    }
}

fn ensure_implicit_node(
    nodes: &mut Vec<Map<String, Value>>,
    positions: &mut HashMap<String, usize>,
    id: &Value,
) {
    let key = value_key(id);
    if positions.contains_key(&key) {
        return;
    }
    let mut node = Map::new();
    node.insert("id".to_owned(), id.clone());
    positions.insert(key, nodes.len());
    nodes.push(node);
}

fn graph_array<'a>(document: &'a Map<String, Value>, field: &str) -> Result<&'a [Value], String> {
    document
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("graph document is missing {field} array"))
}

fn graph_flag(document: &Map<String, Value>, field: &str) -> bool {
    document
        .get(field)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn edge_pair(source: &Value, target: &Value, directed: bool) -> String {
    let source = value_key(source);
    let target = value_key(target);
    if directed || source <= target {
        format!("{source}:{target}")
    } else {
        format!("{target}:{source}")
    }
}

fn value_key(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn networkx_node(node: Map<String, Value>) -> Value {
    let mut output = Map::new();
    for (key, value) in &node {
        if key != "id" {
            output.insert(key.clone(), value.clone());
        }
    }
    output.insert(
        "id".to_owned(),
        node.get("id").cloned().unwrap_or(Value::Null),
    );
    Value::Object(output)
}

fn networkx_edge(edge: Map<String, Value>, multigraph: bool) -> Value {
    let mut output = Map::new();
    for (key, value) in &edge {
        if !matches!(key.as_str(), "source" | "target" | "key") {
            output.insert(key.clone(), value.clone());
        }
    }
    output.insert(
        "source".to_owned(),
        edge.get("source").cloned().unwrap_or(Value::Null),
    );
    output.insert(
        "target".to_owned(),
        edge.get("target").cloned().unwrap_or(Value::Null),
    );
    if multigraph {
        output.insert(
            "key".to_owned(),
            edge.get("key").cloned().unwrap_or(Value::Null),
        );
    }
    Value::Object(output)
}

fn python_pretty_json(value: &Value) -> Result<String, serde_json::Error> {
    let json = serde_json::to_string_pretty(value)?;
    let mut ascii = String::with_capacity(json.len());
    for character in json.chars() {
        if character.is_ascii() {
            ascii.push(character);
        } else {
            use std::fmt::Write as _;
            let point = u32::from(character);
            if point <= 0xffff {
                let _ = write!(ascii, "\\u{point:04x}");
            } else {
                let adjusted = point - 0x1_0000;
                let high = 0xd800 + (adjusted >> 10);
                let low = 0xdc00 + (adjusted & 0x3ff);
                let _ = write!(ascii, "\\u{high:04x}\\u{low:04x}");
            }
        }
    }
    Ok(ascii)
}

fn output_root() -> PathBuf {
    PathBuf::from(std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned()))
}

fn graph_path() -> PathBuf {
    output_root().join("graph.json")
}

fn project_root() -> PathBuf {
    absolute_path(
        std::env::var_os("CLAUDE_PROJECT_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from(".")),
    )
}

fn absolute_path(path: PathBuf) -> PathBuf {
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().map_or(path.clone(), |current| current.join(path))
    };
    fs::canonicalize(&path).unwrap_or_else(|_| normalize_path(path))
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = output.pop();
            }
            component => output.push(component.as_os_str()),
        }
    }
    output
}

fn is_within(path: &Path, root: &Path) -> bool {
    absolute_path(path.to_path_buf()).starts_with(root)
}

pub(super) fn check_update_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Trail => "Usage: trail graph check-update <path>",
        Frontend::Graphify => "Usage: graphify check-update <path>",
    }
    .to_owned()
}

pub(super) fn merge_driver_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Trail => "Usage: trail graph merge-driver <base> <current> <other>",
        Frontend::Graphify => "Usage: graphify merge-driver <base> <current> <other>",
    }
    .to_owned()
}

pub(super) fn global_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Trail => "Usage: trail graph global [add|remove|list|path]",
        Frontend::Graphify => "Usage: graphify global [add|remove|list|path]",
    }
    .to_owned()
}

fn global_add_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Trail => "Usage: trail graph global add <graph.json> [--as <repo-tag>]",
        Frontend::Graphify => "Usage: graphify global add <graph.json> [--as <repo-tag>]",
    }
    .to_owned()
}

fn global_remove_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Trail => "Usage: trail graph global remove <repo-tag>",
        Frontend::Graphify => "Usage: graphify global remove <repo-tag>",
    }
    .to_owned()
}

pub(super) fn clone_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Trail => {
            "Usage: trail graph clone <github-url> [--branch <branch>] [--out <dir>]"
        }
        Frontend::Graphify => {
            "Usage: graphify clone <github-url> [--branch <branch>] [--out <dir>]"
        }
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_overwrites_duplicate_nodes_and_edges_from_other() {
        let current = serde_json::from_value::<Map<String, Value>>(serde_json::json!({
            "directed":true,"multigraph":false,"graph":{"a":1},
            "nodes":[{"id":"a","x":1},{"id":"b"}],
            "links":[{"source":"a","target":"b","relation":"old"}]
        }))
        .unwrap_or_default();
        let other = serde_json::from_value::<Map<String, Value>>(serde_json::json!({
            "directed":true,"multigraph":false,"graph":{"b":2},
            "nodes":[{"id":"a","x":2},{"id":"c"}],
            "links":[{"source":"a","target":"b","relation":"new"}]
        }))
        .unwrap_or_default();
        let merged = compose_graphs(current, other).unwrap_or_default();
        assert_eq!(merged["nodes"][0]["x"], 2);
        assert_eq!(merged["links"][0]["relation"], "new");
        assert_eq!(merged["graph"], serde_json::json!({"a":1,"b":2}));
    }

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    fn error<T>(result: Result<T, String>) -> String {
        result.err().unwrap_or_default()
    }

    #[test]
    fn compose_rejects_incompatible_and_malformed_networkx_documents() {
        let base = || {
            object(serde_json::json!({
                "directed":true,"multigraph":false,"graph":{},"nodes":[],"links":[]
            }))
        };
        let mut other = base();
        other.insert("directed".to_owned(), Value::Bool(false));
        assert_eq!(
            error(compose_graphs(base(), other)),
            "All graphs must be directed or undirected."
        );
        let mut other = base();
        other.insert("multigraph".to_owned(), Value::Bool(true));
        assert_eq!(
            error(compose_graphs(base(), other)),
            "All graphs must be graphs or multigraphs."
        );
        for (field, invalid, expected) in [
            (
                "nodes",
                serde_json::json!([1]),
                "node entry must be an object",
            ),
            ("nodes", serde_json::json!([{}]), "node entry is missing id"),
            (
                "links",
                serde_json::json!([1]),
                "edge entry must be an object",
            ),
            (
                "links",
                serde_json::json!([{"target":"b"}]),
                "edge entry is missing source",
            ),
            (
                "links",
                serde_json::json!([{"source":"a"}]),
                "edge entry is missing target",
            ),
        ] {
            let mut current = base();
            current.insert(field.to_owned(), invalid);
            assert_eq!(error(compose_graphs(current, base())), expected);
        }
        let mut missing_nodes = base();
        missing_nodes.remove("nodes");
        assert!(error(compose_graphs(missing_nodes, base())).contains("nodes array"));
    }

    #[test]
    fn compose_multigraph_keys_implicit_nodes_edges_aliases_and_unicode_are_deterministic()
    -> Result<(), Box<dyn std::error::Error>> {
        let current = object(serde_json::json!({
            "directed":false,"multigraph":true,"graph":{},"nodes":[],
            "edges":[
                {"source":"z","target":"a","key":0,"label":"first"},
                {"source":"z","target":"a","label":"second"},
                {"source":"z","target":"a","label":"third"}
            ]
        }));
        let other = object(serde_json::json!({
            "directed":false,"multigraph":true,"graph":{},"nodes":[{"id":"a","name":"A"}],
            "links":[{"source":"a","target":"z","key":1,"other":true}]
        }));
        let merged = compose_graphs(current, other)?;
        assert_eq!(merged["nodes"].as_array().map(Vec::len), Some(2));
        assert_eq!(merged["links"].as_array().map(Vec::len), Some(3));
        assert_eq!(merged["links"][1]["key"], 1);
        assert_eq!(merged["links"][1]["other"], true);
        assert_eq!(merged["links"][2]["key"], 2);

        let encoded = python_pretty_json(&serde_json::json!({"bmp":"é","astral":"🚀"}))?;
        assert!(encoded.contains("\\u00e9"));
        assert!(encoded.contains("\\ud83d\\ude80"));
        assert!(!encoded.contains('🚀'));

        assert_eq!(
            normalize_path(PathBuf::from("one/./two/../three")),
            PathBuf::from("one/three")
        );
        assert_eq!(
            edge_pair(&Value::from("z"), &Value::from("a"), false),
            "\"a\":\"z\""
        );
        Ok(())
    }

    #[test]
    fn bounded_merge_loading_scalar_rendering_and_help_variants_are_total()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(python_string_value(&Value::Null), "None");
        assert_eq!(python_string_value(&Value::Bool(true)), "True");
        assert_eq!(
            python_string_value(&Value::String("text".to_owned())),
            "text"
        );
        assert_eq!(python_string_value(&Value::from(7)), "7");
        assert_eq!(python_string(None), "");
        assert_eq!(python_string(Some(&Value::Null)), "");
        assert_eq!(python_string(Some(&Value::Bool(false))), "False");
        assert_eq!(python_string(Some(&Value::from(3))), "3");
        assert_eq!(python_string(Some(&serde_json::json!([1]))), "[1]");
        assert_eq!(python_repr("a'b\\c"), "'a\\'b\\\\c'");
        for frontend in [Frontend::Trail, Frontend::Graphify] {
            assert!(check_update_help(frontend).contains("check-update"));
            assert!(merge_driver_help(frontend).contains("merge-driver"));
            assert!(global_add_help(frontend).contains("global add"));
            assert!(global_remove_help(frontend).contains("global remove"));
        }

        let directory = tempfile::tempdir()?;
        let missing = directory.path().join("missing.json");
        assert!(error(load_merge_graph(&missing)).contains("cannot stat"));
        let scalar = directory.path().join("scalar.json");
        fs::write(&scalar, "[]")?;
        assert_eq!(
            error(load_merge_graph(&scalar)),
            "graph document must be a JSON object"
        );
        let invalid = directory.path().join("invalid.json");
        fs::write(&invalid, "not json")?;
        assert!(!error(load_merge_graph(&invalid)).is_empty());
        let valid = directory.path().join("valid.json");
        fs::write(&valid, "{}")?;
        assert!(load_merge_graph(&valid)?.is_empty());
        let oversized = directory.path().join("oversized.json");
        let file = fs::File::create(&oversized)?;
        file.set_len(MERGE_MAX_BYTES + 1)?;
        assert!(error(load_merge_graph(&oversized)).contains("exceeds"));
        Ok(())
    }
}
