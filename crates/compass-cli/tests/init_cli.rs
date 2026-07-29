use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io::Cursor;
use std::process::{Command, Stdio};

use compass_files::{BuildGuard, ProjectConfig};
use compass_model::GraphDocument;
use serde_json::Value;

#[test]
fn init_persists_scope_and_builds_only_matching_files() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    fs::create_dir(root.path().join("tools"))?;
    fs::write(root.path().join("src/lib.rs"), "pub fn included() {}\n")?;
    fs::write(root.path().join("tools/task.rs"), "pub fn excluded() {}\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--include", "src", "--yes", "--timing"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Compass init completed in "),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("[compass timing] deterministic extract:"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.path().join(".compass/config.toml").is_file());
    let graph = GraphDocument::load(&BuildGuard::resolve_artifact(
        &root.path().join("compass-out"),
        "graph.json",
    )?)?;
    assert!(graph.nodes.iter().any(|node| node.label() == "included()"));
    assert!(!graph.nodes.iter().any(|node| node.label() == "excluded()"));
    Ok(())
}

#[test]
fn init_refuses_overwrite_and_unmatched_scope() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("main.rs"), "fn main() {}\n")?;
    fs::write(root.path().join("other.rs"), "fn other() {}\n")?;
    let first = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--yes"])
        .current_dir(root.path())
        .output()?;
    assert!(first.status.success());
    let second = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--yes"])
        .current_dir(root.path())
        .output()?;
    assert_eq!(second.status.code(), Some(2));
    let forced = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--include", "main.rs", "--yes", "--force"])
        .current_dir(root.path())
        .output()?;
    assert!(forced.status.success());
    let config = ProjectConfig::load(root.path())?.ok_or("missing forced config")?;
    assert_eq!(config.build.include, ["main.rs"]);
    let graph = GraphDocument::load(&BuildGuard::resolve_artifact(
        &root.path().join("compass-out"),
        "graph.json",
    )?)?;
    assert!(graph.nodes.iter().all(|node| node.label() != "other()"));

    let other = tempfile::tempdir()?;
    fs::write(other.path().join("main.rs"), "fn main() {}\n")?;
    let unmatched = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--include", "missing/**", "--yes"])
        .current_dir(other.path())
        .output()?;
    assert_eq!(unmatched.status.code(), Some(2));
    assert!(!other.path().join(".compass/config.toml").exists());
    Ok(())
}

#[test]
fn update_reuses_scope_and_invalid_config_never_widens_it() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    fs::create_dir(root.path().join("tools"))?;
    fs::write(root.path().join("src/lib.rs"), "pub fn included() {}\n")?;
    fs::write(root.path().join("tools/task.rs"), "pub fn excluded() {}\n")?;
    let init = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--include", "src", "--yes"])
        .current_dir(root.path())
        .output()?;
    assert!(init.status.success());

    fs::write(
        root.path().join("tools/task.rs"),
        "pub fn newly_excluded() {}\n",
    )?;
    let update = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["update", ".", "--timing"])
        .current_dir(root.path())
        .output()?;
    assert!(update.status.success());
    assert!(
        String::from_utf8_lossy(&update.stdout).contains("Compass update completed in "),
        "stdout: {}",
        String::from_utf8_lossy(&update.stdout)
    );
    assert!(
        String::from_utf8_lossy(&update.stderr).contains("[compass timing] total:"),
        "stderr: {}",
        String::from_utf8_lossy(&update.stderr)
    );
    let graph = GraphDocument::load(&BuildGuard::resolve_artifact(
        &root.path().join("compass-out"),
        "graph.json",
    )?)?;
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| node.label() == "newly_excluded()")
    );

    fs::write(
        root.path().join(".compass/config.toml"),
        "version = 1\nunknown = true\n[build]\n",
    )?;
    let invalid = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["update", "."])
        .current_dir(root.path())
        .output()?;
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid Compass project config"));
    Ok(())
}

#[test]
fn non_interactive_init_requires_yes() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("main.rs"), "fn main() {}\n")?;
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", "."])
        .current_dir(root.path())
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("pass --yes"));
    assert!(!root.path().join(".compass/config.toml").exists());
    Ok(())
}

#[test]
fn interactive_init_uses_the_same_validated_configuration() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    fs::write(root.path().join("src/lib.rs"), "pub fn included() {}\n")?;
    let arguments = vec![OsString::from(root.path())];
    let mut input = Cursor::new(b"custom\nsrc\n\nyes\n");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let code = compass_cli::run_init(&arguments, &mut input, &mut stdout, &mut stderr, true);

    assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&stderr));
    let config = ProjectConfig::load(root.path())?.ok_or("missing interactive config")?;
    assert_eq!(config.build.include, ["src/"]);
    Ok(())
}

#[test]
fn jsonl_init_reports_each_indexed_file_against_the_total() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    for index in 0..260 {
        fs::write(
            root.path().join(format!("src/file_{index:03}.rs")),
            format!("pub fn file_{index:03}() {{}}\n"),
        )?;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "init",
            ".",
            "--include",
            "src",
            "--yes",
            "--events",
            "jsonl",
        ])
        .current_dir(root.path())
        .output()?;

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let events = String::from_utf8(output.stdout)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let indexing = events
        .iter()
        .filter(|event| event["phase"] == "indexing")
        .collect::<Vec<_>>();
    assert_eq!(indexing.len(), 260);
    for (index, event) in indexing.iter().enumerate() {
        assert_eq!(event["current"], index + 1);
        assert_eq!(event["total"], 260);
    }
    assert!(indexing.iter().all(|event| {
        event["message"]
            .as_str()
            .is_some_and(|message| message.ends_with(".rs"))
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["terminal"] == true)
            .count(),
        1
    );
    Ok(())
}
