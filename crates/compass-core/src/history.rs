use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compass_files::{DetectOptions, IgnorePolicy, detect};
use compass_graph::{Communities, god_nodes, score_communities, surprising_connections};
use compass_history::{
    BuildProfile, CommitId, CompletedGraphArtifacts, CompletionEvidence, CorruptPreferredToken,
    DerivedCacheNamespace, ExtractionFingerprint, ExtractionFingerprintInput, GraphArtifacts,
    HISTORY_GRAPH_SCHEMA, HistoryError, HistoryStore, PublishRequest, PublishedVersion,
    RealizationId, Repository, WorktreeGuard,
};
use compass_languages::Registry;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::program::current_provider_manifest;

/// Build boundary used by both production extraction and deterministic materialization tests.
pub trait CompleteGraphBuilder {
    fn promote_current(
        &self,
        _repository_root: &Path,
        _commit: &CommitId,
    ) -> Result<Option<CompletedGraphArtifacts>, MaterializeError> {
        Ok(None)
    }

    fn build(
        &self,
        checkout: &Path,
        output_root: &Path,
    ) -> Result<CompletedGraphArtifacts, MaterializeError>;

    fn default_viewer_projection(
        &self,
        _completed: &CompletedGraphArtifacts,
        _repository_root: &Path,
        _commit: &CommitId,
    ) -> Result<Option<Vec<u8>>, MaterializeError> {
        Ok(None)
    }
}

