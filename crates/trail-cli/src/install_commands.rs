use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use trail_files::write_text_atomic;

use crate::{Frontend, Outcome};

const COMPAT_VERSION: &str = "0.9.20";
const PLATFORM_NAMES: &[&str] = &[
    "claude",
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
    skill_file: &'static str,
    skill_destination: &'static str,
    references: Option<&'static str>,
}

pub(crate) fn is_direct_command(command: &str) -> bool {
    DIRECT_COMMANDS.contains(&command)
}

pub(crate) fn command_install(frontend: Frontend, args: &[String]) -> Outcome {
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
            "note: --strict applies to the project PreToolUse hook; run `{prefix} install --project --strict` or `graphify claude install --strict`."
        );
        return outcome;
    }
    install_platform(selected, project, Path::new("."), strict, prefix)
}

pub(crate) fn command_uninstall(frontend: Frontend, args: &[String]) -> Outcome {
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

pub(crate) fn command_platform(frontend: Frontend, command: &str, args: &[String]) -> Outcome {
    let prefix = command_prefix(frontend);
    let Some(action) = args.first().map(String::as_str) else {
        return Outcome::failure(format!("Usage: {prefix} {command} [install|uninstall]"));
    };
    if !matches!(action, "install" | "uninstall") {
        return Outcome::failure(format!("Usage: {prefix} {command} [install|uninstall]"));
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
            strip_section_file(&root.join("AGENTS.md"), "## graphify", &mut lines);
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

fn command_prefix(frontend: Frontend) -> &'static str {
    match frontend {
        Frontend::Trail => "trail graph",
        Frontend::Graphify => "graphify",
    }
}

fn install_help(prefix: &str) -> String {
    format!(
        "Usage: {prefix} install [--project] [--strict] [--platform P|P]\nPlatforms: {}, gemini, cursor\n  --strict  block the first raw file read per session until one `graphify query` runs (Claude Code project hook only; needs --project)",
        PLATFORM_NAMES.join(", ")
    )
}

fn platform(name: &str) -> Option<Platform> {
    let name = PLATFORM_NAMES
        .iter()
        .copied()
        .find(|candidate| *candidate == name)?;
    let value = match name {
        "claude" => Platform::new(
            name,
            "skill.md",
            ".claude/skills/graphify/SKILL.md",
            Some("claude"),
        ),
        "windows" => Platform::new(
            name,
            "skill-windows.md",
            ".claude/skills/graphify/SKILL.md",
            Some("windows"),
        ),
        "codex" => Platform::new(
            name,
            "skill-codex.md",
            ".codex/skills/graphify/SKILL.md",
            Some("codex"),
        ),
        "opencode" => Platform::new(
            name,
            "skill-opencode.md",
            ".config/opencode/skills/graphify/SKILL.md",
            Some("opencode"),
        ),
        "kilo" => Platform::new(
            name,
            "skill-kilo.md",
            ".config/kilo/skills/graphify/SKILL.md",
            Some("kilo"),
        ),
        "aider" => Platform::new(name, "skill-aider.md", ".aider/graphify/SKILL.md", None),
        "copilot" => Platform::new(
            name,
            "skill-copilot.md",
            ".copilot/skills/graphify/SKILL.md",
            Some("copilot"),
        ),
        "claw" | "hermes" => Platform::new(
            name,
            "skill-claw.md",
            ".openclaw/skills/graphify/SKILL.md",
            Some("claw"),
        ),
        "droid" => Platform::new(
            name,
            "skill-droid.md",
            ".factory/skills/graphify/SKILL.md",
            Some("droid"),
        ),
        "trae" | "trae-cn" => Platform::new(
            name,
            "skill-trae.md",
            ".trae/skills/graphify/SKILL.md",
            Some("trae"),
        ),
        "kiro" => Platform::new(
            name,
            "skill-kiro.md",
            ".kiro/skills/graphify/SKILL.md",
            Some("kiro"),
        ),
        "pi" => Platform::new(
            name,
            "skill-pi.md",
            ".pi/agent/skills/graphify/SKILL.md",
            Some("pi"),
        ),
        "codebuddy" | "antigravity" => Platform::new(
            name,
            "skill.md",
            ".agents/skills/graphify/SKILL.md",
            Some("claude"),
        ),
        "antigravity-windows" => Platform::new(
            name,
            "skill-windows.md",
            ".agents/skills/graphify/SKILL.md",
            Some("windows"),
        ),
        "kimi" => Platform::new(
            name,
            "skill.md",
            ".kimi/skills/graphify/SKILL.md",
            Some("claude"),
        ),
        "amp" => Platform::new(
            name,
            "skill-amp.md",
            ".agents/skills/graphify/SKILL.md",
            Some("amp"),
        ),
        "agents" => Platform::new(
            name,
            "skill-agents.md",
            ".agents/skills/graphify/SKILL.md",
            Some("agents"),
        ),
        "devin" => Platform::new(
            name,
            "skill-devin.md",
            ".config/devin/skills/graphify/SKILL.md",
            None,
        ),
        _ => return None,
    };
    Some(value.with_specific_destination())
}

impl Platform {
    const fn new(
        name: &'static str,
        skill_file: &'static str,
        skill_destination: &'static str,
        references: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            skill_file,
            skill_destination,
            references,
        }
    }

    fn with_specific_destination(mut self) -> Self {
        self.skill_destination = match self.name {
            "opencode" => ".config/opencode/skills/graphify/SKILL.md",
            "hermes" => ".hermes/skills/graphify/SKILL.md",
            "trae-cn" => ".trae-cn/skills/graphify/SKILL.md",
            "codebuddy" => ".codebuddy/skills/graphify/SKILL.md",
            "antigravity" | "antigravity-windows" => ".agents/skills/graphify/SKILL.md",
            "amp" | "agents" => ".agents/skills/graphify/SKILL.md",
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
                        "one `graphify query` runs (toggle with GRAPHIFY_HOOK_STRICT=0)."
                            .to_owned(),
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
                if let Err(error) = write_owned(
                    root.join(".kiro/steering/graphify.md"),
                    asset_text("always_on/kiro-steering.md").unwrap_or_default(),
                ) {
                    return Outcome::failure(error);
                }
                lines.push(
                    "  .kiro/steering/graphify.md  ->  always-on steering written".to_owned(),
                );
                lines.push(String::new());
                lines.push(
                    "Kiro will now read the knowledge graph before every conversation.".to_owned(),
                );
                lines.push("Use /graphify to build or update the graph.".to_owned());
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
                    write_owned(root.join(".windsurf/rules/graphify.md"), DEVIN_RULES)
                {
                    return Outcome::failure(error);
                }
                lines.push("  rules written  ->  .windsurf/rules/graphify.md".to_owned());
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
        lines.push("one `graphify query` runs (toggle with GRAPHIFY_HOOK_STRICT=0).".to_owned());
    }
    Outcome::success(lines.join("\n"))
}

fn uninstall_claude_direct(root: &Path) -> Outcome {
    let mut lines = Vec::new();
    strip_section_file(&root.join("CLAUDE.md"), "## graphify", &mut lines);
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
    Outcome::success(if lines.is_empty() {
        "No CLAUDE.md found in current directory - nothing to do".to_owned()
    } else {
        lines.join("\n")
    })
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
        "## graphify",
        asset_text("always_on/claude-md.md").unwrap_or_default(),
    ) {
        return Outcome::failure(error);
    }
    lines.push(format!(
        "graphify section written to {}",
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
    strip_section_file(&root.join("CODEBUDDY.md"), "## graphify", &mut lines);
    remove_json_hooks(
        &root.join(".codebuddy/settings.json"),
        "PreToolUse",
        &mut lines,
    );
    Outcome::success(if lines.is_empty() {
        "No CODEBUDDY.md found in current directory - nothing to do".to_owned()
    } else {
        lines.join("\n")
    })
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
    strip_section_file(&root.join("AGENTS.md"), "## graphify", &mut lines);
    Outcome::success(if lines.is_empty() {
        "No AGENTS.md found in current directory - nothing to do".to_owned()
    } else {
        lines.join("\n")
    })
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
    Outcome::success(lines.join("\n"))
}

fn uninstall_kilo_direct(root: &Path) -> Outcome {
    let mut lines = Vec::new();
    strip_section_file(&root.join("AGENTS.md"), "## graphify", &mut lines);
    remove_kilo(root, &mut lines);
    if let Some(home) = home_directory() {
        let command = home.join(".config/kilo/command/graphify.md");
        let skill = home.join(".config/kilo/skills/graphify/SKILL.md");
        let mut removed = Vec::new();
        if command.exists() {
            let _ = fs::remove_file(&command);
            removed.push(format!("command removed: {}", command.display()));
        }
        if skill.exists() {
            let _ = fs::remove_file(&skill);
            removed.push(format!("skill removed: {}", skill.display()));
        }
        let _ = fs::remove_file(skill.with_file_name(".graphify_version"));
        remove_empty_ancestors(&skill.with_file_name("placeholder"), &home);
        if removed.is_empty() {
            lines.push("nothing to remove".to_owned());
        } else {
            lines.push(removed.join("; "));
        }
    }
    Outcome::success(lines.join("\n"))
}

fn uninstall_kiro_direct(root: &Path) -> Outcome {
    let Some(config) = platform("kiro") else {
        return Outcome::failure("error: Kiro platform is unavailable".to_owned());
    };
    let mut lines = Vec::new();
    let skill = skill_destination(config, true, root).ok();
    let removed_skill = skill.as_ref().is_some_and(|path| path.exists());
    remove_skill(config, true, root, &mut lines);
    let steering = root.join(".kiro/steering/graphify.md");
    let removed_steering = steering.exists();
    let _ = fs::remove_file(&steering);
    let mut removed = Vec::new();
    if removed_skill {
        removed.push(".kiro/skills/graphify/SKILL.md");
    }
    if removed_steering {
        removed.push(".kiro/steering/graphify.md");
    }
    lines.push(format!(
        "Removed: {}",
        if removed.is_empty() {
            "nothing to remove".to_owned()
        } else {
            removed.join(", ")
        }
    ));
    Outcome::success(lines.join("\n"))
}

fn uninstall_antigravity(root: &Path, project: bool) -> Outcome {
    let mut lines = Vec::new();
    let rule = root.join(".agents/rules/graphify.md");
    if rule.exists() {
        let _ = fs::remove_file(&rule);
        lines.push(format!(
            "graphify rule removed from {}",
            absolute_display(&rule)
        ));
    } else {
        lines.push("No graphify Antigravity rule found - nothing to do".to_owned());
    }
    let workflow = root.join(".agents/workflows/graphify.md");
    if workflow.exists() {
        let _ = fs::remove_file(&workflow);
        lines.push(format!(
            "graphify workflow removed from {}",
            absolute_display(&workflow)
        ));
    }
    if let Some(config) = platform("antigravity")
        && let Ok(skill) = skill_destination(config, project, root)
    {
        if skill.exists() {
            let _ = fs::remove_file(&skill);
            lines.push(format!(
                "graphify skill removed from {}",
                display_path(&skill, project, root)
            ));
        }
        let _ = fs::remove_file(skill.with_file_name(".graphify_version"));
        let _ = fs::remove_dir_all(skill.with_file_name("references"));
    }
    Outcome::success(lines.join("\n"))
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
    outcome.stdout.push_str("\n\nAntigravity will now check the knowledge graph before answering\ncodebase questions. Run /graphify first to build the graph.");
    outcome.stdout.push_str("\n\nTo enable full MCP architecture navigation, add this to ~/.gemini/antigravity/mcp_config.json:\n  \"graphify\": {\n    \"command\": \"uv\",\n    \"args\": [\"run\", \"--with\", \"graphifyy\", \"--with\", \"mcp\", \"-m\", \"graphify.serve\", \"${workspace.path}/graphify-out/graph.json\"]\n  }");
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
    if let Err(error) = write_owned(
        root.join(".kiro/steering/graphify.md"),
        asset_text("always_on/kiro-steering.md").unwrap_or_default(),
    ) {
        return Outcome::failure(error);
    }
    lines.push("  .kiro/steering/graphify.md  ->  always-on steering written".to_owned());
    lines.push(String::new());
    lines.push("Kiro will now read the knowledge graph before every conversation.".to_owned());
    lines.push("Use /graphify to build or update the graph.".to_owned());
    Outcome::success(lines.join("\n"))
}

fn append_done(lines: &mut Vec<String>) {
    lines.push(String::new());
    lines.push("Done. Open your AI coding assistant and type:".to_owned());
    lines.push(String::new());
    lines.push("  /graphify .".to_owned());
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
}

fn install_skill(
    config: Platform,
    project: bool,
    project_dir: &Path,
) -> Result<SkillInstall, String> {
    let destination = skill_destination(config, project, project_dir)?;
    let parent = destination
        .parent()
        .ok_or_else(|| "error: invalid skill destination".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("error: could not create {}: {error}", parent.display()))?;
    let mut messages = Vec::new();
    let refs_destination = parent.join("references");
    if let Some(bundle) = config.references {
        install_asset_tree(&format!("skills/{bundle}/references/"), &refs_destination)?;
        messages.push(format!(
            "  references       ->  {}",
            display_path(&refs_destination, project, project_dir)
        ));
    } else {
        remove_dir_if_exists(&refs_destination)?;
    }
    let body = asset_text(config.skill_file).ok_or_else(|| {
        format!(
            "error: {} not found in package - reinstall graphify",
            config.skill_file
        )
    })?;
    write_owned(destination.clone(), body)?;
    write_owned(parent.join(".graphify_version"), COMPAT_VERSION)?;
    messages.push(format!(
        "  skill installed  ->  {}",
        display_path(&destination, project, project_dir)
    ));
    Ok(SkillInstall {
        path: destination,
        messages,
    })
}

fn skill_destination(
    config: Platform,
    project: bool,
    project_dir: &Path,
) -> Result<PathBuf, String> {
    if project {
        return Ok(project_dir.join(match config.name {
            "opencode" => ".opencode/skills/graphify/SKILL.md",
            "devin" => ".devin/skills/graphify/SKILL.md",
            _ => config.skill_destination,
        }));
    }
    let home = home_directory()
        .ok_or_else(|| "error: could not determine user home directory".to_owned())?;
    if matches!(config.name, "claude" | "windows")
        && let Some(directory) = env::var_os("CLAUDE_CONFIG_DIR")
    {
        return Ok(PathBuf::from(directory).join("skills/graphify/SKILL.md"));
    }
    Ok(match config.name {
        "opencode" => home.join(".config/opencode/skills/graphify/SKILL.md"),
        "hermes" if cfg!(windows) => env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"))
            .join("hermes/skills/graphify/SKILL.md"),
        "devin" => home.join(".config/devin/skills/graphify/SKILL.md"),
        "amp" => home.join(".config/agents/skills/graphify/SKILL.md"),
        "agents" => home.join(".agents/skills/graphify/SKILL.md"),
        "antigravity" | "antigravity-windows" => {
            home.join(".gemini/config/skills/graphify/SKILL.md")
        }
        _ => home.join(config.skill_destination),
    })
}

fn install_gemini(project: bool, project_dir: &Path) -> Outcome {
    let config = Platform::new(
        "gemini",
        "skill.md",
        ".gemini/skills/graphify/SKILL.md",
        Some("claude"),
    );
    let skill = match install_skill(config, project, project_dir) {
        Ok(value) => value,
        Err(error) => return Outcome::failure(error),
    };
    let mut lines = skill.messages;
    let target = project_dir.join("GEMINI.md");
    if let Err(error) = update_section(
        &target,
        "## graphify",
        asset_text("always_on/gemini-md.md").unwrap_or_default(),
    ) {
        return Outcome::failure(error);
    }
    lines.push(format!(
        "graphify section written to {}",
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
    let path = project_dir.join(".cursor/rules/graphify.mdc");
    if let Err(error) = write_owned(path.clone(), CURSOR_RULE) {
        return Outcome::failure(error);
    }
    let mut output = format!(
        "graphify rule written at {}\n\nCursor will now always include the knowledge graph context.\nRun /graphify . first to build the graph if you haven't already.",
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
    let config = Platform::new(
        "vscode",
        "skill-vscode.md",
        ".copilot/skills/graphify/SKILL.md",
        Some("vscode"),
    );
    let skill = match install_skill_at(config, home.join(".copilot/skills/graphify/SKILL.md")) {
        Ok(value) => value,
        Err(error) => return Outcome::failure(error),
    };
    let mut lines = skill.messages;
    let instructions = project_dir.join(".github/copilot-instructions.md");
    if let Err(error) = update_section(
        &instructions,
        "## graphify",
        asset_text("always_on/vscode-instructions.md").unwrap_or_default(),
    ) {
        return Outcome::failure(error);
    }
    lines.push(format!(
        "  {}  ->  created",
        relative_display(&instructions, project_dir)
    ));
    lines.push(String::new());
    lines.push(
        "VS Code Copilot Chat configured. Type /graphify in the chat panel to build the graph."
            .to_owned(),
    );
    lines.push("Note: for GitHub Copilot CLI (terminal), use: graphify copilot install".to_owned());
    Outcome::success(lines.join("\n"))
}

fn install_skill_at(config: Platform, destination: PathBuf) -> Result<SkillInstall, String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "error: invalid skill destination".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| format!("error: {error}"))?;
    let mut messages = Vec::new();
    if let Some(bundle) = config.references {
        let refs = parent.join("references");
        install_asset_tree(&format!("skills/{bundle}/references/"), &refs)?;
        messages.push(format!("  references       ->  {}", refs.display()));
    }
    write_owned(
        destination.clone(),
        asset_text(config.skill_file).unwrap_or_default(),
    )?;
    write_owned(parent.join(".graphify_version"), COMPAT_VERSION)?;
    messages.push(format!("  skill installed  ->  {}", destination.display()));
    Ok(SkillInstall {
        path: destination,
        messages,
    })
}

fn uninstall_platform(name: &str, project: bool, project_dir: &Path, _prefix: &str) -> Outcome {
    if name == "codebuddy" && project {
        return uninstall_codebuddy_direct(project_dir, false);
    }
    if name == "kiro" && project {
        return uninstall_kiro_direct(project_dir);
    }
    if matches!(name, "antigravity" | "antigravity-windows") && project {
        return uninstall_antigravity(project_dir, true);
    }
    if name == "cursor" {
        return remove_owned_file(
            project_dir.join(".cursor/rules/graphify.mdc"),
            "No graphify Cursor rule found - nothing to do",
            "graphify Cursor rule removed",
        );
    }
    if name == "vscode" {
        return uninstall_vscode(project_dir);
    }
    if name == "gemini" {
        let mut lines = Vec::new();
        if let Some(config) = Some(Platform::new(
            "gemini",
            "skill.md",
            ".gemini/skills/graphify/SKILL.md",
            Some("claude"),
        )) {
            remove_skill(config, project, project_dir, &mut lines);
        }
        strip_section_file(&project_dir.join("GEMINI.md"), "## graphify", &mut lines);
        remove_json_hooks(
            &project_dir.join(".gemini/settings.json"),
            "BeforeTool",
            &mut lines,
        );
        return Outcome::success(lines.join("\n"));
    }
    let Some(config) = platform(name) else {
        return Outcome::failure(format!("error: unknown platform '{name}'"));
    };
    let mut lines = Vec::new();
    remove_skill(config, project, project_dir, &mut lines);
    if project {
        match name {
            "claude" | "windows" => {
                remove_registration(&project_dir.join(".claude/CLAUDE.md"), &mut lines);
                strip_section_file(&project_dir.join("CLAUDE.md"), "## graphify", &mut lines);
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
            "codex" | "opencode" | "aider" | "amp" | "claw" | "droid" | "trae" | "trae-cn"
            | "hermes" => {
                strip_section_file(&project_dir.join("AGENTS.md"), "## graphify", &mut lines);
                if name == "codex" {
                    remove_json_hooks(
                        &project_dir.join(".codex/hooks.json"),
                        "PreToolUse",
                        &mut lines,
                    );
                } else if name == "opencode" {
                    remove_opencode(project_dir, &mut lines);
                }
            }
            "kiro" => remove_file(&project_dir.join(".kiro/steering/graphify.md"), &mut lines),
            "devin" => remove_labeled_file(
                &project_dir.join(".windsurf/rules/graphify.md"),
                "  rules removed  ->  .windsurf/rules/graphify.md",
                &mut lines,
            ),
            "antigravity" | "antigravity-windows" => {
                remove_file(&project_dir.join(".agents/rules/graphify.md"), &mut lines);
                remove_file(
                    &project_dir.join(".agents/workflows/graphify.md"),
                    &mut lines,
                );
            }
            _ => {}
        }
    }
    if lines.is_empty() {
        lines.push("nothing to remove".to_owned());
    }
    Outcome::success(lines.join("\n"))
}

fn uninstall_all(project: bool, purge: bool, project_dir: &Path, prefix: &str) -> Outcome {
    let mut lines = vec![
        if project {
            "Uninstalling project-scoped graphify files...".to_owned()
        } else {
            "Uninstalling graphify from all detected platforms...".to_owned()
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
        let output = env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
        let target = project_dir.join(output);
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
        "## graphify",
        asset_text("always_on/agents-md.md").unwrap_or_default(),
    )?;
    lines.push(format!(
        "graphify section written to {}",
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
    let registration = "# graphify\n- **graphify** (`.claude/skills/graphify/SKILL.md`) - any input to knowledge graph. Trigger: `/graphify`\nWhen the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.\n";
    append_registration(&path, registration)?;
    lines.push("  CLAUDE.md        ->  created at .claude/CLAUDE.md".to_owned());
    Ok(())
}

fn register_global_claude(lines: &mut Vec<String>) -> Result<(), String> {
    let home = home_directory()
        .ok_or_else(|| "error: could not determine user home directory".to_owned())?;
    let path = home.join(".claude/CLAUDE.md");
    let registration = "# graphify\n- **graphify** (`~/.claude/skills/graphify/SKILL.md`) - any input to knowledge graph. Trigger: `/graphify`\nWhen the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.\n";
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
    let registration = "# graphify\n- **graphify** (`~/.codebuddy/skills/graphify/SKILL.md`) - any input to knowledge graph. Trigger: `/graphify`\nWhen the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.\n";
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
        "## graphify",
        asset_text("always_on/claude-md.md").unwrap_or_default(),
    )?;
    lines.push(format!(
        "graphify section written to {}",
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
    let mut document = load_json_object(&path);
    let hooks = object_child(&mut document, "hooks")?;
    let existing = hooks
        .remove("PreToolUse")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut values = existing
        .into_iter()
        .filter(|value| !value.to_string().contains("graphify"))
        .collect::<Vec<_>>();
    let executable = graphify_executable();
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
    let mut document = load_json_object(&path);
    let hooks = object_child(&mut document, "hooks")?;
    let existing = hooks
        .remove("PreToolUse")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut values = existing
        .into_iter()
        .filter(|value| !value.to_string().contains("graphify"))
        .collect::<Vec<_>>();
    let executable = graphify_executable();
    values.push(json!({"matcher":"Bash|Grep","hooks":[{"type":"command","command":format!("{executable} hook-guard search")}]}));
    values.push(json!({"matcher":"Read|Glob","hooks":[{"type":"command","command":format!("{executable} hook-guard read")}]}));
    hooks.insert("PreToolUse".to_owned(), Value::Array(values));
    write_json_object(path, &document)
}

fn install_gemini_hook(root: &Path) -> Result<(), String> {
    let path = root.join(".gemini/settings.json");
    let mut document = load_json_object(&path);
    let hooks = object_child(&mut document, "hooks")?;
    let existing = hooks
        .remove("BeforeTool")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut values = existing
        .into_iter()
        .filter(|value| !value.to_string().contains("graphify"))
        .collect::<Vec<_>>();
    values.push(json!({"matcher":"read_file|list_directory","hooks":[{"type":"command","command":format!("{} hook-guard gemini", graphify_executable())}]}));
    hooks.insert("BeforeTool".to_owned(), Value::Array(values));
    write_json_object(path, &document)
}

fn install_codex_hook(root: &Path, lines: &mut Vec<String>) -> Result<(), String> {
    let path = root.join(".codex/hooks.json");
    let mut document = load_json_object(&path);
    let hooks = object_child(&mut document, "hooks")?;
    let existing = hooks
        .remove("PreToolUse")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut values = existing
        .into_iter()
        .filter(|value| !value.to_string().contains("graphify"))
        .collect::<Vec<_>>();
    let executable = graphify_executable();
    values.push(json!({"matcher":"Bash","hooks":[{"type":"command","command":format!("{executable} hook-check")}]}));
    hooks.insert("PreToolUse".to_owned(), Value::Array(values));
    write_json_object(path, &document)?;
    lines.push(format!(
        "  .codex/hooks.json  ->  PreToolUse hook registered ({executable} hook-check)"
    ));
    Ok(())
}

fn install_opencode(root: &Path, lines: &mut Vec<String>) -> Result<(), String> {
    let plugin = root.join(".opencode/plugins/graphify.js");
    write_owned(plugin, OPENCODE_PLUGIN)?;
    lines.push("  .opencode/plugins/graphify.js  ->  tool.execute.before hook written".to_owned());
    let config = root.join(".opencode/opencode.json");
    let mut document = load_json_object(&config);
    let entry = ".opencode/plugins/graphify.js";
    let plugins = document
        .entry("plugin".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !plugins.is_array() {
        *plugins = Value::Array(Vec::new());
    }
    let array = plugins
        .as_array_mut()
        .ok_or_else(|| "error: invalid OpenCode plugin list".to_owned())?;
    if !array.iter().any(|value| value.as_str() == Some(entry)) {
        array.push(Value::String(entry.to_owned()));
    }
    write_json_object(config, &document)?;
    lines.push("  .opencode/opencode.json  ->  plugin registered".to_owned());
    Ok(())
}

fn install_kilo_plugin(root: &Path, lines: &mut Vec<String>) -> Result<(), String> {
    let plugin = root.join(".kilo/plugins/graphify.js");
    write_owned(plugin.clone(), KILO_PLUGIN)?;
    lines.push("  .kilo/plugins/graphify.js  ->  tool.execute.before hook written".to_owned());
    let config = root.join(".kilo/kilo.json");
    let mut document = load_json_object(&config);
    let plugins = document
        .entry("plugin".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !plugins.is_array() {
        *plugins = Value::Array(Vec::new());
    }
    let array = plugins
        .as_array_mut()
        .ok_or_else(|| "error: invalid Kilo plugin list".to_owned())?;
    let absolute = fs::canonicalize(&plugin).unwrap_or(plugin);
    let entry = if cfg!(windows) {
        format!("file:///{}", absolute.to_string_lossy().replace('\\', "/"))
    } else {
        format!("file://{}", absolute.display())
    };
    if !array.iter().any(|value| value.as_str() == Some(&entry)) {
        array.push(Value::String(entry));
    }
    write_json_object(config, &document)?;
    lines.push("  .kilo/kilo.json  ->  plugin registered".to_owned());
    Ok(())
}

fn finalize_antigravity(root: &Path, skill: &Path, lines: &mut Vec<String>) -> Result<(), String> {
    let body = fs::read_to_string(skill).map_err(|error| format!("error: {error}"))?;
    if !body.starts_with("---\n") {
        write_owned(
            skill.to_path_buf(),
            &format!(
                "---\nname: graphify-manager\ndescription: Rebuild the code graph or perform manual CLI queries when MCP server is offline.\n---\n\n{body}"
            ),
        )?;
    }
    let rules = root.join(".agents/rules/graphify.md");
    write_owned(
        rules.clone(),
        asset_text("always_on/antigravity-rules.md").unwrap_or_default(),
    )?;
    lines.push(format!(
        "graphify rule written to {}",
        absolute_display(&rules)
    ));
    let workflow = root.join(".agents/workflows/graphify.md");
    write_owned(workflow.clone(), ANTIGRAVITY_WORKFLOW)?;
    lines.push(format!(
        "graphify workflow written to {}",
        absolute_display(&workflow)
    ));
    Ok(())
}

fn install_kilo_command(lines: &mut Vec<String>) -> Result<(), String> {
    let home = home_directory()
        .ok_or_else(|| "error: could not determine user home directory".to_owned())?;
    let path = home.join(".config/kilo/command/graphify.md");
    write_owned(
        path.clone(),
        asset_text("command-kilo.md").unwrap_or_default(),
    )?;
    lines.push(format!("  command installed ->  {}", path.display()));
    Ok(())
}

fn remove_skill(config: Platform, project: bool, project_dir: &Path, lines: &mut Vec<String>) {
    let Ok(path) = skill_destination(config, project, project_dir) else {
        return;
    };
    let parent = path.parent().map(Path::to_path_buf);
    if path.exists() && fs::remove_file(&path).is_ok() {
        lines.push(format!(
            "  skill removed    ->  {}",
            display_path(&path, project, project_dir)
        ));
    }
    if let Some(parent) = parent {
        let _ = fs::remove_file(parent.join(".graphify_version"));
        let _ = fs::remove_dir_all(parent.join("references"));
        remove_empty_ancestors(&parent, if project { project_dir } else { Path::new("") });
    }
}

fn uninstall_vscode(project_dir: &Path) -> Outcome {
    let mut lines = Vec::new();
    if let Some(home) = home_directory() {
        let path = home.join(".copilot/skills/graphify/SKILL.md");
        if path.exists() && fs::remove_file(&path).is_ok() {
            lines.push(format!("  skill removed    ->  {}", path.display()));
        }
        if let Some(parent) = path.parent() {
            let _ = fs::remove_file(parent.join(".graphify_version"));
            let _ = fs::remove_dir_all(parent.join("references"));
        }
    }
    let instructions = project_dir.join(".github/copilot-instructions.md");
    if let Ok(content) = fs::read_to_string(&instructions)
        && content.lines().any(|line| line.trim() == "## graphify")
    {
        let clean = strip_heading_section(&content, "## graphify");
        if clean.trim().is_empty() {
            if fs::remove_file(&instructions).is_ok() {
                lines.push(
                    "  .github/copilot-instructions.md  ->  deleted (was empty after removal)"
                        .to_owned(),
                );
            }
        } else if write_owned(instructions, &clean).is_ok() {
            lines
                .push("  graphify section removed from .github/copilot-instructions.md".to_owned());
        }
    }
    Outcome::success(lines.join("\n"))
}

fn remove_opencode(root: &Path, lines: &mut Vec<String>) {
    let plugin = root.join(".opencode/plugins/graphify.js");
    if plugin.exists() && fs::remove_file(&plugin).is_ok() {
        lines.push("  .opencode/plugins/graphify.js  ->  removed".to_owned());
    }
    let path = root.join(".opencode/opencode.json");
    let mut document = load_json_object(&path);
    if let Some(plugins) = document.get_mut("plugin").and_then(Value::as_array_mut) {
        let before = plugins.len();
        plugins.retain(|value| value.as_str() != Some(".opencode/plugins/graphify.js"));
        let changed = plugins.len() != before;
        let empty = plugins.is_empty();
        if empty {
            document.remove("plugin");
        }
        if changed && write_json_object(path, &document).is_ok() {
            lines.push("  .opencode/opencode.json  ->  plugin deregistered".to_owned());
        }
    }
}

fn remove_kilo(root: &Path, lines: &mut Vec<String>) {
    let plugin = root.join(".kilo/plugins/graphify.js");
    if plugin.exists() && fs::remove_file(&plugin).is_ok() {
        lines.push("  .kilo/plugins/graphify.js  ->  removed".to_owned());
    }
    let path = root.join(".kilo/kilo.json");
    let mut document = load_json_object(&path);
    if let Some(plugins) = document.get_mut("plugin").and_then(Value::as_array_mut) {
        let before = plugins.len();
        plugins.retain(|value| {
            !value
                .as_str()
                .is_some_and(|entry| entry.contains("/.kilo/plugins/graphify.js"))
        });
        let changed = plugins.len() != before;
        let empty = plugins.is_empty();
        if empty {
            document.remove("plugin");
        }
        if changed && write_json_object(path, &document).is_ok() {
            lines.push("  .kilo/kilo.json  ->  plugin deregistered".to_owned());
        }
    }
}

fn remove_json_hooks(path: &Path, event: &str, lines: &mut Vec<String>) {
    if !path.exists() {
        return;
    }
    let mut document = load_json_object(path);
    let Some(hooks) = document.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(values) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
        return;
    };
    let before = values.len();
    values.retain(|value| !value.to_string().contains("graphify"));
    if values.len() != before && write_json_object(path.to_path_buf(), &document).is_ok() {
        lines.push(format!(
            "  {}  ->  {event} hook removed",
            lexical_path(path).display()
        ));
    }
}

fn remove_registration(path: &Path, lines: &mut Vec<String>) {
    if !path.exists() {
        return;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    let clean = strip_heading_section(&content, "# graphify");
    if clean.trim().is_empty() {
        if fs::remove_file(path).is_ok() {
            lines.push(format!(
                "  CLAUDE.md        ->  deleted {}",
                lexical_path(path).display()
            ));
        }
    } else if write_owned(path.to_path_buf(), &clean).is_ok() {
        lines.push(format!(
            "  CLAUDE.md        ->  graphify skill registration removed from {}",
            lexical_path(path).display()
        ));
    }
}

fn strip_section_file(path: &Path, marker: &str, lines: &mut Vec<String>) {
    if !path.exists() {
        return;
    }
    let Ok(content) = fs::read_to_string(path) else {
        return;
    };
    if !content.lines().any(|line| line.trim() == marker) {
        return;
    }
    let clean = strip_heading_section(&content, marker);
    if clean.trim().is_empty() {
        if fs::remove_file(path).is_ok() {
            lines.push(format!(
                "{} was empty after removal - deleted {}",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("file"),
                absolute_display(path)
            ));
        }
    } else if write_owned(path.to_path_buf(), &clean).is_ok() {
        lines.push(format!(
            "graphify section removed from {}",
            absolute_display(path)
        ));
    }
}

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

fn remove_file(path: &Path, lines: &mut Vec<String>) {
    if path.exists() && fs::remove_file(path).is_ok() {
        lines.push(format!("removed {}", path.display()));
    }
}

fn remove_labeled_file(path: &Path, label: &str, lines: &mut Vec<String>) {
    if path.is_file() && fs::remove_file(path).is_ok() {
        lines.push(label.to_owned());
    }
}

fn append_registration(path: &Path, registration: &str) -> Result<(), String> {
    let current = fs::read_to_string(path).unwrap_or_default();
    if current.contains("graphify") {
        return Ok(());
    }
    let output = if current.trim().is_empty() {
        registration.to_owned()
    } else {
        format!("{}\n{}", current.trim_end(), registration)
    };
    write_owned(path.to_path_buf(), &output)
}

fn update_section(path: &Path, marker: &str, section: &str) -> Result<(), String> {
    let current = fs::read_to_string(path).unwrap_or_default();
    write_owned(
        path.to_path_buf(),
        &replace_or_append_section(&current, marker, section),
    )
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
            "error: references for package bundle '{prefix}' are missing"
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

fn write_owned(path: PathBuf, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("error: could not create {}: {error}", parent.display()))?;
    }
    write_text_atomic(&path, content).map_err(|error| format!("error: {error}"))
}

fn load_json_object(path: &Path) -> Map<String, Value> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
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
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .ok_or_else(|| format!("error: could not create JSON object '{name}'"))
}

fn graphify_executable() -> String {
    executable_on_path("graphify")
        .or_else(|| env::current_exe().ok())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| "graphify".to_owned())
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let extensions = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';')
            .map(str::to_owned)
            .collect::<Vec<_>>()
    } else {
        vec![String::new()]
    };
    env::split_paths(&path).find_map(|directory| {
        extensions
            .iter()
            .map(|extension| directory.join(format!("{name}{extension}")))
            .find(|candidate| candidate.is_file())
    })
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

const ANTIGRAVITY_WORKFLOW: &str = "---\nname: graphify\ndescription: Turn any folder of files into a navigable knowledge graph\n---\n\n# Workflow: graphify\n\nFollow the graphify skill installed at ~/.gemini/config/skills/graphify/SKILL.md to run the full pipeline.\n\nIf no path argument is given, use `.` (current directory).\n";
const DEVIN_RULES: &str = "## graphify\n\nThis project has a graphify knowledge graph at graphify-out/.\n\nRules:\n- For codebase or architecture questions, when `graphify-out/graph.json` exists, first run `graphify query \"<question>\"` (or `graphify path \"<A>\" \"<B>\"` / `graphify explain \"<concept>\"`). These return a scoped subgraph, usually much smaller than `GRAPH_REPORT.md` or raw grep output.\n- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files\n- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context\n- After modifying code files in this session, run `graphify update .` to keep the graph current (AST-only, no API cost)\n";
const CURSOR_RULE: &str = "---\ndescription: graphify knowledge graph context\nalwaysApply: true\n---\n\nThis project has a graphify knowledge graph at graphify-out/.\n\n**MANDATORY: Before using Read, Grep, Glob, or Bash to explore the codebase, you MUST run graphify first:**\n- `graphify query \"<question>\"` — scoped subgraph for any codebase or architecture question\n- `graphify path \"<A>\" \"<B>\"` — dependency path between two symbols\n- `graphify explain \"<concept>\"` — all nodes related to a concept\n\nThis applies to YOU and to every subagent you spawn. Include this rule explicitly in every subagent prompt that involves code exploration. Do not skip graphify because files are \"already known\" or because you are executing a plan — the graph surfaces cross-file dependencies and INFERRED edges that grep and Read cannot find.\n\nOnly use Read/Grep/Glob directly when:\n1. graphify has already oriented you and you need to modify or debug specific lines\n2. `graphify-out/graph.json` does not exist yet\n\n- If `graphify-out/wiki/index.md` exists, navigate it instead of reading raw files\n- Read `graphify-out/GRAPH_REPORT.md` only for broad architecture review when query/path/explain do not surface enough context\n- After modifying code files, run `graphify update .` to keep the graph current (AST-only, no API cost)\n";
const OPENCODE_PLUGIN: &str = "// graphify OpenCode plugin\n// Injects a knowledge graph reminder before bash tool calls when the graph exists.\n//\n// IMPORTANT: keep the reminder string free of backticks and $(...) constructs.\n// The hook prepends `echo \"<reminder>\" && <cmd>` to the user's bash command;\n// backticks inside the double-quoted echo trigger bash command substitution,\n// which both corrupts tool output and silently executes the very graphify\n// command we are only suggesting. Plain words render fine in opencode's TUI.\nimport { existsSync } from \"fs\";\nimport { join } from \"path\";\n\nexport const GraphifyPlugin = async ({ directory }) => {\n  let reminded = false;\n\n  return {\n    \"tool.execute.before\": async (input, output) => {\n      if (reminded) return;\n      if (!existsSync(join(directory, \"graphify-out\", \"graph.json\"))) return;\n\n      if (input.tool === \"bash\") {\n        // ';' not '&&' — Windows PowerShell 5.1 rejects '&&' as a statement\n        // separator, breaking the first bash command of the session (#1646).\n        output.args.command =\n          'echo \"[graphify] knowledge graph at graphify-out/. For focused questions, run graphify query with your question (scoped subgraph, usually much smaller than GRAPH_REPORT.md) instead of grepping raw files. Read GRAPH_REPORT.md only for broad architecture context.\" ; ' +\n          output.args.command;\n        reminded = true;\n      }\n    },\n  };\n};\n";
const KILO_PLUGIN: &str = "// graphify Kilo plugin\n// Injects a knowledge graph reminder before bash tool calls when the graph exists.\nimport { existsSync } from \"fs\";\nimport { join } from \"path\";\n\nexport const GraphifyPlugin = async ({ directory }) => {\n  let reminded = false;\n\n  return {\n    \"tool.execute.before\": async (input, output) => {\n      if (reminded) return;\n      if (!existsSync(join(directory, \"graphify-out\", \"graph.json\"))) return;\n\n      if (input.tool === \"bash\") {\n        // Separate with ';' not '&&' — Windows PowerShell 5.1 rejects '&&' as a\n        // statement separator (\"not a valid statement separator\"), which broke\n        // the first bash command in every OpenCode session on Windows (#1646).\n        // ';' works in PowerShell 5.1, Bash, and POSIX shells alike.\n        output.args.command =\n          'echo \"[graphify] Knowledge graph available. Read graphify-out/GRAPH_REPORT.md for god nodes and architecture context before searching files.\" ; ' +\n          output.args.command;\n        reminded = true;\n      }\n    },\n  };\n};\n";

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
        let input = "# User\n\n## graphify\nold\n\n## Keep\nvalue\n";
        assert_eq!(
            replace_or_append_section(input, "## graphify", "## graphify\nnew\n"),
            "# User\n\n## graphify\nnew\n\n## Keep\nvalue\n"
        );
    }

    #[test]
    fn packaged_reference_bundles_are_nonempty() {
        for bundle in [
            "agents", "amp", "claude", "claw", "codex", "copilot", "droid", "kilo", "kiro",
            "opencode", "pi", "trae", "vscode", "windows",
        ] {
            assert!(EMBEDDED_ASSETS.iter().any(|asset| {
                asset
                    .path
                    .starts_with(&format!("skills/{bundle}/references/"))
            }));
        }
    }

    #[test]
    fn parser_and_platform_boundaries_fail_without_mutation() {
        let conflicting =
            command_install(Frontend::Trail, &["claude".to_owned(), "codex".to_owned()]);
        assert_eq!(conflicting.code, 1);
        assert!(conflicting.stderr.contains("only once"));
        let conflicting_equals = command_install(
            Frontend::Trail,
            &["--platform=claude".to_owned(), "codex".to_owned()],
        );
        assert_eq!(conflicting_equals.code, 1);
        assert_eq!(command_platform(Frontend::Trail, "codex", &[]).code, 1);
        assert_eq!(
            command_platform(Frontend::Trail, "codex", &["bad".to_owned()]).code,
            1
        );
        assert_eq!(
            install_platform("bad", true, Path::new("."), false, "trail graph").code,
            1
        );
        assert_eq!(
            uninstall_platform("bad", true, Path::new("."), "trail graph").code,
            1
        );
        assert!(platform("bad").is_none());
        assert_eq!(canonical_platform("skills"), "agents");
        assert!(is_install_platform("cursor"));
    }

    #[test]
    fn project_uninstall_all_purges_only_the_scoped_output()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("graphify-out");
        fs::create_dir_all(&output)?;
        fs::write(output.join("graph.json"), "{}")?;
        let outcome = uninstall_all(true, true, directory.path(), "trail graph");
        assert_eq!(outcome.code, 0);
        assert!(outcome.stdout.contains("project-scoped"));
        assert!(outcome.stdout.contains("removed"));
        assert!(outcome.stdout.ends_with("Done."));
        assert!(!output.exists());
        Ok(())
    }

    #[test]
    fn hook_installers_replace_invalid_shapes_and_preserve_unowned_entries()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        write(
            &root.join(".claude/settings.json"),
            r#"{"hooks":{"PreToolUse":[{"command":"keep"},{"command":"graphify old"}]}}"#,
        )?;
        install_claude_hook(root, true)?;
        let claude = load_json_object(&root.join(".claude/settings.json"));
        let hooks = claude["hooks"]["PreToolUse"]
            .as_array()
            .ok_or("missing Claude hooks")?;
        assert_eq!(hooks.len(), 3);
        assert!(
            hooks
                .iter()
                .any(|hook| hook.to_string().contains("--strict"))
        );

        write(&root.join(".codebuddy/settings.json"), r#"{"hooks":7}"#)?;
        install_codebuddy_hook(root)?;
        assert_eq!(
            load_json_object(&root.join(".codebuddy/settings.json"))["hooks"]["PreToolUse"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        write(&root.join(".gemini/settings.json"), r#"{"hooks":null}"#)?;
        install_gemini_hook(root)?;
        assert_eq!(
            load_json_object(&root.join(".gemini/settings.json"))["hooks"]["BeforeTool"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        Ok(())
    }

    #[test]
    fn plugin_install_and_cleanup_round_trip_handles_scalar_configs()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        write(&root.join(".opencode/opencode.json"), r#"{"plugin":7}"#)?;
        write(&root.join(".kilo/kilo.json"), r#"{"plugin":false}"#)?;
        let mut lines = Vec::new();
        install_opencode(root, &mut lines)?;
        install_kilo_plugin(root, &mut lines)?;
        assert!(root.join(".opencode/plugins/graphify.js").is_file());
        assert!(root.join(".kilo/plugins/graphify.js").is_file());
        remove_opencode(root, &mut lines);
        remove_kilo(root, &mut lines);
        assert!(!root.join(".opencode/plugins/graphify.js").exists());
        assert!(!root.join(".kilo/plugins/graphify.js").exists());
        assert!(lines.iter().any(|line| line.contains("deregistered")));
        Ok(())
    }

    #[test]
    fn owned_markdown_cleanup_covers_empty_preserved_and_unmarked_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let mut lines = Vec::new();

        let registration = root.join("CLAUDE.md");
        append_registration(&registration, "# graphify\nowned\n")?;
        append_registration(&registration, "# graphify\nduplicate\n")?;
        remove_registration(&registration, &mut lines);
        assert!(!registration.exists());

        let agents = root.join("AGENTS.md");
        write(&agents, "# User\n\n## graphify\nowned\n\n## Keep\nvalue\n")?;
        strip_section_file(&agents, "## graphify", &mut lines);
        assert_eq!(fs::read_to_string(&agents)?, "# User\n\n## Keep\nvalue\n\n");
        let untouched = root.join("untouched.md");
        write(&untouched, "# User\n")?;
        strip_section_file(&untouched, "## graphify", &mut lines);
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
            r#"{"hooks":{"PreToolUse":[{"command":"keep"},{"command":"graphify hook"}]}}"#,
        )?;
        let mut lines = Vec::new();
        remove_json_hooks(&hooks, "PreToolUse", &mut lines);
        let document = load_json_object(&hooks);
        assert_eq!(
            document["hooks"]["PreToolUse"].as_array().map(Vec::len),
            Some(1)
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
        assert!(install_asset_tree("skills/codex/references/", &destination).is_ok());
        assert!(destination.is_dir());
        assert!(install_asset_tree("missing-prefix/", &destination).is_err());

        let mut object = Map::new();
        object.insert("hooks".to_owned(), Value::Bool(false));
        object_child(&mut object, "hooks")?.insert("ready".to_owned(), Value::Bool(true));
        assert_eq!(object["hooks"]["ready"], true);

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
        assert_eq!(capitalize("graphify"), "Graphify");
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
        assert!(fs::read_to_string(&skill)?.starts_with("---\nname: graphify-manager"));
        assert!(directory.path().join(".agents/rules/graphify.md").is_file());
        assert!(
            directory
                .path()
                .join(".agents/workflows/graphify.md")
                .is_file()
        );
        finalize_antigravity(directory.path(), &skill, &mut lines)?;
        assert_eq!(lines.len(), 4);
        Ok(())
    }
}
