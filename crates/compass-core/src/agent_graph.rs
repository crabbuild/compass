use std::fs::{self, File};
use std::io::{Read, Take};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use compass_agent_graph::{
    AgentGraphError, AgentGraphErrorCode, AgentGraphOverlay, AgentGraphPaths, ArtifactRecord,
    BaseGenerationId, BaseGenerationProvider, BaseGenerationView, ChangeBatch, CommitReceipt,
    Digest, OverlayRepository, OverlayRevisionId, PriorAssertionRecord, ReadRequest, ReadResult,
    RebaseCommitRequest, RepositoryId, WriteGrant,
};
use compass_history::Repository;
use compass_model::code_graph::GraphDocument;
use compass_store::SqliteStore;
use sha2::{Digest as _, Sha256};

const MAX_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;

/// Exact current Base Generation backed by repository-confined, bounded reads.
#[derive(Clone)]
pub struct CurrentBaseGenerationProvider {
    generation: Arc<CurrentBaseGeneration>,
}

impl CurrentBaseGenerationProvider {
    pub fn open(project_root: &Path, graph_path: &Path) -> Result<Self, AgentGraphError> {
        reject_symlink(graph_path, "Base Graph artifact")?;
        let project_root = project_root.canonicalize().map_err(|error| {
            AgentGraphError::new(
                AgentGraphErrorCode::UnknownBaseGeneration,
                format!("cannot canonicalize project root: {error}"),
            )
        })?;
        reject_symlink(&project_root, "project root")?;
        let (graph, _artifact_digest) = GraphDocument::load_with_artifact_digest(graph_path)
            .map_err(|error| {
                AgentGraphError::new(
                    AgentGraphErrorCode::UnknownBaseGeneration,
                    format!("cannot open exact Base Graph: {error}"),
                )
            })?;
        let identity = BaseGenerationId {
            generation_id: graph.graph.build.generation_id.clone(),
            graph_digest: Digest::raw_bytes(&compass_agent_graph::canonical_bytes(&graph)?),
        };
        let artifact_root = graph_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .map_err(|error| {
                AgentGraphError::new(
                    AgentGraphErrorCode::UnknownBaseGeneration,
                    format!("cannot canonicalize Base Graph artifact root: {error}"),
                )
            })?;
        Ok(Self {
            generation: Arc::new(CurrentBaseGeneration {
                identity,
                graph,
                project_root,
                artifact_root,
            }),
        })
    }

    #[must_use]
    pub fn identity(&self) -> &BaseGenerationId {
        &self.generation.identity
    }
}

/// Exact immutable historical Base Generation. The detached worktree is held for the provider's
/// lifetime, uses Compass history's offline checkout policy, and is never modified.
#[derive(Clone)]
pub struct HistoricalBaseGenerationProvider {
    generation: Arc<HistoricalBaseGeneration>,
}

impl HistoricalBaseGenerationProvider {
    pub fn open_exact(
        repository: &Repository,
        history: &compass_history::HistoryStore,
        realization: &compass_history::RealizationId,
    ) -> Result<Self, AgentGraphError> {
        let reader = history.reader(realization).map_err(history_error)?;
        let graph = reader.graph_document().map_err(history_error)?;
        let commit = reader
            .version()
            .version
            .git_commit
            .parse::<compass_history::CommitId>()
            .map_err(history_error)?;
        let checkout = repository
            .detached_worktree(&commit)
            .map_err(history_error)?;
        if !checkout.limitations().is_empty() {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnknownBaseGeneration,
                "historical Base Generation has unsupported Git target limitations",
            ));
        }
        let identity = BaseGenerationId {
            generation_id: graph.graph.build.generation_id.clone(),
            graph_digest: Digest::raw_bytes(&compass_agent_graph::canonical_bytes(&graph)?),
        };
        Ok(Self {
            generation: Arc::new(HistoricalBaseGeneration {
                identity,
                graph,
                checkout,
            }),
        })
    }

    #[must_use]
    pub fn identity(&self) -> &BaseGenerationId {
        &self.generation.identity
    }
}

