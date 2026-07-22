use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use prolly::{
    BatchBuilder, Config, KeyBuilder, ManifestStoreScan, NamedRootUpdate, Prolly,
    TransactionUpdate, Tree,
};
use prolly_store_sqlite::{SqliteStore, SqliteStoreConfig};

use crate::keys::root_name;
use crate::validate::{RealizationTrees, validate_trees};
use crate::{
    ActivityGuard, CommitId, GraphVersion, HistoryError, MaintenanceGuard, PublishRequest,
    PublishedVersion, RealizationId, Repository, StoredTree, StructuralSharing,
    canonical_json_bytes,
};

pub(crate) const STORE_FORMAT_ROOT: &[u8] = b"compass/store-format/v1";
const STORE_FORMAT_KEY: &[u8] = b"format";
const STORE_FORMAT_VALUE: &[u8] = br#"{"adapter":"prolly-store-sqlite","canonical_encoding":1,"history_schema":1,"typed_keys":1}"#;
type Records = Vec<(Vec<u8>, Vec<u8>)>;

/// Project-owned wrapper around the pinned SQLite Prolly adapter.
pub struct HistoryStore {
    pub(crate) root: PathBuf,
    pub(crate) repository_root: PathBuf,
    database_path: PathBuf,
    lock_path: PathBuf,
    pub(crate) prolly: Prolly<Arc<SqliteStore>>,
}

/// Opaque exact observation required to repair one corrupt preferred pointer.
pub struct CorruptPreferredToken {
    database_path: PathBuf,
    commit: CommitId,
    manifest: Tree,
}

/// Fully validated publication staged before any catalog named root becomes visible.
pub struct PreparedPublication {
    id: RealizationId,
    version: GraphVersion,
    nodes: Tree,
    edges: Tree,
    hyperedges: Tree,
    analysis: Tree,
    metadata: Tree,
    manifest: Tree,
    preferred_name: Vec<u8>,
    observed_preferred: Option<Tree>,
    observed_preferred_id: Option<RealizationId>,
    make_preferred: bool,
}

impl PreparedPublication {
    #[must_use]
    pub fn id(&self) -> &RealizationId {
        &self.id
    }

    #[must_use]
    pub fn observed_preferred(&self) -> Option<&RealizationId> {
        self.observed_preferred_id.as_ref()
    }
}

impl HistoryStore {
    /// Create or open the repository's shared history store.
    pub fn create(repository: &Repository) -> Result<Self, HistoryError> {
        let paths = HistoryPaths::create(repository)?;
        let existed = paths.database_path.exists();
        if existed {
            let guard = ActivityGuard::acquire(&paths.lock_path, false)?;
            let store = Self::open(paths)?;
            store.verify_store_format()?;
            drop(guard);
            Ok(store)
        } else {
            let guard = MaintenanceGuard::acquire(&paths.lock_path, false)?;
            let appeared_while_waiting = paths.database_path.exists();
            let store = Self::open(paths)?;
            if appeared_while_waiting {
                store.verify_store_format()?;
            } else {
                store.initialize_store_format()?;
            }
            drop(guard);
            Ok(store)
        }
    }

    /// Open an existing store without creating any file or directory.
    pub fn open_existing(repository: &Repository) -> Result<Option<Self>, HistoryError> {
        let Some(paths) = HistoryPaths::existing(repository)? else {
            return Ok(None);
        };
        let guard = ActivityGuard::acquire(&paths.lock_path, false)?;
        let store = Self::open(paths)?;
        store.verify_store_format()?;
        drop(guard);
        Ok(Some(store))
    }

    /// Return the owner-protected history resource directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the SQLite database path.
    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Acquire the shared activity lock.
    pub fn activity(&self) -> Result<ActivityGuard, HistoryError> {
        ActivityGuard::acquire(&self.lock_path, false)
    }

    /// Acquire the exclusive maintenance lock.
    pub fn maintenance(&self) -> Result<MaintenanceGuard, HistoryError> {
        MaintenanceGuard::acquire(&self.lock_path, false)
    }

