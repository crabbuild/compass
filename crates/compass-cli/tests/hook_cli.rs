use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use compass_history::{HistoryQueue, JobState, Repository};

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("GRAPHIFY_REPO_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

fn python_executable(repo: &Path) -> PathBuf {
    if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    }
}

fn initialize_repo(path: &Path) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(path)?;
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(path)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

fn run_graphify(
    executable: &Path,
    repo: &Path,
    cwd: &Path,
    args: &[&str],
) -> Result<Output, Box<dyn Error>> {
    let mut command = Command::new(executable);
    if executable == python_executable(repo) {
        command.args(["-m", "graphify"]);
        command.env("PYTHONPATH", repo);
    }
    Ok(command
        .args(args)
        .current_dir(cwd)
        .env("HOME", cwd)
        .env("USERPROFILE", cwd)
        .env_remove("GRAPHIFY_OUT")
        .output()?)
}

fn normalized(bytes: &[u8], root: &Path) -> String {
    String::from_utf8_lossy(bytes).replace(&root.to_string_lossy().to_string(), "<ROOT>")
}

#[test]
fn graphify_hook_lifecycle_matches_python_oracle() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let python_root = directory.path().join("python");
    let native_root = directory.path().join("native");
    initialize_repo(&python_root)?;
    initialize_repo(&native_root)?;
    let repo = repository_root();
    let python_exe = python_executable(&repo);
    if !python_exe.is_file() {
        return Ok(());
    }
    let native_exe = Path::new(env!("CARGO_BIN_EXE_graphify"));

    for args in [
        ["hook", "status"],
        ["hook", "install"],
        ["hook", "install"],
        ["hook", "status"],
        ["hook", "uninstall"],
        ["hook", "status"],
    ] {
        let python = run_graphify(&python_exe, &repo, &python_root, &args)?;
        let native = run_graphify(native_exe, &repo, &native_root, &args)?;
        assert_eq!(native.status.code(), python.status.code(), "{args:?}");
        assert_eq!(
            normalized(&native.stdout, &native_root),
            normalized(&python.stdout, &python_root),
            "{args:?}"
        );
        assert_eq!(native.stderr, python.stderr, "{args:?}");
    }
    Ok(())
}

#[test]
fn native_hooks_are_self_contained_safe_and_preserve_user_content() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    initialize_repo(root)?;
    let existing = root.join(".git/hooks/post-commit");
    std::fs::write(&existing, "#!/bin/sh\necho user-hook\n")?;

    let repo = repository_root();
    let native_exe = Path::new(env!("CARGO_BIN_EXE_graphify"));
    let installed = run_graphify(native_exe, &repo, root, &["hook", "install"])?;
    assert!(installed.status.success());

    for name in ["post-commit", "post-checkout", "post-merge"] {
        let script = std::fs::read_to_string(root.join(".git/hooks").join(name))?;
        assert!(script.contains("hook-spawn"));
        if name != "post-merge" {
            assert!(script.contains("GRAPHIFY_SKIP_HOOK"));
            assert!(script.contains("rebase-merge"));
            assert!(script.contains("MERGE_HEAD"));
            assert!(script.contains("git rev-parse --git-common-dir"));
        }
        assert!(!script.contains("python"));
        assert!(!script.contains("nohup"));
    }
    let driver = Command::new("git")
        .args(["config", "--get", "merge.graphify.driver"])
        .current_dir(root)
        .output()?;
    let driver = String::from_utf8(driver.stdout)?;
    assert!(driver.contains(&native_exe.to_string_lossy().to_string()));
    assert!(driver.contains("merge-driver %O %A %B"));

    let removed = run_graphify(native_exe, &repo, root, &["hook", "uninstall"])?;
    assert!(removed.status.success());
    assert_eq!(
        std::fs::read_to_string(existing)?,
        "#!/bin/sh\necho user-hook\n"
    );
    assert!(!root.join(".git/hooks/post-checkout").exists());
    assert!(!root.join(".git/hooks/post-merge").exists());
    Ok(())
}

#[test]
fn oversized_existing_hook_is_rejected_without_overwrite() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    initialize_repo(root)?;
    let hook = root.join(".git/hooks/post-commit");
    let file = std::fs::File::create(&hook)?;
    file.set_len(4 * 1024 * 1024 + 1)?;

    let repo = repository_root();
    let output = run_graphify(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        root,
        &["hook", "install"],
    )?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("safety limit"));
    assert_eq!(hook.metadata()?.len(), 4 * 1024 * 1024 + 1);
    Ok(())
}

