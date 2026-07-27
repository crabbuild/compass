use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use compass_core::{
    CompleteGraphBuilder, MaterializeError, MaterializeObserver, MaterializeRequest,
    MaterializeStage, materialize_history, materialize_history_with_observer,
};
use compass_history::{
    BuildProfile, CommitId, CompletedGraphArtifacts, CompletionEvidence, GraphArtifacts,
    HistoryStore, Repository,
};
use compass_model::GraphDocument;
use serde_json::json;

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

#[derive(Default)]
struct RecordingBuilder {
    builds: Mutex<usize>,
    promotion: Option<CompletedGraphArtifacts>,
}

impl RecordingBuilder {
    fn builds(&self) -> Result<usize, MaterializeError> {
        self.builds
            .lock()
            .map(|value| *value)
            .map_err(|error| MaterializeError::Builder(error.to_string()))
    }

    fn promoting(commit: &CommitId) -> Result<Self, Box<dyn std::error::Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "directed": true,
            "multigraph": false,
            "nodes": [{"id":"promoted","label":"Promoted","source_file":"service.rs"}],
            "links": [],
            "built_at_commit": commit
        }))?;
        Ok(Self {
            builds: Mutex::default(),
            promotion: Some(CompletedGraphArtifacts {
                artifacts: GraphArtifacts {
                    document,
                    program: None,
                    analysis: None,
                    labels: None,
                    manifest: Some(json!({"service.rs":{"ast_hash":"fixture"}})),
                    authoritative_sidecars: Default::default(),
                },
                completion: CompletionEvidence {
                    extraction_succeeded: true,
                    allow_partial: false,
                    semantic_files_expected: 0,
                    semantic_files_completed: 0,
                    failed_chunks: 0,
                },
            }),
        })
    }
}

impl CompleteGraphBuilder for RecordingBuilder {
    fn promote_current(
        &self,
        _repository_root: &Path,
        _commit: &CommitId,
    ) -> Result<Option<CompletedGraphArtifacts>, MaterializeError> {
        Ok(self.promotion.clone())
    }

    fn build(
        &self,
        checkout: &Path,
        _output_root: &Path,
    ) -> Result<CompletedGraphArtifacts, MaterializeError> {
        let mut builds = self
            .builds
            .lock()
            .map_err(|error| MaterializeError::Builder(error.to_string()))?;
        *builds += 1;
        let commit = git(checkout, &["rev-parse", "HEAD"])
            .map_err(|error| MaterializeError::Builder(error.to_string()))?;
        let source = std::fs::read_to_string(checkout.join("service.rs"))
            .map_err(|error| MaterializeError::Builder(error.to_string()))?;
        let id = if source.contains("new") { "new" } else { "old" };
        let document: GraphDocument = serde_json::from_value(json!({
            "directed":true,
            "multigraph":false,
            "nodes":[{"id":id,"label":id,"source_file":"service.rs"}],
            "links":[],
            "built_at_commit":commit
        }))
        .map_err(|error| MaterializeError::Builder(error.to_string()))?;
        Ok(CompletedGraphArtifacts {
            artifacts: GraphArtifacts {
                document,
                program: None,
                analysis: None,
                labels: None,
                manifest: Some(json!({
                    "service.rs":{"ast_hash":"fixture"},
                    "settings.toml":{"ast_hash":"fixture"},
                    "config/tsconfig.json":{"ast_hash":"fixture"}
                })),
                authoritative_sidecars: Default::default(),
            },
            completion: CompletionEvidence {
                extraction_succeeded: true,
                allow_partial: false,
                semantic_files_expected: 0,
                semantic_files_completed: 0,
                failed_chunks: 0,
            },
        })
    }
}

