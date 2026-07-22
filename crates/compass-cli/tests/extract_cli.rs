use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("GRAPHIFY_REPO_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

fn command_environment(command: &mut Command, home: &Path) {
    for key in [
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
        "AWS_ACCESS_KEY_ID",
        "OLLAMA_BASE_URL",
        "GRAPHIFY_FORCE",
        "GRAPHIFY_OUT",
    ] {
        command.env_remove(key);
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("GRAPHIFY_NO_UPDATE_CHECK", "1")
        .env("GRAPHIFY_NO_TIPS", "1");
}

fn run_python(arguments: &[&str], home: &Path) -> Result<Output, Box<dyn Error>> {
    let repository = repository_root();
    let mut command = Command::new(repository.join(".venv/bin/python"));
    command
        .args(["-m", "graphify", "extract"])
        .args(arguments)
        .current_dir(&repository)
        .env("PYTHONPATH", &repository);
    command_environment(&mut command, home);
    Ok(command.output()?)
}

fn run_rust(arguments: &[&str], home: &Path) -> Result<Output, Box<dyn Error>> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_graphify"));
    command
        .arg("extract")
        .args(arguments)
        .current_dir(repository_root());
    command_environment(&mut command, home);
    Ok(command.output()?)
}

fn assert_same_output(expected: &Output, actual: &Output) {
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout, "stdout mismatch");
    assert_eq!(actual.stderr, expected.stderr, "stderr mismatch");
}

fn remove_output(root: &Path) -> Result<(), Box<dyn Error>> {
    let output = root.join("graphify-out");
    if output.exists() {
        fs::remove_dir_all(output)?;
    }
    Ok(())
}

fn artifact(root: &Path, name: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(fs::read(root.join("graphify-out").join(name))?)
}

fn fixture(root: &Path) -> Result<(), Box<dyn Error>> {
    fs::write(
        root.join("auth.py"),
        "def login(user):\n    return validate(user)\n\ndef validate(user):\n    return True\n",
    )?;
    Ok(())
}

#[test]
fn graphify_help_matches_python() -> Result<(), Box<dyn Error>> {
    let repository = repository_root();
    let home = tempfile::tempdir()?;
    let mut python = Command::new(repository.join(".venv/bin/python"));
    python
        .args(["-m", "graphify", "--help"])
        .current_dir(&repository)
        .env("PYTHONPATH", &repository);
    command_environment(&mut python, home.path());
    let expected = python.output()?;

    let mut rust = Command::new(env!("CARGO_BIN_EXE_graphify"));
    rust.arg("--help").current_dir(&repository);
    command_environment(&mut rust, home.path());
    assert_same_output(&expected, &rust.output()?);
    for argument in ["-?", "-v", "version"] {
        let mut python = Command::new(repository.join(".venv/bin/python"));
        python
            .args(["-m", "graphify", argument])
            .current_dir(&repository)
            .env("PYTHONPATH", &repository);
        command_environment(&mut python, home.path());
        let mut rust = Command::new(env!("CARGO_BIN_EXE_graphify"));
        rust.arg(argument).current_dir(&repository);
        command_environment(&mut rust, home.path());
        assert_same_output(&python.output()?, &rust.output()?);
    }
    Ok(())
}

#[test]
fn cold_force_and_raw_extract_match_python() -> Result<(), Box<dyn Error>> {
    for extra in [Vec::<&str>::new(), vec!["--force"], vec!["--no-cluster"]] {
        let directory = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        fixture(directory.path())?;
        let root = directory.path().to_string_lossy();
        let mut arguments = vec![root.as_ref(), "--code-only", "--no-viz"];
        arguments.extend(extra);

        let expected = run_python(&arguments, home.path())?;
        let names = if arguments.contains(&"--no-cluster") {
            vec!["graph.json", "manifest.json"]
        } else {
            vec!["graph.json", ".graphify_analysis.json", "manifest.json"]
        };
        let expected_artifacts = names
            .iter()
            .map(|name| artifact(directory.path(), name))
            .collect::<Result<Vec<_>, _>>()?;
        remove_output(directory.path())?;

        let actual = run_rust(&arguments, home.path())?;
        assert_same_output(&expected, &actual);
        for (name, expected) in names.into_iter().zip(expected_artifacts) {
            assert_eq!(
                artifact(directory.path(), name)?,
                expected,
                "{name} mismatch"
            );
        }
    }
    Ok(())
}

