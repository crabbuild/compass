mod detect;
mod model;
mod registry;
mod report;
mod request;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use compass_files::{write_bytes_atomic, write_text_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use self::detect::detect_agents;
use self::model::{
    InstallReport, InstallRequest, InstallScope, InstallStatus, OutputFormat, SupportTier,
    TargetResult,
};
use self::registry::AgentRegistry;
use self::report::render_report;
use self::request::{parse_install_request, resolve_scope};
use crate::{Frontend, Outcome};

const SKILL_VERSION: &str = env!("CARGO_PKG_VERSION");
const OPENCODE_PLUGIN: &str = include_str!("../assets/compass-integrations/opencode-plugin.js");
const KILO_PLUGIN: &str = include_str!("../assets/compass-integrations/kilo-plugin.js");
const SKILL_ASSET: &str = "compass-skill/SKILL.md";
const REFERENCE_BUNDLE: &str = "compass-skill";
const PLATFORM_NAMES: &[&str] = &[
    "claude",
    "cline",
    "codex",
    "opencode",
    "kilo",
    "aider",
    "copilot",
    "claw",
    "droid",
    "trae",
    "trae-cn",
    "hermes",
    "kiro",
    "pi",
    "codebuddy",
    "antigravity",
    "antigravity-windows",
    "windows",
    "kimi",
    "amp",
    "agents",
    "devin",
];
const DIRECT_COMMANDS: &[&str] = &[
    "agents",
    "skills",
    "aider",
    "amp",
    "antigravity",
    "claude",
    "cline",
    "claw",
    "codebuddy",
    "codex",
    "copilot",
    "cursor",
    "devin",
    "droid",
    "gemini",
    "hermes",
    "kilo",
    "kiro",
    "opencode",
    "pi",
    "trae",
    "trae-cn",
    "vscode",
];

struct EmbeddedAsset {
    path: &'static str,
    bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/install_assets.rs"));

#[derive(Clone, Copy)]
struct Platform {
    name: &'static str,
    skill_destination: &'static str,
}

pub(crate) fn is_direct_command(command: &str) -> bool {
    DIRECT_COMMANDS.contains(&command)
}

pub(crate) fn command_install(frontend: Frontend, args: &[String]) -> Outcome {
    if frontend == Frontend::Compass {
        return command_install_compass(args);
    }
    command_install_legacy(frontend, args)
}

fn command_install_legacy(frontend: Frontend, args: &[String]) -> Outcome {
    let prefix = command_prefix(frontend);
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return Outcome::success(install_help(prefix));
    }
    let default = if cfg!(windows) { "windows" } else { "claude" };
    let mut selected = None::<String>;
    let mut project = false;
    let mut strict = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" => project = true,
            "--strict" => strict = true,
            "--platform" if index + 1 < args.len() => {
                if let Err(error) = set_platform(&mut selected, &args[index + 1]) {
                    return Outcome::failure(error);
                }
                index += 1;
            }
            value if value.starts_with("--platform=") => {
                if let Err(error) = set_platform(&mut selected, &value[11..]) {
                    return Outcome::failure(error);
                }
            }
            "--platform" => {
                return Outcome::failure("error: --platform requires a value".to_owned());
            }
            value if value.starts_with('-') => {
                return Outcome::failure(format!("error: unknown install option '{value}'"));
            }
            value => {
                if let Err(error) = set_platform(&mut selected, value) {
                    return Outcome::failure(error);
                }
            }
        }
        index += 1;
    }
    let selected = canonical_platform(selected.as_deref().unwrap_or(default));
    if !is_install_platform(selected) {
        return Outcome::failure(format!(
            "error: unknown platform '{selected}'. Choose from: {}, gemini, cursor",
            PLATFORM_NAMES.join(", ")
        ));
    }
    if strict && !project {
        let mut outcome = install_platform(selected, false, Path::new("."), false, prefix);
        outcome.stderr = format!(
            "note: --strict applies to the project PreToolUse hook; run `{prefix} install --project --strict` or `compass claude install --strict`."
        );
        return outcome;
    }
    install_platform(selected, project, Path::new("."), strict, prefix)
}

fn command_install_compass(args: &[String]) -> Outcome {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return Outcome::success(install_help("compass"));
    }
    let request = match parse_install_request(args) {
        Ok(request) => request,
        Err(error) => return Outcome::failure(error),
    };
    let registry = match AgentRegistry::new() {
        Ok(registry) => registry,
        Err(error) => return Outcome::failure(format!("error: invalid agent registry: {error}")),
    };
    let scope = match resolve_scope(&request) {
        Ok(scope) => scope,
        Err(error) => return Outcome::failure(error),
    };
    let detected = if request.platforms.is_empty() && !request.all {
        detect_agents(&registry, &scope)
    } else {
        BTreeMap::new()
    };
    let selected = if request.all {
        registry
            .iter()
            .map(|agent| agent.id.to_owned())
            .collect::<Vec<_>>()
    } else if request.platforms.is_empty() {
        let mut values = detected.keys().cloned().collect::<Vec<_>>();
        values.push("agents".to_owned());
        match registry.canonicalize(&values) {
            Ok(values) => values,
            Err(error) => return Outcome::failure(error),
        }
    } else {
        match registry.canonicalize(&request.platforms) {
            Ok(values) => values,
            Err(error) => return Outcome::failure(error),
        }
    };
    if request.strict && !scope.is_project() {
        return Outcome::failure(
            "error: --strict requires project scope because it installs a project hook".to_owned(),
        );
    }
    if request.strict && !selected.iter().any(|platform| platform == "claude") {
        return Outcome::failure(
            "error: --strict currently requires the Claude platform; add `--platform claude`"
                .to_owned(),
        );
    }
    let _scope_lock = if request.dry_run {
        None
    } else {
        match InstallLock::acquire(scope.root()) {
            Ok(lock) => Some(lock),
            Err(error) => return Outcome::failure(error),
        }
    };

    let selected_platforms = selected.clone();
    let selected_consumers = selected.iter().cloned().collect::<BTreeSet<_>>();
    let mut skill_targets = BTreeMap::<PathBuf, BTreeSet<String>>::new();
    let mut adapter_only = Vec::new();
    for id in selected {
        let Some(agent) = registry.resolve(&id) else {
            continue;
        };
        if let Some(destination) = agent.skill_destination(&scope) {
            skill_targets
                .entry(destination)
                .or_default()
                .insert(agent.id.to_owned());
        } else if agent.tier == SupportTier::AdapterOnly {
            adapter_only.push(agent.id.to_owned());
        }
    }

    let mut results = Vec::new();
    for (destination, consumers) in skill_targets {
        results.push(execute_skill_target(
            &scope,
            &request,
            destination,
            consumers,
        ));
    }
    for consumer in adapter_only {
        results.push(execute_adapter_target(&scope, &request, &consumer));
    }
    let codex_installed =
        results.iter().any(|result| {
            result.consumers.contains("codex")
                && matches!(
                    result.status,
                    InstallStatus::Installed | InstallStatus::Updated | InstallStatus::Current
                )
        }) && is_managed_skill(&scope.root().join(".agents/skills/compass/SKILL.md"));
    if selected_consumers.contains("codex")
        && codex_installed
        && let Some(result) = migrate_legacy_codex_skill(&scope, request.dry_run)
    {
        results.push(result);
    }

    let output = scope.root().join("compass-out");
    let graph_exists = compass_files::BuildGuard::resolve_artifact(&output, "graph.json")
        .is_ok_and(|path| path.is_file());
    let ready_consumers = results
        .iter()
        .filter(|result| {
            matches!(
                result.status,
                InstallStatus::Installed | InstallStatus::Updated | InstallStatus::Current
            )
        })
        .flat_map(|result| result.consumers.iter().cloned())
        .collect::<BTreeSet<_>>();
    let successful = !ready_consumers.is_empty();
    let incomplete = results.iter().any(|result| {
        matches!(
            result.status,
            InstallStatus::Skipped | InstallStatus::Failed
        )
    });
    let has_failures = results
        .iter()
        .any(|result| result.status == InstallStatus::Failed);
    let next_actions = install_next_actions(
        &scope,
        &ready_consumers,
        graph_exists,
        request.dry_run,
        incomplete,
        has_failures,
    );
    let report = InstallReport {
        schema: 1,
        scope: scope.kind(),
        root: scope.root().to_path_buf(),
        selected: selected_platforms,
        detected,
        results,
        graph_exists,
        next_actions,
    };
    let output = match render_report(&report, request.format) {
        Ok(output) => output,
        Err(error) => return Outcome::failure(error),
    };
    let failed = (!request.dry_run && !successful) || (request.require_all && incomplete);
    Outcome {
        code: u8::from(failed),
        stdout: output,
        stderr: String::new(),
        stdout_trailing_newline: true,
        stderr_trailing_newline: true,
        html_output: None,
    }
}

fn install_next_actions(
    scope: &InstallScope,
    ready_consumers: &BTreeSet<String>,
    graph_exists: bool,
    dry_run: bool,
    incomplete: bool,
    has_failures: bool,
) -> Vec<String> {
    if dry_run {
        if has_failures {
            return vec![
                "Resolve the reported preflight failures, then rerun `compass install --dry-run`."
                    .to_owned(),
            ];
        }
        return vec![
            "Review the plan, then rerun without `--dry-run` to install these targets.".to_owned(),
        ];
    }
    if ready_consumers.is_empty() {
        return vec![
            "Resolve the reported installation failures, then rerun `compass install`.".to_owned(),
        ];
    }
    let mut actions = Vec::new();
    if incomplete {
        actions.push(
            "Review skipped or failed targets above; configured targets are ready to activate."
                .to_owned(),
        );
    }
    if !graph_exists && scope.is_project() {
        actions.push(
            "Run `compass update .` now, or ask the configured assistant an architecture question to build the project graph on first use."
                .to_owned(),
        );
    }
    if scope.is_project() && ready_consumers.contains("codex") {
        actions.push(
            "In Codex, open `/hooks` and trust the Compass project hook before relying on graph-first search guidance."
                .to_owned(),
        );
    }
    if ready_consumers.contains("gemini") {
        actions.push("In Gemini CLI, run `/skills reload` to activate Compass now.".to_owned());
    }
    actions.push(
        "Start a new coding-agent session, or use its skill reload command, then ask a codebase question."
            .to_owned(),
    );
    actions
}

fn migrate_legacy_codex_skill(scope: &InstallScope, dry_run: bool) -> Option<TargetResult> {
    let legacy = scope.root().join(".codex/skills/compass/SKILL.md");
    let directory = legacy.parent()?.to_path_buf();
    if !directory.exists() {
        return None;
    }
    let consumers = BTreeSet::from(["codex".to_owned()]);
    if !is_managed_skill(&legacy) {
        return Some(TargetResult {
            id: "legacy-codex-migration".to_owned(),
            consumers,
            status: InstallStatus::Skipped,
            paths: vec![legacy],
            reason: Some(
                "legacy Codex skill is unowned or modified and was left in place".to_owned(),
            ),
            rollback: None,
        });
    }
    if dry_run {
        return Some(TargetResult {
            id: "legacy-codex-migration".to_owned(),
            consumers,
            status: InstallStatus::Skipped,
            paths: vec![directory],
            reason: Some("dry run: managed legacy Codex skill would be removed".to_owned()),
            rollback: None,
        });
    }
    Some(match fs::remove_dir_all(&directory) {
        Ok(()) => TargetResult {
            id: "legacy-codex-migration".to_owned(),
            consumers,
            status: InstallStatus::Updated,
            paths: vec![directory],
            reason: Some("migrated legacy .codex skill to shared .agents skill".to_owned()),
            rollback: None,
        },
        Err(error) => TargetResult {
            id: "legacy-codex-migration".to_owned(),
            consumers,
            status: InstallStatus::Failed,
            paths: vec![directory],
            reason: Some(format!(
                "error: could not remove managed legacy skill: {error}"
            )),
            rollback: None,
        },
    })
}

fn execute_skill_target(
    scope: &InstallScope,
    request: &InstallRequest,
    destination: PathBuf,
    consumers: BTreeSet<String>,
) -> TargetResult {
    let id = if consumers.len() > 1
        || consumers.iter().any(|consumer| {
            matches!(
                consumer.as_str(),
                "agents" | "codex" | "gemini" | "opencode" | "copilot"
            )
        }) {
        "shared-agent-skill".to_owned()
    } else {
        consumers.iter().next().cloned().unwrap_or_default()
    };
    if request.dry_run {
        let paths = planned_target_paths(scope, Some(&destination), &consumers);
        let preflight = validate_skill_destination(&destination, scope.root())
            .and_then(|()| require_owned_or_absent(&destination))
            .and_then(|()| {
                consumers
                    .iter()
                    .try_for_each(|consumer| preflight_agent_adapter(scope, consumer))
            });
        return TargetResult {
            id,
            consumers,
            status: if preflight.is_ok() {
                InstallStatus::Skipped
            } else {
                InstallStatus::Failed
            },
            paths,
            reason: Some(
                preflight.err().unwrap_or_else(|| {
                    "dry run: target is ready; no files were changed".to_owned()
                }),
            ),
            rollback: None,
        };
    }
    let existed = destination.exists();
    if let Err(error) = validate_skill_destination(&destination, scope.root()) {
        return TargetResult {
            id,
            consumers,
            status: InstallStatus::Failed,
            paths: vec![destination],
            reason: Some(error),
            rollback: None,
        };
    }
    if let Err(error) = require_owned_or_absent(&destination) {
        return TargetResult {
            id,
            consumers,
            status: InstallStatus::Failed,
            paths: vec![destination],
            reason: Some(error),
            rollback: None,
        };
    }
    for consumer in &consumers {
        if let Err(error) = preflight_agent_adapter(scope, consumer) {
            return TargetResult {
                id,
                consumers,
                status: InstallStatus::Failed,
                paths: vec![destination],
                reason: Some(error),
                rollback: Some("preflight failed; no files were changed".to_owned()),
            };
        }
    }
    let adapter_snapshots = match snapshot_files(&adapter_paths_for(scope, &consumers)) {
        Ok(snapshots) => snapshots,
        Err(error) => {
            return TargetResult {
                id,
                consumers,
                status: InstallStatus::Failed,
                paths: vec![destination],
                reason: Some(error),
                rollback: Some("snapshot failed; no files were changed".to_owned()),
            };
        }
    };
    let skill_snapshot = match SkillSnapshot::capture(&destination) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return TargetResult {
                id,
                consumers,
                status: InstallStatus::Failed,
                paths: vec![destination],
                reason: Some(error),
                rollback: Some("snapshot failed; no files were changed".to_owned()),
            };
        }
    };
    let scope_kind = match scope.kind() {
        model::ScopeKind::Project => "project",
        model::ScopeKind::User => "user",
    };
    let skill = match install_skill_at_scoped(
        destination.clone(),
        consumers.clone(),
        scope_kind,
        scope.root(),
    ) {
        Ok(skill) => skill,
        Err(error) => {
            let rollback = if error.contains("could not restore previous package") {
                restore_transaction(&skill_snapshot, &adapter_snapshots)
            } else {
                "package staging or activation failed before changing the installed package"
                    .to_owned()
            };
            return TargetResult {
                id,
                consumers,
                status: InstallStatus::Failed,
                paths: vec![destination],
                reason: Some(error),
                rollback: Some(rollback),
            };
        }
    };
    let mut adapter_paths = Vec::new();
    for consumer in &consumers {
        match install_agent_adapter(scope, consumer, request.strict, &mut adapter_paths) {
            Ok(()) => {}
            Err(error) => {
                let rollback = restore_transaction(&skill_snapshot, &adapter_snapshots);
                return TargetResult {
                    id,
                    consumers,
                    status: InstallStatus::Failed,
                    paths: std::iter::once(destination).chain(adapter_paths).collect(),
                    reason: Some(error),
                    rollback: Some(rollback),
                };
            }
        }
    }
    let adapter_changed = snapshots_changed(&adapter_snapshots);
    TargetResult {
        id,
        consumers,
        status: if !skill.changed && !adapter_changed {
            InstallStatus::Current
        } else if existed {
            InstallStatus::Updated
        } else {
            InstallStatus::Installed
        },
        paths: std::iter::once(destination).chain(adapter_paths).collect(),
        reason: None,
        rollback: None,
    }
}

#[derive(Clone)]
struct FileSnapshot {
    path: PathBuf,
    content: Option<Vec<u8>>,
}

struct SkillSnapshot {
    directory: PathBuf,
    existed: bool,
    files: Vec<FileSnapshot>,
}

impl SkillSnapshot {
    fn capture(destination: &Path) -> Result<Self, String> {
        let directory = destination
            .parent()
            .ok_or_else(|| "error: invalid skill destination".to_owned())?
            .to_path_buf();
        let existed = directory.exists();
        let files = if existed {
            snapshot_directory(&directory)?
        } else {
            Vec::new()
        };
        Ok(Self {
            directory,
            existed,
            files,
        })
    }

