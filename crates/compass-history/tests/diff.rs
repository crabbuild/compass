use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use compass_history::{
    ChangeKind, ChangeSink, CompletionEvidence, ExtractionFingerprint, GraphArtifacts, GraphChange,
    HistoryError, HistoryStore, PublishRequest, RecordKind, Repository,
};
use compass_model::GraphDocument;
use serde_json::{Value, json};

#[derive(Default)]
struct VecSink(Vec<GraphChange>);

impl ChangeSink for VecSink {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        self.0.push(change);
        Ok(())
    }
}

fn repository() -> Result<(tempfile::TempDir, Repository), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    for arguments in [
        vec!["init", "--quiet"],
        vec!["config", "user.name", "Compass Test"],
        vec!["config", "user.email", "compass@example.invalid"],
    ] {
        git(directory.path(), &arguments)?;
    }
    std::fs::write(directory.path().join("README.md"), "fixture\n")?;
    git(directory.path(), &["add", "README.md"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;
    let repository = Repository::discover(directory.path())?;
    Ok((directory, repository))
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
    nodes: Vec<Value>,
    links: Vec<Value>,
    hyperedges: Vec<Value>,
    score: u64,
) -> Result<PublishRequest, Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": true,
        "nodes": nodes,
        "links": links,
        "hyperedges": hyperedges
    }))?;
    Ok(PublishRequest {
        commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?,
        parents: Vec::new(),
        fingerprint: std::iter::repeat_n(fingerprint, 64)
            .collect::<String>()
            .parse::<ExtractionFingerprint>()?,
        artifacts: GraphArtifacts {
            document,
            analysis: Some(json!({"score": score})),
            labels: None,
            manifest: None,
            authoritative_sidecars: BTreeMap::new(),
        },
        completion: CompletionEvidence {
            extraction_succeeded: true,
            allow_partial: false,
            semantic_files_expected: 0,
            semantic_files_completed: 0,
            failed_chunks: 0,
        },
        make_preferred: false,
    })
}

#[test]
fn diff_reports_topology_attribute_and_analysis_changes() -> Result<(), Box<dyn std::error::Error>>
{
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let old = history.publish(request(
        'a',
        vec![json!({"id":"a"}), json!({"id":"b"})],
        vec![json!({"source":"a","target":"b","relation":"calls","key":"main","confidence":0.5})],
        vec![json!({"id":"flow","nodes":["a","b"],"weight":1})],
        1,
    )?)?;
    let new = history.publish(request(
        'b',
        vec![json!({"id":"a"}), json!({"id":"b"}), json!({"id":"c"})],
        vec![json!({"source":"a","target":"b","relation":"calls","key":"main","confidence":0.9})],
        vec![json!({"id":"flow","nodes":["a","b"],"weight":2})],
        2,
    )?)?;
    let mut changes = VecSink::default();
    history.diff(&old.id, &new.id, &mut changes)?;
    assert!(
        changes.0.iter().any(|change| {
            change.record == RecordKind::Node && change.change == ChangeKind::Added
        })
    );
    assert!(changes.0.iter().any(|change| {
        change.record == RecordKind::Edge && change.change == ChangeKind::Changed
    }));
    assert!(changes.0.iter().any(|change| {
        change.record == RecordKind::Hyperedge && change.change == ChangeKind::Changed
    }));
    assert!(
        changes
            .0
            .iter()
            .any(|change| change.record == RecordKind::Analysis)
    );
    Ok(())
}

#[test]
fn identity_changes_are_remove_add_equal_roots_are_empty_and_sink_errors_stop()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let nodes = vec![json!({"id":"a"}), json!({"id":"b"})];
    let old = history.publish(request(
        'c',
        nodes.clone(),
        vec![json!({"source":"a","target":"b","relation":"calls","key":"main"})],
        vec![json!({"nodes":["a","b"],"weight":1})],
        1,
    )?)?;
    let new = history.publish(request(
        'd',
        nodes,
        vec![json!({"source":"a","target":"b","relation":"imports","key":"main"})],
        vec![json!({"nodes":["a","b"],"weight":2})],
        1,
    )?)?;
    let mut changes = VecSink::default();
    history.diff(&old.id, &new.id, &mut changes)?;
    for record in [RecordKind::Edge, RecordKind::Hyperedge] {
        assert!(
            changes
                .0
                .iter()
                .any(|change| { change.record == record && change.change == ChangeKind::Removed })
        );
        assert!(
            changes
                .0
                .iter()
                .any(|change| { change.record == record && change.change == ChangeKind::Added })
        );
    }
    let mut equal = VecSink::default();
    history.diff(&old.id, &old.id, &mut equal)?;
    assert!(equal.0.is_empty());

    struct FailingSink(usize);
    impl ChangeSink for FailingSink {
        fn change(&mut self, _change: GraphChange) -> Result<(), HistoryError> {
            self.0 += 1;
            Err(HistoryError::Git("sink stopped".to_owned()))
        }
    }
    let mut failing = FailingSink(0);
    assert!(history.diff(&old.id, &new.id, &mut failing).is_err());
    assert_eq!(failing.0, 1);
    Ok(())
}