#[test]
fn compass_graph_hook_uses_compass_invocation_and_custom_hook_path() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    initialize_repo(root)?;
    let configured = Command::new("git")
        .args(["config", "--local", "core.hooksPath", ".husky"])
        .current_dir(root)
        .output()?;
    assert!(configured.status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["hook", "install"])
        .current_dir(root)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let hook = std::fs::read_to_string(root.join(".husky/post-commit"))?;
    assert!(hook.contains(" hook-spawn ."));
    assert!(!hook.contains(" graph hook-spawn ."));
    assert!(!root.join(".git/hooks/post-commit").exists());
    Ok(())
}

#[test]
fn native_hook_refresh_honors_recorded_scan_root() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source_root = root.join("workspace/service");
    std::fs::create_dir_all(&source_root)?;
    std::fs::write(
        source_root.join("app.py"),
        "def hook_target():\n    return 1\n",
    )?;
    let output_root = root.join("graphify-out");
    std::fs::create_dir_all(&output_root)?;
    std::fs::write(
        output_root.join(".graphify_root"),
        source_root.to_string_lossy().as_bytes(),
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_graphify"))
        .args(["hook-refresh", "."])
        .current_dir(root)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env_remove("GRAPHIFY_OUT")
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let graph = std::fs::read_to_string(output_root.join("graph.json"))?;
    assert!(graph.contains("hook_target"));
    assert!(!source_root.join("graphify-out/graph.json").exists());
    Ok(())
}

#[cfg(not(windows))]
#[test]
fn windows_style_hook_path_fails_without_creating_junk() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    initialize_repo(root)?;
    let configured = Command::new("git")
        .args(["config", "--local", "core.hooksPath", r"C:\Users\u\hooks"])
        .current_dir(root)
        .output()?;
    assert!(configured.status.success());

    let repo = repository_root();
    let output = run_graphify(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        root,
        &["hook", "install"],
    )?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Windows path"));
    assert!(!root.join(r"C:\Users\u\hooks").exists());
    Ok(())
}

