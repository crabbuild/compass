use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repository_root() -> PathBuf {
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

    for name in ["post-commit", "post-checkout"] {
        let script = std::fs::read_to_string(root.join(".git/hooks").join(name))?;
        assert!(script.contains("hook-spawn"));
        assert!(script.contains("GRAPHIFY_SKIP_HOOK"));
        assert!(script.contains("rebase-merge"));
        assert!(script.contains("MERGE_HEAD"));
        assert!(script.contains("git rev-parse --git-common-dir"));
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
fn trail_graph_hook_uses_trail_invocation_and_custom_hook_path() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    initialize_repo(root)?;
    let configured = Command::new("git")
        .args(["config", "--local", "core.hooksPath", ".husky"])
        .current_dir(root)
        .output()?;
    assert!(configured.status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_trail"))
        .args(["graph", "hook", "install"])
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
    assert!(hook.contains(" graph hook-spawn ."));
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