#[test]
fn current_snapshot_is_published_without_invoking_the_exact_builder()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("service.rs"), "fn service() {}\n")?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "service"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let history = HistoryStore::create(&repository)?;
    let builder = RecordingBuilder::promoting(&commit)?;

    let published = materialize_history(
        &history,
        &builder,
        request(&repository, commit.clone(), false)?,
    )?;

    assert!(builder.builds()? == 0, "exact builder unexpectedly ran");
    let artifacts = history.artifacts(&published.id)?;
    assert_eq!(artifacts.artifacts.document.nodes[0].id, "promoted");
    let inventory: serde_json::Value = serde_json::from_slice(
        artifacts
            .artifacts
            .authoritative_sidecars
            .get(".compass_source_inventory.json")
            .ok_or("source inventory was not published")?,
    )?;
    assert_eq!(inventory["schema"], "compass.history.source_inventory/1");
    assert_eq!(inventory["code_files"]["service.rs"]["status"], "extracted");
    assert!(
        inventory["code_files"]["service.rs"]["git_object"]
            .as_str()
            .is_some_and(|object| object.len() >= 40)
    );
    assert_eq!(published.version.git_commit, commit.to_string());
    Ok(())
}

#[test]
fn exact_materialization_rejects_a_code_file_without_an_ast_completion_stamp()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("service.rs"), "fn service() {}\n")?;
    std::fs::write(directory.path().join("omitted.py"), "def omitted(): pass\n")?;
    git(directory.path(), &["add", "."])?;
    git(directory.path(), &["commit", "--quiet", "-m", "sources"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let history = HistoryStore::create(&repository)?;

    let error = materialize_history(
        &history,
        &RecordingBuilder::default(),
        request(&repository, commit.clone(), false)?,
    )
    .err()
    .ok_or("materialization unexpectedly accepted an omitted code source")?;

    assert!(error.to_string().contains("omitted.py"));
    assert!(history.preferred(&commit)?.is_none());
    Ok(())
}

#[test]
fn explicit_rebuild_bypasses_current_snapshot_promotion() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("service.rs"), "fn service() {}\n")?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "service"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let history = HistoryStore::create(&repository)?;
    let builder = RecordingBuilder::promoting(&commit)?;

    let published = materialize_history(&history, &builder, request(&repository, commit, true)?)?;

    assert_eq!(builder.builds()?, 1);
    let artifacts = history.artifacts(&published.id)?;
    assert_eq!(artifacts.artifacts.document.nodes[0].id, "old");
    Ok(())
}

fn request(
    repository: &Repository,
    commit: CommitId,
    rebuild: bool,
) -> Result<MaterializeRequest, compass_history::HistoryError> {
    let mut profile = BuildProfile::default();
    profile.insert("graph_schema", compass_history::HISTORY_GRAPH_SCHEMA)?;
    Ok(MaterializeRequest {
        repository: repository.clone(),
        commit,
        profile,
        rebuild,
        replace_corrupt: false,
    })
}

#[test]
fn materializer_builds_target_without_reconstructing_an_ancestor()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("service.rs"), "fn old() {}\n")?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "old"])?;
    let repository = Repository::discover(directory.path())?;
    let parent = repository.resolve("HEAD")?;
    std::fs::write(directory.path().join("service.rs"), "fn new() {}\n")?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "new"])?;
    let target = repository.resolve("HEAD")?;
    let store = HistoryStore::create(&repository)?;
    let builder = RecordingBuilder::default();

    materialize_history(
        &store,
        &builder,
        request(&repository, parent.clone(), false)?,
    )?;
    let mut phases = Vec::new();
    struct Observer<'a>(&'a mut Vec<MaterializeStage>);
    impl MaterializeObserver for Observer<'_> {
        fn entered(&mut self, stage: MaterializeStage) -> Result<(), MaterializeError> {
            self.0.push(stage);
            Ok(())
        }
    }
    let published = materialize_history_with_observer(
        &store,
        &builder,
        request(&repository, target.clone(), false)?,
        &mut Observer(&mut phases),
    )?;
    assert_eq!(published.version.git_commit, target.to_string());
    assert!(published.preferred);
    assert_eq!(builder.builds()?, 2);
    assert_eq!(
        phases,
        [
            MaterializeStage::Building,
            MaterializeStage::Validating,
            MaterializeStage::Publishing
        ]
    );
    assert_eq!(
        store.preferred(&target)?.map(|value| value.id),
        Some(published.id.clone())
    );

    let before = builder.builds()?;
    let existing = materialize_history(&store, &builder, request(&repository, target, false)?)?;
    assert_eq!(existing.id, published.id);
    assert_eq!(builder.builds()?, before);

    let mut invalid_recovery = request(&repository, parent, true)?;
    invalid_recovery.replace_corrupt = true;
    assert!(matches!(
        materialize_history(&store, &builder, invalid_recovery),
        Err(MaterializeError::ReplaceCorruptNotApplicable)
    ));
    Ok(())
}