    /// Publish one immutable graph realization.
    pub fn publish(&self, request: PublishRequest) -> Result<PublishedVersion, HistoryError> {
        let guard = self.activity()?;
        self.publish_with_activity(request, &guard)
    }

    /// Publish while reusing a materializer's existing activity guard.
    pub fn publish_with_activity(
        &self,
        request: PublishRequest,
        guard: &ActivityGuard,
    ) -> Result<PublishedVersion, HistoryError> {
        let prepared = self.prepare_publish_with_activity(request, guard)?;
        self.commit_prepared_with_activity(prepared, guard)
    }

    /// Stage and validate all immutable trees without publishing catalog roots.
    pub fn prepare_publish_with_activity(
        &self,
        request: PublishRequest,
        _guard: &ActivityGuard,
    ) -> Result<PreparedPublication, HistoryError> {
        request.completion.validate()?;
        let preferred_name = preferred_root_name(&request.commit);
        let observed_preferred = self.prolly.load_named_root(&preferred_name)?;
        let observed_preferred_id = if request.make_preferred {
            observed_preferred
                .as_ref()
                .map(|tree| self.validated_manifest_pointer(tree).map(|value| value.id))
                .transpose()?
        } else {
            observed_preferred.as_ref().and_then(|tree| {
                self.validated_manifest_pointer(tree)
                    .ok()
                    .map(|value| value.id)
            })
        };

        let partitioned = request.artifacts.partition(&request.completion)?;
        let node_count = count(partitioned.nodes.len())?;
        let edge_count = count(partitioned.edges.len())?;
        let hyperedge_count = count(partitioned.hyperedges.len())?;
        let analysis_count = count(partitioned.analysis.len())?;
        let metadata_count = count(partitioned.metadata.len())?;
        let nodes = self.build_tree(partitioned.nodes)?;
        let edges = self.build_tree(partitioned.edges)?;
        let hyperedges = self.build_tree(partitioned.hyperedges)?;
        let analysis = self.build_tree(partitioned.analysis)?;
        let metadata = self.build_tree(partitioned.metadata)?;
        let version = GraphVersion {
            schema_version: crate::HISTORY_SCHEMA_VERSION,
            git_commit: request.commit.to_string(),
            git_parents: request.parents.iter().map(ToString::to_string).collect(),
            extraction_fingerprint: request.fingerprint.to_string(),
            nodes_root: StoredTree::from_tree(&nodes),
            edges_root: StoredTree::from_tree(&edges),
            hyperedges_root: StoredTree::from_tree(&hyperedges),
            analysis_root: StoredTree::from_tree(&analysis),
            metadata_root: StoredTree::from_tree(&metadata),
            node_count,
            edge_count,
            hyperedge_count,
            analysis_count,
            metadata_count,
        };
        let id = RealizationId::for_version(&version)?;
        let manifest = self.build_tree(vec![(
            KeyBuilder::new().push_str("manifest").finish(),
            canonical_json_bytes(&serde_json::to_value(&version)?)?,
        )])?;

        validate_trees(
            &self.prolly,
            &id,
            &version,
            RealizationTrees {
                nodes: &nodes,
                edges: &edges,
                hyperedges: &hyperedges,
                analysis: &analysis,
                metadata: &metadata,
            },
        )?;

        Ok(PreparedPublication {
            id,
            version,
            nodes,
            edges,
            hyperedges,
            analysis,
            metadata,
            manifest,
            preferred_name,
            observed_preferred,
            observed_preferred_id,
            make_preferred: request.make_preferred,
        })
    }