    fn restore(&self) -> Result<(), String> {
        if self.directory.exists() {
            fs::remove_dir_all(&self.directory).map_err(|error| {
                format!(
                    "could not remove failed skill transaction {}: {error}",
                    self.directory.display()
                )
            })?;
        }
        if self.existed {
            for snapshot in &self.files {
                let Some(bytes) = &snapshot.content else {
                    continue;
                };
                if let Some(parent) = snapshot.path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("could not restore {}: {error}", parent.display())
                    })?;
                }
                write_bytes_atomic(&snapshot.path, bytes).map_err(|error| {
                    format!("could not restore {}: {error}", snapshot.path.display())
                })?;
            }
        }
        Ok(())
    }
}

fn snapshot_directory(directory: &Path) -> Result<Vec<FileSnapshot>, String> {
    fn visit(directory: &Path, snapshots: &mut Vec<FileSnapshot>) -> Result<(), String> {
        for entry in fs::read_dir(directory)
            .map_err(|error| format!("error: could not inspect {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| {
                format!("error: could not inspect {}: {error}", directory.display())
            })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("error: could not inspect {}: {error}", path.display()))?;
            if file_type.is_dir() {
                visit(&path, snapshots)?;
            } else if file_type.is_file() {
                snapshots.push(FileSnapshot {
                    content: Some(fs::read(&path).map_err(|error| {
                        format!("error: could not snapshot {}: {error}", path.display())
                    })?),
                    path,
                });
            } else {
                return Err(format!(
                    "error: {} is not a regular managed file; no files were changed",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    let mut snapshots = Vec::new();
    visit(directory, &mut snapshots)?;
    Ok(snapshots)
}

fn snapshot_files(paths: &[PathBuf]) -> Result<Vec<FileSnapshot>, String> {
    let mut snapshots = Vec::new();
    for path in paths.iter().collect::<BTreeSet<_>>() {
        let content = if path.exists() {
            Some(fs::read(path).map_err(|error| {
                format!("error: could not snapshot {}: {error}", path.display())
            })?)
        } else {
            None
        };
        snapshots.push(FileSnapshot {
            path: path.clone(),
            content,
        });
    }
    Ok(snapshots)
}

fn snapshots_changed(snapshots: &[FileSnapshot]) -> bool {
    snapshots
        .iter()
        .any(|snapshot| fs::read(&snapshot.path).ok() != snapshot.content)
}

fn restore_files(snapshots: &[FileSnapshot]) -> String {
    let mut errors = Vec::new();
    for snapshot in snapshots {
        let result = match &snapshot.content {
            Some(bytes) => {
                if let Some(parent) = snapshot.path.parent()
                    && let Err(error) = fs::create_dir_all(parent)
                {
                    errors.push(format!("{}: {error}", parent.display()));
                    continue;
                }
                write_bytes_atomic(&snapshot.path, bytes).map_err(|error| error.to_string())
            }
            None if snapshot.path.exists() => {
                fs::remove_file(&snapshot.path).map_err(|error| error.to_string())
            }
            None => Ok(()),
        };
        if let Err(error) = result {
            errors.push(format!("{}: {error}", snapshot.path.display()));
        }
    }
    if errors.is_empty() {
        "restored all adapter and configuration files".to_owned()
    } else {
        format!("rollback incomplete: {}", errors.join("; "))
    }
}

fn restore_transaction(skill: &SkillSnapshot, adapters: &[FileSnapshot]) -> String {
    let adapter_result = restore_files(adapters);
    match skill.restore() {
        Ok(()) if !adapter_result.starts_with("rollback incomplete") => {
            "restored skill, adapter, and configuration files".to_owned()
        }
        Ok(()) => adapter_result,
        Err(error) => format!("{adapter_result}; skill rollback incomplete: {error}"),
    }
}

fn adapter_paths_for(scope: &InstallScope, consumers: &BTreeSet<String>) -> Vec<PathBuf> {
    let root = scope.root();
    let mut paths = Vec::new();
    for name in consumers {
        if !scope.is_project() {
            match name.as_str() {
                "claude" => paths.push(claude_config_root(root).join("CLAUDE.md")),
                "opencode" => paths.extend([
                    root.join(".opencode/plugins/compass.js"),
                    root.join(".opencode/opencode.json"),
                ]),
                "kilo" => paths.extend([
                    root.join(".kilo/plugins/compass.js"),
                    root.join(".kilo/kilo.json"),
                ]),
                _ => {}
            }
            continue;
        }
        match name.as_str() {
            "claude" => paths.extend([
                root.join(".claude/CLAUDE.md"),
                root.join("CLAUDE.md"),
                root.join(".claude/settings.json"),
            ]),
            "agents" | "aider" | "amp" | "claw" | "droid" | "trae" | "trae-cn" | "hermes" => {
                paths.push(root.join("AGENTS.md"))
            }
            "codex" => paths.extend([root.join("AGENTS.md"), root.join(".codex/hooks.json")]),
            "opencode" => paths.extend([
                root.join("AGENTS.md"),
                root.join(".opencode/plugins/compass.js"),
                root.join(".opencode/opencode.json"),
            ]),
            "kilo" => paths.extend([
                root.join("AGENTS.md"),
                root.join(".kilo/plugins/compass.js"),
                root.join(".kilo/kilo.json"),
            ]),
            "gemini" => paths.extend([root.join("GEMINI.md"), root.join(".gemini/settings.json")]),
            "copilot" => paths.push(root.join(".github/copilot-instructions.md")),
            "kiro" => paths.push(root.join(".kiro/steering/compass.md")),
            "cursor" => paths.push(root.join(".cursor/rules/compass.mdc")),
            "devin" => paths.push(root.join(".windsurf/rules/compass.md")),
            "antigravity" => paths.extend([
                root.join(".agents/rules/compass.md"),
                root.join(".agents/workflows/compass.md"),
            ]),
            _ => {}
        }
    }
    paths
}

fn planned_target_paths(
    scope: &InstallScope,
    skill_destination: Option<&Path>,
    consumers: &BTreeSet<String>,
) -> Vec<PathBuf> {
    skill_destination
        .into_iter()
        .map(Path::to_path_buf)
        .chain(adapter_paths_for(scope, consumers))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn uninstall_paths_for(scope: &InstallScope, consumers: &BTreeSet<String>) -> Vec<PathBuf> {
    let mut paths = adapter_paths_for(scope, consumers);
    if scope.is_project() {
        for name in consumers {
            match name.as_str() {
                "claude" => paths.push(scope.root().join(".claude/settings.local.json")),
                "codebuddy" => paths.extend([
                    scope.root().join("CODEBUDDY.md"),
                    scope.root().join(".codebuddy/settings.json"),
                ]),
                _ => {}
            }
        }
    }
    paths
}

fn preflight_agent_adapter(scope: &InstallScope, name: &str) -> Result<(), String> {
    let root = scope.root();
    if !scope.is_project() {
        match name {
            "claude" => preflight_registration(&claude_config_root(root).join("CLAUDE.md"))?,
            "opencode" => {
                preflight_plugin_array(&root.join(".opencode/opencode.json"))?;
                preflight_managed_adapter(
                    &root.join(".opencode/plugins/compass.js"),
                    OPENCODE_PLUGIN,
                )?;
            }
            "kilo" => {
                preflight_plugin_array(&root.join(".kilo/kilo.json"))?;
                preflight_managed_adapter(&root.join(".kilo/plugins/compass.js"), KILO_PLUGIN)?;
            }
            _ => {}
        }
        return Ok(());
    }
    match name {
        "claude" => {
            preflight_registration(&root.join(".claude/CLAUDE.md"))?;
            preflight_markdown_section(&root.join("CLAUDE.md"), "## compass")?;
            preflight_hook_array(&root.join(".claude/settings.json"), "PreToolUse")?;
        }
        "agents" | "aider" | "amp" | "claw" | "droid" | "trae" | "trae-cn" | "hermes" => {
            preflight_markdown_section(&root.join("AGENTS.md"), "## compass")?
        }
        "kilo" => {
            preflight_markdown_section(&root.join("AGENTS.md"), "## compass")?;
            preflight_plugin_array(&root.join(".kilo/kilo.json"))?;
            preflight_managed_adapter(&root.join(".kilo/plugins/compass.js"), KILO_PLUGIN)?;
        }
        "codex" => {
            preflight_markdown_section(&root.join("AGENTS.md"), "## compass")?;
            preflight_hook_array(&root.join(".codex/hooks.json"), "PreToolUse")?;
        }
        "opencode" => {
            preflight_markdown_section(&root.join("AGENTS.md"), "## compass")?;
            preflight_plugin_array(&root.join(".opencode/opencode.json"))?;
            preflight_managed_adapter(&root.join(".opencode/plugins/compass.js"), OPENCODE_PLUGIN)?;
        }
        "gemini" => {
            preflight_markdown_section(&root.join("GEMINI.md"), "## compass")?;
            preflight_hook_array(&root.join(".gemini/settings.json"), "BeforeTool")?;
        }
        "copilot" => {
            preflight_markdown_section(&root.join(".github/copilot-instructions.md"), "## compass")?
        }
        "kiro" => preflight_managed_adapter(
            &root.join(".kiro/steering/compass.md"),
            asset_text("compass-integrations/kiro-steering.md").unwrap_or_default(),
        )?,
        "cursor" => {
            preflight_managed_adapter(&root.join(".cursor/rules/compass.mdc"), CURSOR_RULE)?
        }
        "devin" => {
            preflight_managed_adapter(&root.join(".windsurf/rules/compass.md"), DEVIN_RULES)?
        }
        "antigravity" => {
            preflight_managed_adapter(
                &root.join(".agents/rules/compass.md"),
                asset_text("compass-integrations/antigravity-rules.md").unwrap_or_default(),
            )?;
            preflight_managed_adapter(
                &root.join(".agents/workflows/compass.md"),
                asset_text("compass-integrations/antigravity-workflow.md").unwrap_or_default(),
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn preflight_markdown_section(path: &Path, marker: &str) -> Result<(), String> {
    let content = read_optional_text(path)?;
    if content.contains(COMPASS_SECTION_START) && content.contains(COMPASS_SECTION_END) {
        return Ok(());
    }
    if content.lines().any(|line| line.trim() == marker)
        && !legacy_section_is_owned(&content, marker)
    {
        return Err(format!(
            "error: {} contains an unowned '{marker}' section; file was not changed",
            path.display()
        ));
    }
    Ok(())
}

fn preflight_registration(path: &Path) -> Result<(), String> {
    let content = read_optional_text(path)?;
    if content.contains(COMPASS_REGISTRATION_START) && content.contains(COMPASS_REGISTRATION_END) {
        return Ok(());
    }
    if content.lines().any(|line| line.trim() == "# compass")
        && !legacy_registration_is_owned(&content)
    {
        return Err(format!(
            "error: {} contains an unowned '# compass' section; file was not changed",
            path.display()
        ));
    }
    Ok(())
}

fn preflight_hook_array(path: &Path, event: &str) -> Result<(), String> {
    let document = load_json_object(path)?;
    let Some(hooks) = document.get("hooks") else {
        return Ok(());
    };
    let hooks = hooks.as_object().ok_or_else(|| {
        format!(
            "error: {} field 'hooks' must be an object; file was not changed",
            path.display()
        )
    })?;
    if hooks.get(event).is_some_and(|value| !value.is_array()) {
        return Err(format!(
            "error: {} hook '{event}' must be an array; file was not changed",
            path.display()
        ));
    }
    Ok(())
}

fn preflight_plugin_array(path: &Path) -> Result<(), String> {
    let document = load_json_object(path)?;
    if document
        .get("plugin")
        .is_some_and(|value| !value.is_array())
    {
        return Err(format!(
            "error: {} field 'plugin' must be an array; file was not changed",
            path.display()
        ));
    }
    Ok(())
}

fn execute_adapter_target(
    scope: &InstallScope,
    request: &InstallRequest,
    consumer: &str,
) -> TargetResult {
    let consumers = BTreeSet::from([consumer.to_owned()]);
    let planned_paths = planned_target_paths(scope, None, &consumers);
    if planned_paths.is_empty() {
        return TargetResult {
            id: consumer.to_owned(),
            consumers,
            status: InstallStatus::Failed,
            paths: Vec::new(),
            reason: Some(format!(
                "error: {consumer} has no {}-scoped Compass installation target",
                if scope.is_project() {
                    "project"
                } else {
                    "user"
                }
            )),
            rollback: None,
        };
    }
    if request.dry_run {
        let preflight = preflight_agent_adapter(scope, consumer);
        return TargetResult {
            id: consumer.to_owned(),
            consumers,
            status: if preflight.is_ok() {
                InstallStatus::Skipped
            } else {
                InstallStatus::Failed
            },
            paths: planned_paths,
            reason: Some(
                preflight.err().unwrap_or_else(|| {
                    "dry run: target is ready; no files were changed".to_owned()
                }),
            ),
            rollback: None,
        };
    }
    let mut paths = Vec::new();
    if let Err(error) = preflight_agent_adapter(scope, consumer) {
        return TargetResult {
            id: consumer.to_owned(),
            consumers,
            status: InstallStatus::Failed,
            paths,
            reason: Some(error),
            rollback: Some("preflight failed; no files were changed".to_owned()),
        };
    }
    let snapshots = match snapshot_files(&adapter_paths_for(
        scope,
        &BTreeSet::from([consumer.to_owned()]),
    )) {
        Ok(snapshots) => snapshots,
        Err(error) => {
            return TargetResult {
                id: consumer.to_owned(),
                consumers,
                status: InstallStatus::Failed,
                paths,
                reason: Some(error),
                rollback: Some("snapshot failed; no files were changed".to_owned()),
            };
        }
    };
    match install_agent_adapter(scope, consumer, request.strict, &mut paths) {
        Ok(()) => TargetResult {
            id: consumer.to_owned(),
            consumers,
            status: if snapshots_changed(&snapshots) {
                if snapshots.iter().all(|snapshot| snapshot.content.is_none()) {
                    InstallStatus::Installed
                } else {
                    InstallStatus::Updated
                }
            } else {
                InstallStatus::Current
            },
            paths,
            reason: None,
            rollback: None,
        },
        Err(error) => {
            let rollback = restore_files(&snapshots);
            TargetResult {
                id: consumer.to_owned(),
                consumers,
                status: InstallStatus::Failed,
                paths,
                reason: Some(error),
                rollback: Some(rollback),
            }
        }
    }
}

fn install_agent_adapter(
    scope: &InstallScope,
    name: &str,
    strict: bool,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let root = scope.root();
    if !scope.is_project() {
        match name {
            "claude" => {
                let config_root = claude_config_root(root);
                let path = config_root.join("CLAUDE.md");
                let skill = config_root.join("skills/compass/SKILL.md");
                let registration = format!(
                    "# compass\n- **compass** (`{}`) - use Compass for knowledge-graph navigation and codebase questions. Trigger: `/compass`\n",
                    skill.display()
                );
                append_registration(&path, &registration)?;
                paths.push(path);
            }
            "opencode" => {
                let mut lines = Vec::new();
                install_opencode(root, &mut lines)?;
                paths.extend([
                    root.join(".opencode/plugins/compass.js"),
                    root.join(".opencode/opencode.json"),
                ]);
            }
            "kilo" => {
                let mut lines = Vec::new();
                install_kilo_plugin(root, &mut lines)?;
                paths.extend([
                    root.join(".kilo/plugins/compass.js"),
                    root.join(".kilo/kilo.json"),
                ]);
            }
            _ => {}
        }
        return Ok(());
    }
    match name {
        "agents" => {
            let mut lines = Vec::new();
            install_agents(root, name, &mut lines)?;
            paths.push(root.join("AGENTS.md"));
        }
        "claude" => {
            let mut lines = Vec::new();
            register_claude_skill(root, &mut lines)?;
            install_markdown_and_claude_hook(root, strict, &mut lines)?;
            paths.extend([
                root.join(".claude/CLAUDE.md"),
                root.join("CLAUDE.md"),
                root.join(".claude/settings.json"),
            ]);
        }
        "codex" | "opencode" | "aider" | "amp" | "claw" | "droid" | "trae" | "trae-cn"
        | "hermes" | "kilo" => {
            let mut lines = Vec::new();
            install_agents(root, name, &mut lines)?;
            paths.push(root.join("AGENTS.md"));
            if name == "codex" {
                paths.push(root.join(".codex/hooks.json"));
            } else if name == "opencode" {
                paths.extend([
                    root.join(".opencode/plugins/compass.js"),
                    root.join(".opencode/opencode.json"),
                ]);
            } else if name == "kilo" {
                paths.extend([
                    root.join(".kilo/plugins/compass.js"),
                    root.join(".kilo/kilo.json"),
                ]);
            }
        }
        "gemini" => {
            let target = root.join("GEMINI.md");
            update_section(
                &target,
                "## compass",
                asset_text("compass-integrations/gemini-md.md").unwrap_or_default(),
            )?;
            install_gemini_hook(root)?;
            paths.extend([target, root.join(".gemini/settings.json")]);
        }
        "copilot" => {
            let target = root.join(".github/copilot-instructions.md");
            update_section(
                &target,
                "## compass",
                asset_text("compass-integrations/vscode-instructions.md").unwrap_or_default(),
            )?;
            paths.push(target);
        }
        "kiro" => {
            let target = root.join(".kiro/steering/compass.md");
            write_managed_adapter(
                target.clone(),
                asset_text("compass-integrations/kiro-steering.md").unwrap_or_default(),
            )?;
            paths.push(target);
        }
        "cursor" => {
            let target = root.join(".cursor/rules/compass.mdc");
            write_managed_adapter(target.clone(), CURSOR_RULE)?;
            paths.push(target);
        }
        "devin" => {
            let target = root.join(".windsurf/rules/compass.md");
            write_managed_adapter(target.clone(), DEVIN_RULES)?;
            paths.push(target);
        }
        "antigravity" => {
            let skill = root.join(".agents/skills/compass/SKILL.md");
            let mut lines = Vec::new();
            finalize_antigravity(root, &skill, &mut lines)?;
            paths.extend([
                root.join(".agents/rules/compass.md"),
                root.join(".agents/workflows/compass.md"),
            ]);
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn command_uninstall(frontend: Frontend, args: &[String]) -> Outcome {
    if frontend == Frontend::Compass {
        return command_uninstall_compass(args);
    }
    let prefix = command_prefix(frontend);
    let mut selected = None::<String>;
    let mut project = false;
    let mut purge = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" => project = true,
            "--purge" => purge = true,
            "--platform" if index + 1 < args.len() => {
                selected = Some(args[index + 1].clone());
                index += 1;
            }
            value if value.starts_with("--platform=") => selected = Some(value[11..].to_owned()),
            "--platform" => {
                return Outcome::failure("error: --platform requires a value".to_owned());
            }
            value if value.starts_with('-') => {
                return Outcome::failure(format!("error: unknown uninstall option '{value}'"));
            }
            value => selected = Some(value.to_owned()),
        }
        index += 1;
    }
    if let Some(platform) = selected {
        return uninstall_platform(
            canonical_platform(&platform),
            project,
            Path::new("."),
            prefix,
        );
    }
    uninstall_all(project, purge, Path::new("."), prefix)
}

fn command_uninstall_compass(args: &[String]) -> Outcome {
    let purge = args.iter().any(|argument| argument == "--purge");
    let filtered = args
        .iter()
        .filter(|argument| argument.as_str() != "--purge")
        .cloned()
        .collect::<Vec<_>>();
    let request = match parse_install_request(&filtered) {
        Ok(request) => request,
        Err(error) => return Outcome::failure(error),
    };
    if request.strict || request.dry_run || request.require_all {
        return Outcome::failure(
            "error: --strict, --dry-run, and --require-all apply only to install".to_owned(),
        );
    }
    if request.format != OutputFormat::Text {
        return Outcome::failure("error: --format applies only to install".to_owned());
    }
    let scope = match resolve_scope(&request) {
        Ok(scope) => scope,
        Err(error) => return Outcome::failure(error),
    };
    let _scope_lock = match InstallLock::acquire(scope.root()) {
        Ok(lock) => lock,
        Err(error) => return Outcome::failure(error),
    };
    let registry = match AgentRegistry::new() {
        Ok(registry) => registry,
        Err(error) => return Outcome::failure(format!("error: invalid agent registry: {error}")),
    };
    let selected = if request.platforms.is_empty() || request.all {
        registry
            .iter()
            .map(|agent| agent.id.to_owned())
            .collect::<Vec<_>>()
    } else {
        match registry.canonicalize(&request.platforms) {
            Ok(platforms) => platforms,
            Err(error) => return Outcome::failure(error),
        }
    };
    let mut lines = vec![format!(
        "Uninstalling Compass guidance from {} scope...",
        if scope.is_project() {
            "project"
        } else {
            "user"
        }
    )];
    let mut failed = false;
    for platform in selected {
        let consumers = BTreeSet::from([platform.clone()]);
        let adapter_snapshots = match snapshot_files(&uninstall_paths_for(&scope, &consumers)) {
            Ok(snapshots) => snapshots,
            Err(error) => {
                failed = true;
                lines.push(error);
                continue;
            }
        };
        let skill_snapshot = registry
            .resolve(&platform)
            .and_then(|agent| agent.skill_destination(&scope))
            .filter(|destination| is_managed_skill(destination))
            .map(|destination| SkillSnapshot::capture(&destination))
            .transpose();
        let skill_snapshot = match skill_snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                failed = true;
                lines.push(error);
                continue;
            }
        };
        let outcome = uninstall_platform(&platform, scope.is_project(), scope.root(), "compass");
        if outcome.code != 0 {
            failed = true;
            let rollback = restore_files(&adapter_snapshots);
            let rollback = match skill_snapshot {
                Some(snapshot) => match snapshot.restore() {
                    Ok(()) if !rollback.starts_with("rollback incomplete") => {
                        "restored skill, adapter, and configuration files".to_owned()
                    }
                    Ok(()) => rollback,
                    Err(error) => format!("{rollback}; skill rollback incomplete: {error}"),
                },
                None => rollback,
            };
            lines.push(format!("rollback for {platform}: {rollback}"));
        }
        if !outcome.stdout.is_empty() && outcome.stdout != "nothing to remove" {
            lines.push(outcome.stdout);
        }
        if !outcome.stderr.is_empty() {
            lines.push(outcome.stderr);
        }
    }
    if purge {
        let target = match scoped_compass_output(scope.root()) {
            Ok(target) => target,
            Err(error) => return Outcome::failure(error),
        };
        if target.exists() {
            if let Err(error) = fs::remove_dir_all(&target) {
                failed = true;
                lines.push(format!(
                    "error: could not remove {}: {error}",
                    target.display()
                ));
            } else {
                lines.push(format!("removed {}", target.display()));
            }
        }
    }
    Outcome {
        code: u8::from(failed),
        stdout: lines.join("\n"),
        stderr: String::new(),
        stdout_trailing_newline: true,
        stderr_trailing_newline: true,
        html_output: None,
    }
}

pub(crate) fn command_platform(frontend: Frontend, command: &str, args: &[String]) -> Outcome {
    let prefix = command_prefix(frontend);
    let Some(action) = args.first().map(String::as_str) else {
        return Outcome::failure(format!("Usage: {prefix} {command} [install|uninstall]"));
    };
    if !matches!(action, "install" | "uninstall") {
        return Outcome::failure(format!("Usage: {prefix} {command} [install|uninstall]"));
    }
    if frontend == Frontend::Compass {
        let mut forwarded = vec!["--platform".to_owned(), command.to_owned()];
        forwarded.extend(args[1..].iter().cloned());
        return if action == "install" {
            command_install_compass(&forwarded)
        } else {
            command_uninstall_compass(&forwarded)
        };
    }
    let project = args[1..].iter().any(|argument| argument == "--project");
    let strict = args[1..].iter().any(|argument| argument == "--strict");
    let platform = canonical_platform(command);
    if action == "install" {
        install_direct(platform, project, strict, Path::new("."), prefix)
    } else {
        uninstall_direct(platform, project, Path::new("."), prefix)
    }
}

fn install_direct(name: &str, project: bool, strict: bool, root: &Path, prefix: &str) -> Outcome {
    match name {
        "claude" if !project => install_claude_direct(root, strict),
        "codebuddy" => install_codebuddy_direct(root),
        "gemini" | "cursor" | "vscode" => install_platform(name, project, root, strict, prefix),
        "kiro" => install_kiro_direct(root),
        "codex" | "opencode" | "aider" | "claw" | "droid" | "trae" | "trae-cn" | "hermes"
            if !project =>
        {
            let mut lines = Vec::new();
            match install_agents(root, name, &mut lines) {
                Ok(()) => Outcome::success(lines.join("\n")),
                Err(error) => Outcome::failure(error),
            }
        }
        "amp" | "agents" if !project => install_agents_with_global_skill(name, root),
        "kilo" if !project => install_kilo_direct(root),
        "antigravity" if !project => install_antigravity_direct(root, prefix),
        _ => install_platform(name, project, root, strict, prefix),
    }
}

fn uninstall_direct(name: &str, project: bool, root: &Path, prefix: &str) -> Outcome {
    match name {
        "claude" if !project => uninstall_claude_direct(root),
        "codebuddy" => uninstall_codebuddy_direct(root, project),
        "copilot" | "devin" if !project => uninstall_global_skill_with_summary(name, root),
        "codex" | "opencode" | "aider" | "claw" | "droid" | "trae" | "trae-cn" | "hermes"
            if !project =>
        {
            let mut lines = Vec::new();
            strip_section_file(&root.join("AGENTS.md"), "## compass", &mut lines);
            if name == "codex" {
                remove_json_hooks(&root.join(".codex/hooks.json"), "PreToolUse", &mut lines);
            } else if name == "opencode" {
                remove_opencode(root, &mut lines);
            }
            Outcome::success(if lines.is_empty() {
                "nothing to remove".to_owned()
            } else {
                lines.join("\n")
            })
        }
        "amp" | "agents" if !project => uninstall_agents_with_global_skill(name, root),
        "kilo" if !project => uninstall_kilo_direct(root),
        "kiro" => uninstall_kiro_direct(root),
        "antigravity" if !project => uninstall_antigravity(root, false),
        _ => uninstall_platform(name, project, root, prefix),
    }
}

fn set_platform(selected: &mut Option<String>, candidate: &str) -> Result<(), String> {
    if selected
        .as_deref()
        .is_some_and(|current| current != candidate)
    {
        return Err("error: specify install platform only once".to_owned());
    }
    *selected = Some(candidate.to_owned());
    Ok(())
}

fn canonical_platform(platform: &str) -> &str {
    if platform == "skills" {
        "agents"
    } else {
        platform
    }
}

fn is_install_platform(platform: &str) -> bool {
    PLATFORM_NAMES.contains(&platform) || matches!(platform, "gemini" | "cursor")
}

fn command_prefix(_frontend: Frontend) -> &'static str {
    "compass"
}

fn install_help(prefix: &str) -> String {
    format!(
        "Usage: {prefix} install [PLATFORM] [OPTIONS]\n\nInstall Compass skills for detected or explicitly selected coding agents.\nInside Git, the default scope is the repository root; elsewhere it is the user home.\n\nOptions:\n  -p, --platform PLATFORM  Select a platform (repeatable; bypasses detection)\n      --all                Install every supported platform\n      --project            Force repository scope (requires a Git repository)\n      --user               Force user scope\n      --strict             Require a Compass query before Claude Code project reads\n      --dry-run            Show the resolved targets without changing files\n      --require-all        Fail if any selected target is skipped or fails\n      --format text|json   Select human or machine-readable output\n  -h, --help               Show this help\n\nPlatforms: {}, gemini, cursor, cline\n\nExamples:\n  {prefix} install\n  {prefix} install --platform codex --platform claude\n  {prefix} install --all --dry-run\n  {prefix} install --user --format json",
        PLATFORM_NAMES.join(", ")
    )
}

fn platform(name: &str) -> Option<Platform> {
    let name = PLATFORM_NAMES
        .iter()
        .copied()
        .find(|candidate| *candidate == name)?;
    let destination = match name {
        "claude" | "windows" => ".claude/skills/compass/SKILL.md",
        "codex" | "opencode" | "copilot" => ".agents/skills/compass/SKILL.md",
        "cline" => ".cline/skills/compass/SKILL.md",
        "kilo" => ".config/kilo/skills/compass/SKILL.md",
        "aider" => ".aider/compass/SKILL.md",
        "claw" | "hermes" => ".openclaw/skills/compass/SKILL.md",
        "droid" => ".factory/skills/compass/SKILL.md",
        "trae" | "trae-cn" => ".trae/skills/compass/SKILL.md",
        "kiro" => ".kiro/skills/compass/SKILL.md",
        "pi" => ".pi/agent/skills/compass/SKILL.md",
        "codebuddy" | "antigravity" | "antigravity-windows" | "amp" | "agents" => {
            ".agents/skills/compass/SKILL.md"
        }
        "kimi" => ".kimi/skills/compass/SKILL.md",
        "devin" => ".config/devin/skills/compass/SKILL.md",
        _ => return None,
    };
    Some(Platform::new(name, destination).with_specific_destination())
}

impl Platform {
    const fn new(name: &'static str, skill_destination: &'static str) -> Self {
        Self {
            name,
            skill_destination,
        }
    }

    fn with_specific_destination(mut self) -> Self {
        self.skill_destination = match self.name {
            "hermes" => ".hermes/skills/compass/SKILL.md",
            "trae-cn" => ".trae-cn/skills/compass/SKILL.md",
            "codebuddy" => ".codebuddy/skills/compass/SKILL.md",
            "antigravity" | "antigravity-windows" => ".agents/skills/compass/SKILL.md",
            "amp" | "agents" | "codex" | "opencode" | "copilot" => {
                ".agents/skills/compass/SKILL.md"
            }
            _ => self.skill_destination,
        };
        self
    }
}

fn install_platform(
    name: &str,
    project: bool,
    project_dir: &Path,
    strict: bool,
    prefix: &str,
) -> Outcome {
    if name == "cursor" {
        return install_cursor(project_dir, project);
    }
    if name == "vscode" {
        return install_vscode(project_dir);
    }
    if name == "gemini" {
        return install_gemini(project, project_dir);
    }
    let Some(config) = platform(name) else {
        return Outcome::failure(format!("error: unknown platform '{name}'"));
    };
    let skill = match install_skill(config, project, project_dir) {
        Ok(skill) => skill,
        Err(error) => return Outcome::failure(error),
    };
    let mut lines = skill.messages;
    let root = project_dir;
    if project {
        let top = project_scope_root(&skill.path, root);
        let mut hint_paths = vec![top];
        match name {
            "claude" | "windows" => {
                if let Err(error) = register_claude_skill(root, &mut lines) {
                    return Outcome::failure(error);
                }
                append_project_hint(&mut lines, root, &hint_paths);
                append_done(&mut lines);
                if let Err(error) = install_markdown_and_claude_hook(root, strict, &mut lines) {
                    return Outcome::failure(error);
                }
                lines.push(String::new());
                lines.push(
                    "Claude Code will now check the knowledge graph before answering".to_owned(),
                );
                lines.push("codebase questions and rebuild it after code changes.".to_owned());
                if strict {
                    lines.push(
                        "Strict mode: the first raw file read per session is blocked until"
                            .to_owned(),
                    );
                    lines.push(
                        "one `compass query` runs (toggle with COMPASS_HOOK_STRICT=0).".to_owned(),
                    );
                }
                hint_paths.push(root.join("CLAUDE.md"));
            }
            "codex" | "opencode" | "aider" | "amp" | "claw" | "droid" | "trae" | "trae-cn"
            | "hermes" => {
                if let Err(error) = install_agents(root, name, &mut lines) {
                    return Outcome::failure(error);
                }
                hint_paths.push(root.join("AGENTS.md"));
            }
            "kiro" => {
                if let Err(error) = write_managed_adapter(
                    root.join(".kiro/steering/compass.md"),
                    asset_text("compass-integrations/kiro-steering.md").unwrap_or_default(),
                ) {
                    return Outcome::failure(error);
                }
                lines
                    .push("  .kiro/steering/compass.md  ->  always-on steering written".to_owned());
                lines.push(String::new());
                lines.push(
                    "Kiro will now read the knowledge graph before every conversation.".to_owned(),
                );
                lines.push("Use /compass to build or update the graph.".to_owned());
            }
            "kilo" => {
                if let Err(error) = install_kilo_command(&mut lines) {
                    return Outcome::failure(error);
                }
                append_project_hint(&mut lines, root, &hint_paths);
                append_done(&mut lines);
                return Outcome::success(lines.join("\n"));
            }
            "codebuddy" => {
                if let Err(error) = register_codebuddy(&mut lines) {
                    return Outcome::failure(error);
                }
                append_project_hint(&mut lines, root, &hint_paths);
                append_done(&mut lines);
                return Outcome::success(lines.join("\n"));
            }
            "devin" => {
                if let Err(error) =
                    write_managed_adapter(root.join(".windsurf/rules/compass.md"), DEVIN_RULES)
                {
                    return Outcome::failure(error);
                }
                lines.push("  rules written  ->  .windsurf/rules/compass.md".to_owned());
                hint_paths.push(root.join(".windsurf"));
            }
            "antigravity" | "antigravity-windows" => {
                if let Err(error) = finalize_antigravity(root, &skill.path, &mut lines) {
                    return Outcome::failure(error);
                }
            }
            _ => {}
        }
        append_project_hint(&mut lines, root, &hint_paths);
    } else {
        if name == "kilo"
            && let Err(error) = install_kilo_command(&mut lines)
        {
            return Outcome::failure(error);
        }
        if name == "opencode"
            && let Err(error) = install_opencode(project_dir, &mut lines)
        {
            return Outcome::failure(error);
        }
        if matches!(name, "claude" | "windows")
            && let Err(error) = register_global_claude(&mut lines)
        {
            return Outcome::failure(error);
        }
        if name == "codebuddy"
            && let Err(error) = register_codebuddy(&mut lines)
        {
            return Outcome::failure(error);
        }
        append_done(&mut lines);
    }
    let _ = prefix;
    Outcome::success(lines.join("\n"))
}

fn install_claude_direct(root: &Path, strict: bool) -> Outcome {
    let mut lines = Vec::new();
    if let Err(error) = install_markdown_and_claude_hook(root, strict, &mut lines) {
        return Outcome::failure(error);
    }
    lines.push(String::new());
    lines.push("Claude Code will now check the knowledge graph before answering".to_owned());
    lines.push("codebase questions and rebuild it after code changes.".to_owned());
    if strict {
        lines.push("Strict mode: the first raw file read per session is blocked until".to_owned());
        lines.push("one `compass query` runs (toggle with COMPASS_HOOK_STRICT=0).".to_owned());
    }
    Outcome::success(lines.join("\n"))
}

fn uninstall_claude_direct(root: &Path) -> Outcome {
    let mut lines = Vec::new();
    strip_section_file(&root.join("CLAUDE.md"), "## compass", &mut lines);
    remove_json_hooks(
        &root.join(".claude/settings.json"),
        "PreToolUse",
        &mut lines,
    );
    remove_json_hooks(
        &root.join(".claude/settings.local.json"),
        "PreToolUse",
        &mut lines,
    );
    uninstall_outcome(
        lines,
        "No CLAUDE.md found in current directory - nothing to do",
    )
}

fn install_codebuddy_direct(root: &Path) -> Outcome {
    let Some(config) = platform("codebuddy") else {
        return Outcome::failure("error: CodeBuddy platform is unavailable".to_owned());
    };
    let skill = match install_skill(config, false, root) {
        Ok(value) => value,
        Err(error) => return Outcome::failure(error),
    };
    let mut lines = skill.messages;
    let markdown = root.join("CODEBUDDY.md");
    if let Err(error) = update_section(
        &markdown,
        "## compass",
        asset_text("compass-integrations/claude-md.md").unwrap_or_default(),
    ) {
        return Outcome::failure(error);
    }
    lines.push(format!(
        "compass section written to {}",
        absolute_display(&markdown)
    ));
    if let Err(error) = install_codebuddy_hook(root) {
        return Outcome::failure(error);
    }
    lines.push("  .codebuddy/settings.json  ->  PreToolUse hooks registered".to_owned());
    lines.push(String::new());
    lines.push("CodeBuddy will now check the knowledge graph before answering".to_owned());
    lines.push("codebase questions and rebuild it after code changes.".to_owned());
    Outcome::success(lines.join("\n"))
}

fn uninstall_codebuddy_direct(root: &Path, project: bool) -> Outcome {
    let mut lines = Vec::new();
    if let Some(config) = platform("codebuddy") {
        remove_skill(config, project, root, &mut lines);
    }
    strip_section_file(&root.join("CODEBUDDY.md"), "## compass", &mut lines);
    remove_json_hooks(
        &root.join(".codebuddy/settings.json"),
        "PreToolUse",
        &mut lines,
    );
    uninstall_outcome(
        lines,
        "No CODEBUDDY.md found in current directory - nothing to do",
    )
}

fn uninstall_outcome(mut lines: Vec<String>, empty: &str) -> Outcome {
    if lines.is_empty() {
        lines.push(empty.to_owned());
    }
    let text = lines.join("\n");
    if lines
        .iter()
        .flat_map(|line| line.lines())
        .any(|line| line.trim_start().starts_with("error:"))
    {
        Outcome::failure(text)
    } else {
        Outcome::success(text)
    }
}

fn uninstall_agents_with_global_skill(name: &str, root: &Path) -> Outcome {
    let Some(config) = platform(name) else {
        return Outcome::failure(format!("error: unknown platform '{name}'"));
    };
    let mut lines = Vec::new();
    let destination = skill_destination(config, false, root).ok();
    let removed = destination.as_ref().is_some_and(|path| path.exists());
    remove_skill(config, false, root, &mut lines);
    if removed {
        lines.push("skill removed".to_owned());
    }
    strip_section_file(&root.join("AGENTS.md"), "## compass", &mut lines);
    uninstall_outcome(
        lines,
        "No AGENTS.md found in current directory - nothing to do",
    )
}

fn uninstall_global_skill_with_summary(name: &str, root: &Path) -> Outcome {
    let Some(config) = platform(name) else {
        return Outcome::failure(format!("error: unknown platform '{name}'"));
    };
    let mut lines = Vec::new();
    let destination = skill_destination(config, false, root).ok();
    let removed = destination.as_ref().is_some_and(|path| path.exists());
    remove_skill(config, false, root, &mut lines);
    lines.push(if removed {
        "skill removed".to_owned()
    } else {
        "nothing to remove".to_owned()
    });
    uninstall_outcome(lines, "nothing to remove")
}

fn uninstall_kilo_direct(root: &Path) -> Outcome {
    let mut lines = Vec::new();
    strip_section_file(&root.join("AGENTS.md"), "## compass", &mut lines);
    remove_kilo(root, &mut lines);
    if let Some(home) = home_directory() {
        let command = home.join(".config/kilo/command/compass.md");
        let skill = home.join(".config/kilo/skills/compass/SKILL.md");
        let mut removed = Vec::new();
        let removed_command = remove_managed_adapter(
            &command,
            asset_text("compass-integrations/kilo-command.md").unwrap_or_default(),
            &mut lines,
            &format!("command removed: {}", command.display()),
        );
        if is_managed_skill(&skill) {
            let _ = fs::remove_file(&skill);
            removed.push(format!("skill removed: {}", skill.display()));
            let _ = fs::remove_file(skill.with_file_name(".compass_version"));
            let _ = fs::remove_dir_all(skill.with_file_name("references"));
        }
        remove_empty_ancestors(&skill.with_file_name("placeholder"), &home);
        if removed.is_empty() && !removed_command {
            lines.push("nothing to remove".to_owned());
        } else if !removed.is_empty() {
            lines.push(removed.join("; "));
        }
    }
    uninstall_outcome(lines, "nothing to remove")
}

fn uninstall_kiro_direct(root: &Path) -> Outcome {
    let Some(config) = platform("kiro") else {
        return Outcome::failure("error: Kiro platform is unavailable".to_owned());
    };
    let mut lines = Vec::new();
    let skill = skill_destination(config, true, root).ok();
    let removed_skill = skill.as_ref().is_some_and(|path| is_managed_skill(path));
    remove_skill(config, true, root, &mut lines);
    let steering = root.join(".kiro/steering/compass.md");
    let removed_steering = remove_managed_adapter(
        &steering,
        asset_text("compass-integrations/kiro-steering.md").unwrap_or_default(),
        &mut lines,
        "  .kiro/steering/compass.md  ->  removed",
    );
    let mut removed = Vec::new();
    if removed_skill {
        removed.push(".kiro/skills/compass/SKILL.md");
    }
    if removed_steering {
        removed.push(".kiro/steering/compass.md");
    }
    lines.push(format!(
        "Removed: {}",
        if removed.is_empty() {
            "nothing to remove".to_owned()
        } else {
            removed.join(", ")
        }
    ));
    uninstall_outcome(lines, "nothing to remove")
}

fn uninstall_antigravity(root: &Path, project: bool) -> Outcome {
    let mut lines = Vec::new();
    let rule = root.join(".agents/rules/compass.md");
    let removed_rule = remove_managed_adapter(
        &rule,
        asset_text("compass-integrations/antigravity-rules.md").unwrap_or_default(),
        &mut lines,
        &format!("compass rule removed from {}", absolute_display(&rule)),
    );
    if !removed_rule && !rule.exists() {
        lines.push("No compass Antigravity rule found - nothing to do".to_owned());
    }
    let workflow = root.join(".agents/workflows/compass.md");
    remove_managed_adapter(
        &workflow,
        asset_text("compass-integrations/antigravity-workflow.md").unwrap_or_default(),
        &mut lines,
        &format!(
            "compass workflow removed from {}",
            absolute_display(&workflow)
        ),
    );
    if let Some(config) = platform("antigravity") {
        remove_skill(config, project, root, &mut lines);
    }
    uninstall_outcome(lines, "nothing to remove")
}

fn install_agents_with_global_skill(name: &str, root: &Path) -> Outcome {
    let Some(config) = platform(name) else {
        return Outcome::failure(format!("error: unknown platform '{name}'"));
    };
    let skill = match install_skill(config, false, root) {
        Ok(value) => value,
        Err(error) => return Outcome::failure(error),
    };
    let mut lines = skill.messages;
    match install_agents(root, name, &mut lines) {
        Ok(()) => Outcome::success(lines.join("\n")),
        Err(error) => Outcome::failure(error),
    }
}

fn install_kilo_direct(root: &Path) -> Outcome {
    let Some(config) = platform("kilo") else {
        return Outcome::failure("error: Kilo platform is unavailable".to_owned());
    };
    let skill = match install_skill(config, false, root) {
        Ok(value) => value,
        Err(error) => return Outcome::failure(error),
    };
    let mut lines = skill.messages;
    if let Err(error) = install_kilo_command(&mut lines) {
        return Outcome::failure(error);
    }
    append_done(&mut lines);
    if let Err(error) = install_agents(root, "kilo", &mut lines) {
        return Outcome::failure(error);
    }
    Outcome::success(lines.join("\n"))
}

fn install_antigravity_direct(root: &Path, prefix: &str) -> Outcome {
    let mut outcome = install_platform("antigravity", false, root, false, prefix);
    if outcome.code != 0 {
        return outcome;
    }
    let Some(config) = platform("antigravity") else {
        return Outcome::failure("error: Antigravity platform is unavailable".to_owned());
    };
    let Ok(skill) = skill_destination(config, false, root) else {
        return Outcome::failure("error: could not resolve Antigravity skill".to_owned());
    };
    let mut lines = Vec::new();
    if let Err(error) = finalize_antigravity(root, &skill, &mut lines) {
        return Outcome::failure(error);
    }
    outcome.stdout.push_str(&format!("\n{}", lines.join("\n")));
    outcome.stdout.push_str("\n\nAntigravity will now check the knowledge graph before answering\ncodebase questions. Run /compass first to build the graph.");
    outcome.stdout.push_str("\n\nTo enable full MCP architecture navigation, add this to ~/.gemini/antigravity/mcp_config.json:\n  \"compass\": {\n    \"command\": \"compass\",\n    \"args\": [\"serve\", \"${workspace.path}/compass-out/graph.json\"]\n  }");
    outcome
}

fn install_kiro_direct(root: &Path) -> Outcome {
    let Some(config) = platform("kiro") else {
        return Outcome::failure("error: Kiro platform is unavailable".to_owned());
    };
    let skill = match install_skill(config, true, root) {
        Ok(value) => value,
        Err(error) => return Outcome::failure(error),
    };
    let mut lines = skill.messages;
    if let Err(error) = write_managed_adapter(
        root.join(".kiro/steering/compass.md"),
        asset_text("compass-integrations/kiro-steering.md").unwrap_or_default(),
    ) {
        return Outcome::failure(error);
    }
    lines.push("  .kiro/steering/compass.md  ->  always-on steering written".to_owned());
    lines.push(String::new());
    lines.push("Kiro will now read the knowledge graph before every conversation.".to_owned());
    lines.push("Use /compass to build or update the graph.".to_owned());
    Outcome::success(lines.join("\n"))
}

fn append_done(lines: &mut Vec<String>) {
    lines.push(String::new());
    lines.push("Done. Open your AI coding assistant and type:".to_owned());
    lines.push(String::new());
    lines.push("  /compass .".to_owned());
    lines.push(String::new());
}

fn append_project_hint(lines: &mut Vec<String>, root: &Path, paths: &[PathBuf]) {
    let mut values = Vec::new();
    for path in paths {
        let mut value = relative_display(path, root)
            .trim_end_matches('/')
            .to_owned();
        if path.is_dir() {
            value.push('/');
        }
        if !values.contains(&value) {
            values.push(value);
        }
    }
    lines.push(String::new());
    lines.push("Project-scoped install. Add to version control:".to_owned());
    lines.push(format!("  git add {}", values.join(" ")));
}

struct SkillInstall {
    path: PathBuf,
    messages: Vec<String>,
    changed: bool,
}

fn require_owned_or_absent(destination: &Path) -> Result<(), String> {
    if !destination.exists() {
        if let Some(parent) = destination.parent()
            && parent.exists()
            && fs::read_dir(parent)
                .map_err(|error| format!("error: could not inspect {}: {error}", parent.display()))?
                .next()
                .is_some()
        {
            return Err(format!(
                "error: {} exists but is not an empty or Compass-managed skill directory",
                parent.display()
            ));
        }
        return Ok(());
    }
    if is_managed_skill(destination) {
        Ok(())
    } else {
        Err(format!(
            "error: {} exists but is not managed by Compass",
            destination.display()
        ))
    }
}

fn is_managed_skill(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if parent.join(".compass-install.json").is_file() {
        return verify_manifest(parent).is_ok();
    }
    parent.join(".compass_version").is_file() && legacy_skill_is_unmodified(parent)
}

fn install_skill(
    config: Platform,
    project: bool,
    project_dir: &Path,
) -> Result<SkillInstall, String> {
    let destination = skill_destination(config, project, project_dir)?;
    let consumers = BTreeSet::from([config.name.to_owned()]);
    let scope = if project { "project" } else { "user" };
    let mut install = install_skill_at_scoped(destination.clone(), consumers, scope, project_dir)?;
    let parent = destination
        .parent()
        .ok_or_else(|| "error: invalid skill destination".to_owned())?;
    install.messages = vec![
        format!(
            "  references       ->  {}",
            display_path(&parent.join("references"), project, project_dir)
        ),
        format!(
            "  skill installed  ->  {}",
            display_path(&destination, project, project_dir)
        ),
    ];
    Ok(install)
}

fn skill_destination(
    config: Platform,
    project: bool,
    project_dir: &Path,
) -> Result<PathBuf, String> {
    if project {
        return Ok(project_dir.join(match config.name {
            "devin" => ".devin/skills/compass/SKILL.md",
            _ => config.skill_destination,
        }));
    }
    let home = home_directory()
        .ok_or_else(|| "error: could not determine user home directory".to_owned())?;
    if matches!(config.name, "claude" | "windows") && env::var_os("CLAUDE_CONFIG_DIR").is_some() {
        return Ok(claude_config_root(&home).join("skills/compass/SKILL.md"));
    }
    Ok(match config.name {
        "hermes" if cfg!(windows) => env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"))
            .join("hermes/skills/compass/SKILL.md"),
        "devin" => home.join(".config/devin/skills/compass/SKILL.md"),
        "amp" => home.join(".agents/skills/compass/SKILL.md"),
        "agents" => home.join(".agents/skills/compass/SKILL.md"),
        "antigravity" | "antigravity-windows" => home.join(".agents/skills/compass/SKILL.md"),
        _ => home.join(config.skill_destination),
    })
}

fn claude_config_root(default_root: &Path) -> PathBuf {
    env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                default_root.join(path)
            }
        })
        .unwrap_or_else(|| default_root.join(".claude"))
}

fn install_gemini(project: bool, project_dir: &Path) -> Outcome {
    let config = Platform::new("gemini", ".agents/skills/compass/SKILL.md");
    let skill = match install_skill(config, project, project_dir) {
        Ok(value) => value,
        Err(error) => return Outcome::failure(error),
    };
    let mut lines = skill.messages;
    let target = project_dir.join("GEMINI.md");
    if let Err(error) = update_section(
        &target,
        "## compass",
        asset_text("compass-integrations/gemini-md.md").unwrap_or_default(),
    ) {
        return Outcome::failure(error);
    }
    lines.push(format!(
        "compass section written to {}",
        absolute_display(&target)
    ));
    if let Err(error) = install_gemini_hook(project_dir) {
        return Outcome::failure(error);
    }
    lines.push("  .gemini/settings.json  ->  BeforeTool hook registered".to_owned());
    if project {
        lines.push(String::new());
        lines.push("Project-scoped install. Add to version control:".to_owned());
        lines.push("  git add .gemini/ GEMINI.md".to_owned());
    }
    lines.push(String::new());
    lines.push("Gemini CLI will now check the knowledge graph before answering".to_owned());
    lines.push("codebase questions and rebuild it after code changes.".to_owned());
    Outcome::success(lines.join("\n"))
}

fn install_cursor(project_dir: &Path, project_hint: bool) -> Outcome {
    let path = project_dir.join(".cursor/rules/compass.mdc");
    if let Err(error) = write_managed_adapter(path.clone(), CURSOR_RULE) {
        return Outcome::failure(error);
    }
    let mut output = format!(
        "compass rule written at {}\n\nCursor will now always include the knowledge graph context.\nRun /compass . first to build the graph if you haven't already.",
        absolute_display(&path)
    );
    if project_hint {
        output.push_str("\n\nProject-scoped install. Add to version control:\n  git add .cursor/");
    }
    Outcome::success(output)
}

fn install_vscode(project_dir: &Path) -> Outcome {
    let Some(home) = home_directory() else {
        return Outcome::failure("error: could not determine user home directory".to_owned());
    };
    let skill = match install_skill_at(home.join(".copilot/skills/compass/SKILL.md")) {
        Ok(value) => value,
        Err(error) => return Outcome::failure(error),
    };
    let mut lines = skill.messages;
    let instructions = project_dir.join(".github/copilot-instructions.md");
    if let Err(error) = update_section(
        &instructions,
        "## compass",
        asset_text("compass-integrations/vscode-instructions.md").unwrap_or_default(),
    ) {
        return Outcome::failure(error);
    }
    lines.push(format!(
        "  {}  ->  created",
        relative_display(&instructions, project_dir)
    ));
    lines.push(String::new());
    lines.push(
        "VS Code Copilot Chat configured. Type /compass in the chat panel to build the graph."
            .to_owned(),
    );
    lines.push("Note: for GitHub Copilot CLI (terminal), use: compass copilot install".to_owned());
    Outcome::success(lines.join("\n"))
}

fn install_skill_at(destination: PathBuf) -> Result<SkillInstall, String> {
    let root = destination
        .ancestors()
        .nth(4)
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    install_skill_at_scoped(
        destination,
        BTreeSet::from(["legacy".to_owned()]),
        "user",
        &root,
    )
}

fn install_skill_at_scoped(
    destination: PathBuf,
    consumers: BTreeSet<String>,
    scope: &str,
    root: &Path,
) -> Result<SkillInstall, String> {
    require_owned_or_absent(&destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| "error: invalid skill destination".to_owned())?
        .to_path_buf();
    let container = parent
        .parent()
        .ok_or_else(|| "error: invalid skill destination".to_owned())?;
    fs::create_dir_all(container).map_err(|error| {
        format!(
            "error: could not create skill container {}: {error}",
            container.display()
        )
    })?;
    let _lock = InstallLock::acquire(container)?;
    let mut merged_consumers = consumers;
    if let Ok(Some(existing)) = read_manifest(&parent) {
        merged_consumers.extend(existing.consumers);
    }
    let body = asset_text(SKILL_ASSET)
        .ok_or_else(|| format!("error: {SKILL_ASSET} not found in package - reinstall compass"))?;
    if manifest_is_current(&parent, body, &merged_consumers)? {
        return Ok(SkillInstall {
            path: destination,
            messages: vec![format!("  skill current    ->  {}", parent.display())],
            changed: false,
        });
    }

    let unique = format!("{}-{}", std::process::id(), monotonic_stamp());
    let stage = container.join(format!(".compass-stage-{unique}"));
    let backup = container.join(format!(".compass-backup-{unique}"));
    fs::create_dir(&stage)
        .map_err(|error| format!("error: could not stage {}: {error}", stage.display()))?;
    let staged_result = (|| {
        install_asset_tree(
            &format!("{REFERENCE_BUNDLE}/references/"),
            &stage.join("references"),
        )?;
        install_asset_tree(
            &format!("{REFERENCE_BUNDLE}/agents/"),
            &stage.join("agents"),
        )?;
        write_owned(stage.join("SKILL.md"), body)?;
        write_owned(stage.join(".compass_version"), SKILL_VERSION)?;
        let manifest = build_manifest(&stage, scope, root, merged_consumers)?;
        let text = serde_json::to_string_pretty(&manifest)
            .map_err(|error| format!("error: could not encode ownership manifest: {error}"))?;
        write_owned(stage.join(".compass-install.json"), &text)
    })();
    if let Err(error) = staged_result {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }

    let had_previous = parent.exists();
    if had_previous {
        fs::rename(&parent, &backup).map_err(|error| {
            let _ = fs::remove_dir_all(&stage);
            format!(
                "error: could not prepare existing skill {} for update: {error}",
                parent.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(&stage, &parent) {
        let mut restore_error = None;
        if had_previous && let Err(error) = fs::rename(&backup, &parent) {
            restore_error = Some(error);
        }
        let _ = fs::remove_dir_all(&stage);
        return Err(format!(
            "error: could not activate staged skill {}: {error}{}",
            parent.display(),
            restore_error.map_or_else(String::new, |restore| format!(
                "; could not restore previous package: {restore}"
            ))
        ));
    }
    if had_previous {
        // The new package is already atomically active. A stale backup is safer
        // than reporting a failed installation after the requested state exists.
        let _ = fs::remove_dir_all(&backup);
    }
    Ok(SkillInstall {
        path: destination,
        messages: vec![
            format!(
                "  references       ->  {}",
                parent.join("references").display()
            ),
            format!(
                "  skill installed  ->  {}",
                parent.join("SKILL.md").display()
            ),
        ],
        changed: true,
    })
}

fn uninstall_platform(name: &str, project: bool, project_dir: &Path, _prefix: &str) -> Outcome {
    if name == "codebuddy" && project {
        return uninstall_codebuddy_direct(project_dir, true);
    }
    if name == "kiro" && project {
        return uninstall_kiro_direct(project_dir);
    }
    if matches!(name, "antigravity" | "antigravity-windows") && project {
        return uninstall_antigravity(project_dir, true);
    }
    if name == "cursor" {
        let path = project_dir.join(".cursor/rules/compass.mdc");
        let mut lines = Vec::new();
        if !path.exists() {
            return Outcome::success("No compass Cursor rule found - nothing to do".to_owned());
        }
        remove_managed_adapter(
            &path,
            CURSOR_RULE,
            &mut lines,
            "compass Cursor rule removed",
        );
        return uninstall_outcome(lines, "nothing to remove");
    }
    if name == "vscode" {
        return uninstall_vscode(project_dir);
    }
    if name == "gemini" {
        let mut lines = Vec::new();
        let config = Platform::new("gemini", ".agents/skills/compass/SKILL.md");
        let _remaining = remove_skill(config, project, project_dir, &mut lines);
        strip_section_file(&project_dir.join("GEMINI.md"), "## compass", &mut lines);
        remove_json_hooks(
            &project_dir.join(".gemini/settings.json"),
            "BeforeTool",
            &mut lines,
        );
        return uninstall_outcome(lines, "nothing to remove");
    }
    let Some(config) = platform(name) else {
        return Outcome::failure(format!("error: unknown platform '{name}'"));
    };
    let mut lines = Vec::new();
    let remaining = remove_skill(config, project, project_dir, &mut lines);
    if project {
        match name {
            "claude" | "windows" => {
                remove_registration(&project_dir.join(".claude/CLAUDE.md"), &mut lines);
                strip_section_file(&project_dir.join("CLAUDE.md"), "## compass", &mut lines);
                remove_json_hooks(
                    &project_dir.join(".claude/settings.json"),
                    "PreToolUse",
                    &mut lines,
                );
                remove_json_hooks(
                    &project_dir.join(".claude/settings.local.json"),
                    "PreToolUse",
                    &mut lines,
                );
            }
            "agents" | "codex" | "opencode" | "aider" | "amp" | "claw" | "droid" | "trae"
            | "trae-cn" | "hermes" | "kilo" => {
                let agents_consumers = [
                    "agents", "codex", "opencode", "aider", "amp", "claw", "droid", "trae",
                    "trae-cn", "hermes", "kilo",
                ];
                if !remaining
                    .iter()
                    .any(|consumer| agents_consumers.contains(&consumer.as_str()))
                {
                    strip_section_file(&project_dir.join("AGENTS.md"), "## compass", &mut lines);
                }
                if name == "codex" {
                    remove_json_hooks(
                        &project_dir.join(".codex/hooks.json"),
                        "PreToolUse",
                        &mut lines,
                    );
                } else if name == "opencode" {
                    remove_opencode(project_dir, &mut lines);
                } else if name == "kilo" {
                    remove_kilo(project_dir, &mut lines);
                }
            }
            "copilot" => {
                strip_section_file(
                    &project_dir.join(".github/copilot-instructions.md"),
                    "## compass",
                    &mut lines,
                );
            }
            "kiro" => {
                remove_managed_adapter(
                    &project_dir.join(".kiro/steering/compass.md"),
                    asset_text("compass-integrations/kiro-steering.md").unwrap_or_default(),
                    &mut lines,
                    "  .kiro/steering/compass.md  ->  removed",
                );
            }
            "devin" => {
                remove_managed_adapter(
                    &project_dir.join(".windsurf/rules/compass.md"),
                    DEVIN_RULES,
                    &mut lines,
                    "  rules removed  ->  .windsurf/rules/compass.md",
                );
            }
            "antigravity" | "antigravity-windows" => {
                remove_managed_adapter(
                    &project_dir.join(".agents/rules/compass.md"),
                    asset_text("compass-integrations/antigravity-rules.md").unwrap_or_default(),
                    &mut lines,
                    "  .agents/rules/compass.md  ->  removed",
                );
                remove_managed_adapter(
                    &project_dir.join(".agents/workflows/compass.md"),
                    asset_text("compass-integrations/antigravity-workflow.md").unwrap_or_default(),
                    &mut lines,
                    "  .agents/workflows/compass.md  ->  removed",
                );
            }
            _ => {}
        }
    } else {
        match name {
            "claude" | "windows" => remove_registration(
                &claude_config_root(project_dir).join("CLAUDE.md"),
                &mut lines,
            ),
            "opencode" => remove_opencode(project_dir, &mut lines),
            "kilo" => remove_kilo(project_dir, &mut lines),
            _ => {}
        }
    }
    if lines.is_empty() {
        lines.push("nothing to remove".to_owned());
    }
    uninstall_outcome(lines, "nothing to remove")
}

fn uninstall_all(project: bool, purge: bool, project_dir: &Path, prefix: &str) -> Outcome {
    let mut lines = vec![
        if project {
            "Uninstalling project-scoped compass files...".to_owned()
        } else {
            "Uninstalling compass from all detected platforms...".to_owned()
        },
        String::new(),
    ];
    for name in PLATFORM_NAMES.iter().copied().chain(["gemini", "cursor"]) {
        let outcome = uninstall_platform(name, project, project_dir, prefix);
        if !outcome.stdout.is_empty() && outcome.stdout != "nothing to remove" {
            lines.push(outcome.stdout);
        }
    }
    if purge {
        let target = match scoped_compass_output(project_dir) {
            Ok(target) => target,
            Err(error) => return Outcome::failure(error),
        };
        if target.exists() {
            if let Err(error) = fs::remove_dir_all(&target) {
                return Outcome::failure(format!(
                    "error: could not remove {}: {error}",
                    target.display()
                ));
            }
            lines.push(format!("removed {}", target.display()));
        }
    }
    lines.push(String::new());
    lines.push("Done.".to_owned());
    Outcome::success(lines.join("\n"))
}

fn install_agents(root: &Path, name: &str, lines: &mut Vec<String>) -> Result<(), String> {
    let path = root.join("AGENTS.md");
    update_section(
        &path,
        "## compass",
        asset_text("compass-integrations/agents-md.md").unwrap_or_default(),
    )?;
    lines.push(format!(
        "compass section written to {}",
        absolute_display(&path)
    ));
    match name {
        "codex" => install_codex_hook(root, lines)?,
        "opencode" => install_opencode(root, lines)?,
        "kilo" => install_kilo_plugin(root, lines)?,
        _ => {}
    }
    lines.push(String::new());
    lines.push(format!(
        "{} will now check the knowledge graph before answering",
        capitalize(name)
    ));
    lines.push("codebase questions and rebuild it after code changes.".to_owned());
    if !matches!(name, "codex" | "opencode" | "kilo") {
        lines.push(String::new());
        lines.push(
            "Note: unlike Claude Code, there is no PreToolUse hook equivalent for".to_owned(),
        );
        lines.push(format!(
            "{} — the AGENTS.md rules are the always-on mechanism.",
            capitalize(name)
        ));
    }
    Ok(())
}

fn register_claude_skill(root: &Path, lines: &mut Vec<String>) -> Result<(), String> {
    let path = root.join(".claude/CLAUDE.md");
    let registration = "# compass\n- **compass** (`.claude/skills/compass/SKILL.md`) - any input to knowledge graph. Trigger: `/compass`\nWhen the user types `/compass`, use the installed compass skill or instructions before doing anything else.\n";
    append_registration(&path, registration)?;
    lines.push("  CLAUDE.md        ->  created at .claude/CLAUDE.md".to_owned());
    Ok(())
}

fn register_global_claude(lines: &mut Vec<String>) -> Result<(), String> {
    let home = home_directory()
        .ok_or_else(|| "error: could not determine user home directory".to_owned())?;
    let path = home.join(".claude/CLAUDE.md");
    let registration = "# compass\n- **compass** (`~/.claude/skills/compass/SKILL.md`) - any input to knowledge graph. Trigger: `/compass`\nWhen the user types `/compass`, use the installed compass skill or instructions before doing anything else.\n";
    append_registration(&path, registration)?;
    lines.push(format!(
        "  CLAUDE.md        ->  created at {}",
        path.display()
    ));
    Ok(())
}

fn register_codebuddy(lines: &mut Vec<String>) -> Result<(), String> {
    let home = home_directory()
        .ok_or_else(|| "error: could not determine user home directory".to_owned())?;
    let path = home.join(".codebuddy/CODEBUDDY.md");
    let registration = "# compass\n- **compass** (`~/.codebuddy/skills/compass/SKILL.md`) - any input to knowledge graph. Trigger: `/compass`\nWhen the user types `/compass`, use the installed compass skill or instructions before doing anything else.\n";
    append_registration(&path, registration)?;
    lines.push(format!(
        "  CODEBUDDY.md     ->  created at {}",
        path.display()
    ));
    Ok(())
}

fn install_markdown_and_claude_hook(
    root: &Path,
    strict: bool,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    let path = root.join("CLAUDE.md");
    update_section(
        &path,
        "## compass",
        asset_text("compass-integrations/claude-md.md").unwrap_or_default(),
    )?;
    lines.push(format!(
        "compass section written to {}",
        absolute_display(&path)
    ));
    install_claude_hook(root, strict)?;
    lines.push(format!(
        "  .claude/settings.json  ->  PreToolUse hooks registered (Bash|Grep search + Read/Glob){}",
        if strict { " (strict)" } else { "" }
    ));
    Ok(())
}

fn install_claude_hook(root: &Path, strict: bool) -> Result<(), String> {
    let path = root.join(".claude/settings.json");
    let mut document = load_json_object(&path)?;
    let hooks = object_child(&mut document, "hooks")?;
    let existing = hooks
        .remove("PreToolUse")
        .map(strict_json_array)
        .transpose()?
        .unwrap_or_default();
    let mut values = existing
        .into_iter()
        .filter(|value| !is_compass_hook(value))
        .collect::<Vec<_>>();
    let executable = compass_executable();
    values.push(json!({"matcher":"Bash|Grep","hooks":[{"type":"command","command":format!("{executable} hook-guard search")}]}));
    let read = format!(
        "{executable} hook-guard read{}",
        if strict { " --strict" } else { "" }
    );
    values.push(json!({"matcher":"Read|Glob","hooks":[{"type":"command","command":read}]}));
    hooks.insert("PreToolUse".to_owned(), Value::Array(values));
    write_json_object(path, &document)
}

fn install_codebuddy_hook(root: &Path) -> Result<(), String> {
    let path = root.join(".codebuddy/settings.json");
    let mut document = load_json_object(&path)?;
    let hooks = object_child(&mut document, "hooks")?;
    let existing = hooks
        .remove("PreToolUse")
        .map(strict_json_array)
        .transpose()?
        .unwrap_or_default();
    let mut values = existing
        .into_iter()
        .filter(|value| !is_compass_hook(value))
        .collect::<Vec<_>>();
    let executable = compass_executable();
    values.push(json!({"matcher":"Bash|Grep","hooks":[{"type":"command","command":format!("{executable} hook-guard search")}]}));
    values.push(json!({"matcher":"Read|Glob","hooks":[{"type":"command","command":format!("{executable} hook-guard read")}]}));
    hooks.insert("PreToolUse".to_owned(), Value::Array(values));
    write_json_object(path, &document)
}

fn install_gemini_hook(root: &Path) -> Result<(), String> {
    let path = root.join(".gemini/settings.json");
    let mut document = load_json_object(&path)?;
    let hooks = object_child(&mut document, "hooks")?;
    let existing = hooks
        .remove("BeforeTool")
        .map(strict_json_array)
        .transpose()?
        .unwrap_or_default();
    let mut values = existing
        .into_iter()
        .filter(|value| !is_compass_hook(value))
        .collect::<Vec<_>>();
    values.push(json!({"matcher":"read_file|list_directory","hooks":[{"type":"command","command":format!("{} hook-guard gemini", compass_executable())}]}));
    hooks.insert("BeforeTool".to_owned(), Value::Array(values));
    write_json_object(path, &document)
}

fn install_codex_hook(root: &Path, lines: &mut Vec<String>) -> Result<(), String> {
    let path = root.join(".codex/hooks.json");
    let mut document = load_json_object(&path)?;
    let hooks = object_child(&mut document, "hooks")?;
    let existing = hooks
        .remove("PreToolUse")
        .map(strict_json_array)
        .transpose()?
        .unwrap_or_default();
    let mut values = existing
        .into_iter()
        .filter(|value| !is_compass_hook(value))
        .collect::<Vec<_>>();
    let executable = compass_executable();
    values.push(json!({"matcher":"Bash","hooks":[{"type":"command","command":format!("{executable} hook-guard search")}]}));
    hooks.insert("PreToolUse".to_owned(), Value::Array(values));
    write_json_object(path, &document)?;
    lines.push(format!(
        "  .codex/hooks.json  ->  PreToolUse graph-first search guard registered ({executable} hook-guard search)"
    ));
    Ok(())
}

fn install_opencode(root: &Path, lines: &mut Vec<String>) -> Result<(), String> {
    let config = root.join(".opencode/opencode.json");
    let plugin = root.join(".opencode/plugins/compass.js");
    preflight_plugin_array(&config)?;
    preflight_managed_adapter(&plugin, OPENCODE_PLUGIN)?;
    write_managed_adapter(plugin, OPENCODE_PLUGIN)?;
    lines.push(
        "  .opencode/plugins/compass.js  ->  auto-discovered tool.execute.before hook written"
            .to_owned(),
    );
    if remove_plugin_registrations(
        &config,
        &["./plugins/compass.js", ".opencode/plugins/compass.js"],
    )? {
        lines.push("  .opencode/opencode.json  ->  duplicate registration removed".to_owned());
    }
    Ok(())
}

fn install_kilo_plugin(root: &Path, lines: &mut Vec<String>) -> Result<(), String> {
    let plugin = root.join(".kilo/plugins/compass.js");
    let config = root.join(".kilo/kilo.json");
    preflight_plugin_array(&config)?;
    preflight_managed_adapter(&plugin, KILO_PLUGIN)?;
    write_managed_adapter(plugin.clone(), KILO_PLUGIN)?;
    lines.push(
        "  .kilo/plugins/compass.js  ->  auto-discovered tool.execute.before hook written"
            .to_owned(),
    );
    let legacy_entry = legacy_kilo_plugin_entry(&plugin);
    if remove_plugin_registrations(
        &config,
        &[
            kilo_plugin_entry(),
            legacy_entry.as_str(),
            "file:./.kilo/plugins/compass.js",
        ],
    )? {
        lines.push("  .kilo/kilo.json  ->  duplicate registration removed".to_owned());
    }
    Ok(())
}

fn remove_plugin_registrations(config: &Path, entries: &[&str]) -> Result<bool, String> {
    if !config.is_file() {
        return Ok(false);
    }
    let mut document = load_json_object(config)?;
    let Some(plugins) = document.get_mut("plugin") else {
        return Ok(false);
    };
    let array = plugins.as_array_mut().ok_or_else(|| {
        format!(
            "error: {} field 'plugin' must be an array; file was not changed",
            config.display()
        )
    })?;
    let before = array.len();
    array.retain(|value| value.as_str().is_none_or(|entry| !entries.contains(&entry)));
    let changed = array.len() != before;
    if !changed {
        return Ok(false);
    }
    if array.is_empty() {
        document.remove("plugin");
    }
    write_json_object(config.to_path_buf(), &document)?;
    Ok(true)
}

fn finalize_antigravity(root: &Path, skill: &Path, lines: &mut Vec<String>) -> Result<(), String> {
    let body = fs::read_to_string(skill).map_err(|error| format!("error: {error}"))?;
    if !body.starts_with("---\n") {
        write_owned(
            skill.to_path_buf(),
            &format!(
                "---\nname: compass-manager\ndescription: Rebuild the code graph or perform manual CLI queries when MCP server is offline.\n---\n\n{body}"
            ),
        )?;
    }
    let rules = root.join(".agents/rules/compass.md");
    write_managed_adapter(
        rules.clone(),
        asset_text("compass-integrations/antigravity-rules.md").unwrap_or_default(),
    )?;
    lines.push(format!(
        "compass rule written to {}",
        absolute_display(&rules)
    ));
    let workflow = root.join(".agents/workflows/compass.md");
    write_managed_adapter(
        workflow.clone(),
        asset_text("compass-integrations/antigravity-workflow.md").unwrap_or_default(),
    )?;
    lines.push(format!(
        "compass workflow written to {}",
        absolute_display(&workflow)
    ));
    Ok(())
}

fn install_kilo_command(lines: &mut Vec<String>) -> Result<(), String> {
    let home = home_directory()
        .ok_or_else(|| "error: could not determine user home directory".to_owned())?;
    let path = home.join(".config/kilo/command/compass.md");
    write_managed_adapter(
        path.clone(),
        asset_text("compass-integrations/kilo-command.md").unwrap_or_default(),
    )?;
    lines.push(format!("  command installed ->  {}", path.display()));
    Ok(())
}

fn remove_skill(
    config: Platform,
    project: bool,
    project_dir: &Path,
    lines: &mut Vec<String>,
) -> BTreeSet<String> {
    let Ok(path) = skill_destination(config, project, project_dir) else {
        return BTreeSet::new();
    };
    let validation_root = if project {
        project_dir.to_path_buf()
    } else {
        home_directory().unwrap_or_else(|| project_dir.to_path_buf())
    };
    if let Err(error) = validate_skill_destination(&path, &validation_root) {
        lines.push(format!("  preserved unsafe skill destination: {error}"));
        return BTreeSet::new();
    }
    let parent = path.parent().map(Path::to_path_buf);
    if !is_managed_skill(&path) {
        return BTreeSet::new();
    }
    if let Some(parent) = &parent
        && let Ok(Some(mut manifest)) = read_manifest(parent)
    {
        let consumer = match config.name {
            "windows" => "claude",
            "antigravity-windows" => "antigravity",
            other => other,
        };
        let original_consumers = manifest.consumers.clone();
        manifest.consumers.remove(consumer);
        if !manifest.consumers.is_empty() {
            match serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("error: could not encode ownership manifest: {error}"))
                .and_then(|text| write_owned(parent.join(".compass-install.json"), &text))
            {
                Ok(()) => lines.push(format!(
                    "  {consumer} removed from shared skill consumers -> {}",
                    parent.display()
                )),
                Err(error) => {
                    lines.push(error);
                    return original_consumers;
                }
            }
            return manifest.consumers;
        }
    }
    if path.exists() {
        match fs::remove_file(&path) {
            Ok(()) => lines.push(format!(
                "  skill removed    ->  {}",
                display_path(&path, project, project_dir)
            )),
            Err(error) => {
                lines.push(format!(
                    "error: could not remove {}: {error}",
                    path.display()
                ));
                return BTreeSet::new();
            }
        }
    }
    if let Some(parent) = parent {
        for metadata in [
            parent.join(".compass_version"),
            parent.join(".compass-install.json"),
        ] {
            if metadata.exists()
                && let Err(error) = fs::remove_file(&metadata)
            {
                lines.push(format!(
                    "error: could not remove {}: {error}",
                    metadata.display()
                ));
            }
        }
        let references = parent.join("references");
        if references.exists()
            && let Err(error) = fs::remove_dir_all(&references)
        {
            lines.push(format!(
                "error: could not remove {}: {error}",
                references.display()
            ));
        }
        remove_empty_ancestors(&parent, if project { project_dir } else { Path::new("") });
    }
    BTreeSet::new()
}

fn uninstall_vscode(project_dir: &Path) -> Outcome {
    let mut lines = Vec::new();
    if let Some(home) = home_directory() {
        let path = home.join(".copilot/skills/compass/SKILL.md");
        if is_managed_skill(&path) && fs::remove_file(&path).is_ok() {
            lines.push(format!("  skill removed    ->  {}", path.display()));
            if let Some(parent) = path.parent() {
                let _ = fs::remove_file(parent.join(".compass_version"));
                let _ = fs::remove_dir_all(parent.join("references"));
            }
        }
    }
    let instructions = project_dir.join(".github/copilot-instructions.md");
    if let Ok(content) = fs::read_to_string(&instructions)
        && content.lines().any(|line| line.trim() == "## compass")
    {
        let clean = strip_heading_section(&content, "## compass");
        if clean.trim().is_empty() {
            if fs::remove_file(&instructions).is_ok() {
                lines.push(
                    "  .github/copilot-instructions.md  ->  deleted (was empty after removal)"
                        .to_owned(),
                );
            }
        } else if write_owned(instructions, &clean).is_ok() {
            lines.push("  compass section removed from .github/copilot-instructions.md".to_owned());
        }
    }
    Outcome::success(lines.join("\n"))
}

fn remove_opencode(root: &Path, lines: &mut Vec<String>) {
    let plugin = root.join(".opencode/plugins/compass.js");
    let existed = plugin.exists();
    let removed = remove_managed_adapter(
        &plugin,
        OPENCODE_PLUGIN,
        lines,
        "  .opencode/plugins/compass.js  ->  removed",
    );
    if existed && !removed {
        return;
    }
    let path = root.join(".opencode/opencode.json");
    let mut document = match load_json_object(&path) {
        Ok(document) => document,
        Err(error) => {
            lines.push(error);
            return;
        }
    };
    if let Some(plugins) = document.get_mut("plugin").and_then(Value::as_array_mut) {
        let before = plugins.len();
        plugins.retain(|value| {
            !matches!(
                value.as_str(),
                Some("./plugins/compass.js" | ".opencode/plugins/compass.js")
            )
        });
        let changed = plugins.len() != before;
        let empty = plugins.is_empty();
        if empty {
            document.remove("plugin");
        }
        if changed {
            match write_json_object(path, &document) {
                Ok(()) => {
                    lines.push("  .opencode/opencode.json  ->  plugin deregistered".to_owned())
                }
                Err(error) => lines.push(error),
            }
        }
    }
}

fn remove_kilo(root: &Path, lines: &mut Vec<String>) {
    let plugin = root.join(".kilo/plugins/compass.js");
    let entry = kilo_plugin_entry();
    let legacy_entry = legacy_kilo_plugin_entry(&plugin);
    let existed = plugin.exists();
    let removed = remove_managed_adapter(
        &plugin,
        KILO_PLUGIN,
        lines,
        "  .kilo/plugins/compass.js  ->  removed",
    );
    if existed && !removed {
        return;
    }
    let path = root.join(".kilo/kilo.json");
    let mut document = match load_json_object(&path) {
        Ok(document) => document,
        Err(error) => {
            lines.push(error);
            return;
        }
    };
    if let Some(plugins) = document.get_mut("plugin").and_then(Value::as_array_mut) {
        let before = plugins.len();
        plugins.retain(|value| {
            !matches!(value.as_str(), Some(candidate) if candidate == entry || candidate == legacy_entry || candidate == "file:./.kilo/plugins/compass.js")
        });
        let changed = plugins.len() != before;
        let empty = plugins.is_empty();
        if empty {
            document.remove("plugin");
        }
        if changed {
            match write_json_object(path, &document) {
                Ok(()) => lines.push("  .kilo/kilo.json  ->  plugin deregistered".to_owned()),
                Err(error) => lines.push(error),
            }
        }
    }
}

fn kilo_plugin_entry() -> &'static str {
    "./plugins/compass.js"
}

fn legacy_kilo_plugin_entry(plugin: &Path) -> String {
    let absolute = fs::canonicalize(plugin).unwrap_or_else(|_| plugin.to_path_buf());
    if cfg!(windows) {
        format!("file:///{}", absolute.to_string_lossy().replace('\\', "/"))
    } else {
        format!("file://{}", absolute.display())
    }
}

fn remove_json_hooks(path: &Path, event: &str, lines: &mut Vec<String>) {
    if !path.exists() {
        return;
    }
    let mut document = match load_json_object(path) {
        Ok(document) => document,
        Err(error) => {
            lines.push(error);
            return;
        }
    };
    let Some(hooks_value) = document.get_mut("hooks") else {
        return;
    };
    let Some(hooks) = hooks_value.as_object_mut() else {
        lines.push(format!(
            "error: {} field 'hooks' must be an object; file was not changed",
            path.display()
        ));
        return;
    };
    let Some(event_value) = hooks.get_mut(event) else {
        return;
    };
    let Some(values) = event_value.as_array_mut() else {
        lines.push(format!(
            "error: {} hook '{event}' must be an array; file was not changed",
            path.display()
        ));
        return;
    };
    let before = values.len();
    values.retain(|value| !is_compass_hook(value));
    if values.len() != before {
        match write_json_object(path.to_path_buf(), &document) {
            Ok(()) => lines.push(format!(
                "  {}  ->  {event} hook removed",
                lexical_path(path).display()
            )),
            Err(error) => lines.push(error),
        }
    }
}

fn remove_registration(path: &Path, lines: &mut Vec<String>) {
    if !path.exists() {
        return;
    }
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            lines.push(format!("error: could not read {}: {error}", path.display()));
            return;
        }
    };
    let clean = if content.contains(COMPASS_REGISTRATION_START)
        && content.contains(COMPASS_REGISTRATION_END)
    {
        strip_registration(&content)
    } else if legacy_registration_is_owned(&content) {
        strip_heading_section(&content, "# compass")
    } else {
        return;
    };
    if clean.trim().is_empty() {
        match fs::remove_file(path) {
            Ok(()) => lines.push(format!(
                "  CLAUDE.md        ->  deleted {}",
                lexical_path(path).display()
            )),
            Err(error) => lines.push(format!(
                "error: could not remove {}: {error}",
                path.display()
            )),
        }
    } else {
        match write_owned(path.to_path_buf(), &clean) {
            Ok(()) => lines.push(format!(
                "  CLAUDE.md        ->  compass skill registration removed from {}",
                lexical_path(path).display()
            )),
            Err(error) => lines.push(error),
        }
    }
}

fn strip_section_file(path: &Path, marker: &str, lines: &mut Vec<String>) {
    if !path.exists() {
        return;
    }
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            lines.push(format!("error: could not read {}: {error}", path.display()));
            return;
        }
    };
    let clean = if content.contains(COMPASS_SECTION_START) && content.contains(COMPASS_SECTION_END)
    {
        strip_managed_section(&content)
    } else if legacy_section_is_owned(&content, marker) {
        strip_heading_section(&content, marker)
    } else {
        return;
    };
    if clean.trim().is_empty() {
        match fs::remove_file(path) {
            Ok(()) => lines.push(format!(
                "{} was empty after removal - deleted {}",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file"),
                absolute_display(path)
            )),
            Err(error) => lines.push(format!(
                "error: could not remove {}: {error}",
                path.display()
            )),
        }
    } else {
        match write_owned(path.to_path_buf(), &clean) {
            Ok(()) => lines.push(format!(
                "compass section removed from {}",
                absolute_display(path)
            )),
            Err(error) => lines.push(error),
        }
    }
}

#[cfg(test)]
fn remove_owned_file(path: PathBuf, missing: &str, removed: &str) -> Outcome {
    if !path.exists() {
        return Outcome::success(missing.to_owned());
    }
    match fs::remove_file(&path) {
        Ok(()) => Outcome::success(format!("{removed} from {}", absolute_display(&path))),
        Err(error) => Outcome::failure(format!(
            "error: could not remove {}: {error}",
            path.display()
        )),
    }
}

#[cfg(test)]
fn remove_file(path: &Path, lines: &mut Vec<String>) {
    if path.exists() && fs::remove_file(path).is_ok() {
        lines.push(format!("removed {}", path.display()));
    }
}

fn remove_managed_adapter(
    path: &Path,
    legacy_body: &str,
    lines: &mut Vec<String>,
    removed_label: &str,
) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(current) = fs::read_to_string(path) else {
        lines.push(format!(
            "error: preserved unreadable managed adapter {}",
            path.display()
        ));
        return false;
    };
    if current != legacy_body && managed_adapter_body(&current).is_none() {
        lines.push(format!(
            "preserved unowned or user-modified adapter {}",
            path.display()
        ));
        return false;
    }
    match fs::remove_file(path) {
        Ok(()) => {
            lines.push(removed_label.to_owned());
            true
        }
        Err(error) => {
            lines.push(format!(
                "error: could not remove {}: {error}",
                path.display()
            ));
            false
        }
    }
}

