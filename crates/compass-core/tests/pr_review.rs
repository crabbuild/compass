use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use compass_history::{
    BuildProfile, CompletionEvidence, ExtractionFingerprint, GraphArtifacts, HistoryStore,
    PublishRequest, Repository,
};
use compass_model::GraphDocument;
use compass_pr_intelligence::{
    ChangeRequest, Completeness, MergeOutcome, RepositoryIdentity, RevisionSet,
};
use compass_semantic_diff::{Comparison, GraphDelta, SemanticDiffReport};
use serde_json::json;

const BASE: &str = "1111111111111111111111111111111111111111";
const RESULT: &str = "2222222222222222222222222222222222222222";
const HEAD: &str = "3333333333333333333333333333333333333333";

fn git(root: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned().into())
    }
}

fn repository() -> Result<(tempfile::TempDir, Repository), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    for arguments in [
        ["init", "--quiet"].as_slice(),
        ["config", "user.name", "Compass Test"].as_slice(),
        ["config", "user.email", "compass@example.invalid"].as_slice(),
    ] {
        git(directory.path(), arguments)?;
    }
    std::fs::write(directory.path().join("README.md"), "fixture\n")?;
    git(directory.path(), &["add", "README.md"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;
    let repository = Repository::discover(directory.path())?;
    Ok((directory, repository))
}

fn profile(extra: Option<(&str, &str)>) -> Result<BuildProfile, Box<dyn std::error::Error>> {
    let mut profile = BuildProfile::default();
    profile.insert("graph_schema", compass_history::HISTORY_GRAPH_SCHEMA)?;
    profile.insert("extractor_version", "compass-test/1")?;
    if let Some((key, value)) = extra {
        profile.insert(key, value)?;
    }
    Ok(profile)
}

fn publish_request(
    commit: &str,
    marker: char,
    profile: BuildProfile,
) -> Result<PublishRequest, Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": true,
        "nodes": [],
        "links": [],
        "hyperedges": []
    }))?;
    Ok(PublishRequest {
        commit: commit.parse()?,
        parents: Vec::new(),
        profile,
        fingerprint: std::iter::repeat_n(marker, 64)
            .collect::<String>()
            .parse::<ExtractionFingerprint>()?,
        artifacts: GraphArtifacts {
            document,
            program: None,
            analysis: None,
            labels: None,
            manifest: None,
            authoritative_sidecars: BTreeMap::new(),
        },
        completion: CompletionEvidence {
            extraction_succeeded: true,
            allow_partial: false,
            semantic_files_expected: 0,
            semantic_files_completed: 0,
            failed_chunks: 0,
        },
        make_preferred: true,
    })
}

fn change_request() -> ChangeRequest {
    ChangeRequest {
        repository: RepositoryIdentity {
            forge: "github".to_owned(),
            host: "github.com".to_owned(),
            owner: "crabbuild".to_owned(),
            name: "compass".to_owned(),
        },
        pull_request_number: Some(14),
        revisions: RevisionSet {
            merge_base: BASE.to_owned(),
            pull_request_head: HEAD.to_owned(),
            target_head: BASE.to_owned(),
            merge_result: MergeOutcome::Clean {
                object_id: RESULT.to_owned(),
            },
        },
        hunks: Vec::new(),
    }
}

fn semantic_report() -> SemanticDiffReport {
    SemanticDiffReport {
        schema: compass_semantic_diff::REPORT_SCHEMA.to_owned(),
        comparison: Comparison {
            old_commit: BASE.to_owned(),
            new_commit: RESULT.to_owned(),
            fingerprint: format!("sha256:{}", "a".repeat(64)),
        },
        findings: Vec::new(),
        feature_groups: Vec::new(),
        collapsed_groups: Vec::new(),
        source_changes: Vec::new(),
        graph_delta: GraphDelta::default(),
        entity_display_names: BTreeMap::new(),
        completeness: BTreeMap::new(),
        limitations: Vec::new(),
    }
}

#[test]
fn exact_realizations_survive_reopen_and_bind_report_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let common = profile(None)?;
    let base = history.publish(publish_request(BASE, 'a', common.clone())?)?;
    let result = history.publish(publish_request(RESULT, 'b', common)?)?;
    drop(history);

    let reopened = HistoryStore::open_existing(&repository)?.ok_or("missing history")?;
    let report = compass_core::review_change_request(
        &reopened,
        &change_request(),
        &base,
        &result,
        &semantic_report(),
        Completeness::DownstreamComplete,
    )?;
    assert_eq!(report.identity.revisions.target_head, BASE);
    assert_eq!(
        report.identity.revisions.merge_result.object_id(),
        Some(RESULT)
    );
    assert_eq!(report.identity.pull_request_number, Some(14));
    assert_eq!(report.completeness, Completeness::DownstreamComplete);
    Ok(())
}

#[test]
fn profile_mismatch_fails_before_risk_evaluation() -> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let base = history.publish(publish_request(BASE, 'a', profile(None)?)?)?;
    let result = history.publish(publish_request(
        RESULT,
        'b',
        profile(Some(("feature", "different")))?,
    )?)?;
    let error = compass_core::review_change_request(
        &history,
        &change_request(),
        &base,
        &result,
        &semantic_report(),
        Completeness::LocalExact,
    )
    .expect_err("profile mismatch must fail");
    assert!(error.to_string().contains("profile"));
    Ok(())
}