    /// Atomically publish a previously staged immutable realization.
    pub fn commit_prepared_with_activity(
        &self,
        prepared: PreparedPublication,
        _guard: &ActivityGuard,
    ) -> Result<PublishedVersion, HistoryError> {
        let PreparedPublication {
            id,
            version,
            nodes,
            edges,
            hyperedges,
            analysis,
            metadata,
            manifest,
            preferred_name,
            observed_preferred,
            observed_preferred_id: _,
            make_preferred,
        } = prepared;

        self.publish_catalog_roots(
            &id,
            [
                (b"nodes".as_slice(), &nodes),
                (b"edges".as_slice(), &edges),
                (b"hyperedges".as_slice(), &hyperedges),
                (b"analysis".as_slice(), &analysis),
                (b"metadata".as_slice(), &metadata),
                (b"manifest".as_slice(), &manifest),
            ],
        )?;
        let reopened = self.get_without_activity(&id)?;
        if reopened.version != version {
            return Err(HistoryError::CorruptHistory(format!(
                "reopened realization {id} differs from its manifest"
            )));
        }

        let preferred = if make_preferred {
            matches!(
                self.prolly.compare_and_swap_named_root(
                    &preferred_name,
                    observed_preferred.as_ref(),
                    Some(&manifest),
                )?,
                NamedRootUpdate::Applied
            )
        } else {
            false
        };
        Ok(PublishedVersion {
            id,
            version,
            preferred,
        })
    }

    /// Resolve the preferred complete realization for an exact commit.
    pub fn preferred(&self, commit: &CommitId) -> Result<Option<PublishedVersion>, HistoryError> {
        let guard = self.activity()?;
        self.preferred_with_activity(commit, &guard)
    }

    pub fn preferred_with_activity(
        &self,
        commit: &CommitId,
        _guard: &ActivityGuard,
    ) -> Result<Option<PublishedVersion>, HistoryError> {
        self.preferred_without_activity(commit)
    }

    fn preferred_without_activity(
        &self,
        commit: &CommitId,
    ) -> Result<Option<PublishedVersion>, HistoryError> {
        let Some(tree) = self.prolly.load_named_root(&preferred_root_name(commit))? else {
            return Ok(None);
        };
        let mut published = self.validated_manifest_pointer(&tree)?;
        if published.version.git_commit != commit.as_str() {
            return Err(HistoryError::CorruptHistory(format!(
                "preferred pointer for {commit} targets commit {}",
                published.version.git_commit
            )));
        }
        published.preferred = true;
        Ok(Some(published))
    }

    /// Load and verify one exact immutable realization.
    pub fn get(&self, id: &RealizationId) -> Result<PublishedVersion, HistoryError> {
        let guard = self.activity()?;
        self.get_with_activity(id, &guard)
    }

    pub fn get_with_activity(
        &self,
        id: &RealizationId,
        _guard: &ActivityGuard,
    ) -> Result<PublishedVersion, HistoryError> {
        self.get_without_activity(id)
    }

    fn get_without_activity(&self, id: &RealizationId) -> Result<PublishedVersion, HistoryError> {
        let manifest_name = version_root_name(id, b"manifest");
        let manifest = self
            .prolly
            .load_named_root(&manifest_name)?
            .ok_or_else(|| HistoryError::CorruptHistory(format!("missing realization {id}")))?;
        let published = self.version_from_manifest_tree(&manifest)?;
        if &published.id != id {
            return Err(HistoryError::CorruptHistory(format!(
                "manifest name {id} contains realization {}",
                published.id
            )));
        }
        self.verify_direct_roots(id, &published.version)?;
        Ok(published)
    }

    /// List immutable realizations, optionally filtered by exact commit.
    pub fn list(&self, commit: Option<&CommitId>) -> Result<Vec<PublishedVersion>, HistoryError> {
        let _guard = self.activity()?;
        self.list_without_activity(commit)
    }

    pub(crate) fn list_without_activity(
        &self,
        commit: Option<&CommitId>,
    ) -> Result<Vec<PublishedVersion>, HistoryError> {
        let mut versions = Vec::new();
        for named in self.prolly.store().list_roots()? {
            let Ok(segments) = prolly::decode_segments(&named.name) else {
                continue;
            };
            if segments.len() != 5
                || segments[0] != b"compass"
                || segments[1] != b"v1"
                || segments[2] != b"version"
                || segments[4] != b"manifest"
            {
                continue;
            }
            let id = std::str::from_utf8(&segments[3])
                .map_err(|error| HistoryError::CorruptHistory(error.to_string()))?
                .parse()?;
            let published = self.get_without_activity(&id)?;
            if commit.is_none_or(|expected| published.version.git_commit == expected.as_str()) {
                versions.push(published);
            }
        }
        versions.sort_by(|left, right| {
            (
                &left.version.git_commit,
                &left.version.extraction_fingerprint,
                left.id.as_hex(),
            )
                .cmp(&(
                    &right.version.git_commit,
                    &right.version.extraction_fingerprint,
                    right.id.as_hex(),
                ))
        });
        for published in &mut versions {
            let commit: CommitId = published.version.git_commit.parse()?;
            published.preferred = self
                .preferred_without_activity(&commit)?
                .is_some_and(|preferred| preferred.id == published.id);
        }
        Ok(versions)
    }

