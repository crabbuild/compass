use std::error::Error;
use std::path::Path;
use std::process::Command;

use compass_files::BuildGuard;
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

    let output = root.path().join("compass-out");
    assert!(BuildGuard::resolve_artifact(&output, "graph.json")?.is_file());
    assert!(output.join("graph.json").is_file());
    assert!(output.join("GRAPH_REPORT.md").is_file());
    assert!(output.join("manifest.json").is_file());
    assert!(output.join("cache").is_dir());
    assert!(!output.join("graph.html").exists());
    assert!(!root.path().join("graphify-out").exists());
    Ok(())
}

#[test]
fn html_export_is_materialized_directly_in_compass_out() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    run_update(root.path(), |_| {})?;
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["export", "html"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        output.status.success(),
        "compass export html failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(root.path().join("compass-out/graph.html").is_file());
    Ok(())
}

#[test]
fn update_materializes_html_directly_in_compass_out() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join("sample.rs"), "fn sample() {}\n")?;
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["update", ".", "--code-only"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        output.status.success(),
        "compass update failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(root.path().join("compass-out/graph.html").is_file());
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

    assert!(
        BuildGuard::resolve_artifact(&root.path().join("chosen-output"), "graph.json")?.is_file()
    );
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

#[test]
fn install_help_is_compass_native() -> Result<(), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["install", "--help"])
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("compass install"));
    assert!(!stdout.to_ascii_lowercase().contains("graphify"));
    Ok(())
}

#[test]
fn installation_managed_commands_have_compass_native_help() -> Result<(), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .arg("--help")
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("hook-check"));
    assert!(stdout.contains("hook-guard"));

    for command in ["hook-check", "hook-guard"] {
        let output = Command::new(env!("CARGO_BIN_EXE_compass"))
            .args([command, "--help"])
            .output()?;
        assert!(output.status.success(), "{command} --help failed");
        let help = String::from_utf8(output.stdout)?;
        assert!(
            help.contains(&format!("compass {command}")),
            "{command} has no dedicated Compass help: {help}"
        );
        assert!(
            !help.to_ascii_lowercase().contains("graphify"),
            "{command} help contains retired branding: {help}"
        );
    }
    Ok(())
}

#[test]
fn inference_level_defaults_to_low_and_controls_published_graph_breadth()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("source");
    std::fs::create_dir(&source)?;
    std::fs::write(
        source.join("sample.rs"),
        r#"use external_crate::ExternalType;

fn run(value: ExternalType) {
    value.execute();
    external_crate::Service::call();
}
"#,
    )?;

    let mut counts = Vec::new();
    let mut level_graphs = Vec::new();
    for level in ["low", "medium", "high", "max"] {
        let destination = directory.path().join(format!("{level}-artifacts"));
        let run = |force: bool| -> Result<Vec<u8>, Box<dyn Error>> {
            let mut command = Command::new(env!("CARGO_BIN_EXE_compass"));
            command.args([
                "update",
                ".",
                "--code-only",
                "--no-viz",
                "--no-cluster",
                "--store",
                "json",
                "--inference-level",
                level,
                "--out",
            ]);
            command.arg(&destination);
            if force {
                command.arg("--force");
            }
            let output = command
                .current_dir(&source)
                .env_remove("COMPASS_OUT")
                .output()?;
            assert!(
                output.status.success(),
                "compass update --inference-level {level} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let graph_path =
                BuildGuard::resolve_artifact(&destination.join("compass-out"), "graph.json")?;
            Ok(std::fs::read(graph_path)?)
        };
        let graph_bytes = run(false)?;
        let rebuilt_graph_bytes = run(true)?;
        assert!(
            graph_bytes == rebuilt_graph_bytes,
            "{level} inference graph changed across a forced rebuild"
        );

        let graph: Value = serde_json::from_slice(&graph_bytes)?;
        let nodes = graph["nodes"].as_array().ok_or("nodes")?.len();
        let links = graph["links"].as_array().ok_or("links")?;
        let inferred = links
            .iter()
            .filter(|link| {
                link["evidence"].as_array().is_some_and(|evidence| {
                    evidence.iter().any(|item| item["confidence"] == "inferred")
                })
            })
            .count();
        counts.push((nodes, links.len(), inferred));
        level_graphs.push(graph_bytes);
    }

    let default_destination = directory.path().join("default-artifacts");
    let default = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "update",
            ".",
            "--code-only",
            "--no-viz",
            "--no-cluster",
            "--store",
            "json",
            "--out",
        ])
        .arg(&default_destination)
        .current_dir(&source)
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        default.status.success(),
        "compass update with the default inference level failed: {}",
        String::from_utf8_lossy(&default.stderr)
    );
    let default_graph_path =
        BuildGuard::resolve_artifact(&default_destination.join("compass-out"), "graph.json")?;
    let default_graph = std::fs::read(default_graph_path)?;
    let low_graph = level_graphs.first().ok_or("missing low inference graph")?;
    assert_eq!(default_graph.as_slice(), low_graph.as_slice());

    let low = counts[0];
    let max = counts[3];
    assert_eq!(low.2, 0);
    assert!(max.2 > 0);
    assert!(low.0 < max.0);
    assert!(low.1 < max.1);
    assert!(counts.windows(2).all(|levels| {
        levels[0].0 <= levels[1].0 && levels[0].1 <= levels[1].1 && levels[0].2 <= levels[1].2
    }));

    let invalid = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["update", ".", "--inference-level", "automatic"])
        .current_dir(&source)
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("low, medium, high, or max"));
    Ok(())
}
