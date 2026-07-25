use std::error::Error;
use std::path::Path;
use std::process::Command;

fn seed(root: &Path) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(root.join("crates/app/src"))?;
    std::fs::create_dir_all(root.join("crates/core/src"))?;
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )?;
    std::fs::write(
        root.join("crates/app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n[dependencies]\ncore = { path = \"../core\" }\nserde = \"1\"\n",
    )?;
    std::fs::write(
        root.join("crates/core/Cargo.toml"),
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\n",
    )?;
    std::fs::write(root.join("crates/app/src/lib.rs"), "pub fn run() {}\n")?;
    std::fs::write(root.join("crates/core/src/lib.rs"), "pub struct Core;\n")?;
    Ok(())
}

#[test]
fn cargo_extract_emits_workspace_dependency_facts() -> Result<(), Box<dyn Error>> {
    let project = tempfile::tempdir()?;
    seed(project.path())?;
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "extract",
            ".",
            "--cargo",
            "--code-only",
            "--no-cluster",
            "--no-viz",
            "--max-workers=1",
        ])
        .current_dir(project.path())
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Cargo: 2 nodes, 1 edges"));

    let graph: serde_json::Value = serde_json::from_slice(&std::fs::read(
        project.path().join("compass-out/graph.json"),
    )?)?;
    let crate_nodes = graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| {
            node["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("crate:"))
        })
        .count();
    let dependency_edges = graph
        .get("links")
        .or_else(|| graph.get("edges"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|edge| edge["relation"] == "crate_depends_on")
        .count();
    assert_eq!(crate_nodes, 2);
    assert_eq!(dependency_edges, 1);
    Ok(())
}

#[test]
fn native_extract_can_merge_into_the_global_graph() -> Result<(), Box<dyn Error>> {
    let project = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    seed(project.path())?;
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_compass"))
            .args([
                "extract",
                ".",
                "--code-only",
                "--no-cluster",
                "--no-viz",
                "--max-workers=1",
                "--global",
                "--as",
                "fixture",
            ])
            .current_dir(project.path())
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .output()
    };
    let first = run()?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        String::from_utf8_lossy(&first.stdout)
            .contains("[compass global] 'fixture' merged into global graph")
    );
    assert!(home.path().join(".compass/global-graph.json").is_file());

    let second = run()?;
    assert!(second.status.success());
    assert!(
        String::from_utf8_lossy(&second.stdout)
            .contains("[compass global] 'fixture' unchanged since last add - skipped.")
    );
    Ok(())
}
