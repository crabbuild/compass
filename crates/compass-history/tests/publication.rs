use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use compass_history::{
    CommitId, CompletionEvidence, ExtractionFingerprint, GraphArtifacts, HistoryStore,
    PublishRequest, Repository,
};
use compass_model::GraphDocument;
use serde_json::json;

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        git(directory.path(), &["init", "--quiet"])?;
        git(directory.path(), &["config", "user.name", "Compass Test"])?;
        git(
            directory.path(),
            &["config", "user.email", "compass@example.invalid"],
        )?;
        std::fs::write(directory.path().join("README.md"), "fixture\n")?;
        git(directory.path(), &["add", "README.md"])?;
        git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;
        let path = directory.path().to_path_buf();
        Ok(Self {
            _directory: directory,
            path,
        })
    }
}

fn git(directory: &Path, arguments: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned().into())
    }
}

fn request(
    fingerprint: char,
    extra_node: bool,
) -> Result<PublishRequest, Box<dyn std::error::Error>> {
    let mut nodes = vec![json!({"id": "a", "label": "A"})];
    let mut links = Vec::new();
    if extra_node {
        nodes.push(json!({"id": "b", "label": "B"}));
        links.push(json!({"source": "a", "target": "b", "relation": "calls"}));
    }
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "nodes": nodes,
        "links": links
    }))?;
    Ok(PublishRequest {
        commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?,
        parents: vec!["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".parse()?],
        fingerprint: std::iter::repeat_n(fingerprint, 64)
            .collect::<String>()
            .parse::<ExtractionFingerprint>()?,
        artifacts: GraphArtifacts {
            document,
            analysis: Some(json!({"score": u8::from(extra_node)})),
            labels: None,
            manifest: Some(json!({"complete": true})),
            authoritative_sidecars: BTreeMap::new(),
        },
        completion: CompletionEvidence {
            extraction_succeeded: true,
            allow_partial: false,
            semantic_files_expected: 1,
            semantic_files_completed: 1,
            failed_chunks: 0,
        },
        make_preferred: true,
    })
}

#[test]
fn publication_is_atomic_reopenable_and_content_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let publish = request('a', false)?;
    let first = history.publish(publish.clone())?;
    let second = history.publish(publish)?;
    assert_eq!(first.id, second.id);
    assert!(first.preferred && second.preferred);
    drop(history);

    let reopened = HistoryStore::open_existing(&repository)?
        .ok_or_else(|| std::io::Error::other("history store missing"))?;
    let commit: CommitId = first.version.git_commit.parse()?;
    assert_eq!(
        reopened
            .preferred(&commit)?
            .ok_or_else(|| std::io::Error::other("preferred realization missing"))?
            .id,
        first.id
    );
    assert_eq!(reopened.get(&first.id)?.version, first.version);
    assert_eq!(reopened.list(None)?.len(), 1);
    Ok(())
}

#[test]
fn multiple_realizations_remain_addressable_and_preference_uses_cas()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let first = history.publish(request('a', false)?)?;
    let second = history.publish(request('b', true)?)?;
    assert_ne!(first.id, second.id);
    let commit: CommitId = first.version.git_commit.parse()?;
    let listed = history.list(Some(&commit))?;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed.iter().filter(|version| version.preferred).count(), 1);
    assert_eq!(
        history
            .preferred(&commit)?
            .ok_or_else(|| std::io::Error::other("preferred realization missing"))?
            .id,
        second.id
    );

    assert!(!history.compare_and_set_preferred(&commit, None, &first.id)?);
    assert!(history.compare_and_set_preferred(&commit, Some(&second.id), &first.id)?);
    assert_eq!(
        history
            .preferred(&commit)?
            .ok_or_else(|| std::io::Error::other("preferred realization missing"))?
            .id,
        first.id
    );
    assert!(history.get(&second.id).is_ok());
    Ok(())
}

#[test]
fn validation_rejects_missing_endpoints_before_catalog_publication()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let mut invalid = request('c', true)?;
    invalid.artifacts.document.links[0].target = "missing".to_owned();
    let error = match history.publish(invalid) {
        Ok(_) => return Err("missing endpoint unexpectedly published".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("MissingEdgeEndpoint"));
    assert!(history.list(None)?.is_empty());

    let valid = history.publish(request('d', true)?)?;
    let report = history.validate(&valid.id)?;
    assert_eq!((report.nodes, report.edges), (2, 1));
    Ok(())
}

#[test]
fn gc_keeps_all_published_versions_and_removes_orphans() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let first = history.publish(request('a', false)?)?;
    let second = history.publish(request('b', true)?)?;
    let activity = history.activity()?;
    let orphan = history.prepare_publish_with_activity(request('c', false)?, &activity)?;
    drop(orphan);
    drop(activity);

    let plan = history.plan_gc(false)?;
    assert!(plan.reclaimable_nodes > 0);
    assert_eq!(plan.prunable_realizations, 0);
    let sweep = history.sweep_gc(plan)?;
    assert!(sweep.deleted_nodes > 0);
    assert_eq!(history.list(None)?.len(), 2);
    assert!(history.get(&first.id).is_ok());
    assert!(history.get(&second.id).is_ok());
    Ok(())
}