#[cfg(test)]
fn remove_labeled_file(path: &Path, label: &str, lines: &mut Vec<String>) {
    if path.is_file() && fs::remove_file(path).is_ok() {
        lines.push(label.to_owned());
    }
}

fn append_registration(path: &Path, registration: &str) -> Result<(), String> {
    let current = read_optional_text(path)?;
    let managed = format!(
        "{COMPASS_REGISTRATION_START}\n{}\n{COMPASS_REGISTRATION_END}",
        registration.trim()
    );
    let clean = if current.contains(COMPASS_REGISTRATION_START)
        && current.contains(COMPASS_REGISTRATION_END)
    {
        strip_registration(&current)
    } else if current.lines().any(|line| line.trim() == "# compass") {
        if !legacy_registration_is_owned(&current) {
            return Err(format!(
                "error: {} contains an unowned '# compass' section; file was not changed",
                path.display()
            ));
        }
        strip_heading_section(&current, "# compass")
    } else {
        current
    };
    let output = if clean.trim().is_empty() {
        format!("{managed}\n")
    } else {
        format!("{}\n\n{managed}\n", clean.trim_end())
    };
    write_owned(path.to_path_buf(), &output)
}

const COMPASS_REGISTRATION_START: &str = "<!-- compass:registration:start -->";
const COMPASS_REGISTRATION_END: &str = "<!-- compass:registration:end -->";

