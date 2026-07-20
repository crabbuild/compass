use std::error::Error;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use regex::Regex;

const PROVIDER_ENVIRONMENT: &[&str] = &[
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "MOONSHOT_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "DEEPSEEK_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "AZURE_OPENAI_ENDPOINT",
    "AWS_PROFILE",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "OLLAMA_BASE_URL",
];

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

fn run(
    executable: &Path,
    repo: &Path,
    root: &Path,
    isolated_home: &Path,
    extra: &[&str],
) -> Result<Output, Box<dyn Error>> {
    run_with_environment(executable, repo, root, isolated_home, extra, &[])
}

fn run_with_environment(
    executable: &Path,
    repo: &Path,
    root: &Path,
    isolated_home: &Path,
    extra: &[&str],
    environment: &[(&str, &str)],
) -> Result<Output, Box<dyn Error>> {
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let mut command = Command::new(executable);
    if executable == python {
        command.args(["-m", "graphify"]);
        command.env("PYTHONPATH", repo);
    }
    for key in PROVIDER_ENVIRONMENT {
        command.env_remove(key);
    }
    command
        .current_dir(root)
        .env("HOME", isolated_home)
        .env("USERPROFILE", isolated_home)
        .args([
            OsStr::new("label"),
            root.as_os_str(),
            OsStr::new("--no-viz"),
        ]);
    command.args(extra);
    command.envs(environment.iter().copied());
    Ok(command.output()?)
}

fn seed(root: &Path) -> Result<(), Box<dyn Error>> {
    let output = root.join("graphify-out");
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("graph.json"),
        br#"{
  "directed": false,
  "multigraph": false,
  "graph": {},
  "nodes": [
    {"id": "orders", "label": "OrderService", "file_type": "code", "source_file": "orders.py"},
    {"id": "repository", "label": "OrderRepository", "file_type": "code", "source_file": "orders.py"},
    {"id": "payments", "label": "PaymentService", "file_type": "code", "source_file": "payments.py"}
  ],
  "links": [
    {"source": "orders", "target": "repository", "relation": "CALLS", "confidence": "EXTRACTED", "source_file": "orders.py"}
  ]
}"#,
    )?;
    Ok(())
}

#[test]
fn label_without_provider_matches_python_surface_and_artifacts() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let python_root = parent.path().join("python");
    let native_root = parent.path().join("native");
    let isolated_home = parent.path().join("home");
    seed(&python_root)?;
    seed(&native_root)?;
    std::fs::create_dir_all(&isolated_home)?;
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let expected = run(&python, &repo, &python_root, &isolated_home, &[])?;
    let actual = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &isolated_home,
        &[],
    )?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
    for artifact in [
        ".graphify_labels.json",
        ".graphify_labels.json.sig",
        ".graphify_analysis.json",
    ] {
        let expected = std::fs::read(python_root.join("graphify-out").join(artifact))?;
        let actual = std::fs::read(native_root.join("graphify-out").join(artifact))?;
        assert_eq!(actual, expected, "{artifact}");
    }
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(python_root.join("graphify-out/graph.json"))?)?;
    let actual: serde_json::Value =
        serde_json::from_slice(&std::fs::read(native_root.join("graphify-out/graph.json"))?)?;
    assert_eq!(actual, expected, "graph.json");
    Ok(())
}

#[test]
fn missing_only_preserves_curated_labels_like_python() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let python_root = parent.path().join("python");
    let native_root = parent.path().join("native");
    let isolated_home = parent.path().join("home");
    seed(&python_root)?;
    seed(&native_root)?;
    std::fs::create_dir_all(&isolated_home)?;
    let labels = br#"{"0": "Curated Orders", "1": "Community 1"}"#;
    std::fs::write(
        python_root.join("graphify-out/.graphify_labels.json"),
        labels,
    )?;
    std::fs::write(
        native_root.join("graphify-out/.graphify_labels.json"),
        labels,
    )?;
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let expected = run(
        &python,
        &repo,
        &python_root,
        &isolated_home,
        &["--missing-only"],
    )?;
    let actual = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &isolated_home,
        &["--missing-only"],
    )?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
    assert_eq!(
        std::fs::read(native_root.join("graphify-out/.graphify_labels.json"))?,
        std::fs::read(python_root.join("graphify-out/.graphify_labels.json"))?
    );
    Ok(())
}

#[test]
fn trail_label_help_is_namespaced() -> Result<(), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_trail"))
        .args(["graph", "label", "--help"])
        .output()?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("trail graph label"));
    Ok(())
}

#[test]
fn project_local_provider_gate_matches_python_warning() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let python_root = parent.path().join("python");
    let native_root = parent.path().join("native");
    let isolated_home = parent.path().join("home");
    seed(&python_root)?;
    seed(&native_root)?;
    std::fs::create_dir_all(&isolated_home)?;
    for root in [&python_root, &native_root] {
        std::fs::create_dir_all(root.join(".graphify"))?;
        std::fs::write(
            root.join(".graphify/providers.json"),
            r#"{"local":{"base_url":"https://example.invalid/v1","default_model":"fixture","env_key":"FIXTURE_KEY"}}"#,
        )?;
    }
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let expected = run(&python, &repo, &python_root, &isolated_home, &[])?;
    let actual = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &isolated_home,
        &[],
    )?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
    Ok(())
}

