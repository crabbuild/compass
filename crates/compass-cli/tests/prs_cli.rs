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
