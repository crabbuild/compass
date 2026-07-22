use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use compass_history::{
    CommitId, CompletionEvidence, ExtractionFingerprint, GraphArtifacts, HistoryStore,
    PublishRequest, Repository,
};
use compass_model::GraphDocument;
use prolly::{Config, KeyBuilder, Prolly};
use prolly_store_sqlite::SqliteStore;
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
        profile: compass_history::BuildProfile::default(),
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
    let mut publish = request('a', false)?;
    publish.profile.insert("provider", "none")?;
    let expected_profile = publish.profile.clone();
    let first = history.publish(publish.clone())?;
    let second = history.publish(publish)?;
    assert_eq!(first.id, second.id);
    assert!(first.preferred && second.preferred);
    assert_eq!(first.version.build_profile, expected_profile);
    assert_eq!(first.version.profile_digest.len(), 64);
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

    let mut duplicate_node = request('e', false)?;
    duplicate_node
        .artifacts
        .document
        .nodes
        .push(duplicate_node.artifacts.document.nodes[0].clone());
    assert!(history.publish(duplicate_node).is_err());

    let mut missing_hyperedge_member = request('f', false)?;
    missing_hyperedge_member.artifacts.document.extras.insert(
        "hyperedges".to_owned(),
        json!([{"id":"flow","members":["a","missing"]}]),
    );
    let error = match history.publish(missing_hyperedge_member) {
        Ok(_) => return Err("missing hyperedge member unexpectedly published".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("MissingHyperedgeMember"));

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

#[test]
fn structural_sharing_and_cross_commit_preference_guards_are_explicit()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let first = history.publish(request('a', false)?)?;
    let second = history.publish(request('b', true)?)?;
    let sharing = history.structural_sharing(&first.id, &second.id)?;
    assert!(sharing.first_total_nodes > 0);
    assert!(sharing.second_total_nodes > 0);
    assert!(sharing.union_nodes <= sharing.first_total_nodes + sharing.second_total_nodes);
    assert_eq!(
        sharing.shared_nodes,
        sharing.first_total_nodes + sharing.second_total_nodes - sharing.union_nodes
    );

    let first_commit: CommitId = first.version.git_commit.parse()?;
    let mut other_request = request('c', false)?;
    other_request.commit = "cccccccccccccccccccccccccccccccccccccccc".parse()?;
    other_request.parents.clear();
    let other = history.publish(other_request)?;
    let other_commit: CommitId = other.version.git_commit.parse()?;
    assert!(
        history
            .compare_and_set_preferred(&first_commit, Some(&second.id), &other.id)
            .is_err()
    );
    assert!(
        history
            .compare_and_set_preferred(&first_commit, Some(&other.id), &first.id)
            .is_err()
    );
    assert!(history.corrupt_preferred_token(&first_commit).is_err());
    assert!(history.corrupt_preferred_token(&other_commit).is_err());
    Ok(())
}

#[test]
fn corrupt_preferred_recovery_requires_the_exact_observation_and_commit()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let first = history.publish(request('a', false)?)?;
    let mut alternate_request = request('b', true)?;
    alternate_request.make_preferred = false;
    let alternate = history.publish(alternate_request)?;
    let commit: CommitId = first.version.git_commit.parse()?;
    let absent: CommitId = "dddddddddddddddddddddddddddddddddddddddd".parse()?;
    assert!(history.corrupt_preferred_token(&absent).is_err());

    let adapter = Arc::new(SqliteStore::open_existing(history.database_path())?);
    let prolly = Prolly::new(adapter, Config::default());
    let preferred_name = KeyBuilder::new()
        .push_segment(b"compass")
        .push_segment(b"v1")
        .push_segment(b"preferred")
        .push_segment(commit.as_str().as_bytes())
        .finish();
    let observed = prolly
        .load_named_root(&preferred_name)?
        .ok_or("preferred root")?;
    let corrupt = prolly.put(&prolly.create(), b"corrupt".to_vec(), b"pointer".to_vec())?;
    assert!(matches!(
        prolly.compare_and_swap_named_root(&preferred_name, Some(&observed), Some(&corrupt))?,
        prolly::NamedRootUpdate::Applied
    ));

    let token = history.corrupt_preferred_token(&commit)?;
    assert!(
        history
            .recover_corrupt_preferred_with_activity(
                &absent,
                &token,
                &alternate.id,
                &history.activity()?,
            )
            .is_err()
    );
    assert!(history.recover_corrupt_preferred_with_activity(
        &commit,
        &token,
        &alternate.id,
        &history.activity()?,
    )?);
    assert_eq!(
        history.preferred(&commit)?.ok_or("recovered preferred")?.id,
        alternate.id
    );

    let second_fixture = Fixture::new()?;
    let second_repository = Repository::discover(&second_fixture.path)?;
    let second_store = HistoryStore::create(&second_repository)?;
    assert!(
        second_store
            .recover_corrupt_preferred_with_activity(
                &commit,
                &token,
                &alternate.id,
                &second_store.activity()?,
            )
            .is_err()
    );
    Ok(())
}