    /// Move a preferred pointer only when its exact observed identity matches.
    pub fn compare_and_set_preferred(
        &self,
        commit: &CommitId,
        expected: Option<&RealizationId>,
        replacement: &RealizationId,
    ) -> Result<bool, HistoryError> {
        let _guard = self.activity()?;
        let replacement = self.get_without_activity(replacement)?;
        if replacement.version.git_commit != commit.as_str() {
            return Err(HistoryError::CorruptHistory(
                "replacement realization belongs to a different commit".to_owned(),
            ));
        }
        let replacement_tree = self
            .prolly
            .load_named_root(&version_root_name(&replacement.id, b"manifest"))?
            .ok_or_else(|| {
                HistoryError::CorruptHistory("missing replacement manifest".to_owned())
            })?;
        let expected_tree = match expected {
            Some(id) => {
                let expected = self.get_without_activity(id)?;
                if expected.version.git_commit != commit.as_str() {
                    return Err(HistoryError::CorruptHistory(
                        "expected realization belongs to a different commit".to_owned(),
                    ));
                }
                Some(
                    self.prolly
                        .load_named_root(&version_root_name(id, b"manifest"))?
                        .ok_or_else(|| {
                            HistoryError::CorruptHistory("missing expected manifest".to_owned())
                        })?,
                )
            }
            None => None,
        };
        Ok(matches!(
            self.prolly.compare_and_swap_named_root(
                &preferred_root_name(commit),
                expected_tree.as_ref(),
                Some(&replacement_tree),
            )?,
            NamedRootUpdate::Applied
        ))
    }

    /// Observe an unreadable preferred pointer for an explicit exact-CAS repair.
    pub fn corrupt_preferred_token(
        &self,
        commit: &CommitId,
    ) -> Result<CorruptPreferredToken, HistoryError> {
        let guard = self.activity()?;
        self.corrupt_preferred_token_with_activity(commit, &guard)
    }

    pub fn corrupt_preferred_token_with_activity(
        &self,
        commit: &CommitId,
        _guard: &ActivityGuard,
    ) -> Result<CorruptPreferredToken, HistoryError> {
        let manifest = self
            .prolly
            .load_named_root(&preferred_root_name(commit))?
            .ok_or_else(|| {
                HistoryError::CorruptHistory("preferred pointer is absent".to_owned())
            })?;
        match self.validated_manifest_pointer(&manifest) {
            Ok(_) => {
                return Err(HistoryError::CorruptHistory(
                    "preferred pointer is valid and cannot use corrupt recovery".to_owned(),
                ));
            }
            Err(error) if error.is_catalog_corruption() => {}
            Err(error) => return Err(error),
        }
        Ok(CorruptPreferredToken {
            database_path: self.database_path.clone(),
            commit: commit.clone(),
            manifest,
        })
    }

    /// Replace the exact corrupt pointer represented by `observed` with a validated candidate.
    pub fn recover_corrupt_preferred_with_activity(
        &self,
        commit: &CommitId,
        observed: &CorruptPreferredToken,
        replacement: &RealizationId,
        _guard: &ActivityGuard,
    ) -> Result<bool, HistoryError> {
        if observed.database_path != self.database_path || &observed.commit != commit {
            return Err(HistoryError::CorruptHistory(
                "corrupt preferred observation belongs to a different store or commit".to_owned(),
            ));
        }
        self.validate_without_activity(replacement)?;
        let replacement = self.get_without_activity(replacement)?;
        if replacement.version.git_commit != commit.as_str() {
            return Err(HistoryError::CorruptHistory(
                "replacement realization belongs to a different commit".to_owned(),
            ));
        }
        let replacement_manifest = self
            .prolly
            .load_named_root(&version_root_name(&replacement.id, b"manifest"))?
            .ok_or_else(|| {
                HistoryError::CorruptHistory("missing replacement manifest".to_owned())
            })?;
        Ok(matches!(
            self.prolly.compare_and_swap_named_root(
                &preferred_root_name(commit),
                Some(&observed.manifest),
                Some(&replacement_manifest),
            )?,
            NamedRootUpdate::Applied
        ))
    }