impl BaseGenerationProvider for HistoricalBaseGenerationProvider {
    fn open(
        &self,
        identity: &BaseGenerationId,
    ) -> Result<Arc<dyn BaseGenerationView>, AgentGraphError> {
        if identity != &self.generation.identity {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnknownBaseGeneration,
                "requested Base Generation is not the selected historical realization",
            ));
        }
        Ok(Arc::clone(&self.generation) as Arc<dyn BaseGenerationView>)
    }
}

struct HistoricalBaseGeneration {
    identity: BaseGenerationId,
    graph: GraphDocument,
    checkout: compass_history::WorktreeGuard,
}

impl BaseGenerationView for HistoricalBaseGeneration {
    fn identity(&self) -> &BaseGenerationId {
        &self.identity
    }

    fn graph(&self) -> &GraphDocument {
        &self.graph
    }

    fn source_bytes(&self, repository_path: &str) -> Result<Option<Vec<u8>>, AgentGraphError> {
        let Some(inventory) = self
            .graph
            .graph
            .files
            .iter()
            .find(|file| file.path == repository_path)
        else {
            return Ok(None);
        };
        let path = confined_regular_file(
            self.checkout.path(),
            repository_path,
            "historical source file",
        )?;
        read_bounded(&path, inventory.byte_size).map(Some)
    }

    fn artifact(&self, _artifact: &str) -> Result<Option<ArtifactRecord>, AgentGraphError> {
        // History sidecars are intentionally not projected as arbitrary files. A historical
        // assertion that needs one must cite source/Base facts or use a separately registered
        // bounded artifact adapter.
        Ok(None)
    }

    fn prior_assertion(
        &self,
        _revision: &OverlayRevisionId,
        _assertion: &compass_agent_graph::AssertionId,
    ) -> Result<Option<PriorAssertionRecord>, AgentGraphError> {
        Ok(None)
    }
}

fn history_error(error: compass_history::HistoryError) -> AgentGraphError {
    AgentGraphError::new(
        AgentGraphErrorCode::UnknownBaseGeneration,
        format!("cannot open exact historical Base Generation: {error}"),
    )
}

impl BaseGenerationProvider for CurrentBaseGenerationProvider {
    fn open(
        &self,
        identity: &BaseGenerationId,
    ) -> Result<Arc<dyn BaseGenerationView>, AgentGraphError> {
        if identity != &self.generation.identity {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::UnknownBaseGeneration,
                "requested Base Generation is not the exact current graph",
            ));
        }
        Ok(Arc::clone(&self.generation) as Arc<dyn BaseGenerationView>)
    }
}

struct CurrentBaseGeneration {
    identity: BaseGenerationId,
    graph: GraphDocument,
    project_root: PathBuf,
    artifact_root: PathBuf,
}

impl BaseGenerationView for CurrentBaseGeneration {
    fn identity(&self) -> &BaseGenerationId {
        &self.identity
    }

    fn graph(&self) -> &GraphDocument {
        &self.graph
    }

    fn source_bytes(&self, repository_path: &str) -> Result<Option<Vec<u8>>, AgentGraphError> {
        let Some(inventory) = self
            .graph
            .graph
            .files
            .iter()
            .find(|file| file.path == repository_path)
        else {
            return Ok(None);
        };
        let path = confined_regular_file(&self.project_root, repository_path, "source file")?;
        let bytes = read_bounded(&path, inventory.byte_size)?;
        Ok(Some(bytes))
    }

