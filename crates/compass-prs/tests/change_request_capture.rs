use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use compass_pr_intelligence::{MergeOutcome, RepositoryIdentity};
use compass_prs::{
    ChangeRequestSource, GithubChangeRequestSource, LocalGitChangeRequestSource, ProcessOutput,
    ProcessRunner, PrsError, SystemRunner, detect_repository_identity,
};

fn git(repository: &Path, arguments: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(arguments)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git {:?} failed: {}",
            arguments,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn commit_file(
    repository: &Path,
    path: &str,
    contents: &str,
    message: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    fs::write(repository.join(path), contents)?;
    git(repository, &["add", "--", path])?;
    git(repository, &["commit", "-m", message])?;
    git(repository, &["rev-parse", "HEAD"])
}

struct RepositoryFixture {
    directory: tempfile::TempDir,
    base: String,
    target: String,
    head: String,
}

fn repository_fixture(conflict: bool) -> Result<RepositoryFixture, Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    let base = commit_file(directory.path(), "api.txt", "base\n", "base")?;
    git(directory.path(), &["checkout", "-b", "feature"])?;
    let head = commit_file(
        directory.path(),
        "api.txt",
        if conflict {
            "feature\n"
        } else {
            "base\nfeature\n"
        },
        "feature",
    )?;
    git(directory.path(), &["checkout", "-B", "target", &base])?;
    let target = if conflict {
        commit_file(directory.path(), "api.txt", "target\n", "target")?
    } else {
        commit_file(directory.path(), "target.txt", "target\n", "target")?
    };
    Ok(RepositoryFixture {
        directory,
        base,
        target,
        head,
    })
}

fn identity() -> RepositoryIdentity {
    RepositoryIdentity {
        forge: "git".to_owned(),
        host: "local".to_owned(),
        owner: "fixture".to_owned(),
        name: "repository".to_owned(),
    }
}

#[test]
fn local_capture_freezes_four_revisions_and_hunks_without_checkout()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = repository_fixture(false)?;
    let before = git(fixture.directory.path(), &["rev-parse", "HEAD"])?;
    let source = LocalGitChangeRequestSource::new(
        &SystemRunner,
        fixture.directory.path(),
        identity(),
        &fixture.target,
        &fixture.head,
    );
    let request = source.capture()?;
    assert_eq!(request.revisions.merge_base, fixture.base);
    assert_eq!(request.revisions.target_head, fixture.target);
    assert_eq!(request.revisions.pull_request_head, fixture.head);
    let MergeOutcome::Clean { object_id } = request.revisions.merge_result else {
        return Err("expected a clean synthetic merge".into());
    };
    assert_eq!(object_id.len(), 40);
    assert!(!request.hunks.is_empty());
    assert_eq!(request.hunks[0].new_path, "api.txt");
    assert_eq!(
        git(fixture.directory.path(), &["rev-parse", "HEAD"])?,
        before
    );
    Ok(())
}

#[test]
fn local_capture_reports_conflicts_and_missing_objects_explicitly()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = repository_fixture(true)?;
    let request = LocalGitChangeRequestSource::new(
        &SystemRunner,
        fixture.directory.path(),
        identity(),
        &fixture.target,
        &fixture.head,
    )
    .capture()?;
    assert!(matches!(
        request.revisions.merge_result,
        MergeOutcome::Conflicted { .. }
    ));
    let missing = LocalGitChangeRequestSource::new(
        &SystemRunner,
        fixture.directory.path(),
        identity(),
        "does-not-exist",
        &fixture.head,
    )
    .capture();
    assert!(missing.is_err());
    Ok(())
}

#[test]
fn local_capture_keeps_rename_binary_and_utf8_file_records()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    fs::write(directory.path().join("old.txt"), "same\n")?;
    fs::write(directory.path().join("binary.bin"), [0_u8, 1, 2])?;
    fs::write(directory.path().join("café.txt"), "old\n")?;
    git(directory.path(), &["add", "."])?;
    git(directory.path(), &["commit", "--quiet", "-m", "base"])?;
    let base = git(directory.path(), &["rev-parse", "HEAD"])?;
    git(directory.path(), &["mv", "old.txt", "new.txt"])?;
    fs::write(directory.path().join("binary.bin"), [0_u8, 9, 2])?;
    fs::write(directory.path().join("café.txt"), "new\n")?;
    git(directory.path(), &["add", "."])?;
    git(directory.path(), &["commit", "--quiet", "-m", "changes"])?;
    let head = git(directory.path(), &["rev-parse", "HEAD"])?;

    let request =
        LocalGitChangeRequestSource::new(&SystemRunner, directory.path(), identity(), &base, &head)
            .capture()?;
    assert!(request.hunks.iter().any(|hunk| {
        hunk.status == "renamed" && hunk.old_path == "old.txt" && hunk.new_path == "new.txt"
    }));
    assert!(
        request
            .hunks
            .iter()
            .any(|hunk| { hunk.old_path == "binary.bin" && hunk.new_path == "binary.bin" })
    );
    assert!(request.hunks.iter().any(|hunk| hunk.new_path == "café.txt"));
    Ok(())
}

