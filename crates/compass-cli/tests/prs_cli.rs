#![cfg(unix)]

use std::error::Error;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn write_executable(path: &Path, content: &str) -> Result<(), Box<dyn Error>> {
    std::fs::write(path, content)?;
    let mut permissions = path.metadata()?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[test]
fn prs_help_and_dashboard_use_compass_identity() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let bin = directory.path().join("bin");
    std::fs::create_dir_all(&bin)?;
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '%s\n' '[{"number":101,"title":"Add authentication flow","headRefName":"feature-auth","baseRefName":"main","author":{"login":"alice"},"isDraft":false,"reviewDecision":"","statusCheckRollup":[{"conclusion":"SUCCESS","status":"COMPLETED"}],"updatedAt":"2026-07-19T08:00:00Z"}]'
  exit 0
fi
if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
  printf '%s\n' '{"defaultBranchRef":{"name":"main"}}'
  exit 0
fi
exit 1
"#,
    )?;

    let help = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["prs", "--help"])
        .output()?;
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("compass prs"));

    let path = std::env::join_paths([bin, PathBuf::from("/usr/bin"), PathBuf::from("/bin")])?;
    let dashboard = Command::new(env!("CARGO_BIN_EXE_compass"))
        .current_dir(directory.path())
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .args(["prs", "--base", "main"])
        .output()?;
    assert!(
        dashboard.status.success(),
        "{}",
        String::from_utf8_lossy(&dashboard.stderr)
    );
    let stdout = String::from_utf8_lossy(&dashboard.stdout);
    assert!(stdout.contains("compass prs  ·  base: main"));
    assert!(stdout.contains("#101"));
    Ok(())
}

#[test]
fn prs_impact_reads_only_the_current_snapshot_when_public_graph_is_absent()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let bin = directory.path().join("bin");
    std::fs::create_dir_all(&bin)?;
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '%s\n' '[{"number":101,"title":"Move handler","headRefName":"feature-handler","baseRefName":"main","author":{"login":"alice"},"isDraft":false,"reviewDecision":"","statusCheckRollup":[{"conclusion":"SUCCESS","status":"COMPLETED"}],"updatedAt":"2026-07-19T08:00:00Z"}]'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "diff" ]; then
  printf '%s\n' 'src/lib.rs'
  exit 0
fi
exit 1
"#,
    )?;
    let output = directory.path().join("compass-out");
    let snapshot = output.join("snapshots").join("snapshot-current");
    std::fs::create_dir_all(&snapshot)?;
    std::fs::write(
        snapshot.join("graph.json"),
        serde_json::to_vec(&serde_json::json!({
            "directed":false,
            "multigraph":false,
            "graph":{},
            "nodes":[{
                "id":"handler",
                "label":"Handler",
                "source_file":"src/lib.rs",
                "community":7
            }],
            "links":[]
        }))?,
    )?;
    std::fs::write(output.join("current-snapshot"), "snapshot-current")?;
    let public = output.join("graph.json");
    assert!(!public.exists());
    let public_arg = public.to_string_lossy().into_owned();

    let path = std::env::join_paths([bin, PathBuf::from("/usr/bin"), PathBuf::from("/bin")])?;
    let detail = Command::new(env!("CARGO_BIN_EXE_compass"))
        .current_dir(directory.path())
        .env("PATH", &path)
        .env("NO_COLOR", "1")
        .args(["prs", "101", "--base", "main", "--graph", &public_arg])
        .output()?;
    assert!(
        detail.status.success(),
        "{}",
        String::from_utf8_lossy(&detail.stderr)
    );
    assert!(
        String::from_utf8_lossy(&detail.stdout).contains("1 node / 1 community"),
        "{}",
        String::from_utf8_lossy(&detail.stdout)
    );

    std::fs::write(output.join("current-snapshot"), "../invalid-snapshot")?;
    let malformed = Command::new(env!("CARGO_BIN_EXE_compass"))
        .current_dir(directory.path())
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .args(["prs", "101", "--base", "main", "--graph", &public_arg])
        .output()?;
    assert!(malformed.status.success());
    assert!(
        !String::from_utf8_lossy(&malformed.stdout).contains("Graph impact:"),
        "{}",
        String::from_utf8_lossy(&malformed.stdout)
    );
    Ok(())
}
