use std::fs::{self, OpenOptions};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use compass_files::write_text_atomic;

use crate::{Frontend, Outcome};

const COMMIT_START: &str = "# graphify-hook-start";
const COMMIT_END: &str = "# graphify-hook-end";
const CHECKOUT_START: &str = "# graphify-checkout-hook-start";
const CHECKOUT_END: &str = "# graphify-checkout-hook-end";
const MAX_HOOK_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ATTRIBUTES_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn command_hook(frontend: Frontend, args: &[String]) -> Outcome {
    match args.first().map(String::as_str).unwrap_or_default() {
        "install" => hook_action(frontend, HookAction::Install),
        "uninstall" => hook_action(frontend, HookAction::Uninstall),
        "status" => hook_action(frontend, HookAction::Status),
        _ => Outcome::failure(hook_help(frontend)),
    }
}

pub(super) fn command_hook_spawn(_frontend: Frontend, args: &[String]) -> Outcome {
    let root = args
        .first()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            return Outcome::failure(format!(
                "error: graph hook could not resolve the Compass executable: {error}"
            ));
        }
    };
    let log = std::env::var_os("GRAPHIFY_REBUILD_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|| cache_home().join("graphify-rebuild.log"));
    if let Some(parent) = log.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let output = OpenOptions::new().create(true).append(true).open(log).ok();
    let mut command = Command::new(executable);
    command.args(["hook-refresh", root.to_string_lossy().as_ref()]);
    command.current_dir(&root).stdin(Stdio::null());
    if let Some(stdout) = output {
        let stderr = stdout.try_clone().ok();
        command.stdout(Stdio::from(stdout));
        command.stderr(stderr.map_or_else(Stdio::null, Stdio::from));
    } else {
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
    }
    configure_detachment(&mut command);
    match command.spawn() {
        Ok(_) => Outcome::success(String::new()),
        Err(error) => Outcome::failure(format!(
            "error: graph hook could not launch the background refresh: {error}"
        )),
    }
}

#[derive(Clone, Copy)]
enum HookAction {
    Install,
    Uninstall,
    Status,
}