    /// Fully validate one cataloged realization.
    pub fn validate(&self, id: &RealizationId) -> Result<crate::ValidationReport, HistoryError> {
        let guard = self.activity()?;
        self.validate_with_activity(id, &guard)
    }

    pub fn validate_with_activity(
        &self,
        id: &RealizationId,
        _guard: &ActivityGuard,
    ) -> Result<crate::ValidationReport, HistoryError> {
        self.validate_without_activity(id)
    }

    fn validate_without_activity(
        &self,
        id: &RealizationId,
    ) -> Result<crate::ValidationReport, HistoryError> {
        let published = self.get_without_activity(id)?;
        let nodes = self.load_realization_root(id, b"nodes")?;
        let edges = self.load_realization_root(id, b"edges")?;
        let hyperedges = self.load_realization_root(id, b"hyperedges")?;
        let analysis = self.load_realization_root(id, b"analysis")?;
        let metadata = self.load_realization_root(id, b"metadata")?;
        validate_trees(
            &self.prolly,
            id,
            &published.version,
            RealizationTrees {
                nodes: &nodes,
                edges: &edges,
                hyperedges: &hyperedges,
                analysis: &analysis,
                metadata: &metadata,
            },
        )
    }

    /// Reconstruct all authoritative content for one validated realization.
    pub fn artifacts(
        &self,
        id: &RealizationId,
    ) -> Result<crate::CompletedGraphArtifacts, HistoryError> {
        let guard = self.activity()?;
        self.artifacts_with_activity(id, &guard)
    }

    pub fn artifacts_with_activity(
        &self,
        id: &RealizationId,
        _guard: &ActivityGuard,
    ) -> Result<crate::CompletedGraphArtifacts, HistoryError> {
        self.validate_without_activity(id)?;
        let partitioned = crate::PartitionedGraph {
            nodes: self.read_tree(&self.load_realization_root(id, b"nodes")?)?,
            edges: self.read_tree(&self.load_realization_root(id, b"edges")?)?,
            hyperedges: self.read_tree(&self.load_realization_root(id, b"hyperedges")?)?,
            analysis: self.read_tree(&self.load_realization_root(id, b"analysis")?)?,
            metadata: self.read_tree(&self.load_realization_root(id, b"metadata")?)?,
        };
        crate::CompletedGraphArtifacts::reconstruct(&partitioned)
    }

    /// Measure logical Prolly-node reuse between two complete realizations.
    ///
    /// This deliberately uses content reachability rather than adapter-private
    /// SQLite tables or the physical database file size.
    pub fn structural_sharing(
        &self,
        first: &RealizationId,
        second: &RealizationId,
    ) -> Result<StructuralSharing, HistoryError> {
        let _guard = self.activity()?;
        self.get_without_activity(first)?;
        self.get_without_activity(second)?;
        let roots = |id: &RealizationId| -> Result<Vec<Tree>, HistoryError> {
            [
                b"nodes".as_slice(),
                b"edges".as_slice(),
                b"hyperedges".as_slice(),
                b"analysis".as_slice(),
                b"metadata".as_slice(),
                b"manifest".as_slice(),
            ]
            .into_iter()
            .map(|kind| self.load_realization_root(id, kind))
            .collect()
        };
        let first_roots = roots(first)?;
        let second_roots = roots(second)?;
        let first = self.prolly.mark_reachable(&first_roots)?;
        let second = self.prolly.mark_reachable(&second_roots)?;
        let union = self.prolly.mark_reachable(
            &first_roots
                .into_iter()
                .chain(second_roots)
                .collect::<Vec<_>>(),
        )?;
        Ok(StructuralSharing {
            first_total_nodes: first.live_nodes,
            second_total_nodes: second.live_nodes,
            union_nodes: union.live_nodes,
            shared_nodes: first
                .live_nodes
                .saturating_add(second.live_nodes)
                .saturating_sub(union.live_nodes),
            first_total_bytes: first.live_bytes,
            second_total_bytes: second.live_bytes,
            union_bytes: union.live_bytes,
            shared_bytes: first
                .live_bytes
                .saturating_add(second.live_bytes)
                .saturating_sub(union.live_bytes),
        })
    }