#[test]
fn eager_history_is_opt_in_non_blocking_and_captures_the_exact_commit() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    initialize_repo(root)?;
    for (key, value) in [
        ("user.name", "Compass Test"),
        ("user.email", "compass@example.invalid"),
    ] {
        let configured = Command::new("git")
            .args(["config", key, value])
            .current_dir(root)
            .output()?;
        assert!(configured.status.success());
    }
    std::fs::write(root.join("service.rs"), "pub struct BaseService;\n")?;
    Command::new("git")
        .args(["add", "service.rs"])
        .current_dir(root)
        .output()?;
    Command::new("git")
        .args(["commit", "--quiet", "-m", "base"])
        .current_dir(root)
        .output()?;

    let compass = Path::new(env!("CARGO_BIN_EXE_compass"));
    let installed = Command::new(compass)
        .args(["hook", "install"])
        .current_dir(root)
        .output()?;
    assert!(installed.status.success());
    std::fs::write(root.join("disabled.rs"), "pub struct DisabledCommit;\n")?;
    Command::new("git")
        .args(["add", "disabled.rs"])
        .current_dir(root)
        .output()?;
    let disabled_commit = Command::new("git")
        .args(["commit", "--quiet", "-m", "disabled"])
        .current_dir(root)
        .env("GRAPHIFY_SKIP_HOOK", "1")
        .output()?;
    assert!(disabled_commit.status.success());
    assert!(!root.join(".git/compass").exists());

    let enabled = Command::new(compass)
        .args(["history", "enable"])
        .current_dir(root)
        .output()?;
    assert!(
        enabled.status.success(),
        "{}",
        String::from_utf8_lossy(&enabled.stderr)
    );
    std::fs::write(root.join("enabled.rs"), "pub struct EagerHistoryService;\n")?;
    Command::new("git")
        .args(["add", "enabled.rs"])
        .current_dir(root)
        .output()?;
    let started = Instant::now();
    let committed = Command::new("git")
        .args(["commit", "--quiet", "-m", "enabled"])
        .current_dir(root)
        .env("GRAPHIFY_SKIP_HOOK", "1")
        .output()?;
    assert!(committed.status.success());
    assert!(started.elapsed() < Duration::from_secs(2));
    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()?;
    let sha = String::from_utf8(sha.stdout)?.trim().to_owned();
    let repository = Repository::discover(root)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(queue) = HistoryQueue::open_existing(&repository)?
            && queue
                .list()?
                .iter()
                .any(|job| job.commit.as_str() == sha && job.state == JobState::Published)
        {
            break;
        }
        if Instant::now() >= deadline {
            let jobs = HistoryQueue::open_existing(&repository)?
                .map(|queue| queue.list())
                .transpose()?;
            let log = std::fs::read_to_string(root.join(".cache/graphify-rebuild.log"))
                .unwrap_or_else(|error| format!("<no worker log: {error}>"));
            return Err(format!(
                "eager history worker did not publish before deadline; jobs={jobs:?}; log={log}"
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let initial_branch = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(root)
        .output()?;
    let initial_branch = String::from_utf8(initial_branch.stdout)?.trim().to_owned();
    let linked = directory.path().join("linked");
    let linked_added = Command::new("git")
        .args([
            "worktree",
            "add",
            "--quiet",
            "-b",
            "linked-history",
            linked.to_str().ok_or("linked path")?,
            "HEAD",
        ])
        .current_dir(root)
        .output()?;
    assert!(linked_added.status.success());
    std::fs::write(linked.join("linked.rs"), "pub struct LinkedHistory;\n")?;
    Command::new("git")
        .args(["add", "linked.rs"])
        .current_dir(&linked)
        .output()?;
    let linked_commit = Command::new("git")
        .args(["commit", "--quiet", "-m", "linked"])
        .current_dir(&linked)
        .env("GRAPHIFY_SKIP_HOOK", "1")
        .output()?;
    assert!(linked_commit.status.success());
    let linked_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&linked)
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();
    assert!(
        HistoryQueue::open_existing(&Repository::discover(&linked)?)?
            .ok_or("linked queue")?
            .list()?
            .iter()
            .any(|job| job.commit.as_str() == linked_sha)
    );

    let merged = Command::new("git")
        .args([
            "merge",
            "--quiet",
            "--no-ff",
            "linked-history",
            "-m",
            "merge",
        ])
        .current_dir(root)
        .env("GRAPHIFY_SKIP_HOOK", "1")
        .output()?;
    assert!(merged.status.success());
    let merge_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();
    assert!(
        HistoryQueue::open_existing(&repository)?
            .ok_or("merge queue")?
            .list()?
            .iter()
            .any(|job| job.commit.as_str() == merge_sha)
    );

    std::fs::write(
        linked.join("picked.rs"),
        "pub struct CherryPickedHistory;\n",
    )?;
    Command::new("git")
        .args(["add", "picked.rs"])
        .current_dir(&linked)
        .output()?;
    let source_commit = Command::new("git")
        .args(["commit", "--quiet", "-m", "pick source"])
        .current_dir(&linked)
        .env("GRAPHIFY_SKIP_HOOK", "1")
        .output()?;
    assert!(source_commit.status.success());
    let source_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&linked)
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();
    let picked = Command::new("git")
        .args(["cherry-pick", "--quiet", &source_sha])
        .current_dir(root)
        .env("GRAPHIFY_SKIP_HOOK", "1")
        .output()?;
    assert!(picked.status.success());
    let picked_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()?
            .stdout,
    )?
    .trim()
    .to_owned();
    assert_ne!(picked_sha, source_sha);
    assert!(
        HistoryQueue::open_existing(&repository)?
            .ok_or("cherry-pick queue")?
            .list()?
            .iter()
            .any(|job| job.commit.as_str() == picked_sha)
    );
    assert_eq!(
        String::from_utf8(
            Command::new("git")
                .args(["branch", "--show-current"])
                .current_dir(root)
                .output()?
                .stdout
        )?
        .trim(),
        initial_branch
    );

    let disabled = Command::new(compass)
        .args(["history", "disable"])
        .current_dir(root)
        .output()?;
    assert!(disabled.status.success());
    let before = HistoryQueue::open_existing(&repository)?
        .ok_or("queue")?
        .list()?
        .len();
    std::fs::write(root.join("later.rs"), "pub struct LaterDisabled;\n")?;
    Command::new("git")
        .args(["add", "later.rs"])
        .current_dir(root)
        .output()?;
    let later = Command::new("git")
        .args(["commit", "--quiet", "-m", "later"])
        .current_dir(root)
        .env("GRAPHIFY_SKIP_HOOK", "1")
        .output()?;
    assert!(later.status.success());
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        HistoryQueue::open_existing(&repository)?
            .ok_or("queue")?
            .list()?
            .len(),
        before
    );
    let query = Command::new(compass)
        .args(["query", "EagerHistoryService", "--at", &sha])
        .current_dir(root)
        .output()?;
    assert!(query.status.success());
    assert!(String::from_utf8_lossy(&query.stdout).contains("EagerHistoryService"));
    Ok(())
}
