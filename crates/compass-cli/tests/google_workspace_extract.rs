#![cfg(unix)]

use std::error::Error;
use std::os::unix::fs::PermissionsExt;
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

fn run(
    executable: &Path,
    repository: &Path,
    root: &Path,
    path: &str,
) -> Result<Output, Box<dyn Error>> {
    let python = repository.join(".venv/bin/python");
    let mut command = Command::new(executable);
    if executable == python {
        command.args(["-m", "graphify"]);
        command.env("PYTHONPATH", repository);
    }
    Ok(command
        .current_dir(root)
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .args([
            "extract",
            ".",
            "--google-workspace",
            "--code-only",
            "--no-cluster",
            "--no-viz",
        ])
        .output()?)
}

#[test]
fn google_workspace_extract_matches_python_graph_and_sidecar() -> Result<(), Box<dyn Error>> {
    let repository = repository_root();
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("project");
    let tools = directory.path().join("bin");
    std::fs::create_dir_all(&root)?;
    std::fs::create_dir_all(&tools)?;
    std::fs::write(
        root.join("Planning.gdoc"),
        r#"{"url":"https://docs.google.com/document/d/doc-123/edit","email":"me@example.com"}"#,
    )?;
    let gws = tools.join("gws");
    std::fs::write(
        &gws,
        "#!/bin/sh\nfor arg in \"$@\"; do output=\"$arg\"; done\nprintf '# Planning\\n\\nExported doc text.\\n' > \"$output\"\n",
    )?;
    let mut permissions = std::fs::metadata(&gws)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&gws, permissions)?;
    let path = format!(
        "{}:{}",
        tools.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let python = run(
        &repository.join(".venv/bin/python"),
        &repository,
        &root,
        &path,
    )?;
    assert!(
        python.status.success(),
        "{}",
        String::from_utf8_lossy(&python.stderr)
    );
    let python_graph = std::fs::read(root.join("graphify-out/graph.json"))?;
    let python_sidecar = std::fs::read(
        std::fs::read_dir(root.join("graphify-out/converted"))?
            .next()
            .ok_or("Python did not create a sidecar")??
            .path(),
    )?;
    std::fs::remove_dir_all(root.join("graphify-out"))?;

    let rust = run(
        Path::new(env!("CARGO_BIN_EXE_compass")),
        &repository,
        &root,
        &path,
    )?;
    assert!(
        rust.status.success(),
        "{}",
        String::from_utf8_lossy(&rust.stderr)
    );
    let rust_graph = std::fs::read(root.join("graphify-out/graph.json"))?;
    let rust_sidecar = std::fs::read(
        std::fs::read_dir(root.join("graphify-out/converted"))?
            .next()
            .ok_or("Rust did not create a sidecar")??
            .path(),
    )?;
    assert_eq!(rust_sidecar, python_sidecar);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&rust_graph)?,
        serde_json::from_slice::<serde_json::Value>(&python_graph)?
    );
    Ok(())
}