    fn build_tree(&self, entries: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Tree, HistoryError> {
        let mut builder = BatchBuilder::new(self.prolly.store().clone(), Config::default());
        for (key, value) in entries {
            builder.add(key, value);
        }
        builder.build().map_err(HistoryError::from)
    }

    fn read_tree(&self, tree: &Tree) -> Result<Records, HistoryError> {
        self.prolly
            .range(tree, &[], None)?
            .map(|entry| entry.map_err(HistoryError::from))
            .collect()
    }

    fn load_realization_root(
        &self,
        id: &RealizationId,
        kind: &'static [u8],
    ) -> Result<Tree, HistoryError> {
        self.prolly
            .load_named_root(&version_root_name(id, kind))?
            .ok_or_else(|| {
                let kind = match kind {
                    b"nodes" => "nodes",
                    b"edges" => "edges",
                    b"hyperedges" => "hyperedges",
                    b"analysis" => "analysis",
                    b"metadata" => "metadata",
                    _ => "unknown",
                };
                HistoryError::InvalidRealization(vec![crate::ValidationProblem::MissingRoot(kind)])
            })
    }

    fn publish_catalog_roots(
        &self,
        id: &RealizationId,
        roots: [(&[u8], &Tree); 6],
    ) -> Result<(), HistoryError> {
        let transaction = self.prolly.begin_transaction()?;
        for &(kind, tree) in &roots {
            let name = version_root_name(id, kind);
            match transaction.load_named_root(&name)? {
                Some(existing) if existing == *tree => {}
                Some(_) => {
                    return Err(HistoryError::CorruptHistory(format!(
                        "immutable root collision for {id}/{}",
                        String::from_utf8_lossy(kind)
                    )));
                }
                None => transaction.publish_named_root(&name, tree)?,
            }
        }
        match transaction.commit()? {
            TransactionUpdate::Applied { .. } => Ok(()),
            TransactionUpdate::Conflict(_) => {
                for &(kind, expected) in &roots {
                    let actual = self.prolly.load_named_root(&version_root_name(id, kind))?;
                    if actual.as_ref() != Some(expected) {
                        return Err(HistoryError::CorruptHistory(format!(
                            "realization catalog conflict for {id}/{}",
                            String::from_utf8_lossy(kind)
                        )));
                    }
                }
                Ok(())
            }
        }
    }

    fn version_from_manifest_tree(
        &self,
        manifest: &Tree,
    ) -> Result<PublishedVersion, HistoryError> {
        let key = KeyBuilder::new().push_str("manifest").finish();
        let bytes = self.prolly.get(manifest, &key)?.ok_or_else(|| {
            HistoryError::CorruptHistory("manifest tree has no manifest record".to_owned())
        })?;
        let version: GraphVersion = serde_json::from_slice(&bytes)?;
        if version.schema_version != crate::HISTORY_SCHEMA_VERSION {
            return Err(HistoryError::CorruptHistory(format!(
                "unsupported realization schema {}",
                version.schema_version
            )));
        }
        let _: CommitId = version.git_commit.parse()?;
        for parent in &version.git_parents {
            let _: CommitId = parent.parse()?;
        }
        let _: crate::ExtractionFingerprint = version.extraction_fingerprint.parse()?;
        let id = RealizationId::for_version(&version)?;
        Ok(PublishedVersion {
            id,
            version,
            preferred: false,
        })
    }

    fn validated_manifest_pointer(
        &self,
        manifest: &Tree,
    ) -> Result<PublishedVersion, HistoryError> {
        let parsed = self.version_from_manifest_tree(manifest)?;
        let catalog = self.get_without_activity(&parsed.id)?;
        let catalog_tree = self
            .prolly
            .load_named_root(&version_root_name(&parsed.id, b"manifest"))?
            .ok_or_else(|| HistoryError::CorruptHistory("missing catalog manifest".to_owned()))?;
        if &catalog_tree != manifest {
            return Err(HistoryError::CorruptHistory(
                "preferred pointer does not match its immutable catalog manifest".to_owned(),
            ));
        }
        Ok(catalog)
    }

    fn verify_direct_roots(
        &self,
        id: &RealizationId,
        version: &GraphVersion,
    ) -> Result<(), HistoryError> {
        for (kind, expected) in [
            (b"nodes".as_slice(), &version.nodes_root),
            (b"edges".as_slice(), &version.edges_root),
            (b"hyperedges".as_slice(), &version.hyperedges_root),
            (b"analysis".as_slice(), &version.analysis_root),
            (b"metadata".as_slice(), &version.metadata_root),
        ] {
            let actual = self
                .prolly
                .load_named_root(&version_root_name(id, kind))?
                .ok_or_else(|| {
                    HistoryError::CorruptHistory(format!(
                        "missing immutable root {id}/{}",
                        String::from_utf8_lossy(kind)
                    ))
                })?;
            if actual != expected.to_tree() {
                return Err(HistoryError::CorruptHistory(format!(
                    "immutable root {id}/{} differs from the manifest",
                    String::from_utf8_lossy(kind)
                )));
            }
        }
        Ok(())
    }

    fn open(paths: HistoryPaths) -> Result<Self, HistoryError> {
        reject_symlink(&paths.database_path, true)?;
        let backend = Arc::new(SqliteStore::open_with_config(
            &paths.database_path,
            SqliteStoreConfig {
                busy_timeout_ms: 10_000,
                enable_wal: true,
                synchronous_normal: false,
            },
        )?);
        set_owner_file(&paths.database_path)?;
        secure_sqlite_sidecars(&paths.database_path)?;
        reject_symlink(&paths.database_path, false)?;
        Ok(Self {
            root: paths.root,
            repository_root: paths.repository_root,
            database_path: paths.database_path,
            lock_path: paths.lock_path,
            prolly: Prolly::new(backend, Config::default()),
        })
    }

    fn initialize_store_format(&self) -> Result<(), HistoryError> {
        let tree = self.prolly.put(
            &self.prolly.create(),
            STORE_FORMAT_KEY.to_vec(),
            STORE_FORMAT_VALUE.to_vec(),
        )?;
        match self
            .prolly
            .compare_and_swap_named_root(STORE_FORMAT_ROOT, None, Some(&tree))?
        {
            NamedRootUpdate::Applied => Ok(()),
            NamedRootUpdate::Conflict { .. } => self.verify_store_format(),
        }
    }

    fn verify_store_format(&self) -> Result<(), HistoryError> {
        let tree = self
            .prolly
            .load_named_root(STORE_FORMAT_ROOT)?
            .ok_or(HistoryError::IncompatibleStoreFormat)?;
        let value = self.prolly.get(&tree, STORE_FORMAT_KEY)?;
        if value.as_deref() == Some(STORE_FORMAT_VALUE) {
            Ok(())
        } else {
            Err(HistoryError::IncompatibleStoreFormat)
        }
    }
}

pub(crate) fn version_root_name(id: &RealizationId, kind: &[u8]) -> Vec<u8> {
    root_name(&[b"compass", b"v1", b"version", id.as_hex().as_bytes(), kind])
}

fn preferred_root_name(commit: &CommitId) -> Vec<u8> {
    root_name(&[b"compass", b"v1", b"preferred", commit.as_str().as_bytes()])
}

fn count(value: usize) -> Result<u64, HistoryError> {
    u64::try_from(value)
        .map_err(|_| HistoryError::InvalidArtifacts("record count exceeds u64".to_owned()))
}

struct HistoryPaths {
    root: PathBuf,
    repository_root: PathBuf,
    database_path: PathBuf,
    lock_path: PathBuf,
}

impl HistoryPaths {
    fn create(repository: &Repository) -> Result<Self, HistoryError> {
        let root = repository.common_dir().join("compass");
        create_owner_dir(&root)?;
        let locks = root.join("locks");
        create_owner_dir(&locks)?;
        let lock_path = locks.join("maintenance.lock");
        create_owner_file(&lock_path)?;
        Ok(Self {
            database_path: root.join("history.sqlite"),
            root,
            repository_root: repository.root().to_path_buf(),
            lock_path,
        })
    }