fn strip_registration(content: &str) -> String {
    let Some(start) = content.find(COMPASS_REGISTRATION_START) else {
        return content.to_owned();
    };
    let Some(end_offset) = content[start..].find(COMPASS_REGISTRATION_END) else {
        return content.to_owned();
    };
    let end = start + end_offset + COMPASS_REGISTRATION_END.len();
    let output = [content[..start].trim_end(), content[end..].trim_start()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if output.is_empty() {
        output
    } else {
        format!("{output}\n")
    }
}

fn legacy_registration_is_owned(content: &str) -> bool {
    heading_section(content, "# compass").is_some_and(|section| {
        section.contains("- **compass** (")
            && section.contains(
                "When the user types `/compass`, use the installed compass skill or instructions",
            )
    })
}

fn update_section(path: &Path, marker: &str, section: &str) -> Result<(), String> {
    let current = read_optional_text(path)?;
    let managed = format!(
        "{COMPASS_SECTION_START}\n{}\n{COMPASS_SECTION_END}",
        section.trim()
    );
    let output = if current.contains(COMPASS_SECTION_START) && current.contains(COMPASS_SECTION_END)
    {
        replace_managed_section(&current, &managed)?
    } else if current.lines().any(|line| line.trim() == marker) {
        if !legacy_section_is_owned(&current, marker) {
            return Err(format!(
                "error: {} contains an unowned '{marker}' section; file was not changed",
                path.display()
            ));
        }
        replace_or_append_section(&current, marker, &managed)
    } else if current.trim().is_empty() {
        format!("{managed}\n")
    } else {
        format!("{}\n\n{managed}\n", current.trim_end())
    };
    write_owned(path.to_path_buf(), &output)
}

const COMPASS_SECTION_START: &str = "<!-- compass:managed:start -->";
const COMPASS_SECTION_END: &str = "<!-- compass:managed:end -->";

fn replace_managed_section(content: &str, section: &str) -> Result<String, String> {
    let start = content
        .find(COMPASS_SECTION_START)
        .ok_or_else(|| "error: Compass section start marker is missing".to_owned())?;
    let tail_start = content[start..]
        .find(COMPASS_SECTION_END)
        .map(|offset| start + offset + COMPASS_SECTION_END.len())
        .ok_or_else(|| "error: Compass section end marker is missing".to_owned())?;
    let mut output = format!(
        "{}{}{}",
        content[..start].trim_end(),
        if content[..start].trim().is_empty() {
            ""
        } else {
            "\n\n"
        },
        section.trim()
    );
    let tail = content[tail_start..].trim_start();
    if !tail.is_empty() {
        output.push_str("\n\n");
        output.push_str(tail);
    }
    output.push('\n');
    Ok(output)
}

fn strip_managed_section(content: &str) -> String {
    let Some(start) = content.find(COMPASS_SECTION_START) else {
        return content.to_owned();
    };
    let Some(end_offset) = content[start..].find(COMPASS_SECTION_END) else {
        return content.to_owned();
    };
    let end = start + end_offset + COMPASS_SECTION_END.len();
    let head = content[..start].trim_end();
    let tail = content[end..].trim_start();
    let output = [head, tail]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if output.is_empty() {
        output
    } else {
        format!("{output}\n")
    }
}

fn legacy_section_is_owned(content: &str, marker: &str) -> bool {
    let Some(section) = heading_section(content, marker) else {
        return false;
    };
    [
        "compass-integrations/agents-md.md",
        "compass-integrations/claude-md.md",
        "compass-integrations/gemini-md.md",
        "compass-integrations/vscode-instructions.md",
    ]
    .into_iter()
    .filter_map(asset_text)
    .any(|asset| asset.trim() == section.trim())
}

fn heading_section<'a>(content: &'a str, marker: &str) -> Option<&'a str> {
    let start = content.find(marker)?;
    let after = start + marker.len();
    let end = content[after..]
        .find("\n## ")
        .map_or(content.len(), |offset| after + offset + 1);
    Some(&content[start..end])
}