#[test]
fn global_provider_endpoint_warning_matches_python() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let python_root = parent.path().join("python");
    let native_root = parent.path().join("native");
    let isolated_home = parent.path().join("home");
    seed(&python_root)?;
    seed(&native_root)?;
    std::fs::create_dir_all(isolated_home.join(".graphify"))?;
    std::fs::write(
        isolated_home.join(".graphify/providers.json"),
        r#"{"remote":{"base_url":"http://example.com/v1","default_model":"fixture","env_key":"FIXTURE_KEY"}}"#,
    )?;
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let expected = run(&python, &repo, &python_root, &isolated_home, &[])?;
    let actual = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &isolated_home,
        &[],
    )?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
    Ok(())
}

#[test]
fn report_learning_section_matches_python() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let root = parent.path().join("project");
    let isolated_home = parent.path().join("home");
    seed(&root)?;
    std::fs::create_dir_all(&isolated_home)?;
    std::fs::write(
        root.join("graphify-out/.graphify_learning.json"),
        r#"{"version":1,"nodes":{"orders":{"status":"preferred","score":1.5,"uses":3,"last":"2026-07-19","label":"OrderService","source_file":"","code_fingerprint":"","provenance":[]}}}"#,
    )?;
    std::fs::create_dir_all(root.join("graphify-out/memory"))?;
    std::fs::write(
        root.join("graphify-out/memory/dead-end.md"),
        "---\ntype: \"query\"\ndate: \"2026-07-19\"\nquestion: \"Where is the retired queue?\"\noutcome: \"dead_end\"\nsource_nodes: [\"orders\"]\n---\n",
    )?;
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let expected_run = run(&python, &repo, &root, &isolated_home, &[])?;
    assert!(expected_run.status.success());
    let expected = std::fs::read(root.join("graphify-out/GRAPH_REPORT.md"))?;

    seed(&root)?;
    for artifact in [
        "GRAPH_REPORT.md",
        ".graphify_labels.json",
        ".graphify_labels.json.sig",
        ".graphify_analysis.json",
    ] {
        let _ = std::fs::remove_file(root.join("graphify-out").join(artifact));
    }
    let actual_run = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &root,
        &isolated_home,
        &[],
    )?;
    assert!(actual_run.status.success());
    let actual = std::fs::read(root.join("graphify-out/GRAPH_REPORT.md"))?;
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn timing_diagnostics_match_python_stage_order() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let python_root = parent.path().join("python");
    let native_root = parent.path().join("native");
    let isolated_home = parent.path().join("home");
    seed(&python_root)?;
    seed(&native_root)?;
    std::fs::create_dir_all(&isolated_home)?;
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let expected = run(&python, &repo, &python_root, &isolated_home, &["--timing"])?;
    let actual = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &isolated_home,
        &["--timing"],
    )?;
    let durations = Regex::new(r"\d+\.\d+s")?;
    let expected_stderr = String::from_utf8(expected.stderr)?;
    let actual_stderr = String::from_utf8(actual.stderr)?;
    let expected_stderr = durations.replace_all(&expected_stderr, "<time>");
    let actual_stderr = durations.replace_all(&actual_stderr, "<time>");
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual_stderr, expected_stderr);
    Ok(())
}

#[test]
fn oversized_graph_warning_and_core_refresh_match_python() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let python_root = parent.path().join("python");
    let native_root = parent.path().join("native");
    let isolated_home = parent.path().join("home");
    seed(&python_root)?;
    seed(&native_root)?;
    std::fs::create_dir_all(&isolated_home)?;
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let environment = [("GRAPHIFY_MAX_GRAPH_BYTES", "100")];
    let expected = run_with_environment(
        &python,
        &repo,
        &python_root,
        &isolated_home,
        &[],
        &environment,
    )?;
    let actual = run_with_environment(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &isolated_home,
        &[],
        &environment,
    )?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
    assert!(native_root.join("graphify-out/GRAPH_REPORT.md").is_file());
    Ok(())
}

#[test]
fn unknown_backend_failure_surface_matches_python() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let python_root = parent.path().join("python");
    let native_root = parent.path().join("native");
    let isolated_home = parent.path().join("home");
    seed(&python_root)?;
    seed(&native_root)?;
    std::fs::create_dir_all(&isolated_home)?;
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let expected = run(
        &python,
        &repo,
        &python_root,
        &isolated_home,
        &["--backend=definitely-unknown"],
    )?;
    let actual = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &isolated_home,
        &["--backend=definitely-unknown"],
    )?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
    Ok(())
}

#[test]
fn graphify_help_flag_retains_python_legacy_behavior() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let python_root = parent.path().join("python");
    let native_root = parent.path().join("native");
    let isolated_home = parent.path().join("home");
    seed(&python_root)?;
    seed(&native_root)?;
    std::fs::create_dir_all(&isolated_home)?;
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let expected = run(&python, &repo, &python_root, &isolated_home, &["--help"])?;
    let actual = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &isolated_home,
        &["--help"],
    )?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
    Ok(())
}

#[test]
fn graph_override_accepts_non_json_extension_like_python() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let parent = tempfile::tempdir()?;
    let python_root = parent.path().join("python");
    let native_root = parent.path().join("native");
    let isolated_home = parent.path().join("home");
    seed(&python_root)?;
    seed(&native_root)?;
    std::fs::create_dir_all(&isolated_home)?;
    for root in [&python_root, &native_root] {
        std::fs::copy(
            root.join("graphify-out/graph.json"),
            root.join("archived-graph.data"),
        )?;
    }
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let python_graph = python_root.join("archived-graph.data");
    let native_graph = native_root.join("archived-graph.data");
    let expected = run(
        &python,
        &repo,
        &python_root,
        &isolated_home,
        &["--graph", &python_graph.to_string_lossy()],
    )?;
    let actual = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &isolated_home,
        &["--graph", &native_graph.to_string_lossy()],
    )?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
    Ok(())
}
