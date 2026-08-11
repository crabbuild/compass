use compass_history::{HistoryStore, PublishedVersion, Repository};
use compass_pr_intelligence::{
    ChangeRequest, Completeness, EvidenceManifest, EvidenceRepository, EvidenceSource, Freshness,
    GraphSnapshot, MergeOutcome, PullRequestReport, analyze, canonical_json_bytes,
};
use compass_semantic_diff::SemanticDiffReport;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("history evidence failed validation: {0}")]
    History(#[from] compass_history::HistoryError),
    #[error("PR Intelligence analysis failed: {0}")]
    Intelligence(#[from] compass_pr_intelligence::PrIntelligenceError),
    #[error("semantic diff analysis failed: {0}")]
    SemanticDiff(#[from] compass_semantic_diff::SemanticDiffError),
    #[error("review evidence is not comparable: {0}")]
    NotComparable(String),
}

/// Compute semantic evidence and review one exact immutable candidate.
pub fn review_change_request_exact(
    repository: &Repository,
    history: &HistoryStore,
    request: &ChangeRequest,
    base: &PublishedVersion,
    comparison: &PublishedVersion,
    completeness: Completeness,
) -> Result<PullRequestReport, ReviewError> {
    let old = repository.resolve(&base.version.git_commit)?;
    let new = repository.resolve(&comparison.version.git_commit)?;
    let source_deltas = repository.source_delta(&old, &new)?;
    let semantic_diff = compass_semantic_diff::compare_history_realizations(
        history,
        base,
        comparison,
        &source_deltas,
    )?;
    review_change_request(
        history,
        request,
        base,
        comparison,
        &semantic_diff,
        completeness,
    )
}

/// Validate exact immutable realizations and invoke the canonical review engine.
///
/// `comparison` is the synthetic merge realization for a clean merge and the
/// pull-request head realization for a conflicted merge. Both readers retain
/// history activity guards until analysis completes.
pub fn review_change_request(
    history: &HistoryStore,
    request: &ChangeRequest,
    base: &PublishedVersion,
    comparison: &PublishedVersion,
    semantic_diff: &SemanticDiffReport,
    completeness: Completeness,
) -> Result<PullRequestReport, ReviewError> {
    let expected_comparison = request
        .revisions
        .merge_result
        .object_id()
        .unwrap_or(&request.revisions.pull_request_head);
    if base.version.git_commit != request.revisions.target_head {
        return Err(ReviewError::NotComparable(format!(
            "base realization is {}, expected target head {}",
            base.version.git_commit, request.revisions.target_head
        )));
    }
    if comparison.version.git_commit != expected_comparison {
        return Err(ReviewError::NotComparable(format!(
            "comparison realization is {}, expected {expected_comparison}",
            comparison.version.git_commit
        )));
    }
    if base.version.build_profile != comparison.version.build_profile {
        return Err(ReviewError::NotComparable(format!(
            "base profile {} differs from comparison profile {}",
            base.version.profile_digest, comparison.version.profile_digest
        )));
    }
    history.validate(&base.id)?;
    history.validate(&comparison.id)?;
    let _base_reader = history.reader(&base.id)?;
    let _comparison_reader = history.reader(&comparison.id)?;

    let base_snapshot = snapshot(base)?;
    let comparison_snapshot = snapshot(comparison)?;
    let source_digest = digest(&canonical_json_bytes(request)?);
    let semantic_digest = digest(&canonical_json_bytes(semantic_diff)?);
    let policy_pack_digest = digest(b"compass-policy-pack/none/1");
    let mut sources = vec![
        EvidenceSource {
            kind: "change_request".to_owned(),
            identity: request.repository.canonical_name(),
            digest: source_digest,
            completeness: Completeness::LocalExact,
        },
        EvidenceSource {
            kind: "semantic_diff".to_owned(),
            identity: semantic_diff.comparison.fingerprint.clone(),
            digest: semantic_digest,
            completeness: Completeness::LocalExact,
        },
        EvidenceSource {
            kind: "graph_realization".to_owned(),
            identity: base.id.to_string(),
            digest: digest(base.id.to_string().as_bytes()),
            completeness: Completeness::LocalExact,
        },
        EvidenceSource {
            kind: "graph_realization".to_owned(),
            identity: comparison.id.to_string(),
            digest: digest(comparison.id.to_string().as_bytes()),
            completeness: Completeness::LocalExact,
        },
    ];
    sources.sort();
    let manifest = EvidenceManifest {
        digest: String::new(),
        graph_schema: base_snapshot.graph_schema.clone(),
        extractor_version: base_snapshot.extractor_version.clone(),
        configuration_digest: base_snapshot.configuration_digest.clone(),
        policy_pack_digest,
        completeness,
        repositories: vec![EvidenceRepository {
            repository: request.repository.clone(),
            graph_revision: Some(base.version.git_commit.clone()),
            observed_head: request.revisions.target_head.clone(),
            freshness: Freshness::ExactHead,
            authorized: true,
            failure: None,
        }],
        sources,
    }
    .seal()?;
    let result_snapshot = match request.revisions.merge_result {
        MergeOutcome::Clean { .. } => Some(&comparison_snapshot),
        MergeOutcome::Conflicted { .. } | MergeOutcome::Unavailable { .. } => None,
    };
    Ok(analyze(
        request,
        &base_snapshot,
        result_snapshot,
        &manifest,
        semantic_diff,
    )?)
}

fn snapshot(version: &PublishedVersion) -> Result<GraphSnapshot, ReviewError> {
    let graph_schema = profile_value(version, "graph_schema")?;
    let extractor_version = profile_value(version, "extractor_version")?;
    Ok(GraphSnapshot {
        revision: version.version.git_commit.clone(),
        realization: version.id.to_string(),
        graph_schema,
        extractor_version,
        configuration_digest: version.version.profile_digest.clone(),
    })
}

fn profile_value(version: &PublishedVersion, key: &str) -> Result<String, ReviewError> {
    version
        .version
        .build_profile
        .value(key)
        .map(str::to_owned)
        .ok_or_else(|| {
            ReviewError::NotComparable(format!(
                "realization {} has no {key} profile field",
                version.id
            ))
        })
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
