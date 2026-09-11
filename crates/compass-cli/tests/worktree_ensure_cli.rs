use std::error::Error;
use std::path::Path;
use std::process::{Command, Output};

fn git(root: &Path, arguments: &[&str]) -> Result<(), Box<dyn Error>> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }
}

fn ensure(root: &Path) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "ensure",
            "--code-only",
            "--no-cluster",
            "--no-viz",
            "--store",
            "json",
        ])
        .current_dir(root)
        .env_remove("COMPASS_OUT")
        .output()?)
}

#[test]
fn ensure_initializes_updates_and_reuses_a_linked_worktree_graph() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let repository = directory.path().join("repository");
    let linked = directory.path().join("agent-worktree");
    std::fs::create_dir(&repository)?;
    git(&repository, &["init", "--quiet"])?;
    git(&repository, &["config", "user.name", "Compass Test"])?;
    git(
        &repository,
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(repository.join("sample.rs"), "pub fn original() {}\n")?;
    git(&repository, &["add", "sample.rs"])?;
    git(&repository, &["commit", "--quiet", "-m", "initial"])?;
    git(
        &repository,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            linked.to_str().ok_or("linked worktree path is not UTF-8")?,
            "HEAD",
        ],
    )?;

    let nested = linked.join("nested");
    std::fs::create_dir(&nested)?;
    let initialized = ensure(&nested)?;
    assert!(
        initialized.status.success(),
        "ensure failed: {}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    assert!(String::from_utf8(initialized.stdout)?.contains("Compass graph initialized."));
    assert!(linked.join("compass-out/graph.json").is_file());
    assert!(!repository.join("compass-out").exists());

    let current = ensure(&nested)?;
    assert!(
        current.status.success(),
        "ensure failed: {}",
        String::from_utf8_lossy(&current.stderr)
    );
    assert!(String::from_utf8(current.stdout)?.contains("Compass graph current."));

    std::fs::write(
        linked.join("sample.rs"),
        "pub fn original() {}\npub fn changed_in_worktree() {}\n",
    )?;
    let updated = ensure(&nested)?;
    assert!(
        updated.status.success(),
        "ensure failed: {}",
        String::from_utf8_lossy(&updated.stderr)
    );
    assert!(String::from_utf8(updated.stdout)?.contains("Compass graph updated."));
    assert!(!repository.join("compass-out").exists());
    Ok(())
}
