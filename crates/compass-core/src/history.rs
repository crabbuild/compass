use std::fs;
use std::path::{Path, PathBuf};

use compass_files::{DetectOptions, IgnorePolicy, detect};
use compass_history::{
    BuildProfile, CommitId, CompletedGraphArtifacts, CorruptPreferredToken, ExtractionFingerprint,
    ExtractionFingerprintInput, GraphArtifacts, HistoryError, HistoryStore, PublishRequest,
    PublishedVersion, Repository, WorktreeGuard,
};
use sha2::{Digest, Sha256};

/// Build boundary used by both production extraction and deterministic materialization tests.
pub trait CompleteGraphBuilder {
    fn build(
        &self,
        checkout: &Path,
        output_root: &Path,
        seed: Option<&GraphArtifacts>,
    ) -> Result<CompletedGraphArtifacts, MaterializeError>;
}

/// Inputs that identify one exact historical materialization attempt.
pub struct MaterializeRequest {
    pub repository: Repository,
    pub commit: CommitId,
    pub profile: BuildProfile,
    pub rebuild: bool,
    pub replace_corrupt: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializeStage {
    Building,
    Validating,
    Publishing,
}

/// Optional phase observer used by durable workers.
pub trait MaterializeObserver {
    fn entered(&mut self, stage: MaterializeStage) -> Result<(), MaterializeError>;
}

#[derive(Debug, thiserror::Error)]
pub enum MaterializeError {
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Files(#[from] compass_files::FileError),
    #[error("graph builder failed: {0}")]
    Builder(String),
    #[error("could not {operation} graph builder process: {source}")]
    BuilderIo {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("graph builder exited with {exit_code:?}; stdout={stdout:?}; stderr={stderr:?}")]
    BuilderProcess {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error("materialized graph is incomplete: {0}")]
    Incomplete(String),
    #[error("materialization observer failed: {0}")]
    Observer(String),
    #[error("corrupt preferred recovery changed concurrently")]
    ConcurrentRecovery,
    #[error("--replace-corrupt requires an existing corrupt preferred realization")]
    ReplaceCorruptNotApplicable,
    #[error("worktree cleanup failed after materialization: {0}")]
    Cleanup(HistoryError),
    #[error("materialization failed ({operation}) and worktree cleanup also failed ({cleanup})")]
    OperationAndCleanup {
        operation: Box<MaterializeError>,
        cleanup: HistoryError,
    },
}

struct NoopObserver;

impl MaterializeObserver for NoopObserver {
    fn entered(&mut self, _stage: MaterializeStage) -> Result<(), MaterializeError> {
        Ok(())
    }
}

pub fn materialize_history(
    store: &HistoryStore,
    builder: &dyn CompleteGraphBuilder,
    request: MaterializeRequest,
) -> Result<PublishedVersion, MaterializeError> {
    materialize_history_with_observer(store, builder, request, &mut NoopObserver)
}

pub fn materialize_history_with_observer(
    store: &HistoryStore,
    builder: &dyn CompleteGraphBuilder,
    request: MaterializeRequest,
    observer: &mut dyn MaterializeObserver,
) -> Result<PublishedVersion, MaterializeError> {
    let activity = store.activity()?;
    let (existing, corrupt) = observe_preferred(store, &request.commit)?;
    if !request.rebuild
        && let Some(existing) = existing
    {
        return Ok(existing);
    }
    if request.replace_corrupt && corrupt.is_none() {
        return Err(MaterializeError::ReplaceCorruptNotApplicable);
    }
    let worktree = request.repository.detached_worktree(&request.commit)?;
    let result = run_materialization(
        store, builder, &request, observer, &activity, &worktree, corrupt,
    );
    let cleanup = worktree.close();
    match (result, cleanup) {
        (Ok(published), Ok(())) => Ok(published),
        (Ok(_), Err(cleanup)) => Err(MaterializeError::Cleanup(cleanup)),
        (Err(operation), Ok(())) => Err(operation),
        (Err(operation), Err(cleanup)) => Err(MaterializeError::OperationAndCleanup {
            operation: Box::new(operation),
            cleanup,
        }),
    }
}

fn run_materialization(
    store: &HistoryStore,
    builder: &dyn CompleteGraphBuilder,
    request: &MaterializeRequest,
    observer: &mut dyn MaterializeObserver,
    activity: &compass_history::ActivityGuard,
    worktree: &WorktreeGuard,
    corrupt: Option<CorruptPreferredToken>,
) -> Result<PublishedVersion, MaterializeError> {
    let fingerprint = resolve_fingerprint(&request.profile, worktree.path())?;
    let seed = compatible_seed(store, &request.repository, &request.commit, &fingerprint)?;
    observer.entered(MaterializeStage::Building)?;
    let completed = builder.build(
        worktree.path(),
        worktree.output_root(),
        seed.as_ref().map(|value| &value.artifacts),
    )?;
    observer.entered(MaterializeStage::Validating)?;
    validate_completed(&completed, &request.commit, &request.profile, worktree)?;
    observer.entered(MaterializeStage::Publishing)?;
    let mut published = store.publish_with_activity(
        PublishRequest {
            commit: request.commit.clone(),
            parents: request.repository.parents(&request.commit)?,
            fingerprint,
            artifacts: completed.artifacts,
            completion: completed.completion,
            make_preferred: corrupt.is_none(),
        },
        activity,
    )?;
    if let Some(observed) = corrupt {
        if request.replace_corrupt {
            if !store.recover_corrupt_preferred_with_activity(
                &request.commit,
                &observed,
                &published.id,
                activity,
            )? {
                return Err(MaterializeError::ConcurrentRecovery);
            }
            published.preferred = true;
        } else {
            published.preferred = false;
        }
    }
    Ok(published)
}

fn observe_preferred(
    store: &HistoryStore,
    commit: &CommitId,
) -> Result<(Option<PublishedVersion>, Option<CorruptPreferredToken>), MaterializeError> {
    match store.preferred(commit) {
        Ok(Some(published)) => {
            store.validate(&published.id)?;
            Ok((Some(published), None))
        }
        Ok(None) => Ok((None, None)),
        Err(original) if original.is_catalog_corruption() => {
            match store.corrupt_preferred_token(commit) {
                Ok(token) => Ok((None, Some(token))),
                Err(error) if error.is_catalog_corruption() => Err(original.into()),
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn compatible_seed(
    store: &HistoryStore,
    repository: &Repository,
    target: &CommitId,
    fingerprint: &ExtractionFingerprint,
) -> Result<Option<CompletedGraphArtifacts>, MaterializeError> {
    for ancestor in repository.first_parent_ancestors(target)? {
        let preferred = match store.preferred(&ancestor) {
            Ok(preferred) => preferred,
            Err(error) if error.is_catalog_corruption() => continue,
            Err(error) => return Err(error.into()),
        };
        let Some(preferred) = preferred else {
            continue;
        };
        if preferred.version.extraction_fingerprint != fingerprint.as_hex() {
            continue;
        }
        match store.validate(&preferred.id) {
            Ok(_) => {}
            Err(error) if error.is_catalog_corruption() => continue,
            Err(error) => return Err(error.into()),
        }
        return store.artifacts(&preferred.id).map(Some).map_err(Into::into);
    }
    Ok(None)
}

fn resolve_fingerprint(
    profile: &BuildProfile,
    checkout: &Path,
) -> Result<ExtractionFingerprint, MaterializeError> {
    let mut input =
        ExtractionFingerprintInput::new(env!("CARGO_PKG_VERSION"), "networkx-node-link/v1");
    input.insert("build_profile_digest", &hex(&profile.digest()?))?;
    input.insert(
        "commit_configuration_digest",
        &configuration_digest(checkout, profile_gitignore(profile))?,
    )?;
    input.digest().map_err(Into::into)
}

fn configuration_digest(
    checkout: &Path,
    include_ignore_files: bool,
) -> Result<String, MaterializeError> {
    let mut files = Vec::new();
    collect_configuration_files(checkout, checkout, include_ignore_files, &mut files)?;
    files.sort();
    let mut digest = Sha256::new();
    for path in files {
        let relative = path.strip_prefix(checkout).map_err(|_| {
            MaterializeError::Incomplete(format!(
                "configuration path escaped checkout: {}",
                path.display()
            ))
        })?;
        digest.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        digest.update([0]);
        digest.update(fs::read(&path).map_err(|source| HistoryError::Io {
            path: path.clone(),
            source,
        })?);
        digest.update([0xff]);
    }
    Ok(hex(&digest.finalize()))
}

fn collect_configuration_files(
    root: &Path,
    directory: &Path,
    include_ignore_files: bool,
    files: &mut Vec<PathBuf>,
) -> Result<(), MaterializeError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| HistoryError::Io {
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| HistoryError::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path == root.join(".git") {
            continue;
        }
        let file_type = entry.file_type().map_err(|source| HistoryError::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            collect_configuration_files(root, &path, include_ignore_files, files)?;
        } else if file_type.is_file() {
            let name = path.file_name().and_then(|value| value.to_str());
            let is_configuration = matches!(
                name,
                Some(
                    ".graphify.toml"
                        | "graphify.toml"
                        | "Cargo.toml"
                        | "pyproject.toml"
                        | "package.json"
                        | "tsconfig.json"
                )
            );
            let is_applied_ignore =
                include_ignore_files && matches!(name, Some(".gitignore" | ".graphifyignore"));
            if is_configuration || is_applied_ignore {
                files.push(path);
            }
        }
    }
    Ok(())
}

fn validate_completed(
    completed: &CompletedGraphArtifacts,
    commit: &CommitId,
    profile: &BuildProfile,
    worktree: &WorktreeGuard,
) -> Result<(), MaterializeError> {
    completed.partition()?;
    let built_at = completed
        .artifacts
        .document
        .extras
        .get("built_at_commit")
        .and_then(serde_json::Value::as_str);
    if built_at != Some(commit.as_str()) {
        return Err(MaterializeError::Incomplete(format!(
            "built_at_commit is {:?}, expected {commit}",
            built_at
        )));
    }
    let detection = detect(
        worktree.path(),
        &DetectOptions {
            gitignore: profile_gitignore(profile),
            ignore_policy: IgnorePolicy::HistoricalCommit,
            extra_excludes: profile_excludes(profile),
            cache_root: Some(worktree.output_root().to_path_buf()),
            ..DetectOptions::default()
        },
    )?;
    let semantic_files = ["document", "paper", "image", "video"]
        .into_iter()
        .flat_map(|kind| detection.files.get(kind).into_iter().flatten())
        .collect::<Vec<_>>();
    let semantic_expected = u64::try_from(semantic_files.len())
        .map_err(|_| MaterializeError::Incomplete("semantic inventory exceeds u64".to_owned()))?;
    if completed.completion.semantic_files_expected != semantic_expected {
        return Err(MaterializeError::Incomplete(format!(
            "semantic completion expected {}, exact worktree contains {semantic_expected}",
            completed.completion.semantic_files_expected
        )));
    }
    if semantic_expected > 0 {
        let manifest = completed
            .artifacts
            .manifest
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                MaterializeError::Incomplete(
                    "semantic extraction requires an object-shaped manifest".to_owned(),
                )
            })?;
        for file in semantic_files {
            let path = Path::new(file);
            let relative = path.strip_prefix(worktree.path()).map_err(|_| {
                MaterializeError::Incomplete(format!(
                    "semantic source escaped exact checkout: {}",
                    path.display()
                ))
            })?;
            let key = relative.to_string_lossy().replace('\\', "/");
            let completed = manifest
                .get(&key)
                .and_then(serde_json::Value::as_object)
                .and_then(|entry| entry.get("semantic_hash"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|hash| !hash.is_empty());
            if !completed {
                return Err(MaterializeError::Incomplete(format!(
                    "extraction manifest has no completed semantic entry for {key}"
                )));
            }
        }
    }
    Ok(())
}

fn profile_gitignore(profile: &BuildProfile) -> bool {
    profile.value("gitignore") != Some("false")
}

fn profile_excludes(profile: &BuildProfile) -> Vec<String> {
    profile
        .entries()
        .filter(|(key, _)| key.starts_with("exclude."))
        .map(|(_, value)| value.to_owned())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