fn replace_or_append_section(content: &str, marker: &str, section: &str) -> String {
    let lines = content.split('\n').collect::<Vec<_>>();
    let Some(start) = lines.iter().rposition(|line| line.trim() == marker) else {
        return if content.trim().is_empty() {
            section.trim_start().to_owned()
        } else {
            format!("{}\n\n{}", content.trim_end(), section.trim_start())
        };
    };
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.starts_with("## "))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let mut parts = Vec::new();
    let head = lines[..start].join("\n");
    if !head.trim().is_empty() {
        parts.push(head.trim_end().to_owned());
    }
    parts.push(section.trim().to_owned());
    let tail = lines[end..].join("\n");
    if !tail.trim().is_empty() {
        parts.push(tail.trim_start().to_owned());
    }
    let output = parts.join("\n\n");
    if output.ends_with('\n') {
        output
    } else {
        format!("{output}\n")
    }
}

fn strip_heading_section(content: &str, marker: &str) -> String {
    let lines = content.split('\n').collect::<Vec<_>>();
    let Some(start) = lines.iter().rposition(|line| line.trim() == marker) else {
        return content.to_owned();
    };
    let heading = if marker.starts_with("## ") {
        "## "
    } else {
        "# "
    };
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.starts_with(heading))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let head = lines[..start].join("\n");
    let tail = lines[end..].join("\n");
    let output = [head.trim_end(), tail.trim_start()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if output.is_empty() {
        output
    } else {
        format!("{output}\n")
    }
}