struct FailingRunner(PrsError);

impl ProcessRunner for FailingRunner {
    fn run(
        &self,
        _program: &str,
        _arguments: &[String],
        _timeout: Duration,
    ) -> Result<ProcessOutput, PrsError> {
        match &self.0 {
            PrsError::Timeout { program, seconds } => Err(PrsError::Timeout {
                program: program.clone(),
                seconds: *seconds,
            }),
            PrsError::OutputTooLarge { program, limit } => Err(PrsError::OutputTooLarge {
                program: program.clone(),
                limit: *limit,
            }),
            _ => Err(PrsError::InvalidRecord(
                "unsupported fixture error".to_owned(),
            )),
        }
    }
}

#[test]
fn capture_propagates_process_timeout_and_output_limits() {
    for runner in [
        FailingRunner(PrsError::Timeout {
            program: "git".to_owned(),
            seconds: 30,
        }),
        FailingRunner(PrsError::OutputTooLarge {
            program: "git".to_owned(),
            limit: 16 * 1024 * 1024,
        }),
    ] {
        let result =
            LocalGitChangeRequestSource::new(&runner, ".", identity(), "base", "head").capture();
        assert!(matches!(
            result,
            Err(PrsError::Timeout { .. } | PrsError::OutputTooLarge { .. })
        ));
    }
}

struct HybridRunner {
    gh: Mutex<VecDeque<ProcessOutput>>,
}

impl HybridRunner {
    fn new(outputs: impl IntoIterator<Item = ProcessOutput>) -> Self {
        Self {
            gh: Mutex::new(outputs.into_iter().collect()),
        }
    }
}

impl ProcessRunner for HybridRunner {
    fn run(
        &self,
        program: &str,
        arguments: &[String],
        timeout: Duration,
    ) -> Result<ProcessOutput, PrsError> {
        if program == "gh" {
            return self
                .gh
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front()
                .ok_or_else(|| PrsError::InvalidRecord("unexpected gh invocation".to_owned()));
        }
        SystemRunner.run(program, arguments, timeout)
    }

    fn run_with_input(
        &self,
        program: &str,
        arguments: &[String],
        input: &[u8],
        timeout: Duration,
    ) -> Result<ProcessOutput, PrsError> {
        SystemRunner.run_with_input(program, arguments, input, timeout)
    }
}

fn successful(stdout: String) -> ProcessOutput {
    ProcessOutput {
        code: 0,
        stdout,
        stderr: String::new(),
    }
}

fn metadata(base: &str, head: &str) -> ProcessOutput {
    successful(format!(
        r#"{{"base":{{"sha":"{base}"}},"head":{{"sha":"{head}"}}}}"#
    ))
}

#[test]
fn github_capture_validates_paginated_files_and_detects_force_push_drift()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = repository_fixture(false)?;
    let runner = HybridRunner::new([
        metadata(&fixture.target, &fixture.head),
        successful(r#"[[{"filename":"api.txt"}],[]]"#.to_owned()),
        metadata(&fixture.target, &fixture.head),
    ]);
    let request = GithubChangeRequestSource::new(
        &runner,
        fixture.directory.path(),
        "github.example.com",
        "owner",
        "repo",
        42,
    )
    .capture()?;
    assert_eq!(request.pull_request_number, Some(42));
    assert_eq!(request.repository.forge, "github");

    let drift = HybridRunner::new([
        metadata(&fixture.target, &fixture.head),
        successful(r#"[[{"filename":"api.txt"}]]"#.to_owned()),
        metadata(&fixture.target, &fixture.base),
    ]);
    let result = GithubChangeRequestSource::new(
        &drift,
        fixture.directory.path(),
        "github.com",
        "owner",
        "repo",
        42,
    )
    .capture();
    assert!(matches!(result, Err(PrsError::RevisionDrift(_))));
    Ok(())
}

#[test]
fn repository_identity_uses_remote_or_bounded_local_fallback()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = repository_fixture(false)?;
    let local = detect_repository_identity(&SystemRunner, fixture.directory.path())?;
    assert_eq!(local.forge, "git");
    assert_eq!(local.host, "local");
    git(
        fixture.directory.path(),
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:crabbuild/compass.git",
        ],
    )?;
    let github = detect_repository_identity(&SystemRunner, fixture.directory.path())?;
    assert_eq!(github.forge, "github");
    assert_eq!(github.owner, "crabbuild");
    assert_eq!(github.name, "compass");
    Ok(())
}