fn hook_action(frontend: Frontend, action: HookAction) -> Outcome {
    let current = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Some(root) = git_root(&current) else {
        return match action {
            HookAction::Status => Outcome::success("Not in a git repository.".to_owned()),
            HookAction::Install | HookAction::Uninstall => Outcome::failure(format!(
                "error: No git repository found at or above {}",
                canonical_or_absolute(&current).display()
            )),
        };
    };
    let (hooks, warning) = match hooks_directory(&root) {
        Ok(resolved) => resolved,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let hooks = if hooks.file_name().and_then(|name| name.to_str()) == Some("_") {
        hooks.parent().unwrap_or(&hooks).to_path_buf()
    } else {
        hooks
    };
    let result = match action {
        HookAction::Install => install(frontend, &root, &hooks),
        HookAction::Uninstall => uninstall(&root, &hooks),
        HookAction::Status => status(&root, &hooks),
    };
    match result {
        Ok(stdout) => Outcome {
            code: 0,
            stdout,
            stderr: warning.unwrap_or_default(),
            stdout_trailing_newline: true,
            stderr_trailing_newline: true,
        },
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

fn install(frontend: Frontend, root: &Path, hooks: &Path) -> Result<String, String> {
    fs::create_dir_all(hooks).map_err(|error| error.to_string())?;
    let invocation = pinned_invocation(frontend)?;
    let commit = install_hook(
        hooks,
        "post-commit",
        &commit_script(&invocation),
        COMMIT_START,
    )?;
    let checkout = install_hook(
        hooks,
        "post-checkout",
        &checkout_script(&invocation),
        CHECKOUT_START,
    )?;
    let merge = register_merge_driver(frontend, root)?;
    Ok(format!(
        "post-commit: {commit}\npost-checkout: {checkout}\nmerge driver: {merge}"
    ))
}

fn uninstall(root: &Path, hooks: &Path) -> Result<String, String> {
    let commit = uninstall_hook(hooks, "post-commit", COMMIT_START, COMMIT_END)?;
    let checkout = uninstall_hook(hooks, "post-checkout", CHECKOUT_START, CHECKOUT_END)?;
    let merge = unregister_merge_driver(root)?;
    Ok(format!(
        "post-commit: {commit}\npost-checkout: {checkout}\nmerge driver: {merge}"
    ))
}

fn status(root: &Path, hooks: &Path) -> Result<String, String> {
    let commit = hook_status(&hooks.join("post-commit"), COMMIT_START)?;
    let checkout = hook_status(&hooks.join("post-checkout"), CHECKOUT_START)?;
    let merge = merge_driver_status(root);
    Ok(format!(
        "post-commit: {commit}\npost-checkout: {checkout}\nmerge driver: {merge}"
    ))
}

fn git_root(path: &Path) -> Option<PathBuf> {
    let start = canonical_or_absolute(path);
    start
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

fn hooks_directory(root: &Path) -> Result<(PathBuf, Option<String>), String> {
    match Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "rev-parse",
            "--git-path",
            "hooks",
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            let raw = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !raw.is_empty() && !raw.contains(['\n', '\r', '\0']) {
                #[cfg(not(windows))]
                if raw.contains('\\')
                    || raw.as_bytes().get(1) == Some(&b':')
                        && raw
                            .as_bytes()
                            .first()
                            .is_some_and(|byte| byte.is_ascii_alphabetic())
                {
                    return Err(format!(
                        "git hooks path from git rev-parse --git-path hooks looks like a Windows path: {raw:?}. On WSL/POSIX this can't resolve to a real directory. Unset it with `git config --local --unset core.hooksPath`, or set a POSIX path."
                    ));
                }
                let path = PathBuf::from(raw);
                return Ok((
                    canonical_or_absolute(&if path.is_absolute() {
                        path
                    } else {
                        root.join(path)
                    }),
                    None,
                ));
            }
            Ok((root.join(".git/hooks"), None))
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let detail = if stderr.is_empty() {
                output.status.code().map_or_else(
                    || "git terminated without an exit code".to_owned(),
                    |code| format!("git exited with code {code}"),
                )
            } else {
                stderr
            };
            Ok((
                root.join(".git/hooks"),
                Some(format!(
                    "[graphify hooks] git could not resolve the hooks path for {}: {detail}",
                    root.display()
                )),
            ))
        }
        Err(_) => Ok((root.join(".git/hooks"), None)),
    }
}

fn install_hook(hooks: &Path, name: &str, script: &str, marker: &str) -> Result<String, String> {
    let path = hooks.join(name);
    if path.exists() {
        let content = read_text_bounded(&path, MAX_HOOK_BYTES)?;
        if content.contains(marker) {
            return Ok(format!("already installed at {}", path.display()));
        }
        let merged = format!("{}\n\n{script}", content.trim_end());
        write_text_atomic(&path, &merged).map_err(|error| error.to_string())?;
        return Ok(format!(
            "appended to existing {name} hook at {}",
            path.display()
        ));
    }
    write_text_atomic(&path, &format!("#!/bin/sh\n{script}")).map_err(|error| error.to_string())?;
    make_executable(&path)?;
    Ok(format!("installed at {}", path.display()))
}

fn uninstall_hook(
    hooks: &Path,
    name: &str,
    marker: &str,
    marker_end: &str,
) -> Result<String, String> {
    let path = hooks.join(name);
    if !path.exists() {
        return Ok(format!("no {name} hook found - nothing to remove."));
    }
    let content = read_text_bounded(&path, MAX_HOOK_BYTES)?;
    let Some(start) = content.find(marker) else {
        return Ok(format!(
            "graphify hook not found in {name} - nothing to remove."
        ));
    };
    let Some(relative_end) = content[start..].find(marker_end) else {
        return Ok(format!(
            "graphify hook not found in {name} - nothing to remove."
        ));
    };
    let mut end = start + relative_end + marker_end.len();
    if content.as_bytes().get(end) == Some(&b'\n') {
        end += 1;
    }
    let remaining = format!("{}{}", &content[..start], &content[end..])
        .trim()
        .to_owned();
    if remaining.is_empty() || matches!(remaining.as_str(), "#!/bin/bash" | "#!/bin/sh") {
        fs::remove_file(&path).map_err(|error| error.to_string())?;
        return Ok(format!("removed {name} hook at {}", path.display()));
    }
    write_text_atomic(&path, &format!("{remaining}\n")).map_err(|error| error.to_string())?;
    Ok(format!(
        "graphify removed from {name} at {} (other hook content preserved)",
        path.display()
    ))
}

fn hook_status(path: &Path, marker: &str) -> Result<&'static str, String> {
    if !path.exists() {
        return Ok("not installed");
    }
    let content = read_text_bounded(path, MAX_HOOK_BYTES)?;
    Ok(if content.contains(marker) {
        "installed"
    } else {
        "not installed (hook exists but graphify not found)"
    })
}

fn register_merge_driver(frontend: Frontend, root: &Path) -> Result<String, String> {
    let invocation = pinned_invocation(frontend)?;
    let driver = format!("{invocation} merge-driver %O %A %B");
    for (key, value) in [
        ("merge.graphify.name", "graphify graph.json union merge"),
        ("merge.graphify.driver", driver.as_str()),
    ] {
        let output = Command::new("git")
            .args(["-C", root.to_string_lossy().as_ref(), "config", key, value])
            .output()
            .map_err(|error| format!("not registered (git config failed: {error})"))?;
        if !output.status.success() {
            return Ok("not registered (git config failed)".to_owned());
        }
    }
    let line = merge_attribute_line();
    let path = root.join(".gitattributes");
    let mut content = if path.exists() {
        read_text_bounded(&path, MAX_ATTRIBUTES_BYTES)?
    } else {
        String::new()
    };
    if has_merge_attribute(&content) {
        return Ok(format!("already registered ({line})"));
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&line);
    content.push('\n');
    write_text_atomic(path, &content).map_err(|error| error.to_string())?;
    Ok(format!("registered ({line})"))
}

fn unregister_merge_driver(root: &Path) -> Result<String, String> {
    for key in ["merge.graphify.name", "merge.graphify.driver"] {
        let _ = Command::new("git")
            .args([
                "-C",
                root.to_string_lossy().as_ref(),
                "config",
                "--unset",
                key,
            ])
            .output();
    }
    let path = root.join(".gitattributes");
    if !path.exists() {
        return Ok("not registered - nothing to remove.".to_owned());
    }
    let content = read_text_bounded(&path, MAX_ATTRIBUTES_BYTES)?;
    let lines = content.lines().collect::<Vec<_>>();
    let kept = lines
        .iter()
        .copied()
        .filter(|line| !has_merge_attribute(line))
        .collect::<Vec<_>>();
    if kept.len() == lines.len() {
        return Ok("gitattributes entry not found - nothing to remove.".to_owned());
    }
    if kept.is_empty() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
        return Ok("removed (.gitattributes deleted - no other entries)".to_owned());
    }
    write_text_atomic(path, &format!("{}\n", kept.join("\n")))
        .map_err(|error| error.to_string())?;
    Ok("removed from .gitattributes (other entries preserved)".to_owned())
}

