use std::error::Error;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn run_update(root: &Path, configure: impl FnOnce(&mut Command)) -> Result<(), Box<dyn Error>> {
    std::fs::write(root.join("sample.rs"), "fn sample() {}\n")?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_compass"));
    command
        .args(["update", ".", "--code-only", "--no-viz"])
        .current_dir(root)
        .env_remove("COMPASS_OUT")
        .env_remove("GRAPHIFY_OUT");
    configure(&mut command);
    let output = command.output()?;
    assert!(
        output.status.success(),
        "compass update failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn update_writes_to_compass_out_by_default() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    run_update(root.path(), |_| {})?;

    assert!(root.path().join("compass-out/graph.json").is_file());
    assert!(!root.path().join("graphify-out").exists());
    Ok(())
}

#[test]
fn compass_out_overrides_the_output_and_graphify_out_is_ignored() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    run_update(root.path(), |command| {
        command
            .env("COMPASS_OUT", "chosen-output")
            .env("GRAPHIFY_OUT", "legacy-output");
    })?;

    assert!(root.path().join("chosen-output/graph.json").is_file());
    assert!(!root.path().join("legacy-output").exists());
    Ok(())
}

#[test]
fn compass_cli_exposes_only_the_compass_binary() -> Result<(), Box<dyn Error>> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("workspace root")?;
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(workspace)
        .output()?;
    assert!(output.status.success());
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let package = metadata["packages"]
        .as_array()
        .and_then(|packages| {
            packages
                .iter()
                .find(|package| package["name"] == "compass-cli")
        })
        .ok_or("compass-cli package")?;
    let mut binaries = package["targets"]
        .as_array()
        .ok_or("targets")?
        .iter()
        .filter(|target| {
            target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .filter_map(|target| target["name"].as_str())
        .collect::<Vec<_>>();
    binaries.sort_unstable();

    assert_eq!(binaries, ["compass"]);
    Ok(())
}
