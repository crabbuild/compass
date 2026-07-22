use std::path::Path;
use std::process::Command;

use compass_history::{CommitId, GitTargetLimitation, HistoryError, Repository};

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
    let root_repository = Repository::discover(directory.path())?;
    assert!(root_repository.parents(&first.parse()?)?.is_empty());
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

#[test]
fn detached_worktree_is_exact_offline_reports_limitations_and_cleans_up()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("old.rs"), "fn old() {}\n")?;
    std::fs::write(
        directory.path().join("asset.bin"),
        "version https://git-lfs.github.com/spec/v1\noid sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nsize 10\n",
    )?;
    git(directory.path(), &["add", "old.rs", "asset.bin"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "old"])?;
    let repository = Repository::discover(directory.path())?;
    let first = repository.resolve("HEAD")?;
    git(
        directory.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},vendor/sub", first.as_str()),
        ],
    )?;
    git(directory.path(), &["commit", "--quiet", "-m", "gitlink"])?;
    let second = repository.resolve("HEAD")?;
    std::fs::remove_file(directory.path().join("old.rs"))?;
    git(directory.path(), &["add", "-u"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "new"])?;
    assert_eq!(
        repository.first_parent_ancestors(&repository.resolve("HEAD")?)?[0],
        second
    );

    let checkout_path;
    {
        let checkout = repository.detached_worktree(&second)?;
        checkout_path = checkout.path().to_path_buf();
        assert_eq!(repository.resolve_at(checkout.path(), "HEAD")?, second);
        assert!(checkout.path().join("old.rs").is_file());
        assert_eq!(
            std::fs::read_to_string(checkout.path().join("asset.bin"))?
                .lines()
                .next(),
            Some("version https://git-lfs.github.com/spec/v1")
        );
        assert!(
            checkout
                .limitations()
                .contains(&GitTargetLimitation::LfsPointer("asset.bin".to_owned()))
        );
        assert!(
            checkout
                .limitations()
                .contains(&GitTargetLimitation::Gitlink("vendor/sub".to_owned()))
        );
        checkout.close()?;
    }
    assert!(!checkout_path.exists());

    git(
        directory.path(),
        &["config", "filter.unsafe.smudge", "external-smudge %f"],
    )?;
    assert!(matches!(
        repository.detached_worktree(&first),
        Err(HistoryError::UnsupportedGitFilter(_))
    ));
    git(
        directory.path(),
        &["config", "filter.unsafe.smudge", "evil-git-lfs-wrapper %f"],
    )?;
    assert!(matches!(
        repository.detached_worktree(&first),
        Err(HistoryError::UnsupportedGitFilter(_))
    ));
    Ok(())
}

#[test]
fn detached_worktree_fails_for_a_missing_object_without_fetching()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("tracked.rs"), "fn tracked() {}\n")?;
    git(directory.path(), &["add", "tracked.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "tracked"])?;
    git(
        directory.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/must-not-fetch",
        ],
    )?;
    let repository = Repository::discover(directory.path())?;
    let missing = "0000000000000000000000000000000000000000".parse::<CommitId>()?;
    let fetch_head = repository.common_dir().join("FETCH_HEAD");
    assert!(!fetch_head.exists());
    assert!(matches!(
        repository.detached_worktree(&missing),
        Err(HistoryError::Git(_))
    ));
    assert!(!fetch_head.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn detached_worktree_disables_hooks_and_refuses_symlink_cleanup_escape()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("tracked.rs"), "fn tracked() {}\n")?;
    git(directory.path(), &["add", "tracked.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "tracked"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;

    let hooks = directory.path().join("hooks");
    std::fs::create_dir(&hooks)?;
    let hook_marker = directory.path().join("hook-ran");
    let hook = hooks.join("post-checkout");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\nprintf ran > '{}'\n", hook_marker.display()),
    )?;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700))?;
    git(
        directory.path(),
        &["config", "core.hooksPath", hooks.to_str().ok_or("hooks")?],
    )?;
    let checkout = repository.detached_worktree(&commit)?;
    assert!(!hook_marker.exists());

    let outside = tempfile::tempdir()?;
    let marker = outside.path().join("must-survive");
    std::fs::write(&marker, "safe")?;
    std::fs::remove_dir_all(checkout.path())?;
    symlink(outside.path(), checkout.path())?;
    drop(checkout);
    assert_eq!(std::fs::read_to_string(marker)?, "safe");
    Ok(())
}