    fn artifact(&self, artifact: &str) -> Result<Option<ArtifactRecord>, AgentGraphError> {
        let path = match confined_regular_file(&self.artifact_root, artifact, "snapshot artifact") {
            Ok(path) => path,
            Err(error) if error.code == AgentGraphErrorCode::InvalidCitation => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = fs::metadata(&path).map_err(|error| {
            AgentGraphError::new(
                AgentGraphErrorCode::InvalidCitation,
                format!("cannot inspect snapshot artifact: {error}"),
            )
        })?;
        if metadata.len() > MAX_ARTIFACT_BYTES {
            return Err(AgentGraphError::new(
                AgentGraphErrorCode::LimitExceeded,
                "snapshot artifact exceeds the 16 MiB verification ceiling",
            ));
        }
        Ok(Some(ArtifactRecord::new(read_bounded(
            &path,
            metadata.len(),
        )?)?))
    }

    fn prior_assertion(
        &self,
        _revision: &OverlayRevisionId,
        _assertion: &compass_agent_graph::AssertionId,
    ) -> Result<Option<PriorAssertionRecord>, AgentGraphError> {
        // Prior assertions are resolved by the overlay repository. The current
        // Base Generation provider never invents or imports them from graph data.
        Ok(None)
    }
}

/// One coherent local agent-graph context pinned to the current Base Generation.
pub struct AgentGraphContext {
    repository: OverlayRepository<SqliteStore, CurrentBaseGenerationProvider>,
    base_generation: BaseGenerationId,
    repository_id: RepositoryId,
    paths: AgentGraphPaths,
}

/// Coherent overlay context pinned to one explicit immutable history realization.
/// Opening it never updates a preferred realization or any history root.
pub struct HistoricalAgentGraphContext {
    repository: OverlayRepository<SqliteStore, HistoricalBaseGenerationProvider>,
    base_generation: BaseGenerationId,
    repository_id: RepositoryId,
    paths: AgentGraphPaths,
}

impl HistoricalAgentGraphContext {
    pub fn open_exact(
        project_root: &Path,
        history: &compass_history::HistoryStore,
        realization: &compass_history::RealizationId,
        non_git_state_root: Option<&Path>,
    ) -> Result<Self, AgentGraphError> {
        let canonical_project = project_root.canonicalize().map_err(|error| {
            AgentGraphError::new(
                AgentGraphErrorCode::StorageFailure,
                format!("cannot canonicalize project root: {error}"),
            )
        })?;
        let discovered = Repository::discover(&canonical_project).ok();
        let repository = discovered.as_ref().ok_or_else(|| {
            AgentGraphError::new(
                AgentGraphErrorCode::UnknownBaseGeneration,
                "historical Base Generation selection requires a Git repository",
            )
        })?;
        let provider =
            HistoricalBaseGenerationProvider::open_exact(repository, history, realization)?;
        let base_generation = provider.identity().clone();
        let repository_id = repository_id(&canonical_project)?;
        let paths = match discovered {
            Some(repository) => AgentGraphPaths::for_git_common_dir(repository.common_dir())?,
            None => {
                let state_root = non_git_state_root.ok_or_else(|| {
                    AgentGraphError::new(
                        AgentGraphErrorCode::StorageFailure,
                        "non-Git agent graph use requires an explicit state root",
                    )
                })?;
                AgentGraphPaths::for_explicit_state_root(state_root)?
            }
        };
        let overlay_repository =
            OverlayRepository::open_local(&paths, provider, repository_id.clone())?;
        Ok(Self {
            repository: overlay_repository,
            base_generation,
            repository_id,
            paths,
        })
    }

    #[must_use]
    pub fn base_generation(&self) -> &BaseGenerationId {
        &self.base_generation
    }

    #[must_use]
    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    #[must_use]
    pub fn paths(&self) -> &AgentGraphPaths {
        &self.paths
    }

    pub fn read(&self, request: ReadRequest) -> Result<ReadResult, AgentGraphError> {
        self.repository.read(request)
    }

    pub fn active_revision(
        &self,
        overlay: &compass_agent_graph::OverlayId,
    ) -> Result<Option<OverlayRevisionId>, AgentGraphError> {
        self.repository.active_revision(overlay)
    }

    pub fn apply(
        &self,
        grant: &WriteGrant,
        batch: ChangeBatch,
    ) -> Result<CommitReceipt, AgentGraphError> {
        self.repository.apply(grant, batch)
    }

