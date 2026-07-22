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
    seeds: Mutex<Vec<Option<String>>>,
}

impl RecordingBuilder {
    fn seeds(&self) -> Result<Vec<Option<String>>, MaterializeError> {
        self.seeds
            .lock()
            .map(|values| values.clone())
            .map_err(|error| MaterializeError::Builder(error.to_string()))
    }
}

impl CompleteGraphBuilder for RecordingBuilder {
    fn build(
        &self,
        checkout: &Path,
        _output_root: &Path,
        seed: Option<&GraphArtifacts>,
    ) -> Result<CompletedGraphArtifacts, MaterializeError> {
        let seed_commit = seed.and_then(|artifacts| {
            artifacts
                .document
                .extras
                .get("built_at_commit")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
        self.seeds
            .lock()
            .map_err(|error| MaterializeError::Builder(error.to_string()))?
            .push(seed_commit);
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
        })
    }
}

fn request(repository: &Repository, commit: CommitId, rebuild: bool) -> MaterializeRequest {
    MaterializeRequest {
        repository: repository.clone(),
        commit,
        profile: BuildProfile::default(),
        rebuild,
        replace_corrupt: false,
    }
}

#[test]
fn materializer_reuses_preferred_ancestor_and_publishes_target()
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
        request(&repository, parent.clone(), false),
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
        request(&repository, target.clone(), false),
        &mut Observer(&mut phases),
    )?;
    assert_eq!(published.version.git_commit, target.to_string());
    assert!(published.preferred);
    assert_eq!(builder.seeds()?, vec![None, Some(parent.to_string())]);
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

    let before = builder.seeds()?.len();
    let existing = materialize_history(&store, &builder, request(&repository, target, false))?;
    assert_eq!(existing.id, published.id);
    assert_eq!(builder.seeds()?.len(), before);

    let mut invalid_recovery = request(&repository, parent, true);
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
            _seed: Option<&GraphArtifacts>,
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
            request(&repository, commit.clone(), false)
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
            _seed: Option<&GraphArtifacts>,
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
        request(&repository, commit.clone(), false),
    )
    .err()
    .ok_or("incomplete semantic manifest unexpectedly published")?;
    assert!(error.to_string().contains("design.md"));
    assert!(store.preferred(&commit)?.is_none());
    assert!(store.list(Some(&commit))?.is_empty());
    Ok(())
}
