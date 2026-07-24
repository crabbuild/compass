use std::sync::atomic::Ordering;

use compass_history::{BuildProfile, CommitId, HistoryStore, MAX_DIAGNOSTIC_BYTES, Repository};
use serde::Serialize;

use crate::history_build::HistoryBuildOptions;
use crate::history_commands::resolve_or_materialize;

pub(crate) struct BatchExecution {
    pub(crate) stdout: String,
    pub(crate) failed: bool,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CommitBuildStatus {
    Built,
    Rebuilt,
    Skipped,
    Failed,
}

impl CommitBuildStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Built => "built",
            Self::Rebuilt => "rebuilt",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Serialize)]
struct CommitBuildResult {
    commit: String,
    status: CommitBuildStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    realization: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<String>,
}

#[derive(Default, Serialize)]
struct BuildCounts {
    total: usize,
    built: usize,
    rebuilt: usize,
    skipped: usize,
    failed: usize,
}

#[derive(Serialize)]
struct BatchReport {
    schema_version: u8,
    #[serde(rename = "ref")]
    reference: String,
    tip: String,
    scope: &'static str,
    profile_digest: String,
    counts: BuildCounts,
    results: Vec<CommitBuildResult>,
}

pub(crate) fn execute(
    repository: &Repository,
    reference: &str,
    tip: CommitId,
    commits: Vec<CommitId>,
    options: &HistoryBuildOptions,
    first_parent: bool,
    format: &str,
) -> Result<BatchExecution, String> {
    let profile = options.profile();
    let profile_digest = profile_digest(&profile)?;
    let cancellation = crate::process_cancellation()?;
    let total = commits.len();
    eprintln!(
        "Building {total} commits reachable from {reference} ({})",
        short_commit(&tip)
    );

    let mut counts = BuildCounts {
        total,
        ..BuildCounts::default()
    };
    let mut results = Vec::with_capacity(total);
    for (offset, commit) in commits.into_iter().enumerate() {
        if cancellation.load(Ordering::Acquire) {
            return Err(format!(
                "history build interrupted after {} of {total} commits; completed publications were preserved",
                results.len()
            ));
        }

        let result = build_one(repository, commit.clone(), options, &profile);
        match result.status {
            CommitBuildStatus::Built => counts.built += 1,
            CommitBuildStatus::Rebuilt => counts.rebuilt += 1,
            CommitBuildStatus::Skipped => counts.skipped += 1,
            CommitBuildStatus::Failed => counts.failed += 1,
        }
        eprintln!(
            "[{}/{}] {} {}",
            offset + 1,
            total,
            short_commit(&commit),
            result.status.as_str()
        );
        results.push(result);
    }

    let failed = counts.failed != 0;
    let report = BatchReport {
        schema_version: 1,
        reference: reference.to_owned(),
        tip: tip.to_string(),
        scope: if first_parent {
            "first_parent"
        } else {
            "reachable"
        },
        profile_digest,
        counts,
        results,
    };
    let stdout = if format == "json" {
        serde_json::to_string(&report).map_err(|error| error.to_string())?
    } else {
        render_text(&report)
    };
    Ok(BatchExecution { stdout, failed })
}

fn build_one(
    repository: &Repository,
    commit: CommitId,
    options: &HistoryBuildOptions,
    profile: &BuildProfile,
) -> CommitBuildResult {
    let existing = match HistoryStore::open_existing(repository) {
        Ok(existing) => existing,
        Err(error) => return failure(commit, error),
    };
    let preferred = match existing
        .as_ref()
        .map(|store| store.preferred(&commit))
        .transpose()
    {
        Ok(preferred) => preferred.flatten(),
        Err(error) => return failure(commit, error),
    };

    let rebuild = if let Some(preferred) = &preferred {
        let Some(history) = existing.as_ref() else {
            return failure(
                commit,
                "history store disappeared while inspecting a realization",
            );
        };
        if let Err(error) = history.validate(&preferred.id) {
            return failure(commit, error);
        }
        if preferred.version.build_profile == *profile {
            return CommitBuildResult {
                commit: commit.to_string(),
                status: CommitBuildStatus::Skipped,
                realization: Some(preferred.id.to_string()),
                diagnostic: None,
            };
        }
        true
    } else {
        false
    };

    match resolve_or_materialize(repository, commit.clone(), options, rebuild, false) {
        Ok((_history, published)) => CommitBuildResult {
            commit: commit.to_string(),
            status: if rebuild {
                CommitBuildStatus::Rebuilt
            } else {
                CommitBuildStatus::Built
            },
            realization: Some(published.id.to_string()),
            diagnostic: None,
        },
        Err(error) => failure(commit, error),
    }
}

fn failure(commit: CommitId, error: impl ToString) -> CommitBuildResult {
    CommitBuildResult {
        commit: commit.to_string(),
        status: CommitBuildStatus::Failed,
        realization: None,
        diagnostic: Some(bounded_diagnostic(error.to_string())),
    }
}

fn bounded_diagnostic(mut diagnostic: String) -> String {
    if diagnostic.len() <= MAX_DIAGNOSTIC_BYTES {
        return diagnostic;
    }
    let mut end = MAX_DIAGNOSTIC_BYTES;
    while !diagnostic.is_char_boundary(end) {
        end -= 1;
    }
    diagnostic.truncate(end);
    diagnostic
}

fn profile_digest(profile: &BuildProfile) -> Result<String, String> {
    profile
        .digest()
        .map(|digest| {
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        })
        .map_err(|error| error.to_string())
}

fn short_commit(commit: &CommitId) -> &str {
    let value = commit.as_str();
    &value[..value.len().min(12)]
}

fn render_text(report: &BatchReport) -> String {
    let mut output = format!(
        "ref: {}\ntip: {}\nscope: {}\nprofile digest: {}\ntotal: {}\nbuilt: {}\nrebuilt: {}\nskipped: {}\nfailed: {}",
        report.reference,
        report.tip,
        report.scope,
        report.profile_digest,
        report.counts.total,
        report.counts.built,
        report.counts.rebuilt,
        report.counts.skipped,
        report.counts.failed
    );
    for result in &report.results {
        output.push_str(&format!("\n{} {}", result.commit, result.status.as_str()));
        if let Some(realization) = &result.realization {
            output.push_str(&format!(" {realization}"));
        }
        if let Some(diagnostic) = &result.diagnostic {
            output.push_str(&format!("\n  diagnostic: {diagnostic}"));
        }
    }
    output
}
