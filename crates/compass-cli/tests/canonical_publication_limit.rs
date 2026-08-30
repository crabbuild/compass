use std::error::Error;
use std::fs;
use std::process::Command;

use compass_files::BuildGuard;

#[test]
fn actual_delta_overrun_preserves_the_active_snapshot() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("sample.rs");
    fs::write(&source, "pub fn sample() -> u64 { 1 }\n// before\n")?;
    let initial = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "update",
            ".",
            "--force",
            "--store",
            "json",
            "--no-cluster",
            "--no-viz",
            "--no-program",
        ])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .env_remove("COMPASS_MAX_GRAPH_BYTES")
        .output()?;
    assert!(
        initial.status.success(),
        "initial publication failed: {}",
        String::from_utf8_lossy(&initial.stderr)
    );

    let output = root.path().join("compass-out");
    let active_graph = BuildGuard::resolve_artifact(&output, "graph.json")?;
    let prior_bytes = fs::read(&active_graph)?;
    fs::write(
        &source,
        format!(
            "pub fn sample() -> u64 {{ 1 }}\n// {}\n",
            "after".repeat(200)
        ),
    )?;

    let rejected = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "update",
            ".",
            "--store",
            "json",
            "--no-cluster",
            "--no-viz",
            "--no-program",
        ])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .env("COMPASS_MAX_GRAPH_BYTES", prior_bytes.len().to_string())
        .output()?;
    assert_eq!(rejected.status.code(), Some(1));
    let stderr = String::from_utf8(rejected.stderr)?;
    assert!(stderr.contains("canonical graph exceeds"), "{stderr}");
    assert!(stderr.contains("COMPASS_MAX_GRAPH_BYTES"), "{stderr}");

    let still_active = BuildGuard::resolve_artifact(&output, "graph.json")?;
    assert_eq!(fs::read(still_active)?, prior_bytes);
    Ok(())
}

#[test]
fn actual_full_overrun_preserves_the_active_sqlite_snapshot() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("sample.rs");
    fs::write(&source, "pub fn sample() -> u64 { 1 }\n")?;
    let initial = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "update",
            ".",
            "--force",
            "--store",
            "sqlite",
            "--no-cluster",
            "--no-viz",
            "--no-program",
        ])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .env_remove("COMPASS_MAX_GRAPH_BYTES")
        .output()?;
    assert!(
        initial.status.success(),
        "initial SQLite publication failed: {}",
        String::from_utf8_lossy(&initial.stderr)
    );

    let output = root.path().join("compass-out");
    let active_graph = BuildGuard::resolve_artifact(&output, "graph.json")?;
    let prior_graph = fs::read(&active_graph)?;
    let active_store_ref = BuildGuard::resolve_artifact(&output, "store.ref")?;
    let prior_store_ref = fs::read(&active_store_ref)?;
    let expanded = (0..200)
        .map(|index| format!("pub fn added_{index}() -> usize {{ {index} }}\n"))
        .collect::<String>();
    fs::write(&source, expanded)?;

    let rejected = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "update",
            ".",
            "--store",
            "sqlite",
            "--no-cluster",
            "--no-viz",
            "--no-program",
        ])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .env("COMPASS_MAX_GRAPH_BYTES", prior_graph.len().to_string())
        .output()?;
    assert_eq!(rejected.status.code(), Some(1));
    let stderr = String::from_utf8(rejected.stderr)?;
    assert!(stderr.contains("canonical graph exceeds"), "{stderr}");

    let still_active_graph = BuildGuard::resolve_artifact(&output, "graph.json")?;
    let still_active_ref = BuildGuard::resolve_artifact(&output, "store.ref")?;
    assert_eq!(fs::read(still_active_graph)?, prior_graph);
    assert_eq!(fs::read(still_active_ref)?, prior_store_ref);
    Ok(())
}
