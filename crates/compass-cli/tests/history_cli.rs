use std::path::Path;
use std::process::{Command, Output};

fn git(directory: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned().into())
    }
}

fn run(
    binary: &str,
    directory: &Path,
    arguments: &[&str],
) -> Result<Output, Box<dyn std::error::Error>> {
    Ok(Command::new(binary)
        .args(arguments)
        .current_dir(directory)
        .output()?)
}

#[test]
fn history_help_and_empty_status_are_actionable_and_non_mutating()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("README.md"), "fixture\n")?;
    git(directory.path(), &["add", "README.md"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;

    let compass = env!("CARGO_BIN_EXE_compass");
    let graphify = env!("CARGO_BIN_EXE_graphify");
    let help = run(compass, directory.path(), &["history", "--help"])?;
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("build REV"));
    let status = run(compass, directory.path(), &["history", "status", "HEAD"])?;
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("no store"));
    assert!(!directory.path().join(".git/compass").exists());
    let alias = run(graphify, directory.path(), &["history", "status", "HEAD"])?;
    assert_eq!(status.status.code(), alias.status.code());
    assert_eq!(status.stdout, alias.stdout);
    assert_eq!(status.stderr, alias.stderr);
    Ok(())
}