/// Convert a verified mutable code-only snapshot into the canonical artifact
/// shape produced by an exact historical extraction.
pub fn normalize_current_code_only_snapshot(
    mut artifacts: GraphArtifacts,
    repository_root: &Path,
    code_files: &[String],
) -> Result<CompletedGraphArtifacts, MaterializeError> {
    let mut communities = Communities::new();
    for node in &mut artifacts.document.nodes {
        let community = node
            .attributes
            .get("community")
            .and_then(|value| {
                value.as_u64().or_else(|| {
                    value
                        .as_object()
                        .and_then(|community| community.get("id"))
                        .and_then(serde_json::Value::as_u64)
                })
            })
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                MaterializeError::Incomplete(format!(
                    "current snapshot node {} has no valid community",
                    node.id
                ))
            })?;
        communities
            .entry(community)
            .or_default()
            .push(node.id.clone());
        node.attributes.remove("community_name");
    }
    for members in communities.values_mut() {
        members.sort();
    }
    let cohesion = score_communities(&artifacts.document, &communities);
    let gods = god_nodes(&artifacts.document, 10);
    let surprises = surprising_connections(&artifacts.document, &communities, 5);
    let analysis = json!({
        "communities": communities
            .iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
        "cohesion": cohesion
            .iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
        "gods": gods,
        "surprises": surprises,
        "tokens": {"input": 0, "output": 0},
    });
    artifacts.analysis = Some(
        serde_json::from_slice(&serde_json::to_vec(&analysis).map_err(HistoryError::from)?)
            .map_err(HistoryError::from)?,
    );
    artifacts.labels = None;
    if let Some(entries) = artifacts
        .manifest
        .as_mut()
        .and_then(serde_json::Value::as_object_mut)
    {
        let root =
            fs::canonicalize(repository_root).map_err(|source| compass_files::FileError::Io {
                path: repository_root.to_path_buf(),
                source,
            })?;
        let code_files = code_files
            .iter()
            .map(|file| {
                Path::new(file)
                    .strip_prefix(&root)
                    .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                    .map_err(|_| {
                        MaterializeError::Incomplete(format!(
                            "code-only manifest path is outside repository: {file}"
                        ))
                    })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        entries.retain(|path, _| code_files.contains(path));
        for entry in entries
            .values_mut()
            .filter_map(serde_json::Value::as_object_mut)
        {
            let ast_hash = entry
                .get("ast_hash")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            entry.insert(
                "semantic_hash".to_owned(),
                serde_json::Value::String(ast_hash),
            );
        }
    }
    let completed = CompletedGraphArtifacts {
        artifacts,
        completion: CompletionEvidence {
            extraction_succeeded: true,
            allow_partial: false,
            semantic_files_expected: 0,
            semantic_files_completed: 0,
            failed_chunks: 0,
        },
    };
    completed.partition()?;
    Ok(completed)
}

/// Resolve the provider inventory an exact build would fingerprint for this
/// checkout and history profile.
pub fn history_provider_manifest(
    checkout: &Path,
    profile: &BuildProfile,
) -> Result<Vec<compass_ir::ProviderDescriptor>, MaterializeError> {
    let checkout = fs::canonicalize(checkout).map_err(|source| compass_files::FileError::Io {
        path: checkout.to_path_buf(),
        source,
    })?;
    program_provider_manifest(&checkout, profile)
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

    fn resolved(&mut self, _fingerprint: &ExtractionFingerprint) -> Result<(), MaterializeError> {
        Ok(())
    }

    fn candidate(
        &mut self,
        _candidate: &RealizationId,
        _observed_preferred: Option<&RealizationId>,
    ) -> Result<(), MaterializeError> {
        Ok(())
    }
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
    let (existing, corrupt) = observe_preferred(store, &request.commit, &activity)?;
    if !request.rebuild
        && let Some(existing) = existing
    {
        return Ok(existing);
    }
    if request.replace_corrupt && corrupt.is_none() {
        return Err(MaterializeError::ReplaceCorruptNotApplicable);
    }
    if !request.rebuild
        && corrupt.is_none()
        && request.repository.resolve("HEAD")? == request.commit
        && let Some(completed) =
            builder.promote_current(request.repository.root(), &request.commit)?
    {
        return run_promoted_materialization(store, &request, observer, &activity, completed);
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

fn run_promoted_materialization(
    store: &HistoryStore,
    request: &MaterializeRequest,
    observer: &mut dyn MaterializeObserver,
    activity: &compass_history::ActivityGuard,
    mut completed: CompletedGraphArtifacts,
) -> Result<PublishedVersion, MaterializeError> {
    let providers = completed
        .artifacts
        .program
        .as_ref()
        .map(|program| program.program.providers.clone())
        .unwrap_or_default();
    let fingerprint =
        resolve_fingerprint_with_providers(&request.profile, request.repository.root(), providers)?;
    observer.resolved(&fingerprint)?;
    observer.entered(MaterializeStage::Building)?;
    observer.entered(MaterializeStage::Validating)?;
    validate_promoted(
        &mut completed,
        &request.repository,
        &request.commit,
        &request.profile,
    )?;
    observer.entered(MaterializeStage::Publishing)?;
    let prepared = store.prepare_publish_with_activity(
        PublishRequest {
            commit: request.commit.clone(),
            parents: request.repository.parents(&request.commit)?,
            profile: request.profile.clone(),
            fingerprint,
            artifacts: completed.artifacts,
            completion: completed.completion,
            make_preferred: true,
        },
        activity,
    )?;
    observer.candidate(prepared.id(), prepared.observed_preferred())?;
    store
        .commit_prepared_with_activity(prepared, activity)
        .map_err(Into::into)
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
    observer.resolved(&fingerprint)?;
    observer.entered(MaterializeStage::Building)?;
    let mut completed = builder.build(worktree.path(), worktree.output_root())?;
    observer.entered(MaterializeStage::Validating)?;
    validate_completed(
        &mut completed,
        &request.repository,
        &request.commit,
        &request.profile,
        worktree,
    )?;
    let default_viewer = builder
        .default_viewer_projection(&completed, request.repository.root(), &request.commit)
        .ok()
        .flatten();
    observer.entered(MaterializeStage::Publishing)?;
    let prepared = store.prepare_publish_with_activity(
        PublishRequest {
            commit: request.commit.clone(),
            parents: request.repository.parents(&request.commit)?,
            profile: request.profile.clone(),
            fingerprint,
            artifacts: completed.artifacts,
            completion: completed.completion,
            make_preferred: corrupt.is_none(),
        },
        activity,
    )?;
    observer.candidate(prepared.id(), prepared.observed_preferred())?;
    let mut published = store.commit_prepared_with_activity(prepared, activity)?;
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
    if let Some(graph_bytes) = default_viewer
        && let Ok(graph) = serde_json::from_slice::<serde_json::Value>(&graph_bytes)
    {
        let cache_key = json!({
            "schema": "compass.history.viewer_key/1",
            "realization": published.id.to_string(),
            "viewer_schema": "compass.history.viewer_graph/1",
            "projection_version": 1,
            "node_limit": 5_000,
            "community": serde_json::Value::Null,
        });
        let envelope = json!({
            "schema": "compass.history.viewer_graph/1",
            "commit": published.version.git_commit.clone(),
            "realization": published.id.to_string(),
            "fingerprint": published.version.extraction_fingerprint.clone(),
            "graph": graph,
        });
        if let Ok(bytes) = compass_history::canonical_json_bytes(&envelope)
            && let Ok(cache) = store.cache()
        {
            let _ = cache.write(DerivedCacheNamespace::Viewer, &cache_key, &bytes);
        }
    }
    Ok(published)
}

fn observe_preferred(
    store: &HistoryStore,
    commit: &CommitId,
    activity: &compass_history::ActivityGuard,
) -> Result<(Option<PublishedVersion>, Option<CorruptPreferredToken>), MaterializeError> {
    match store.preferred_with_activity(commit, activity) {
        Ok(Some(published)) => Ok((Some(published), None)),
        Ok(None) => Ok((None, None)),
        Err(original) if original.is_catalog_corruption() => {
            match store.corrupt_preferred_token_with_activity(commit, activity) {
                Ok(token) => Ok((None, Some(token))),
                Err(error) if error.is_catalog_corruption() => Err(original.into()),
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn resolve_fingerprint(
    profile: &BuildProfile,
    checkout: &Path,
) -> Result<ExtractionFingerprint, MaterializeError> {
    resolve_fingerprint_with_providers(
        profile,
        checkout,
        program_provider_manifest(checkout, profile)?,
    )
}

fn resolve_fingerprint_with_providers(
    profile: &BuildProfile,
    checkout: &Path,
    providers: Vec<compass_ir::ProviderDescriptor>,
) -> Result<ExtractionFingerprint, MaterializeError> {
    let mut input =
        ExtractionFingerprintInput::new(env!("CARGO_PKG_VERSION"), HISTORY_GRAPH_SCHEMA);
    input.insert("definition_hash_version", "tree-sitter-ast/v1")?;
    input.insert("build_profile_digest", &hex(&profile.digest()?))?;
    input.insert(
        "commit_configuration_digest",
        &configuration_digest(checkout, profile_gitignore(profile))?,
    )?;
    input.insert_program_provider_manifest(&providers)?;
    input.digest().map_err(Into::into)
}

fn program_provider_manifest(
    checkout: &Path,
    profile: &BuildProfile,
) -> Result<Vec<compass_ir::ProviderDescriptor>, MaterializeError> {
    let detection = detect(
        checkout,
        &DetectOptions {
            gitignore: profile_gitignore(profile),
            ignore_policy: IgnorePolicy::HistoricalCommit,
            extra_excludes: profile_excludes(profile),
            output_name: "compass-out".to_owned(),
            ..DetectOptions::default()
        },
    )?;
    let mut sources = detection
        .files
        .get("code")
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    sources.extend(
        detection
            .files
            .get("document")
            .into_iter()
            .flatten()
            .map(PathBuf::from)
            .filter(|path| Registry::resolve(path).is_some()),
    );
    let mut options = crate::BuildOptions::new(checkout);
    options.gitignore = profile_gitignore(profile);
    options.ignore_policy = IgnorePolicy::HistoricalCommit;
    options.extra_excludes = profile_excludes(profile);
    options.program_analysis = true;
    current_provider_manifest(checkout, &sources, &options)
        .map_err(|error| MaterializeError::Builder(error.to_string()))
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
                    ".compass.toml"
                        | "compass.toml"
                        | "Cargo.toml"
                        | "pyproject.toml"
                        | "package.json"
                        | "tsconfig.json"
                )
            );
            let is_applied_ignore =
                include_ignore_files && matches!(name, Some(".gitignore" | ".compassignore"));
            if is_configuration || is_applied_ignore {
                files.push(path);
            }
        }
    }
    Ok(())
}

fn validate_completed(
    completed: &mut CompletedGraphArtifacts,
    repository: &Repository,
    commit: &CommitId,
    profile: &BuildProfile,
    worktree: &WorktreeGuard,
) -> Result<(), MaterializeError> {
    let built_at = source_commit(&completed.artifacts.document);
    if built_at != Some(commit.as_str()) {
        return Err(MaterializeError::Incomplete(format!(
            "sourceCommit is {:?}, expected {commit}",
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
    let semantic_files = if profile.value("code_only") == Some("true") {
        Vec::new()
    } else {
        ["document", "paper", "image", "video"]
            .into_iter()
            .flat_map(|kind| detection.files.get(kind).into_iter().flatten())
            .collect::<Vec<_>>()
    };
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
    attach_source_inventory(
        completed,
        repository,
        commit,
        profile,
        worktree.path(),
        &detection.files,
    )?;
    Ok(())
}

fn validate_promoted(
    completed: &mut CompletedGraphArtifacts,
    repository: &Repository,
    commit: &CommitId,
    profile: &BuildProfile,
) -> Result<(), MaterializeError> {
    let built_at = source_commit(&completed.artifacts.document);
    if built_at != Some(commit.as_str()) {
        return Err(MaterializeError::Incomplete(format!(
            "sourceCommit is {:?}, expected {commit}",
            built_at
        )));
    }
    let detection = detect(
        repository.root(),
        &DetectOptions {
            gitignore: profile_gitignore(profile),
            ignore_policy: IgnorePolicy::HistoricalCommit,
            extra_excludes: profile_excludes(profile),
            ..DetectOptions::default()
        },
    )?;
    attach_source_inventory(
        completed,
        repository,
        commit,
        profile,
        repository.root(),
        &detection.files,
    )?;
    Ok(())
}

fn source_commit(document: &compass_model::GraphDocument) -> Option<&str> {
    document
        .graph
        .get("build")
        .and_then(serde_json::Value::as_object)
        .and_then(|build| build.get("sourceCommit"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            document
                .extras
                .get("built_at_commit")
                .and_then(serde_json::Value::as_str)
        })
}

fn attach_source_inventory(
    completed: &mut CompletedGraphArtifacts,
    repository: &Repository,
    commit: &CommitId,
    profile: &BuildProfile,
    checkout: &Path,
    detected: &BTreeMap<String, Vec<String>>,
) -> Result<(), MaterializeError> {
    const INVENTORY_PATH: &str = ".compass_source_inventory.json";
    if completed
        .artifacts
        .authoritative_sidecars
        .contains_key(INVENTORY_PATH)
    {
        return Err(MaterializeError::Incomplete(format!(
            "graph builder attempted to provide reserved artifact {INVENTORY_PATH}"
        )));
    }
    let manifest = completed
        .artifacts
        .manifest
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            MaterializeError::Incomplete(
                "exact extraction requires an object-shaped source manifest".to_owned(),
            )
        })?;
    let blobs = repository.committed_blob_ids(commit)?;
    let mut record_counts = BTreeMap::<String, u64>::new();
    for attributes in completed
        .artifacts
        .document
        .nodes
        .iter()
        .map(|node| &node.attributes)
        .chain(
            completed
                .artifacts
                .document
                .links
                .iter()
                .map(|edge| &edge.attributes),
        )
    {
        let mut attributed = BTreeSet::new();
        for field in ["source_file", "origin_file"] {
            if let Some(source) = attributes
                .get(field)
                .and_then(serde_json::Value::as_str)
                .and_then(|source| repository_relative_source(source, checkout))
            {
                attributed.insert(source);
            }
        }
        for source in attributed {
            *record_counts.entry(source).or_default() += 1;
        }
    }
    let mut entries = BTreeMap::new();
    for file in detected.get("code").into_iter().flatten() {
        let path = Path::new(file);
        let relative = path.strip_prefix(checkout).map_err(|_| {
            MaterializeError::Incomplete(format!(
                "detected code source escaped exact checkout: {}",
                path.display()
            ))
        })?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        let blob = blobs.get(&relative).ok_or_else(|| {
            MaterializeError::Incomplete(format!(
                "detected code source is absent from exact Git tree: {relative}"
            ))
        })?;
        let ast_hash = manifest
            .get(&relative)
            .and_then(serde_json::Value::as_object)
            .and_then(|entry| entry.get("ast_hash"))
            .and_then(serde_json::Value::as_str)
            .filter(|hash| !hash.is_empty())
            .ok_or_else(|| {
                MaterializeError::Incomplete(format!(
                    "extraction manifest has no AST completion stamp for {relative}"
                ))
            })?;
        let record_count = record_counts.get(&relative).copied().unwrap_or_default();
        entries.insert(
            relative.clone(),
            json!({
                "git_object": blob,
                "ast_hash": ast_hash,
                "extension": relative.rsplit_once('.').map(|(_, extension)| extension).unwrap_or(""),
                "status": if record_count == 0 { "no_graph_records" } else { "extracted" },
                "graph_record_count": record_count,
            }),
        );
    }
    let inventory = json!({
        "schema": "compass.history.source_inventory/1",
        "commit": commit,
        "profile_digest": format!("{:x}", Sha256::digest(profile.canonical_bytes()?)),
        "code_files": entries,
    });
    completed.artifacts.authoritative_sidecars.insert(
        INVENTORY_PATH.to_owned(),
        compass_history::canonical_json_bytes(&inventory)?,
    );
    Ok(())
}

fn repository_relative_source(source: &str, checkout: &Path) -> Option<String> {
    let path = Path::new(source);
    let relative = if path.is_absolute() {
        path.strip_prefix(checkout).ok()?
    } else {
        path
    };
    let normalized = relative.to_string_lossy().replace('\\', "/");
    let normalized = normalized.strip_prefix("./").unwrap_or(&normalized);
    (!normalized.is_empty() && !normalized.starts_with("../")).then(|| normalized.to_owned())
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
