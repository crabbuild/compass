use std::error::Error;
use std::fs;
use std::process::Command;

use compass_model::GraphDocument;

#[test]
fn init_persists_scope_and_builds_only_matching_files() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("src"))?;
    fs::create_dir(root.path().join("tools"))?;
    fs::write(root.path().join("src/lib.rs"), "pub fn included() {}\n")?;
    fs::write(root.path().join("tools/task.rs"), "pub fn excluded() {}\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--include", "src", "--yes"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.path().join(".compass/config.toml").is_file());
    let graph = GraphDocument::load(&root.path().join("compass-out/graph.json"))?;
    assert!(graph.nodes.iter().any(|node| node.label() == "included()"));
    assert!(!graph.nodes.iter().any(|node| node.label() == "excluded()"));
    Ok(())
}

#[test]
fn init_refuses_overwrite_and_unmatched_scope() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("main.rs"), "fn main() {}\n")?;
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
