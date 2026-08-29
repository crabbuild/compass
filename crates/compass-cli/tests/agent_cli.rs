use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

#[test]
fn agent_namespace_help_inventory_and_errors_are_public() -> Result<(), Box<dyn Error>> {
    for child in [
        "list",
        "install",
        "doctor",
        "export",
        "validate",
        "mcp-config",
    ] {
        let output = run(Path::new("."), &["agent", child, "--help"])?;
        assert!(output.status.success(), "help failed for {child}");
        assert!(String::from_utf8(output.stdout)?.contains("Usage:"));
    }

    let output = run(Path::new("."), &["agent", "list", "--format", "json"])?;
    assert!(output.status.success());
    let inventory = serde_json::from_slice::<Value>(&output.stdout)?;
    assert_eq!(inventory["schema"], "compass.agent-list/1");
    let ids = inventory["agents"]
        .as_array()
        .ok_or("agents must be an array")?
        .iter()
        .filter_map(|agent| agent["id"].as_str())
        .collect::<Vec<_>>();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted);

    let unknown = run(Path::new("."), &["agent", "not-a-command"])?;
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8(unknown.stderr)?.contains("unknown agent subcommand"));
    Ok(())
}

#[test]
fn mcp_config_renders_each_native_schema() -> Result<(), Box<dyn Error>> {
    let codex = run(
        Path::new("."),
        &[
            "agent",
            "mcp-config",
            "--platform",
            "codex",
            "--transport",
            "stdio",
        ],
    )?;
    assert!(codex.status.success());
    let codex = String::from_utf8(codex.stdout)?;
    assert!(codex.contains("[mcp_servers.compass]"));
    assert!(codex.contains("args = [\"serve\", \"--transport\", \"stdio\"]"));

    for (platform, pointer) in [
        ("claude", "/mcpServers/compass/url"),
        ("opencode", "/mcp/compass/url"),
        ("agents", "/mcpServers/compass/url"),
    ] {
        let output = run(
            Path::new("."),
            &[
                "agent",
                "mcp-config",
                "--platform",
                platform,
                "--transport",
                "http",
            ],
        )?;
        assert!(output.status.success(), "MCP config failed for {platform}");
        let value = serde_json::from_slice::<Value>(&output.stdout)?;
        assert_eq!(
            value.pointer(pointer).and_then(Value::as_str),
            Some("http://127.0.0.1:8080/mcp")
        );
    }
    Ok(())
}

#[test]
fn generic_agents_export_uses_the_doctor_configuration_path() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let bundle = directory.path().join("agents-bundle");
    let bundle_arg = bundle.to_string_lossy().into_owned();
    let export = run(
        directory.path(),
        &[
            "agent",
            "export",
            "--platform",
            "agents",
            "--out",
            &bundle_arg,
        ],
    )?;
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(bundle.join(".agents/mcp.json").is_file());
    assert!(!bundle.join("mcp/agents.json").exists());

    let validate = run(
        directory.path(),
        &[
            "agent",
            "validate",
            "--path",
            &bundle_arg,
            "--platform",
            "agents",
        ],
    )?;
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    Ok(())
}

#[test]
fn exported_bundle_round_trips_and_redacts_unsafe_content() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let bundle = directory.path().join("bundle");
    let bundle_arg = bundle.to_string_lossy().into_owned();
    let export = run(
        directory.path(),
        &[
            "agent",
            "export",
            "--platform",
            "claude",
            "--out",
            &bundle_arg,
            "--format",
            "json",
        ],
    )?;
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    let report = serde_json::from_slice::<Value>(&export.stdout)?;
    assert_eq!(report["schema"], "compass.agent-bundle/1");
    for skill in [
        "compass",
        "compass-architecture",
        "compass-change-impact",
        "compass-debug",
        "compass-index-maintenance",
        "compass-mcp-setup",
        "compass-navigate",
    ] {
        assert!(bundle.join("skills").join(skill).join("SKILL.md").is_file());
    }

    let valid = run(
        directory.path(),
        &[
            "agent",
            "validate",
            "--path",
            &bundle_arg,
            "--platform",
            "claude",
            "--format",
            "json",
        ],
    )?;
    assert!(valid.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&valid.stdout)?["schema"],
        "compass.agent-validation/1"
    );

    fs::write(
        bundle.join(".mcp.json"),
        r#"{"mcpServers":{"compass":{"command":"compass","args":["serve","--transport","stdio"],"api_key":"never-print-this"}}}"#,
    )?;
    let invalid = run(
        directory.path(),
        &[
            "agent",
            "validate",
            "--path",
            &bundle_arg,
            "--format",
            "json",
        ],
    )?;
    assert_eq!(invalid.status.code(), Some(1));
    let rendered = String::from_utf8(invalid.stdout)?;
    assert!(rendered.contains("literal credential"));
    assert!(!rendered.contains("never-print-this"));

    let protected = directory.path().join("protected");
    fs::create_dir(&protected)?;
    fs::write(protected.join("user.txt"), "keep")?;
    let protected_arg = protected.to_string_lossy().into_owned();
    let rejected = run(
        directory.path(),
        &[
            "agent",
            "export",
            "--platform",
            "claude",
            "--out",
            &protected_arg,
        ],
    )?;
    assert_eq!(rejected.status.code(), Some(1));
    assert_eq!(fs::read_to_string(protected.join("user.txt"))?, "keep");
    Ok(())
}