fn merge_driver_status(root: &Path) -> &'static str {
    let config = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "config",
            "--get",
            "merge.graphify.driver",
        ])
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty());
    let attributes = read_text_bounded(&root.join(".gitattributes"), MAX_ATTRIBUTES_BYTES)
        .is_ok_and(|content| has_merge_attribute(&content));
    match (config, attributes) {
        (true, true) => "registered",
        (true, false) => "partially registered (git config set, .gitattributes line missing)",
        (false, true) => "partially registered (.gitattributes line set, git config missing)",
        (false, false) => "not registered",
    }
}

fn has_merge_attribute(content: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        fields
            .first()
            .is_some_and(|field| field.ends_with("graph.json"))
            && fields
                .iter()
                .skip(1)
                .any(|field| *field == "merge=graphify")
    })
}

fn merge_attribute_line() -> String {
    let output = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let output = if output.is_empty() || Path::new(&output).is_absolute() || output.contains('\\') {
        "graphify-out"
    } else {
        output.trim_end_matches('/')
    };
    format!("{output}/graph.json merge=graphify")
}

fn pinned_invocation(frontend: Frontend) -> Result<String, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let quoted = shell_quote(&executable.to_string_lossy());
    Ok(if frontend == Frontend::Compass {
        format!("{quoted} graph")
    } else {
        quoted
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn commit_script(invocation: &str) -> String {
    format!(
        "{COMMIT_START}\n# Native Compass graph refresh installed by: graphify hook install\nGIT_DIR=${{GIT_DIR:-$(git rev-parse --git-dir 2>/dev/null)}}\n[ -d \"$GIT_DIR/rebase-merge\" ] && exit 0\n[ -d \"$GIT_DIR/rebase-apply\" ] && exit 0\n[ -f \"$GIT_DIR/MERGE_HEAD\" ] && exit 0\n[ -f \"$GIT_DIR/CHERRY_PICK_HEAD\" ] && exit 0\n[ \"${{GRAPHIFY_SKIP_HOOK:-0}}\" = \"1\" ] && exit 0\n_GFY_GITDIR=$(cd \"$(git rev-parse --git-dir 2>/dev/null)\" 2>/dev/null && pwd)\n_GFY_COMMONDIR=$(cd \"$(git rev-parse --git-common-dir 2>/dev/null)\" 2>/dev/null && pwd)\n[ -n \"$_GFY_COMMONDIR\" ] && [ \"$_GFY_GITDIR\" != \"$_GFY_COMMONDIR\" ] && exit 0\n_CHANGED=$(git diff --name-only HEAD~1 HEAD 2>/dev/null || git diff --name-only HEAD 2>/dev/null)\n[ -z \"$_CHANGED\" ] && exit 0\n_NON_GRAPH=$(printf '%s\\n' \"$_CHANGED\" | grep -v '^graphify-out/' || true)\n[ -z \"$_NON_GRAPH\" ] && exit 0\nexport GRAPHIFY_CHANGED=\"$_CHANGED\"\n_GRAPHIFY_LOG=${{GRAPHIFY_REBUILD_LOG:-${{HOME:-.}}/.cache/graphify-rebuild.log}}\nexport GRAPHIFY_REBUILD_LOG=\"$_GRAPHIFY_LOG\"\necho \"[graphify hook] launching background rebuild (log: $_GRAPHIFY_LOG)\"\n{invocation} hook-spawn .\n{COMMIT_END}\n"
    )
}

fn checkout_script(invocation: &str) -> String {
    format!(
        "{CHECKOUT_START}\n# Native Compass graph refresh installed by: graphify hook install\n[ \"$3\" != \"1\" ] && exit 0\n[ ! -d \"${{GRAPHIFY_OUT:-graphify-out}}\" ] && exit 0\nGIT_DIR=${{GIT_DIR:-$(git rev-parse --git-dir 2>/dev/null)}}\n[ -d \"$GIT_DIR/rebase-merge\" ] && exit 0\n[ -d \"$GIT_DIR/rebase-apply\" ] && exit 0\n[ -f \"$GIT_DIR/MERGE_HEAD\" ] && exit 0\n[ -f \"$GIT_DIR/CHERRY_PICK_HEAD\" ] && exit 0\n[ \"${{GRAPHIFY_SKIP_HOOK:-0}}\" = \"1\" ] && exit 0\n_GFY_GITDIR=$(cd \"$(git rev-parse --git-dir 2>/dev/null)\" 2>/dev/null && pwd)\n_GFY_COMMONDIR=$(cd \"$(git rev-parse --git-common-dir 2>/dev/null)\" 2>/dev/null && pwd)\n[ -n \"$_GFY_COMMONDIR\" ] && [ \"$_GFY_GITDIR\" != \"$_GFY_COMMONDIR\" ] && exit 0\n_GRAPHIFY_LOG=${{GRAPHIFY_REBUILD_LOG:-${{HOME:-.}}/.cache/graphify-rebuild.log}}\nexport GRAPHIFY_REBUILD_LOG=\"$_GRAPHIFY_LOG\"\necho \"[graphify] Branch switched - launching background rebuild (log: $_GRAPHIFY_LOG)\"\n{invocation} hook-spawn .\n{CHECKOUT_END}\n"
    )
}

#[cfg(windows)]
fn configure_detachment(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(unix)]
fn configure_detachment(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;

    command.process_group(0);
}

#[cfg(all(not(unix), not(windows)))]
fn configure_detachment(_command: &mut Command) {}

fn cache_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join(".cache")
}

fn canonical_or_absolute(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().map_or_else(|_| path.to_path_buf(), |root| root.join(path))
        }
    })
}

pub(super) fn read_text_bounded(path: &Path, limit: u64) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.len() > limit {
        return Err(format!(
            "{} exceeds the {limit}-byte safety limit",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or_default());
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limit {
        return Err(format!(
            "{} exceeds the {limit}-byte safety limit",
            path.display()
        ));
    }
    String::from_utf8(bytes).map_err(|error| format!("{} is not UTF-8: {error}", path.display()))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = path
        .metadata()
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(super) fn hook_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Compass => "Usage: compass hook [install|uninstall|status]",
        Frontend::Graphify => "Usage: graphify hook [install|uninstall|status]",
    }
    .to_owned()
}
