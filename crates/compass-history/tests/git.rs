use std::path::Path;
use std::process::Command;

use compass_history::Repository;

fn git(directory: &Path, arguments: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned().into())
    }
}

#[test]
fn resolve_parents_unknown_revisions_and_linked_worktrees() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("one"), "one")?;
    git(directory.path(), &["add", "one"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "one"])?;
    let first = git(directory.path(), &["rev-parse", "HEAD"])?;
    std::fs::write(directory.path().join("two"), "two")?;
    git(directory.path(), &["add", "two"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "two"])?;

    let repository = Repository::discover(directory.path())?;
    let head = repository.resolve("HEAD")?;
    assert_eq!(head.to_string().len(), 40);
    assert_eq!(repository.parents(&head)?[0].to_string(), first);
    assert!(repository.resolve("does-not-exist").is_err());
    assert!(repository.resolve("--help").is_err());

    let worktree = directory.path().join("linked");
    git(
        directory.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            worktree.to_str().ok_or("path")?,
            "HEAD",
        ],
    )?;
    let linked = Repository::discover(&worktree)?;
    assert_eq!(linked.common_dir(), repository.common_dir());
    assert_eq!(linked.resolve("HEAD")?, head);
    Ok(())
}

#[test]
fn sha256_repository_ids_are_accepted_when_git_supports_them()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    if git(
        directory.path(),
        &["init", "--quiet", "--object-format=sha256"],
    )
    .is_err()
    {
        return Ok(());
    }
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("one"), "one")?;
    git(directory.path(), &["add", "one"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "one"])?;
    assert_eq!(
        Repository::discover(directory.path())?
            .resolve("HEAD")?
            .as_str()
            .len(),
        64
    );
    Ok(())
}