fn install_asset_tree(prefix: &str, destination: &Path) -> Result<(), String> {
    let staged = destination.with_extension("tmp");
    remove_dir_if_exists(&staged)?;
    fs::create_dir_all(&staged)
        .map_err(|error| format!("error: could not create {}: {error}", staged.display()))?;
    let mut count = 0_usize;
    for asset in EMBEDDED_ASSETS
        .iter()
        .filter(|asset| asset.path.starts_with(prefix))
    {
        let relative = asset.path.strip_prefix(prefix).unwrap_or(asset.path);
        let path = staged.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("error: could not create {}: {error}", parent.display())
            })?;
        }
        fs::write(&path, asset.bytes)
            .map_err(|error| format!("error: could not write {}: {error}", path.display()))?;
        count += 1;
    }
    if count == 0 {
        let _ = fs::remove_dir_all(&staged);
        return Err(format!(
            "error: assets for package bundle '{prefix}' are missing"
        ));
    }
    remove_dir_if_exists(destination)?;
    fs::rename(&staged, destination).map_err(|error| {
        format!(
            "error: could not install {}: {error}",
            destination.display()
        )
    })
}

fn asset_text(path: &str) -> Option<&'static str> {
    let asset = EMBEDDED_ASSETS.iter().find(|asset| asset.path == path)?;
    std::str::from_utf8(asset.bytes).ok()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct OwnershipManifest {
    schema: u32,
    compass_version: String,
    scope: String,
    root: PathBuf,
    consumers: BTreeSet<String>,
    files: BTreeMap<String, String>,
}

