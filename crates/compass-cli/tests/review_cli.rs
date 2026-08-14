use std::path::Path;
use std::process::{Command, Output};

use compass_history::{
    ExtractionFingerprint, HistoryConfig, HistoryStore, PublishRequest, Repository,
};
use compass_pr_intelligence::{GateState, MergeOutcome, PullRequestReport, RiskBand};

const SYNTHETIC_ENGINE_IDENTITY: &str = "historical-engine";

fn git(root: &Path, arguments: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned().into())
    }
}

fn run(root: &Path, arguments: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    Ok(Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(arguments)
        .current_dir(root)
        .output()?)
}

fn initialize(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    git(root, &["init", "--quiet", "--initial-branch=main"])?;
    git(root, &["config", "user.name", "Compass Test"])?;
    git(root, &["config", "user.email", "compass@example.invalid"])?;
    std::fs::write(root.join("lib.rs"), "pub fn shared() -> u8 { 1 }\n")?;
    git(root, &["add", "lib.rs"])?;
    git(root, &["commit", "--quiet", "-m", "base"])?;
    Ok(())
}

fn publish_historical_base(root: &Path, commit: &str) -> Result<(), Box<dyn std::error::Error>> {
    let seeded = run(root, &["history", "build", commit, "--code-only"])?;
    if !seeded.status.success() {
        return Err(format!(
            "could not seed current history: {}",
            String::from_utf8_lossy(&seeded.stderr)
        )
        .into());
    }
    let repository = Repository::discover(root)?;
    let commit = repository.resolve(commit)?;
    let history = HistoryStore::open_existing(&repository)?.ok_or("seeded history store")?;
    let current = history.preferred(&commit)?.ok_or("seeded realization")?;
    let completed = history.artifacts(&current.id)?;
    let mut historical_profile = current.version.build_profile;
    historical_profile.insert("compass_version", SYNTHETIC_ENGINE_IDENTITY)?;
    history.publish(PublishRequest {
        commit: commit.clone(),
        parents: repository.parents(&commit)?,
        profile: historical_profile,
        fingerprint: "a".repeat(64).parse::<ExtractionFingerprint>()?,
        artifacts: completed.artifacts,
        completion: completed.completion,
        make_preferred: true,
    })?;
    Ok(())
}

fn persist_historical_repository_profile(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let enabled = run(root, &["history", "enable", "--code-only"])?;
    if !enabled.status.success() {
        return Err(format!(
            "could not enable history: {}",
            String::from_utf8_lossy(&enabled.stderr)
        )
        .into());
    }
    let repository = Repository::discover(root)?;
    let mut profile = HistoryConfig::load(&repository)?
        .profile
        .ok_or("enabled profile")?;
    profile.insert("compass_version", SYNTHETIC_ENGINE_IDENTITY)?;
    HistoryConfig::enable(&repository, profile)?;
    Ok(())
}