    pub fn commit_rebase(
        &self,
        grant: &WriteGrant,
        request: RebaseCommitRequest,
    ) -> Result<CommitReceipt, AgentGraphError> {
        self.repository.commit_rebase(grant, request)
    }
}

impl AgentGraphContext {
    pub fn open_current(
        project_root: &Path,
        graph_path: &Path,
        non_git_state_root: Option<&Path>,
    ) -> Result<Self, AgentGraphError> {
        let provider = CurrentBaseGenerationProvider::open(project_root, graph_path)?;
        let base_generation = provider.identity().clone();
        let canonical_project = project_root.canonicalize().map_err(|error| {
            AgentGraphError::new(
                AgentGraphErrorCode::StorageFailure,
                format!("cannot canonicalize project root: {error}"),
            )
        })?;
        let repository_id = repository_id(&canonical_project)?;
        let paths = match Repository::discover(&canonical_project) {
            Ok(repository) => AgentGraphPaths::for_git_common_dir(repository.common_dir())?,
            Err(_) => {
                let state_root = non_git_state_root.ok_or_else(|| {
                    AgentGraphError::new(
                        AgentGraphErrorCode::StorageFailure,
                        "non-Git agent graph use requires an explicit state root",
                    )
                })?;
                AgentGraphPaths::for_explicit_state_root(state_root)?
            }
        };
        let repository = OverlayRepository::open_local(&paths, provider, repository_id.clone())?;
        Ok(Self {
            repository,
            base_generation,
            repository_id,
            paths,
        })
    }

    #[must_use]
    pub fn base_generation(&self) -> &BaseGenerationId {
        &self.base_generation
    }

    #[must_use]
    pub fn repository_id(&self) -> &RepositoryId {
        &self.repository_id
    }

    #[must_use]
    pub fn paths(&self) -> &AgentGraphPaths {
        &self.paths
    }

    pub fn read(&self, request: ReadRequest) -> Result<ReadResult, AgentGraphError> {
        self.repository.read(request)
    }

    pub fn active_revision(
        &self,
        overlay: &compass_agent_graph::OverlayId,
    ) -> Result<Option<OverlayRevisionId>, AgentGraphError> {
        self.repository.active_revision(overlay)
    }

    pub fn apply(
        &self,
        grant: &WriteGrant,
        batch: ChangeBatch,
    ) -> Result<CommitReceipt, AgentGraphError> {
        self.repository.apply(grant, batch)
    }

    pub fn commit_rebase(
        &self,
        grant: &WriteGrant,
        request: RebaseCommitRequest,
    ) -> Result<CommitReceipt, AgentGraphError> {
        self.repository.commit_rebase(grant, request)
    }
}

fn confined_regular_file(
    root: &Path,
    relative: &str,
    label: &str,
) -> Result<PathBuf, AgentGraphError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            format!("{label} path is not repository-confined"),
        ));
    }
    let lexical = root.join(relative_path);
    reject_symlink(&lexical, label)?;
    let canonical = lexical.canonicalize().map_err(|error| {
        AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            format!("cannot canonicalize {label}: {error}"),
        )
    })?;
    if canonical != lexical || !canonical.starts_with(root) || !canonical.is_file() {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            format!("{label} is not an exact confined regular file"),
        ));
    }
    Ok(canonical)
}

fn read_bounded(path: &Path, expected_bytes: u64) -> Result<Vec<u8>, AgentGraphError> {
    let limit = expected_bytes.checked_add(1).ok_or_else(|| {
        AgentGraphError::new(
            AgentGraphErrorCode::LimitExceeded,
            "bounded file size overflowed",
        )
    })?;
    let file = File::open(path).map_err(|error| {
        AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            format!("cannot open grounded input: {error}"),
        )
    })?;
    let mut reader: Take<File> = file.take(limit);
    let capacity = usize::try_from(expected_bytes).map_err(|_| {
        AgentGraphError::new(
            AgentGraphErrorCode::LimitExceeded,
            "grounded input does not fit the platform address space",
        )
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    reader.read_to_end(&mut bytes).map_err(|error| {
        AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            format!("cannot read grounded input: {error}"),
        )
    })?;
    if bytes.len() as u64 != expected_bytes {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            "grounded input size changed or exceeds its inventory size",
        ));
    }
    Ok(bytes)
}

fn reject_symlink(path: &Path, label: &str) -> Result<(), AgentGraphError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            format!("cannot inspect {label}: {error}"),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(AgentGraphError::new(
            AgentGraphErrorCode::InvalidCitation,
            format!("{label} must not be a symlink"),
        ));
    }
    Ok(())
}

fn repository_id(project_root: &Path) -> Result<RepositoryId, AgentGraphError> {
    let digest = Sha256::digest(project_root.as_os_str().as_encoded_bytes());
    RepositoryId::parse(format!("repository:{digest:x}"))
}