fn build_manifest(
    staged: &Path,
    scope: &str,
    root: &Path,
    consumers: BTreeSet<String>,
) -> Result<OwnershipManifest, String> {
    Ok(OwnershipManifest {
        schema: 1,
        compass_version: SKILL_VERSION.to_owned(),
        scope: scope.to_owned(),
        root: lexical_path(root),
        consumers,
        files: collect_managed_files(staged)?,
    })
}

fn read_manifest(directory: &Path) -> Result<Option<OwnershipManifest>, String> {
    let path = directory.join(".compass-install.json");
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("error: could not read {}: {error}", path.display()))?;
    let manifest = serde_json::from_slice::<OwnershipManifest>(&bytes).map_err(|error| {
        format!(
            "error: invalid ownership manifest {}: {error}",
            path.display()
        )
    })?;
    if manifest.schema != 1 {
        return Err(format!(
            "error: unsupported ownership manifest schema {} in {}",
            manifest.schema,
            path.display()
        ));
    }
    Ok(Some(manifest))
}

fn verify_manifest(directory: &Path) -> Result<OwnershipManifest, String> {
    let manifest = read_manifest(directory)?.ok_or_else(|| {
        format!(
            "error: ownership manifest missing from {}",
            directory.display()
        )
    })?;
    let actual = collect_managed_files(directory)?;
    if actual != manifest.files {
        return Err(format!(
            "error: managed skill {} contains modified, missing, or unowned files; Compass will not overwrite it",
            directory.display()
        ));
    }
    for relative in manifest.files.keys() {
        let _validated = safe_manifest_path(directory, relative)?;
    }
    Ok(manifest)
}

fn manifest_is_current(
    directory: &Path,
    skill_body: &str,
    consumers: &BTreeSet<String>,
) -> Result<bool, String> {
    let Some(manifest) = read_manifest(directory)? else {
        return Ok(false);
    };
    let verified = verify_manifest(directory)?;
    if verified.compass_version != SKILL_VERSION || &verified.consumers != consumers {
        return Ok(false);
    }
    let mut expected = expected_package_digests(skill_body);
    expected.insert(
        ".compass_version".to_owned(),
        digest_bytes(SKILL_VERSION.as_bytes()),
    );
    Ok(manifest.files == expected)
}

fn expected_package_digests(skill_body: &str) -> BTreeMap<String, String> {
    let mut files = BTreeMap::from([("SKILL.md".to_owned(), digest_bytes(skill_body.as_bytes()))]);
    let prefix = format!("{REFERENCE_BUNDLE}/");
    for asset in EMBEDDED_ASSETS {
        if let Some(relative) = asset.path.strip_prefix(&prefix)
            && (relative.starts_with("references/") || relative.starts_with("agents/"))
        {
            files.insert(relative.to_owned(), digest_bytes(asset.bytes));
        }
    }
    files
}

fn collect_managed_files(directory: &Path) -> Result<BTreeMap<String, String>, String> {
    if fs::symlink_metadata(directory)
        .map_err(|error| format!("error: could not inspect {}: {error}", directory.display()))?
        .file_type()
        .is_symlink()
    {
        return Err(format!(
            "error: managed skill directory {} must not be a symbolic link",
            directory.display()
        ));
    }
    let mut files = BTreeMap::new();
    collect_managed_files_from(directory, directory, &mut files)?;
    files.remove(".compass-install.json");
    Ok(files)
}

fn collect_managed_files_from(
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|error| format!("error: could not inspect {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| format!("error: could not inspect skill: {error}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("error: could not inspect {}: {error}", path.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "error: managed skill {} contains symbolic link {}; Compass will not follow it",
                root.display(),
                path.display()
            ));
        }
        if file_type.is_dir() {
            collect_managed_files_from(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| format!("error: invalid managed path: {error}"))?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative, digest_file(&path)?);
        } else {
            return Err(format!(
                "error: managed skill {} contains unsupported file type {}",
                root.display(),
                path.display()
            ));
        }
    }
    Ok(())
}

fn safe_manifest_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "error: invalid managed path '{relative}' in {}",
            root.join(".compass-install.json").display()
        ));
    }
    Ok(root.join(path))
}

fn digest_file(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|bytes| digest_bytes(&bytes))
        .map_err(|error| {
            format!(
                "error: could not read managed file {}: {error}",
                path.display()
            )
        })
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn legacy_skill_is_unmodified(directory: &Path) -> bool {
    let Some(body) = asset_text(SKILL_ASSET) else {
        return false;
    };
    let mut expected = expected_package_digests(body);
    expected.insert(
        ".compass_version".to_owned(),
        digest_bytes(SKILL_VERSION.as_bytes()),
    );
    collect_managed_files(directory).is_ok_and(|actual| actual == expected)
}

struct InstallLock {
    path: PathBuf,
}

impl InstallLock {
    fn acquire(container: &Path) -> Result<Self, String> {
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::time::{Duration, SystemTime};

        let path = container.join(".compass-install.lock");
        if let Ok(metadata) = fs::metadata(&path)
            && metadata
                .modified()
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age > Duration::from_secs(600))
        {
            let _ = fs::remove_file(&path);
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "error: another Compass installation is using {}: {error}",
                    container.display()
                )
            })?;
        writeln!(file, "pid={}", std::process::id())
            .map_err(|error| format!("error: could not initialize install lock: {error}"))?;
        Ok(Self { path })
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn monotonic_stamp() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

const MANAGED_MARKDOWN_PREFIX: &str = "<!-- compass:managed-file sha256:";
const MANAGED_SCRIPT_PREFIX: &str = "// compass:managed-file sha256:";

fn preflight_managed_adapter(path: &Path, body: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let current = fs::read_to_string(path)
        .map_err(|error| format!("error: could not read {}: {error}", path.display()))?;
    if current == body || managed_adapter_body(&current).is_some() {
        return Ok(());
    }
    Err(format!(
        "error: {} exists but is unowned or user-modified; file was not changed",
        path.display()
    ))
}

fn write_managed_adapter(path: PathBuf, body: &str) -> Result<(), String> {
    preflight_managed_adapter(&path, body)?;
    write_owned(path.clone(), &managed_adapter_content(&path, body))
}

fn managed_adapter_content(path: &Path, body: &str) -> String {
    let digest = digest_bytes(body.as_bytes());
    if path.extension().and_then(|extension| extension.to_str()) == Some("js") {
        return format!("{MANAGED_SCRIPT_PREFIX}{digest}\n{body}");
    }
    let marker = format!("{MANAGED_MARKDOWN_PREFIX}{digest} -->\n");
    if body.starts_with("---\n")
        && let Some(offset) = body[4..].find("\n---\n")
    {
        let insertion = 4 + offset + 5;
        return format!("{}{}{}", &body[..insertion], marker, &body[insertion..]);
    }
    format!("{marker}{body}")
}

fn managed_adapter_body(content: &str) -> Option<String> {
    for prefix in [MANAGED_MARKDOWN_PREFIX, MANAGED_SCRIPT_PREFIX] {
        let Some(start) = content.find(prefix) else {
            continue;
        };
        if start != 0 && content.as_bytes().get(start.wrapping_sub(1)) != Some(&b'\n') {
            continue;
        }
        let line_end = content[start..]
            .find('\n')
            .map_or(content.len(), |offset| start + offset);
        let suffix = content[start + prefix.len()..line_end].trim();
        let recorded = suffix.strip_suffix("-->").unwrap_or(suffix).trim();
        let after = if line_end < content.len() {
            line_end + 1
        } else {
            line_end
        };
        let body = format!("{}{}", &content[..start], &content[after..]);
        if digest_bytes(body.as_bytes()) == recorded {
            return Some(body);
        }
    }
    None
}

fn write_owned(path: PathBuf, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("error: could not create {}: {error}", parent.display()))?;
    }
    write_text_atomic(&path, content).map_err(|error| format!("error: {error}"))
}

fn read_optional_text(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!(
            "error: could not read {} as UTF-8 ({error}); file was not changed",
            path.display()
        )),
    }
}

fn scoped_compass_output(root: &Path) -> Result<PathBuf, String> {
    let relative = env::var_os("COMPASS_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("compass-out"));
    let invalid = relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::CurDir
                    | Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        });
    if invalid {
        return Err(
            "error: COMPASS_OUT must be a non-empty relative subdirectory without '.' or '..' for --purge"
                .to_owned(),
        );
    }
    let target = root.join(relative);
    if target.exists() {
        let canonical_root = fs::canonicalize(root)
            .map_err(|error| format!("error: could not resolve {}: {error}", root.display()))?;
        let canonical_target = fs::canonicalize(&target).map_err(|error| {
            format!(
                "error: could not safely resolve purge target {}: {error}",
                target.display()
            )
        })?;
        if canonical_target == canonical_root || !canonical_target.starts_with(&canonical_root) {
            return Err(format!(
                "error: purge target {} resolves outside the selected scope",
                target.display()
            ));
        }
    }
    Ok(target)
}

fn validate_skill_destination(destination: &Path, default_root: &Path) -> Result<(), String> {
    if destination
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "error: skill destination {} contains a parent-directory component",
            destination.display()
        ));
    }
    let boundary = if destination.starts_with(default_root) {
        default_root.to_path_buf()
    } else if let Some(configured) = env::var_os("CLAUDE_CONFIG_DIR") {
        let configured = PathBuf::from(configured);
        let configured = if configured.is_absolute() {
            configured
        } else {
            default_root.join(configured)
        };
        if destination.starts_with(&configured) {
            configured
        } else {
            return Err(format!(
                "error: skill destination {} is outside the selected scope",
                destination.display()
            ));
        }
    } else {
        return Err(format!(
            "error: skill destination {} is outside the selected scope",
            destination.display()
        ));
    };
    let canonical_boundary = match fs::canonicalize(&boundary) {
        Ok(path) => Some(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "error: could not resolve skill destination boundary {}: {error}",
                boundary.display()
            ));
        }
    };

    let mut current = destination.parent();
    while let Some(path) = current {
        if !path.starts_with(&boundary) {
            break;
        }
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let resolved = fs::canonicalize(path).map_err(|error| {
                    format!(
                        "error: could not resolve symbolic link {} while validating skill destination {}: {error}",
                        path.display(),
                        destination.display()
                    )
                })?;
                if canonical_boundary
                    .as_ref()
                    .is_some_and(|boundary| !resolved.starts_with(boundary))
                {
                    return Err(format!(
                        "error: skill destination {} resolves outside the selected scope through symbolic link {}",
                        destination.display(),
                        path.display()
                    ));
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "error: could not inspect skill destination {}: {error}",
                    path.display()
                ));
            }
        }
        if path == boundary {
            break;
        }
        current = path.parent();
    }
    Ok(())
}