    fn existing(repository: &Repository) -> Result<Option<Self>, HistoryError> {
        let root = repository.common_dir().join("compass");
        if !root.exists() {
            reject_symlink(&root, true)?;
            return Ok(None);
        }
        reject_directory(&root)?;
        let database_path = root.join("history.sqlite");
        if !database_path.exists() {
            reject_symlink(&database_path, true)?;
            return Ok(None);
        }
        reject_regular_file(&database_path)?;
        let locks = root.join("locks");
        reject_directory(&locks)?;
        let lock_path = locks.join("maintenance.lock");
        reject_regular_file(&lock_path)?;
        Ok(Some(Self {
            root,
            repository_root: repository.root().to_path_buf(),
            database_path,
            lock_path,
        }))
    }
}

pub(crate) fn create_owner_dir(path: &Path) -> Result<(), HistoryError> {
    reject_symlink(path, true)?;
    if !path.exists() {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        if let Err(source) = builder.create(path)
            && source.kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(crate::error::io_error(path, source));
        }
    }
    reject_directory(path)?;
    set_owner_dir(path)
}

fn create_owner_file(path: &Path) -> Result<(), HistoryError> {
    reject_symlink(path, true)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| crate::error::io_error(path, source))?;
    reject_regular_file(path)?;
    set_owner_file(path)
}

