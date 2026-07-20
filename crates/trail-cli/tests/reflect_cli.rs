use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

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

fn run_python(repo: &Path, root: &Path, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(python_executable(repo))
        .args(["-m", "graphify"])
        .args(arguments)
        .current_dir(root)
        .env("PYTHONPATH", repo)
        .output()?)
}

fn run_native(root: &Path, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_graphify"))
        .args(arguments)
        .current_dir(root)
        .output()?)
}

fn seed(root: &Path) -> Result<(), Box<dyn Error>> {
    let output = root.join("graphify-out");
    let memory = output.join("memory");
    std::fs::create_dir_all(&memory)?;
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(root.join("src/auth.rs"), "fn authenticate() {}\n")?;
    let memory_doc = |date: &str, question: &str, outcome: &str| {
        format!(
            "---\ntype: \"query\"\ndate: \"{date}\"\nquestion: \"{question}\"\ncontributor: \"graphify\"\noutcome: \"{outcome}\"\nsource_nodes: [\"AuthMiddleware\"]\n---\n\n# Q: {question}\n\n## Answer\n\na"
        )
    };
    std::fs::write(
        memory.join("01.md"),
        memory_doc("2026-05-01T00:00:00+00:00", "auth?", "useful"),
    )?;
    std::fs::write(
        memory.join("02.md"),
        memory_doc("2026-05-02T00:00:00+00:00", "login?", "useful"),
    )?;
    std::fs::write(
        output.join("graph.json"),
        serde_json::to_vec(&json!({
            "directed":true,
            "multigraph":false,
            "graph":{},
            "nodes":[{"id":"auth_id","label":"AuthMiddleware","source_file":"src/auth.rs"}],
            "links":[],
        }))?,
    )?;
    std::fs::write(
        output.join(".graphify_analysis.json"),
        serde_json::to_vec(&json!({"communities":{"0":["auth_id"]}}))?,
    )?;
    std::fs::write(
        output.join(".graphify_labels.json"),
        serde_json::to_vec(&json!({"0":"Authentication"}))?,
    )?;
    Ok(())
}

fn normalized_overlay(path: &Path) -> Result<Value, Box<dyn Error>> {
    let mut value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    value["generated_at"] = Value::String("<timestamp>".to_owned());
    Ok(value)
}

#[test]
fn reflect_cli_and_learning_sidecar_match_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let python_root = directory.path().join("python");
    let native_root = directory.path().join("native");
    seed(&python_root)?;
    seed(&native_root)?;
    let repo = repository_root();
    let arguments = ["reflect", "--half-life-days", "0"];
    let python = run_python(&repo, &python_root, &arguments)?;
    let native = run_native(&native_root, &arguments)?;
    assert!(
        python.status.success(),
        "{}",
        String::from_utf8_lossy(&python.stderr)
    );
    assert!(
        native.status.success(),
        "{}",
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(native.stdout, python.stdout);
    assert_eq!(native.stderr, python.stderr);
    assert_eq!(
        std::fs::read(python_root.join("graphify-out/reflections/LESSONS.md"))?,
        std::fs::read(native_root.join("graphify-out/reflections/LESSONS.md"))?
    );
    assert_eq!(
        normalized_overlay(&python_root.join("graphify-out/.graphify_learning.json"))?,
        normalized_overlay(&native_root.join("graphify-out/.graphify_learning.json"))?
    );

    let stale_arguments = ["reflect", "--half-life-days", "0", "--if-stale"];
    let python = run_python(&repo, &python_root, &stale_arguments)?;
    let native = run_native(&native_root, &stale_arguments)?;
    assert_eq!(native.status.code(), python.status.code());
    assert_eq!(native.stdout, python.stdout);
    assert_eq!(native.stderr, python.stderr);
    Ok(())
}