#[test]
fn incomplete_builder_output_is_never_published() -> Result<(), Box<dyn std::error::Error>> {
    struct IncompleteBuilder;
    impl CompleteGraphBuilder for IncompleteBuilder {
        fn build(
            &self,
            _checkout: &Path,
            _output_root: &Path,
        ) -> Result<CompletedGraphArtifacts, MaterializeError> {
            Err(MaterializeError::Incomplete("fixture stopped".to_owned()))
        }
    }

    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("service.rs"), "fn service() {}\n")?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "service"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let store = HistoryStore::create(&repository)?;
    assert!(
        materialize_history(
            &store,
            &IncompleteBuilder,
            request(&repository, commit.clone(), false)?
        )
        .is_err()
    );
    assert!(store.preferred(&commit)?.is_none());
    assert!(store.list(Some(&commit))?.is_empty());
    Ok(())
}

#[test]
fn semantic_manifest_must_cover_each_exact_commit_source() -> Result<(), Box<dyn std::error::Error>>
{
    struct MissingSemanticManifestBuilder;
    impl CompleteGraphBuilder for MissingSemanticManifestBuilder {
        fn build(
            &self,
            checkout: &Path,
            _output_root: &Path,
        ) -> Result<CompletedGraphArtifacts, MaterializeError> {
            let commit = git(checkout, &["rev-parse", "HEAD"])
                .map_err(|error| MaterializeError::Builder(error.to_string()))?;
            let document = serde_json::from_value(json!({
                "directed": true,
                "multigraph": false,
                "nodes": [],
                "links": [],
                "built_at_commit": commit
            }))
            .map_err(|error| MaterializeError::Builder(error.to_string()))?;
            Ok(CompletedGraphArtifacts {
                artifacts: GraphArtifacts {
                    document,
                    program: None,
                    analysis: None,
                    labels: None,
                    manifest: Some(json!({
                        "unrelated.rs": {"semantic_hash": "not-the-document"}
                    })),
                    authoritative_sidecars: Default::default(),
                },
                completion: CompletionEvidence {
                    extraction_succeeded: true,
                    allow_partial: false,
                    semantic_files_expected: 1,
                    semantic_files_completed: 1,
                    failed_chunks: 0,
                },
            })
        }
    }

    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("design.md"), "# Design\n")?;
    git(directory.path(), &["add", "design.md"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "design"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let store = HistoryStore::create(&repository)?;
    let error = materialize_history(
        &store,
        &MissingSemanticManifestBuilder,
        request(&repository, commit.clone(), false)?,
    )
    .err()
    .ok_or("incomplete semantic manifest unexpectedly published")?;
    assert!(error.to_string().contains("design.md"));
    assert!(store.preferred(&commit)?.is_none());
    assert!(store.list(Some(&commit))?.is_empty());
    Ok(())
}

#[test]
fn observers_can_abort_every_materialization_boundary_without_publication()
-> Result<(), Box<dyn std::error::Error>> {
    #[derive(Clone, Copy)]
    enum FailurePoint {
        Resolved,
        Building,
        Validating,
        Publishing,
        Candidate,
    }
    struct FailingObserver(FailurePoint);
    impl MaterializeObserver for FailingObserver {
        fn entered(&mut self, stage: MaterializeStage) -> Result<(), MaterializeError> {
            let matches = matches!(
                (self.0, stage),
                (FailurePoint::Building, MaterializeStage::Building)
                    | (FailurePoint::Validating, MaterializeStage::Validating)
                    | (FailurePoint::Publishing, MaterializeStage::Publishing)
            );
            if matches {
                Err(MaterializeError::Observer("fixture stage".to_owned()))
            } else {
                Ok(())
            }
        }
        fn resolved(
            &mut self,
            _fingerprint: &compass_history::ExtractionFingerprint,
        ) -> Result<(), MaterializeError> {
            if matches!(self.0, FailurePoint::Resolved) {
                Err(MaterializeError::Observer("fixture resolution".to_owned()))
            } else {
                Ok(())
            }
        }
        fn candidate(
            &mut self,
            _candidate: &compass_history::RealizationId,
            _observed_preferred: Option<&compass_history::RealizationId>,
        ) -> Result<(), MaterializeError> {
            if matches!(self.0, FailurePoint::Candidate) {
                Err(MaterializeError::Observer("fixture candidate".to_owned()))
            } else {
                Ok(())
            }
        }
    }

    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("service.rs"), "fn service() {}\n")?;
    std::fs::write(directory.path().join("settings.toml"), "mode = \"deep\"\n")?;
    std::fs::create_dir(directory.path().join("config"))?;
    std::fs::write(
        directory.path().join("config/tsconfig.json"),
        "{\"compilerOptions\":{}}\n",
    )?;
    git(directory.path(), &["add", "."])?;
    git(directory.path(), &["commit", "--quiet", "-m", "service"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let store = HistoryStore::create(&repository)?;
    let builder = RecordingBuilder::default();
    for point in [
        FailurePoint::Resolved,
        FailurePoint::Building,
        FailurePoint::Validating,
        FailurePoint::Publishing,
        FailurePoint::Candidate,
    ] {
        let result = materialize_history_with_observer(
            &store,
            &builder,
            request(&repository, commit.clone(), false)?,
            &mut FailingObserver(point),
        );
        assert!(matches!(result, Err(MaterializeError::Observer(_))));
        assert!(store.preferred(&commit)?.is_none());
    }
    Ok(())
}

#[test]
fn exact_tree_validation_rejects_commit_inventory_and_manifest_mismatches()
-> Result<(), Box<dyn std::error::Error>> {
    #[derive(Clone, Copy)]
    enum InvalidOutput {
        Commit,
        Inventory,
        ManifestShape,
    }
    struct InvalidBuilder(InvalidOutput);
    impl CompleteGraphBuilder for InvalidBuilder {
        fn build(
            &self,
            checkout: &Path,
            _output_root: &Path,
        ) -> Result<CompletedGraphArtifacts, MaterializeError> {
            let commit = git(checkout, &["rev-parse", "HEAD"])
                .map_err(|error| MaterializeError::Builder(error.to_string()))?;
            let built_at = if matches!(self.0, InvalidOutput::Commit) {
                "0000000000000000000000000000000000000000".to_owned()
            } else {
                commit
            };
            let document = serde_json::from_value(json!({
                "nodes": [], "links": [], "built_at_commit": built_at
            }))
            .map_err(|error| MaterializeError::Builder(error.to_string()))?;
            let expected = if matches!(self.0, InvalidOutput::Inventory) {
                0
            } else {
                1
            };
            let manifest = if matches!(self.0, InvalidOutput::ManifestShape) {
                json!([])
            } else {
                json!({"design.md":{"semantic_hash":"complete"}})
            };
            Ok(CompletedGraphArtifacts {
                artifacts: GraphArtifacts {
                    document,
                    program: None,
                    analysis: None,
                    labels: None,
                    manifest: Some(manifest),
                    authoritative_sidecars: Default::default(),
                },
                completion: CompletionEvidence {
                    extraction_succeeded: true,
                    allow_partial: false,
                    semantic_files_expected: expected,
                    semantic_files_completed: expected,
                    failed_chunks: 0,
                },
            })
        }
    }

    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("design.md"), "# Design\n")?;
    git(directory.path(), &["add", "design.md"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "design"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let store = HistoryStore::create(&repository)?;
    for variant in [
        InvalidOutput::Commit,
        InvalidOutput::Inventory,
        InvalidOutput::ManifestShape,
    ] {
        let result = materialize_history(
            &store,
            &InvalidBuilder(variant),
            request(&repository, commit.clone(), false)?,
        );
        assert!(matches!(result, Err(MaterializeError::Incomplete(_))));
        assert!(store.preferred(&commit)?.is_none());
    }
    Ok(())
}