pub(crate) fn reject_directory(path: &Path) -> Result<(), HistoryError> {
    reject_symlink(path, false)?;
    let metadata = fs::metadata(path).map_err(|source| crate::error::io_error(path, source))?;
    if metadata.is_dir() {
        Ok(())
    } else {
        Err(HistoryError::UnsafePath {
            path: path.to_path_buf(),
            reason: "expected a directory".to_owned(),
        })
    }
}

fn reject_regular_file(path: &Path) -> Result<(), HistoryError> {
    reject_symlink(path, false)?;
    let metadata = fs::metadata(path).map_err(|source| crate::error::io_error(path, source))?;
    if metadata.is_file() {
        Ok(())
    } else {
        Err(HistoryError::UnsafePath {
            path: path.to_path_buf(),
            reason: "expected a regular file".to_owned(),
        })
    }
}

pub(crate) fn reject_symlink(path: &Path, missing_ok: bool) -> Result<(), HistoryError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(HistoryError::UnsafePath {
            path: path.to_path_buf(),
            reason: "symbolic links are not allowed".to_owned(),
        }),
        Ok(_) => Ok(()),
        Err(error) if missing_ok && error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(crate::error::io_error(path, source)),
    }
}

#[cfg(unix)]
fn set_owner_dir(path: &Path) -> Result<(), HistoryError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|source| crate::error::io_error(path, source))
}

#[cfg(not(unix))]
fn set_owner_dir(_path: &Path) -> Result<(), HistoryError> {
    Ok(())
}

#[cfg(unix)]
pub(crate) fn set_owner_file(path: &Path) -> Result<(), HistoryError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|source| crate::error::io_error(path, source))
}

#[cfg(not(unix))]
pub(crate) fn set_owner_file(_path: &Path) -> Result<(), HistoryError> {
    Ok(())
}

fn secure_sqlite_sidecars(database_path: &Path) -> Result<(), HistoryError> {
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", database_path.display()));
        if sidecar.exists() {
            reject_regular_file(&sidecar)?;
            set_owner_file(&sidecar)?;
        } else {
            reject_symlink(&sidecar, true)?;
        }
    }
    Ok(())
}