fn load_json_object(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("error: could not read {}: {error}", path.display()))?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
        format!(
            "error: {} contains invalid JSON ({error}); file was not changed",
            path.display()
        )
    })?;
    value.as_object().cloned().ok_or_else(|| {
        format!(
            "error: {} must contain a JSON object; file was not changed",
            path.display()
        )
    })
}

fn write_json_object(path: PathBuf, object: &Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(object).map_err(|error| format!("error: {error}"))?;
    write_owned(path, &text)
}

fn object_child<'a>(
    object: &'a mut Map<String, Value>,
    name: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    let value = object
        .entry(name.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    value.as_object_mut().ok_or_else(|| {
        format!("error: JSON field '{name}' must be an object; file was not changed")
    })
}

fn strict_json_array(value: Value) -> Result<Vec<Value>, String> {
    value.as_array().cloned().ok_or_else(|| {
        "error: managed hook field must be an array; file was not changed".to_owned()
    })
}

fn is_compass_hook(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() == 1 {
        return object
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_managed_compass_command);
    }
    if object.len() != 2 {
        return false;
    }
    let Some(matcher) = object.get("matcher").and_then(Value::as_str) else {
        return false;
    };
    let Some(hooks) = object.get("hooks").and_then(Value::as_array) else {
        return false;
    };
    let [hook] = hooks.as_slice() else {
        return false;
    };
    let Some(hook) = hook.as_object() else {
        return false;
    };
    if hook.len() != 2 || hook.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    let Some(command) = hook.get("command").and_then(Value::as_str) else {
        return false;
    };
    if !is_managed_compass_command(command) {
        return false;
    }
    matches!(
        (matcher, managed_command_suffix(command).as_deref()),
        ("Bash", Some("hook-check"))
            | ("Bash", Some("hook-guard search"))
            | ("Bash|Grep", Some("hook-guard search"))
            | ("Read|Glob", Some("hook-guard read"))
            | ("Read|Glob", Some("hook-guard read --strict"))
            | ("read_file|list_directory", Some("hook-guard gemini"))
    )
}

fn managed_command_suffix(command: &str) -> Option<String> {
    let mut parts = command.split_whitespace();
    parts.next()?;
    Some(parts.collect::<Vec<_>>().join(" "))
}

fn is_managed_compass_command(command: &str) -> bool {
    let mut parts = command.split_whitespace();
    let Some(executable) = parts.next() else {
        return false;
    };
    if Path::new(executable)
        .file_stem()
        .and_then(|value| value.to_str())
        != Some("compass")
    {
        return false;
    }
    matches!(
        parts.collect::<Vec<_>>().as_slice(),
        ["hook-check"]
            | ["hook-guard", "search"]
            | ["hook-guard", "read"]
            | ["hook-guard", "read", "--strict"]
            | ["hook-guard", "gemini"]
    )
}

fn compass_executable() -> String {
    "compass".to_owned()
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("error: could not remove {}: {error}", path.display()))?;
    }
    Ok(())
}

fn remove_empty_ancestors(start: &Path, boundary: &Path) {
    let mut current = Some(start);
    for _ in 0..3 {
        let Some(path) = current else { break };
        if !boundary.as_os_str().is_empty() && path == boundary {
            break;
        }
        if fs::remove_dir(path).is_err() {
            break;
        }
        current = path.parent();
    }
}

fn project_scope_root(path: &Path, project: &Path) -> PathBuf {
    path.strip_prefix(project)
        .ok()
        .and_then(|relative| relative.components().next())
        .map_or_else(
            || path.to_path_buf(),
            |component| project.join(component.as_os_str()),
        )
}

fn display_path(path: &Path, project: bool, project_dir: &Path) -> String {
    if project {
        relative_display(path, project_dir)
    } else {
        path.display().to_string()
    }
}

fn relative_display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn absolute_display(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| {
            if path.is_absolute() {
                lexical_path(path)
            } else {
                lexical_path(&env::current_dir().unwrap_or_default().join(path))
            }
        })
        .display()
        .to_string()
}

fn lexical_path(path: &Path) -> PathBuf {
    path.components()
        .filter(|component| !matches!(component, std::path::Component::CurDir))
        .collect()
}

fn home_directory() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

const DEVIN_RULES: &str = include_str!("../assets/compass-integrations/agents-md.md");
const CURSOR_RULE: &str = include_str!("../assets/compass-integrations/agents-md.md");
#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    #[test]
    fn section_replacement_preserves_surrounding_user_content() {
        let input = "# User\n\n## compass\nold\n\n## Keep\nvalue\n";
        assert_eq!(
            replace_or_append_section(input, "## compass", "## compass\nnew\n"),
            "# User\n\n## compass\nnew\n\n## Keep\nvalue\n"
        );
    }

    #[test]
    fn canonical_compass_skill_package_is_native() {
        let body = asset_text(SKILL_ASSET).unwrap_or_default();
        assert!(body.starts_with("---\nname: compass\n"));
        assert!(
            EMBEDDED_ASSETS
                .iter()
                .all(|asset| !asset.bytes.contains(&b'\r')),
            "embedded text assets must use canonical LF line endings"
        );
        assert!(body.contains("references/query.md"));
        assert!(body.contains("compass query"));
        assert!(body.contains("--text-budget"));
        assert!(body.contains("next=none"));
        let openai_metadata = asset_text("compass-skill/agents/openai.yaml").unwrap_or_default();
        assert!(openai_metadata.contains("display_name: \"Compass\""));
        assert!(openai_metadata.contains("$compass"));
        let query = asset_text("compass-skill/references/query.md").unwrap_or_default();
        assert!(query.contains("--text-budget"));
        assert!(query.contains("--cursor <TOKEN>"));
        assert!(query.contains("additional pages remain"));
        for adapter in [
            "compass-integrations/agents-md.md",
            "compass-integrations/antigravity-rules.md",
            "compass-integrations/claude-md.md",
            "compass-integrations/gemini-md.md",
            "compass-integrations/kiro-steering.md",
            "compass-integrations/vscode-instructions.md",
        ] {
            let adapter = asset_text(adapter).unwrap_or_default();
            assert!(adapter.contains("compass init"));
            assert!(adapter.contains("compass watch"));
            assert!(adapter.contains("--cursor"));
            assert!(adapter.contains("next=none"));
        }
        for adapter in [DEVIN_RULES, CURSOR_RULE] {
            assert!(adapter.contains("compass init"));
            assert!(adapter.contains("compass watch"));
            assert!(adapter.contains("--cursor"));
            assert!(adapter.contains("next=none"));
        }
        assert!(!body.contains("python -m"), "stale token python -m");
        assert!(
            EMBEDDED_ASSETS
                .iter()
                .any(|asset| asset.path.starts_with("compass-skill/references/"))
        );
    }

    #[test]
    fn parser_and_platform_boundaries_fail_without_mutation() -> Result<(), String> {
        let multiple = parse_install_request(&["claude".to_owned(), "codex".to_owned()])?;
        assert_eq!(multiple.platforms, ["claude", "codex"]);
        let multiple_equals =
            parse_install_request(&["--platform=claude".to_owned(), "codex".to_owned()])?;
        assert_eq!(multiple_equals.platforms, ["claude", "codex"]);
        assert!(parse_install_request(&["--all".to_owned(), "codex".to_owned()]).is_err());
        assert_eq!(command_platform(Frontend::Compass, "codex", &[]).code, 1);
        assert_eq!(
            command_platform(Frontend::Compass, "codex", &["bad".to_owned()]).code,
            1
        );
        assert_eq!(
            install_platform("bad", true, Path::new("."), false, "compass").code,
            1
        );
        assert_eq!(
            uninstall_platform("bad", true, Path::new("."), "compass").code,
            1
        );
        assert!(platform("bad").is_none());
        assert_eq!(canonical_platform("skills"), "agents");
        assert_eq!(canonical_platform("codex"), "codex");
        for command in DIRECT_COMMANDS {
            assert!(
                is_direct_command(command),
                "missing direct command {command}"
            );
        }
        assert!(!is_direct_command("install"));
        assert!(!is_direct_command("unknown"));
        for name in PLATFORM_NAMES {
            assert!(is_install_platform(name), "missing install platform {name}");
        }
        assert!(is_install_platform("gemini"));
        assert!(is_install_platform("cursor"));
        assert!(!is_install_platform("vscode"));
        assert!(!is_install_platform("unknown"));
        Ok(())
    }

    #[test]
    fn project_uninstall_all_purges_only_the_scoped_output()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("compass-out");
        fs::create_dir_all(&output)?;
        fs::write(output.join("graph.json"), "{}")?;
        let outcome = uninstall_all(true, true, directory.path(), "compass");
        assert_eq!(outcome.code, 0);
        assert!(outcome.stdout.contains("project-scoped"));
        assert!(outcome.stdout.contains("removed"));
        assert!(outcome.stdout.ends_with("Done."));
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn hook_installers_reject_invalid_shapes_and_preserve_unowned_entries()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        write(
            &root.join(".claude/settings.json"),
            r#"{"hooks":{"PreToolUse":[{"command":"keep"},{"command":"compass old"}]}}"#,
        )?;
        install_claude_hook(root, true)?;
        let claude = load_json_object(&root.join(".claude/settings.json"))?;
        let hooks = claude["hooks"]["PreToolUse"]
            .as_array()
            .ok_or("missing Claude hooks")?;
        assert_eq!(hooks.len(), 4);
        assert!(
            hooks
                .iter()
                .any(|hook| hook.to_string().contains("compass old"))
        );
        assert!(
            hooks
                .iter()
                .any(|hook| hook.to_string().contains("--strict"))
        );

        write(&root.join(".codebuddy/settings.json"), r#"{"hooks":7}"#)?;
        assert!(install_codebuddy_hook(root).is_err());
        assert_eq!(
            fs::read_to_string(root.join(".codebuddy/settings.json"))?,
            r#"{"hooks":7}"#
        );
        write(&root.join(".gemini/settings.json"), r#"{"hooks":null}"#)?;
        assert!(install_gemini_hook(root).is_err());
        assert_eq!(
            fs::read_to_string(root.join(".gemini/settings.json"))?,
            r#"{"hooks":null}"#
        );
        Ok(())
    }

    #[test]
    fn plugin_install_preserves_scalar_configs_without_partial_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        write(&root.join(".opencode/opencode.json"), r#"{"plugin":7}"#)?;
        write(&root.join(".kilo/kilo.json"), r#"{"plugin":false}"#)?;
        let mut lines = Vec::new();
        assert!(install_opencode(root, &mut lines).is_err());
        assert!(install_kilo_plugin(root, &mut lines).is_err());
        assert!(!root.join(".opencode/plugins/compass.js").exists());
        assert!(!root.join(".kilo/plugins/compass.js").exists());
        assert_eq!(
            fs::read_to_string(root.join(".opencode/opencode.json"))?,
            r#"{"plugin":7}"#
        );
        assert_eq!(
            fs::read_to_string(root.join(".kilo/kilo.json"))?,
            r#"{"plugin":false}"#
        );
        Ok(())
    }

    #[test]
    fn transaction_snapshots_restore_original_and_remove_new_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let original = root.join("original.json");
        let created = root.join("created.md");
        write(&original, "{\"keep\":true}")?;
        let snapshots = snapshot_files(&[original.clone(), created.clone()])?;

        write(&original, "{\"changed\":true}")?;
        write(&created, "temporary")?;
        assert!(snapshots_changed(&snapshots));
        assert_eq!(
            restore_files(&snapshots),
            "restored all adapter and configuration files"
        );
        assert_eq!(fs::read_to_string(original)?, "{\"keep\":true}");
        assert!(!created.exists());
        Ok(())
    }

    #[test]
    fn owned_markdown_cleanup_covers_empty_preserved_and_unmarked_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let mut lines = Vec::new();

        let registration = root.join("CLAUDE.md");
        append_registration(&registration, "# compass\nowned\n")?;
        append_registration(&registration, "# compass\nduplicate\n")?;
        remove_registration(&registration, &mut lines);
        assert!(!registration.exists());

        let agents = root.join("AGENTS.md");
        write(&agents, "# User\n\n## compass\nowned\n\n## Keep\nvalue\n")?;
        strip_section_file(&agents, "## compass", &mut lines);
        assert_eq!(
            fs::read_to_string(&agents)?,
            "# User\n\n## compass\nowned\n\n## Keep\nvalue\n"
        );
        write(
            &agents,
            "# User\n\n<!-- compass:managed:start -->\n## compass\nowned\n<!-- compass:managed:end -->\n\n## Keep\nvalue\n",
        )?;
        strip_section_file(&agents, "## compass", &mut lines);
        assert_eq!(fs::read_to_string(&agents)?, "# User\n\n## Keep\nvalue\n\n");
        let untouched = root.join("untouched.md");
        write(&untouched, "# User\n")?;
        strip_section_file(&untouched, "## compass", &mut lines);
        assert_eq!(fs::read_to_string(&untouched)?, "# User\n");

        let labeled = root.join("owned.md");
        write(&labeled, "owned")?;
        remove_labeled_file(&labeled, "removed label", &mut lines);
        let plain = root.join("plain.md");
        write(&plain, "owned")?;
        remove_file(&plain, &mut lines);
        assert!(lines.iter().any(|line| line == "removed label"));
        Ok(())
    }

    #[test]
    fn json_hook_cleanup_and_owned_file_results_are_explicit()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let hooks = root.join("hooks.json");
        write(
            &hooks,
            r#"{"hooks":{"PreToolUse":[{"command":"keep"},{"command":"compass hook"},{"command":"compass hook-check"},{"matcher":"Bash","hooks":[{"type":"command","command":"compass hook-check"},{"type":"command","command":"keep"}]}]}}"#,
        )?;
        let mut lines = Vec::new();
        remove_json_hooks(&hooks, "PreToolUse", &mut lines);
        let document = load_json_object(&hooks)?;
        assert_eq!(
            document["hooks"]["PreToolUse"].as_array().map(Vec::len),
            Some(3)
        );
        remove_json_hooks(&hooks, "Missing", &mut lines);
        remove_json_hooks(&root.join("missing.json"), "PreToolUse", &mut lines);

        let owned = root.join("owned.txt");
        write(&owned, "owned")?;
        assert_eq!(
            remove_owned_file(owned.clone(), "missing", "removed").code,
            0
        );
        assert_eq!(
            remove_owned_file(owned, "missing", "removed").stdout,
            "missing"
        );
        Ok(())
    }

    #[test]
    fn asset_tree_json_and_path_helpers_cover_boundary_shapes()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let destination = directory.path().join("references");
        assert!(install_asset_tree("compass-skill/references/", &destination).is_ok());
        assert!(destination.is_dir());
        assert!(install_asset_tree("missing-prefix/", &destination).is_err());

        let mut object = Map::new();
        object.insert("hooks".to_owned(), Value::Bool(false));
        assert!(object_child(&mut object, "hooks").is_err());
        assert_eq!(object["hooks"], false);

        let nested = directory.path().join("one/two/three");
        fs::create_dir_all(&nested)?;
        remove_empty_ancestors(&nested, directory.path());
        assert!(!nested.exists());
        assert_eq!(
            project_scope_root(&directory.path().join("one/two"), directory.path()),
            directory.path().join("one")
        );
        assert_eq!(
            project_scope_root(Path::new("elsewhere"), directory.path()),
            PathBuf::from("elsewhere")
        );
        assert_eq!(
            display_path(&directory.path().join("x"), true, directory.path()),
            "x"
        );
        assert_eq!(capitalize("compass"), "Compass");
        assert_eq!(capitalize(""), "");
        Ok(())
    }

    #[test]
    fn antigravity_finalization_adds_frontmatter_and_owned_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let skill = directory.path().join("skill.md");
        write(&skill, "# Body\n")?;
        let mut lines = Vec::new();
        finalize_antigravity(directory.path(), &skill, &mut lines)?;
        assert!(fs::read_to_string(&skill)?.starts_with("---\nname: compass-manager"));
        assert!(directory.path().join(".agents/rules/compass.md").is_file());
        assert!(
            directory
                .path()
                .join(".agents/workflows/compass.md")
                .is_file()
        );
        finalize_antigravity(directory.path(), &skill, &mut lines)?;
        assert_eq!(lines.len(), 4);
        Ok(())
    }
}