#[test]
fn warm_clustered_and_raw_extract_match_python() -> Result<(), Box<dyn Error>> {
    for raw in [false, true] {
        let directory = tempfile::tempdir()?;
        let home = tempfile::tempdir()?;
        fixture(directory.path())?;
        let root = directory.path().to_string_lossy();
        let mut arguments = vec![root.as_ref(), "--code-only", "--no-viz"];
        if raw {
            arguments.push("--no-cluster");
        }

        assert!(run_python(&arguments, home.path())?.status.success());
        let expected = run_python(&arguments, home.path())?;
        let expected_graph: Value =
            serde_json::from_slice(&artifact(directory.path(), "graph.json")?)?;
        let expected_manifest = artifact(directory.path(), "manifest.json")?;
        remove_output(directory.path())?;

        assert!(run_rust(&arguments, home.path())?.status.success());
        let actual = run_rust(&arguments, home.path())?;
        assert_same_output(&expected, &actual);
        let actual_graph: Value =
            serde_json::from_slice(&artifact(directory.path(), "graph.json")?)?;
        assert_eq!(actual_graph, expected_graph);
        assert_eq!(
            artifact(directory.path(), "manifest.json")?,
            expected_manifest
        );
    }
    Ok(())
}

#[test]
fn extract_errors_unknown_flags_and_dedup_key_gate_match_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    fixture(directory.path())?;
    let root = directory.path().to_string_lossy();
    for suffix in [
        vec!["--mode", "shallow"],
        vec!["--max-workers", "abc"],
        vec!["--max-workers", "0"],
        vec!["--token-budget", "0"],
        vec!["--max-concurrency", "0"],
        vec!["--api-timeout", "0"],
        vec!["--resolution", "bad"],
        vec!["--resolution", "0"],
        vec!["--unknown", "--code-only", "--no-viz"],
        vec!["--code-only", "--dedup-llm", "--no-viz"],
    ] {
        let mut arguments = vec![root.as_ref()];
        arguments.extend(suffix);
        remove_output(directory.path())?;
        let expected = run_python(&arguments, home.path())?;
        remove_output(directory.path())?;
        let actual = run_rust(&arguments, home.path())?;
        assert_same_output(&expected, &actual);
    }

    assert_same_output(&run_python(&[], home.path())?, &run_rust(&[], home.path())?);
    assert_same_output(
        &run_python(&["--code-only"], home.path())?,
        &run_rust(&["--code-only"], home.path())?,
    );
    let missing = directory.path().join("missing");
    let missing = missing.to_string_lossy();
    assert_same_output(
        &run_python(&[missing.as_ref()], home.path())?,
        &run_rust(&[missing.as_ref()], home.path())?,
    );

    let semantic = tempfile::tempdir()?;
    fs::write(semantic.path().join("guide.md"), "# Guide\n")?;
    let semantic_root = semantic.path().to_string_lossy();
    for suffix in [Vec::<&str>::new(), vec!["--dedup-llm"]] {
        let mut arguments = vec![semantic_root.as_ref()];
        arguments.extend(suffix);
        remove_output(semantic.path())?;
        let expected = run_python(&arguments, home.path())?;
        remove_output(semantic.path())?;
        let actual = run_rust(&arguments, home.path())?;
        assert_same_output(&expected, &actual);
    }
    Ok(())
}
