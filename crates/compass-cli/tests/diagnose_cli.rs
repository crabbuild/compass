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

fn run_python(arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let repository = repository_root();
    Ok(Command::new(repository.join(".venv/bin/python"))
        .args(["-m", "graphify", "diagnose"])
        .args(arguments)
        .current_dir(&repository)
        .env("PYTHONPATH", &repository)
        .output()?)
}

fn run_rust(arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(support::compat_command()
        .arg("diagnose")
        .args(arguments)
        .current_dir(repository_root())
        .output()?)
}

fn assert_same(expected: &Output, actual: &Output) {
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout, "stdout mismatch");
    assert_eq!(actual.stderr, expected.stderr, "stderr mismatch");
}

#[test]
fn diagnose_text_and_json_match_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::to_vec(&serde_json::json!({
            "directed": true,
            "multigraph": false,
            "nodes": [
                {"id": "a", "label": "A", "source_file": "a.py", "source_location": "L1"},
                {"id": "b", "label": "B", "source_file": "b.py", "source_location": "L1"}
            ],
            "links": [
                {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED", "source_file": "a.py", "source_location": "L1", "context": "call"},
                {"source": "a", "target": "b", "relation": "imports", "confidence": "INFERRED", "source_file": "a.py", "source_location": "L1", "context": "import"}
            ]
        }))?,
    )?;
    let graph = graph.to_string_lossy();
    for extra in [Vec::new(), vec!["--json"], vec!["--undirected"]] {
        let mut arguments = vec!["multigraph", "--graph", graph.as_ref()];
        arguments.extend(extra);
        let expected = run_python(&arguments)?;
        let actual = run_rust(&arguments)?;
        assert_same(&expected, &actual);
    }
    let producer = directory.path().join("producer.py");
    std::fs::write(
        &producer,
        "seen_ids: set[tuple[str, str]] = set()\nseen_keys = set()\n",
    )?;
    let producer = producer.to_string_lossy();
    let arguments = [
        "multigraph",
        "--graph",
        graph.as_ref(),
        "--json",
        "--extract-path",
        producer.as_ref(),
    ];
    assert_same(&run_python(&arguments)?, &run_rust(&arguments)?);
    Ok(())
}

#[test]
fn diagnose_usage_and_option_errors_match_python() -> Result<(), Box<dyn Error>> {
    for arguments in [
        vec![],
        vec!["other"],
        vec!["multigraph", "--graph"],
        vec!["multigraph", "--max-examples"],
        vec!["multigraph", "--max-examples", "bad"],
        vec!["multigraph", "--max-examples", "-1"],
        vec!["multigraph", "--directed", "--undirected"],
        vec!["multigraph", "--extract-path"],
        vec!["multigraph", "--unknown"],
    ] {
        let expected = run_python(&arguments)?;
        let actual = run_rust(&arguments)?;
        assert_same(&expected, &actual);
    }
    Ok(())
}

#[test]
fn diagnose_custom_suppression_and_input_failures_match_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    std::fs::write(&graph, r#"{"nodes":[],"edges":[]}"#)?;
    let producer = directory.path().join("extract.py");
    std::fs::write(
        &producer,
        concat!(
            "seen_edges: set[tuple[str, str, str]] = set()\n",
            "  seen_ids = {\"a\"}\n",
            "not_seen_ids = set()\n",
        ),
    )?;
    let missing_producer = directory.path().join("missing.py");
    let graph_text = graph.to_string_lossy();
    let producer_text = producer.to_string_lossy();
    let missing_producer_text = missing_producer.to_string_lossy();
    for arguments in [
        vec![
            "multigraph",
            "--graph",
            graph_text.as_ref(),
            "--extract-path",
            producer_text.as_ref(),
        ],
        vec![
            "multigraph",
            "--graph",
            graph_text.as_ref(),
            "--extract-path",
            producer_text.as_ref(),
            "--json",
            "--max-examples",
            "0",
        ],
        vec![
            "multigraph",
            "--graph",
            graph_text.as_ref(),
            "--extract-path",
            missing_producer_text.as_ref(),
        ],
    ] {
        assert_same(&run_python(&arguments)?, &run_rust(&arguments)?);
    }

    let missing_graph = directory.path().join("missing.json");
    let malformed_graph = directory.path().join("malformed.json");
    let array_graph = directory.path().join("array.json");
    std::fs::write(&malformed_graph, "not json")?;
    std::fs::write(&array_graph, "[]")?;
    for path in [missing_graph, malformed_graph, array_graph] {
        let path = path.to_string_lossy();
        let arguments = ["multigraph", "--graph", path.as_ref()];
        assert_same(&run_python(&arguments)?, &run_rust(&arguments)?);
    }
    Ok(())
}