#[test]
fn local_review_writes_round_trippable_exact_report() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    initialize(directory.path())?;
    git(directory.path(), &["checkout", "--quiet", "-b", "feature"])?;
    std::fs::write(
        directory.path().join("feature.rs"),
        "pub fn feature() -> u8 { 2 }\n",
    )?;
    git(directory.path(), &["add", "feature.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "feature"])?;
    let head = git(directory.path(), &["rev-parse", "HEAD"])?;
    git(directory.path(), &["checkout", "--quiet", "main"])?;
    std::fs::write(
        directory.path().join("main.rs"),
        "pub fn target() -> u8 { 3 }\n",
    )?;
    git(directory.path(), &["add", "main.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "target"])?;
    let base = git(directory.path(), &["rev-parse", "HEAD"])?;
    let report_path = directory.path().join("review.json");

    let output = run(
        directory.path(),
        &[
            "review",
            "--base",
            &base,
            "--head",
            &head,
            "--repo",
            "crabbuild/fixture",
            "--pull-request-number",
            "42",
            "--format",
            "json",
            "--output",
            report_path.to_str().ok_or("report path")?,
        ],
    )?;
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("PR review written to"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("error:"));
    let report = PullRequestReport::from_json(&std::fs::read(report_path)?)?;
    assert_eq!(report.identity.pull_request_number, Some(42));
    assert_eq!(report.identity.repository.owner, "crabbuild");
    assert_eq!(report.identity.revisions.target_head, base);
    assert_eq!(report.identity.revisions.pull_request_head, head);
    assert!(report.identity.revisions.merge_result.is_clean());

    let preserved_path = directory.path().join("bounded-review.md");
    std::fs::write(&preserved_path, "preserve-me")?;
    let bounded = run(
        directory.path(),
        &[
            "review",
            "--base",
            &base,
            "--head",
            &head,
            "--format",
            "markdown",
            "--max-output-bytes",
            "1",
            "--output",
            preserved_path.to_str().ok_or("bounded report path")?,
        ],
    )?;
    assert_eq!(bounded.status.code(), Some(1));
    assert!(bounded.stdout.is_empty());
    assert!(String::from_utf8_lossy(&bounded.stderr).contains("PR review output is"));
    assert_eq!(std::fs::read_to_string(preserved_path)?, "preserve-me");
    Ok(())
}

#[test]
fn local_review_rebuilds_a_comparable_pair_from_a_noncurrent_profile()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    initialize(directory.path())?;
    let base = git(directory.path(), &["rev-parse", "HEAD"])?;
    git(directory.path(), &["checkout", "--quiet", "-b", "feature"])?;
    std::fs::write(
        directory.path().join("feature.rs"),
        "pub fn feature() -> u8 { 2 }\n",
    )?;
    git(directory.path(), &["add", "feature.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "feature"])?;
    let head = git(directory.path(), &["rev-parse", "HEAD"])?;
    publish_historical_base(directory.path(), &base)?;

    let output = run(
        directory.path(),
        &[
            "review", "--base", &base, "--head", &head, "--format", "json",
        ],
    )?;
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = PullRequestReport::from_json(&output.stdout)?;
    assert_eq!(report.identity.revisions.target_head, base);
    assert_eq!(report.identity.revisions.pull_request_head, head);

    let repository = Repository::discover(directory.path())?;
    let history = HistoryStore::open_existing(&repository)?.ok_or("history store")?;
    let base = repository.resolve(&base)?;
    let preferred = history.preferred(&base)?.ok_or("preferred base")?;
    assert_eq!(
        preferred.version.build_profile.value("compass_version"),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert!(history.list(Some(&base))?.iter().any(|realization| {
        realization.version.build_profile.value("compass_version")
            == Some(SYNTHETIC_ENGINE_IDENTITY)
    }));
    Ok(())
}

#[test]
fn local_review_rebuilds_a_persisted_profile_after_a_current_graph_build()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    initialize(directory.path())?;
    let base = git(directory.path(), &["rev-parse", "HEAD"])?;
    git(directory.path(), &["checkout", "--quiet", "-b", "feature"])?;
    std::fs::write(
        directory.path().join("feature.rs"),
        "pub fn feature() -> u8 { 2 }\n",
    )?;
    git(directory.path(), &["add", "feature.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "feature"])?;
    let head = git(directory.path(), &["rev-parse", "HEAD"])?;
    persist_historical_repository_profile(directory.path())?;

    let built = run(
        directory.path(),
        &["extract", ".", "--code-only", "--no-viz"],
    )?;
    assert!(
        built.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&built.stdout),
        String::from_utf8_lossy(&built.stderr)
    );
    let repository = Repository::discover(directory.path())?;
    assert_eq!(
        HistoryConfig::load(&repository)?
            .profile
            .and_then(|profile| profile.value("compass_version").map(str::to_owned))
            .as_deref(),
        Some(SYNTHETIC_ENGINE_IDENTITY)
    );

    let reviewed = run(
        directory.path(),
        &[
            "review", "--base", &base, "--head", &head, "--format", "json",
        ],
    )?;
    assert!(
        reviewed.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&reviewed.stdout),
        String::from_utf8_lossy(&reviewed.stderr)
    );
    let report = PullRequestReport::from_json(&reviewed.stdout)?;
    assert_eq!(report.identity.revisions.target_head, base);
    assert_eq!(report.identity.revisions.pull_request_head, head);
    let comparison = report
        .identity
        .revisions
        .merge_result
        .object_id()
        .unwrap_or(&report.identity.revisions.pull_request_head)
        .to_owned();

    let history = HistoryStore::open_existing(&repository)?.ok_or("history store")?;
    for revision in [base, comparison] {
        let commit = repository.resolve(&revision)?;
        let preferred = history.preferred(&commit)?.ok_or("preferred realization")?;
        assert_eq!(
            preferred.version.build_profile.value("compass_version"),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
    Ok(())
}

fn assert_review_reconciles_existing_realizations(
    noncurrent_head: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    initialize(directory.path())?;
    git(directory.path(), &["checkout", "--quiet", "-b", "feature"])?;
    std::fs::write(
        directory.path().join("lib.rs"),
        "pub fn shared() -> u8 { 2 }\n",
    )?;
    git(directory.path(), &["add", "lib.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "feature"])?;
    let head = git(directory.path(), &["rev-parse", "HEAD"])?;
    git(directory.path(), &["checkout", "--quiet", "main"])?;
    std::fs::write(
        directory.path().join("lib.rs"),
        "pub fn shared() -> u8 { 3 }\n",
    )?;
    git(directory.path(), &["add", "lib.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "target"])?;
    let base = git(directory.path(), &["rev-parse", "HEAD"])?;
    publish_historical_base(directory.path(), &base)?;
    if noncurrent_head {
        publish_historical_base(directory.path(), &head)?;
    } else {
        let built = run(
            directory.path(),
            &["history", "build", &head, "--code-only"],
        )?;
        assert!(
            built.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&built.stdout),
            String::from_utf8_lossy(&built.stderr)
        );
    }

    let reviewed = run(
        directory.path(),
        &[
            "review", "--base", &base, "--head", &head, "--format", "json",
        ],
    )?;
    assert!(
        reviewed.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&reviewed.stdout),
        String::from_utf8_lossy(&reviewed.stderr)
    );
    let report = PullRequestReport::from_json(&reviewed.stdout)?;
    assert!(matches!(
        report.identity.revisions.merge_result,
        MergeOutcome::Conflicted { .. }
    ));

    let repository = Repository::discover(directory.path())?;
    let history = HistoryStore::open_existing(&repository)?.ok_or("history store")?;
    for revision in [base, head] {
        let commit = repository.resolve(&revision)?;
        let preferred = history.preferred(&commit)?.ok_or("preferred realization")?;
        assert_eq!(
            preferred.version.build_profile.value("compass_version"),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
    Ok(())
}

#[test]
fn local_review_reconciles_existing_different_engine_profiles()
-> Result<(), Box<dyn std::error::Error>> {
    assert_review_reconciles_existing_realizations(false)
}

#[test]
fn local_review_hard_cuts_over_matching_noncurrent_engine_profiles()
-> Result<(), Box<dyn std::error::Error>> {
    assert_review_reconciles_existing_realizations(true)
}

#[test]
fn conflicted_review_is_unavailable_without_false_clean_gate()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    initialize(directory.path())?;
    git(directory.path(), &["checkout", "--quiet", "-b", "feature"])?;
    std::fs::write(
        directory.path().join("lib.rs"),
        "pub fn shared() -> u8 { 2 }\n",
    )?;
    git(directory.path(), &["add", "lib.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "feature"])?;
    let head = git(directory.path(), &["rev-parse", "HEAD"])?;
    git(directory.path(), &["checkout", "--quiet", "main"])?;
    std::fs::write(
        directory.path().join("lib.rs"),
        "pub fn shared() -> u8 { 3 }\n",
    )?;
    git(directory.path(), &["add", "lib.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "target"])?;
    let base = git(directory.path(), &["rev-parse", "HEAD"])?;

    let output = run(
        directory.path(),
        &[
            "review", "--base", &base, "--head", &head, "--format", "json",
        ],
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("error:"));
    let report = PullRequestReport::from_json(&output.stdout)?;
    assert_eq!(report.advisory_risk.band, RiskBand::Unavailable);
    assert!(
        report
            .gates
            .iter()
            .all(|gate| gate.state != GateState::Fail)
    );
    assert!(
        report
            .gates
            .iter()
            .any(|gate| gate.state == GateState::Indeterminate)
    );
    Ok(())
}

#[test]
fn review_usage_errors_have_stable_exit_and_streams() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    initialize(directory.path())?;
    let output = run(directory.path(), &["review", "--unknown"])?;
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown option --unknown"));
    Ok(())
}
