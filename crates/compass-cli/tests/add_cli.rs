mod support;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn run(
    executable: &Path,
    repo: &Path,
    cwd: &Path,
    args: &[&str],
) -> Result<Output, Box<dyn Error>> {
    let mut command = support::command(executable);
    if executable == python_executable(repo) {
        command.args(["-m", "graphify"]);
        command.env("PYTHONPATH", repo);
    }
    Ok(command.args(args).current_dir(cwd).output()?)
}

#[test]
fn add_usage_and_blocked_urls_match_python_oracle() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let repo = repository_root();
    let python = python_executable(&repo);
    let native = support::compat_executable();
    for arguments in [
        vec!["add"],
        vec!["add", "--help"],
        vec!["add", "file:///etc/passwd"],
        vec!["add", "http://127.0.0.1/test"],
    ] {
        let expected = run(&python, &repo, directory.path(), &arguments)?;
        let actual = run(native, &repo, directory.path(), &arguments)?;
        assert_eq!(
            actual.status.code(),
            expected.status.code(),
            "{arguments:?}"
        );
        assert_eq!(actual.stdout, expected.stdout, "{arguments:?}");
        assert_eq!(actual.stderr, expected.stderr, "{arguments:?}");
    }
    Ok(())
}

#[test]
fn compass_add_help_is_namespaced() -> Result<(), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["add", "--help"])
        .output()?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("compass add"));
    Ok(())
}
