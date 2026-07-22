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

fn run_python(arguments: &[&str], environment: &[(&str, &str)]) -> Result<Output, Box<dyn Error>> {
    let repository = repository_root();
    Ok(Command::new(repository.join(".venv/bin/python"))
        .args(["-m", "graphify", "tree"])
        .args(arguments)
        .current_dir(&repository)
        .env("PYTHONPATH", &repository)
        .envs(environment.iter().copied())
        .output()?)
}

fn run_rust(arguments: &[&str], environment: &[(&str, &str)]) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_graphify"))
        .arg("tree")
        .args(arguments)
        .current_dir(repository_root())
        .envs(environment.iter().copied())
        .output()?)
}

fn assert_same(expected: &Output, actual: &Output) {
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout, "stdout mismatch");
    assert_eq!(actual.stderr, expected.stderr, "stderr mismatch");
}

fn assert_artifact(expected: &[u8], actual: &[u8]) -> Result<(), Box<dyn Error>> {
    if expected == actual {
        return Ok(());
    }
    let offset = expected
        .iter()
        .zip(actual)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    let start = offset.saturating_sub(80);
    let expected_end = (offset + 160).min(expected.len());
    let actual_end = (offset + 160).min(actual.len());
    Err(format!(
        "HTML artifact mismatch at byte {offset} (expected {} bytes, actual {} bytes)\nexpected: {:?}\nactual: {:?}",
        expected.len(),
        actual.len(),
        String::from_utf8_lossy(&expected[start..expected_end]),
        String::from_utf8_lossy(&actual[start..actual_end]),
    )
    .into())
}

#[test]
fn tree_help_missing_graph_and_size_cap_match_python() -> Result<(), Box<dyn Error>> {
    assert_same(&run_python(&["--help"], &[])?, &run_rust(&["--help"], &[])?);

    let directory = tempfile::tempdir()?;
    let missing = directory.path().join("missing.json");
    let missing = missing.to_string_lossy();
    let arguments = ["--graph", missing.as_ref()];
    assert_same(&run_python(&arguments, &[])?, &run_rust(&arguments, &[])?);

    let oversized = directory.path().join("oversized.json");
    std::fs::write(
        &oversized,
        r#"{"nodes":[],"links":[],"padding":"xxxxxxxx"}"#,
    )?;
    let oversized = oversized.to_string_lossy();
    let arguments = ["--graph", oversized.as_ref()];
    let environment = [("GRAPHIFY_MAX_GRAPH_BYTES", "16")];
    assert_same(
        &run_python(&arguments, &environment)?,
        &run_rust(&arguments, &environment)?,
    );
    Ok(())
}

#[test]
fn tree_html_and_stdout_match_python_byte_for_byte() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("fixture.data");
    std::fs::write(
        &graph,
        serde_json::to_vec(&serde_json::json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id":"file","label":"a.py","file_type":"code","source_file":"src/pkg/a.py"},
                {"id":"alpha","label":"Alpha","file_type":"code","source_file":"src/pkg/a.py"},
                {"id":"private","label":"_private","file_type":"code","source_file":"src/pkg/a.py"},
                {"id":"unsafe","label":"</script> café","file_type":"code","source_file":"src/pkg/a.py"},
                {"id":"beta","label":"Beta","file_type":"code","source_file":"src/b.py"}
            ],
            "links": []
        }))?,
    )?;
    let output = directory.path().join("nested/GRAPH_TREE.html");
    let graph_text = graph.to_string_lossy();
    let output_text = output.to_string_lossy();
    for max_children in ["2", "-1"] {
        let arguments = [
            "--graph",
            graph_text.as_ref(),
            "--output",
            output_text.as_ref(),
            "--root",
            "src",
            "--max-children",
            max_children,
            "--top-k-edges",
            "-4",
            "--label",
            "Compass & Graphify",
            "--ignored",
        ];
        let expected = run_python(&arguments, &[])?;
        let expected_html = std::fs::read(&output)?;
        std::fs::remove_file(&output)?;
        let actual = run_rust(&arguments, &[])?;
        let actual_html = std::fs::read(&output)?;
        assert_same(&expected, &actual);
        assert_artifact(&expected_html, &actual_html)?;
    }
    Ok(())
}

#[test]
fn empty_tree_artifact_matches_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    let output = directory.path().join("empty.html");
    std::fs::write(&graph, r#"{"nodes":[],"edges":[]}"#)?;
    let graph = graph.to_string_lossy();
    let output = output.to_string_lossy();
    let arguments = ["--graph", graph.as_ref(), "--output", output.as_ref()];
    let expected = run_python(&arguments, &[])?;
    let expected_html = std::fs::read(output.as_ref())?;
    std::fs::remove_file(output.as_ref())?;
    let actual = run_rust(&arguments, &[])?;
    let actual_html = std::fs::read(output.as_ref())?;
    assert_same(&expected, &actual);
    assert_artifact(&expected_html, &actual_html)?;
    Ok(())
}
