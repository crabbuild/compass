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

fn run(
    executable: &Path,
    repository: &Path,
    root: &Path,
    cargo: bool,
) -> Result<Output, Box<dyn Error>> {
    let python = repository.join(".venv/bin/python");
    let mut command = Command::new(executable);
    if executable == python {
        command.args(["-m", "graphify"]);
        command.env("PYTHONPATH", repository);
    }
    command
        .current_dir(root)
        .env("NO_COLOR", "1")
        .args(["extract", "."]);
    if cargo {
        command.arg("--cargo");
    }
    Ok(command
        .args(["--code-only", "--no-cluster", "--no-viz", "--max-workers=1"])
        .output()?)
}

fn cargo_facts(graph: &serde_json::Value) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let mut nodes = graph
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|node| {
            node.get("id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| id.starts_with("crate:"))
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut edges = graph
        .get("links")
        .or_else(|| graph.get("edges"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|edge| {
            edge.get("relation").and_then(serde_json::Value::as_str) == Some("crate_depends_on")
        })
        .cloned()
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| node.get("id").map(serde_json::Value::to_string));
    edges.sort_by_key(serde_json::Value::to_string);
    (nodes, edges)
}

#[test]
fn cargo_extract_graph_facts_match_python_oracle() -> Result<(), Box<dyn Error>> {
    let repository = repository_root();
    let python_directory = tempfile::tempdir()?;
    let rust_directory = tempfile::tempdir()?;
    seed(python_directory.path())?;
    seed(rust_directory.path())?;
    let expected_process = run(
        &repository.join(".venv/bin/python"),
        &repository,
        python_directory.path(),
        true,
    )?;
    let actual_process = run(
        Path::new(env!("CARGO_BIN_EXE_compass")),
        &repository,
        rust_directory.path(),
        true,
    )?;
    assert!(
        expected_process.status.success(),
        "{}",
        String::from_utf8_lossy(&expected_process.stderr)
    );
    assert!(
        actual_process.status.success(),
        "{}",
        String::from_utf8_lossy(&actual_process.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&std::fs::read(
        python_directory.path().join("graphify-out/graph.json"),
    )?)?;
    let actual: serde_json::Value = serde_json::from_slice(&std::fs::read(
        rust_directory.path().join("graphify-out/graph.json"),
    )?)?;
    assert_eq!(cargo_facts(&actual), cargo_facts(&expected));
    assert!(String::from_utf8_lossy(&actual_process.stdout).contains("Cargo: 2 nodes, 1 edges"));

    let expected_without_cargo = run(
        &repository.join(".venv/bin/python"),
        &repository,
        python_directory.path(),
        false,
    )?;
    let actual_without_cargo = run(
        Path::new(env!("CARGO_BIN_EXE_compass")),
        &repository,
        rust_directory.path(),
        false,
    )?;
    assert!(expected_without_cargo.status.success());
    assert!(actual_without_cargo.status.success());
    let expected: serde_json::Value = serde_json::from_slice(&std::fs::read(
        python_directory.path().join("graphify-out/graph.json"),
    )?)?;
    let actual: serde_json::Value = serde_json::from_slice(&std::fs::read(
        rust_directory.path().join("graphify-out/graph.json"),
    )?)?;
    assert_eq!(cargo_facts(&actual), cargo_facts(&expected));
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
            .contains("[graphify global] 'fixture' merged into global graph")
    );
    assert!(home.path().join(".graphify/global-graph.json").is_file());

    let second = run()?;
    assert!(second.status.success());
    assert!(
        String::from_utf8_lossy(&second.stdout)
            .contains("[graphify global] 'fixture' unchanged since last add - skipped.")
    );
    Ok(())
}

#[test]
fn extract_timing_stage_order_matches_python() -> Result<(), Box<dyn Error>> {
    let repository = repository_root();
    let python_project = tempfile::tempdir()?;
    let rust_project = tempfile::tempdir()?;
    seed(python_project.path())?;
    seed(rust_project.path())?;

    let python = Command::new(repository.join(".venv/bin/python"))
        .args(["-m", "graphify", "extract"])
        .arg(python_project.path())
        .args([
            "--code-only",
            "--no-cluster",
            "--no-viz",
            "--max-workers=1",
            "--timing",
        ])
        .current_dir(&repository)
        .env("PYTHONPATH", &repository)
        .output()?;
    let rust = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["extract"])
        .arg(rust_project.path())
        .args([
            "--code-only",
            "--no-cluster",
            "--no-viz",
            "--max-workers=1",
            "--timing",
        ])
        .output()?;
    assert!(
        python.status.success(),
        "{}",
        String::from_utf8_lossy(&python.stderr)
    );
    assert!(
        rust.status.success(),
        "{}",
        String::from_utf8_lossy(&rust.stderr)
    );

    let stages = |output: &Output| {
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter_map(|line| {
                line.strip_prefix("[graphify timing] ")
                    .and_then(|value| value.split_once(':'))
                    .map(|(stage, _)| stage.to_owned())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(stages(&rust), stages(&python));
    Ok(())
}
