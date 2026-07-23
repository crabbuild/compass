mod support;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::json;

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
    Ok(command
        .args(args)
        .current_dir(cwd)
        .env_remove("NEO4J_PASSWORD")
        .env_remove("FALKORDB_PASSWORD")
        .output()?)
}

fn seed(root: &Path) -> Result<(), Box<dyn Error>> {
    let output = root.join("graphify-out");
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("graph.json"),
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id": "a'1", "label": "Alpha", "file_type": "python", "source_file": "src/a.py"},
                {"id": "b", "label": "Beta", "file_type": "rust", "source_file": "src/b.rs"}
            ],
            "links": [
                {"source": "a'1", "target": "b", "relation": "calls", "confidence": "EXTRACTED"}
            ]
        }))?,
    )?;
    Ok(())
}

#[test]
fn offline_neo4j_and_falkordb_exports_match_python_exactly() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    seed(directory.path())?;
    let repo = repository_root();
    let python = python_executable(&repo);
    let native = support::compat_executable();
    for format in ["neo4j", "falkordb"] {
        let arguments = ["export", format];
        let expected = run(&python, &repo, directory.path(), &arguments)?;
        let expected_cypher = std::fs::read(directory.path().join("graphify-out/cypher.txt"))?;
        std::fs::remove_file(directory.path().join("graphify-out/cypher.txt"))?;
        let actual = run(native, &repo, directory.path(), &arguments)?;
        assert_eq!(actual.status.code(), expected.status.code(), "{format}");
        assert_eq!(actual.stdout, expected.stdout, "{format}");
        assert_eq!(actual.stderr, expected.stderr, "{format}");
        assert_eq!(
            std::fs::read(directory.path().join("graphify-out/cypher.txt"))?,
            expected_cypher,
            "{format}"
        );
    }
    Ok(())
}

#[test]
fn live_push_validation_is_safe_and_namespaced() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    seed(directory.path())?;
    let missing = support::compat_command()
        .args(["export", "neo4j", "--push", "bolt://127.0.0.1:1"])
        .current_dir(directory.path())
        .env_remove("NEO4J_PASSWORD")
        .output()?;
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&missing.stderr),
        "error: --password required for --push\n"
    );

    let failed = support::compat_command()
        .args([
            "export",
            "neo4j",
            "--push",
            "bolt://127.0.0.1:1",
            "--password",
            "never-print-this",
        ])
        .current_dir(directory.path())
        .env("GRAPHIFY_GRAPHDB_TIMEOUT", "1")
        .output()?;
    assert_eq!(failed.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("never-print-this"));

    let help = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["export", "--help"])
        .output()?;
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("neo4j"));
    assert!(help.contains("falkordb"));
    Ok(())
}