#[test]
fn public_validation_rejects_transport_manifest_and_managed_scope_mismatches()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let bundle = directory.path().join("agents-bundle");
    let bundle_arg = bundle.to_string_lossy().into_owned();
    let export = run(
        directory.path(),
        &[
            "agent",
            "export",
            "--platform",
            "agents",
            "--transport",
            "stdio",
            "--out",
            &bundle_arg,
        ],
    )?;
    assert!(export.status.success());

    let manifest_path = bundle.join("manifest.json");
    let mut manifest = serde_json::from_slice::<Value>(&fs::read(&manifest_path)?)?;
    manifest["transport"] = Value::String("http".to_owned());
    manifest["files"]["../escape"] = Value::String("00".repeat(32));
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    let invalid = run(
        directory.path(),
        &[
            "agent",
            "validate",
            "--path",
            &bundle_arg,
            "--format",
            "json",
        ],
    )?;
    assert_eq!(invalid.status.code(), Some(1));
    let report = String::from_utf8(invalid.stdout)?;
    assert!(report.contains("loopback endpoint"));
    assert!(report.contains("unsafe manifest path '../escape'"));

    let project = directory.path().join("managed-project");
    fs::create_dir_all(project.join(".git"))?;
    let install = run(
        &project,
        &["agent", "install", "--platform", "codex", "--project"],
    )?;
    assert!(install.status.success());
    let managed = project.join(".agents/skills/compass");
    let managed_arg = managed.to_string_lossy().into_owned();
    let scoped = run(
        &project,
        &[
            "agent",
            "validate",
            "--path",
            &managed_arg,
            "--platform",
            "codex",
            "--format",
            "json",
        ],
    )?;
    assert_eq!(scoped.status.code(), Some(1));
    assert!(String::from_utf8(scoped.stdout)?.contains("applies only to exported bundles"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn public_validation_rejects_a_symlinked_bundle_file() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let bundle = directory.path().join("agents-bundle");
    let bundle_arg = bundle.to_string_lossy().into_owned();
    let export = run(
        directory.path(),
        &[
            "agent",
            "export",
            "--platform",
            "agents",
            "--out",
            &bundle_arg,
        ],
    )?;
    assert!(export.status.success());

    let outside = directory.path().join("outside.json");
    fs::write(&outside, b"{}")?;
    let config = bundle.join(".agents/mcp.json");
    fs::remove_file(&config)?;
    symlink(&outside, &config)?;
    let invalid = run(
        directory.path(),
        &[
            "agent",
            "validate",
            "--path",
            &bundle_arg,
            "--format",
            "json",
        ],
    )?;
    assert_eq!(invalid.status.code(), Some(1));
    assert!(String::from_utf8(invalid.stdout)?.contains("symbolic links are not allowed"));
    Ok(())
}

#[test]
fn doctor_reports_healthy_stale_and_corrupt_states() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let project = directory.path().join("project");
    fs::create_dir_all(project.join(".git"))?;
    fs::create_dir_all(project.join("src"))?;
    fs::write(project.join("src/main.rs"), "fn main() {}")?;

    let install = run(
        &project,
        &["agent", "install", "--platform", "codex", "--project"],
    )?;
    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    fs::create_dir_all(project.join(".codex"))?;
    let config = run(
        &project,
        &[
            "agent",
            "mcp-config",
            "--platform",
            "codex",
            "--transport",
            "stdio",
        ],
    )?;
    assert!(config.status.success());
    fs::write(project.join(".codex/config.toml"), config.stdout)?;

    let update = run_with_output(
        &project,
        &["update", ".", "--no-viz", "--no-cluster"],
        "custom-agent-out",
    )?;
    assert!(
        update.status.success(),
        "{}",
        String::from_utf8_lossy(&update.stderr)
    );
    let project_arg = project.to_string_lossy().into_owned();
    let healthy = run_with_output(
        &project,
        &[
            "agent",
            "doctor",
            "--platform",
            "codex",
            "--project-root",
            &project_arg,
            "--format",
            "json",
        ],
        "custom-agent-out",
    )?;
    assert!(
        healthy.status.success(),
        "{}",
        String::from_utf8_lossy(&healthy.stdout)
    );
    let report = serde_json::from_slice::<Value>(&healthy.stdout)?;
    assert_eq!(report["schema"], "compass.agent-doctor/1");
    assert_eq!(report["healthy"], true);

    fs::write(
        project.join("src/main.rs"),
        "fn main() { println!(\"stale\"); }",
    )?;
    let stale = run_with_output(
        &project,
        &[
            "agent",
            "doctor",
            "--platform",
            "codex",
            "--project-root",
            &project_arg,
            "--format",
            "json",
        ],
        "custom-agent-out",
    )?;
    assert_eq!(stale.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&stale.stdout).contains("graph manifest is stale"));

    fs::write(project.join(".agents/skills/compass/SKILL.md"), "corrupt")?;
    let corrupt = run_with_output(
        &project,
        &[
            "agent",
            "doctor",
            "--platform",
            "codex",
            "--project-root",
            &project_arg,
            "--format",
            "json",
        ],
        "custom-agent-out",
    )?;
    assert_eq!(corrupt.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&corrupt.stdout).contains("modified"));
    Ok(())
}

fn run(current: &Path, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(arguments)
        .current_dir(current)
        .env_remove("COMPASS_OUT")
        .env_remove("COMPASS_API_KEY")
        .output()?)
}

fn run_with_output(
    current: &Path,
    arguments: &[&str],
    output_name: &str,
) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(arguments)
        .current_dir(current)
        .env("COMPASS_OUT", output_name)
        .env_remove("COMPASS_API_KEY")
        .output()?)
}
