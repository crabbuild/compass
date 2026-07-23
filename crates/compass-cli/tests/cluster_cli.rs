mod support;

use std::collections::BTreeMap;
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
    let mut command = Command::new(repository.join(".venv/bin/python"));
    command
        .args(["-m", "graphify", "cluster-only"])
        .args(arguments)
        .current_dir(&repository)
        .env("PYTHONPATH", &repository);
    clear_provider_environment(&mut command);
    Ok(command.output()?)
}

fn run_rust(arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    let mut command = support::compat_command();
    command
        .arg("cluster-only")
        .args(arguments)
        .current_dir(repository_root());
    clear_provider_environment(&mut command);
    Ok(command.output()?)
}

fn clear_provider_environment(command: &mut Command) {
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
        "OLLAMA_BASE_URL",
    ] {
        command.env_remove(key);
    }
}

fn assert_same(expected: &Output, actual: &Output) {
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout, "stdout mismatch");
    assert_eq!(actual.stderr, expected.stderr, "stderr mismatch");
}

fn snapshot(directory: &Path) -> Result<BTreeMap<String, Vec<u8>>, Box<dyn Error>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let name = entry
                .file_name()
                .to_string_lossy()
                .replace(".compass_", ".graphify_");
            files.insert(name, std::fs::read(entry.path())?);
        }
    }
    Ok(files)
}

fn reset_fixture(root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let output = root.join("graphify-out");
    if output.exists() {
        std::fs::remove_dir_all(&output)?;
    }
    std::fs::create_dir(&output)?;
    std::fs::copy(
        repository_root().join("tests/fixtures/extraction.json"),
        output.join("graph.json"),
    )?;
    Ok(output)
}

#[test]
fn cluster_only_no_label_matches_python_artifacts() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let output = reset_fixture(root)?;
    let root_text = root.to_string_lossy();
    let arguments = [root_text.as_ref(), "--no-label", "--no-viz"];

    let expected_output = run_python(&arguments)?;
    let expected_files = snapshot(&output)?;
    reset_fixture(root)?;
    let actual_output = run_rust(&arguments)?;

    assert_same(&expected_output, &actual_output);
    assert_eq!(snapshot(&output)?, expected_files);
    Ok(())
}

#[test]
fn cluster_only_missing_graph_matches_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().to_string_lossy();
    let arguments = [root.as_ref(), "--no-label", "--no-viz"];
    assert_same(&run_python(&arguments)?, &run_rust(&arguments)?);
    Ok(())
}

#[test]
fn cluster_only_default_labeling_matches_python_artifacts() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let output = reset_fixture(root)?;
    let root_text = root.to_string_lossy();
    let arguments = [root_text.as_ref(), "--no-viz"];

    let expected_output = run_python(&arguments)?;
    let expected_files = snapshot(&output)?;
    reset_fixture(root)?;
    let actual_output = run_rust(&arguments)?;

    assert_same(&expected_output, &actual_output);
    assert_eq!(snapshot(&output)?, expected_files);
    Ok(())
}

#[test]
fn cluster_only_reuses_existing_labels_like_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let output = reset_fixture(root)?;
    let root_text = root.to_string_lossy();
    let seed_arguments = [root_text.as_ref(), "--no-label", "--no-viz"];
    let seed = run_python(&seed_arguments)?;
    assert!(
        seed.status.success(),
        "{}",
        String::from_utf8_lossy(&seed.stderr)
    );
    let labels = std::fs::read(output.join(".graphify_labels.json"))?;
    let signatures = std::fs::read(output.join(".graphify_labels.json.sig"))?;

    reset_fixture(root)?;
    std::fs::write(output.join(".graphify_labels.json"), &labels)?;
    std::fs::write(output.join(".graphify_labels.json.sig"), &signatures)?;
    let graph = output.join("graph.json").to_string_lossy().into_owned();
    let arguments = [
        root_text.as_ref(),
        "--graph",
        graph.as_str(),
        "--no-viz",
        "--resolution=1.0",
    ];
    let expected_output = run_python(&arguments)?;
    let expected_files = snapshot(&output)?;

    reset_fixture(root)?;
    std::fs::write(output.join(".compass_labels.json"), labels)?;
    std::fs::write(output.join(".compass_labels.json.sig"), signatures)?;
    let actual_output = run_rust(&arguments)?;
    assert_same(&expected_output, &actual_output);
    assert_eq!(snapshot(&output)?, expected_files);
    Ok(())
}

#[test]
fn cluster_only_timing_stage_contract_matches_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root_path = directory.path();
    reset_fixture(root_path)?;
    let root = root_path.to_string_lossy();
    let arguments = [root.as_ref(), "--no-label", "--no-viz", "--timing"];
    let expected = run_python(&arguments)?;
    reset_fixture(root_path)?;
    let actual = run_rust(&arguments)?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    let timing = regex::Regex::new(r"(?m)(\[graphify timing\] [a-z]+: )\d+\.\d+s")?;
    assert_eq!(
        timing.replace_all(&String::from_utf8(expected.stderr)?, "$1<TIME>"),
        timing.replace_all(&String::from_utf8(actual.stderr)?, "$1<TIME>")
    );
    Ok(())
}
