use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use compass_analysis::{AnalysisBundle, analyze};
use compass_history::{
    ChangeKind, ChangeSink, CompletionEvidence, ExtractionFingerprint, GraphArtifacts, GraphChange,
    HistoryError, HistoryStore, PublishRequest, RecordKind, Repository,
};
use compass_ir::{ProgramBundle, ProviderDescriptor, ProviderKind, hex_sha256};
use compass_model::GraphDocument;
use prolly::{VersionedValue, decode_segments};
use rand::{Rng, SeedableRng, rngs::StdRng};
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
    let mut profile = compass_history::BuildProfile::default();
    profile.insert("graph_schema", compass_history::HISTORY_GRAPH_SCHEMA)?;
    Ok(PublishRequest {
        commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?,
        parents: Vec::new(),
        profile,
        fingerprint: std::iter::repeat_n(fingerprint, 64)
            .collect::<String>()
            .parse::<ExtractionFingerprint>()?,
        artifacts: GraphArtifacts {
            document,
            program: None,
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

fn program(input: &[u8]) -> Result<AnalysisBundle, compass_analysis::AnalysisError> {
    analyze(ProgramBundle {
        schema: compass_ir::PROGRAM_SCHEMA.to_owned(),
        providers: vec![ProviderDescriptor {
            id: "scip:fixture".to_owned(),
            kind: ProviderKind::Artifact,
            version: "scip/1".to_owned(),
            scope: "repository".to_owned(),
            input_digest: hex_sha256(input),
            configuration_digest: hex_sha256(b"manifest"),
        }],
        evidence: Vec::new(),
        modules: Vec::new(),
    })
}

fn naive_changes(
    record: RecordKind,
    old: &[(Vec<u8>, Vec<u8>)],
    new: &[(Vec<u8>, Vec<u8>)],
) -> Result<Vec<GraphChange>, Box<dyn std::error::Error>> {
    let old = old.iter().cloned().collect::<BTreeMap<_, _>>();
    let new = new.iter().cloned().collect::<BTreeMap<_, _>>();
    let keys = old
        .keys()
        .chain(new.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let decode = |bytes: &[u8]| -> Result<Value, Box<dyn std::error::Error>> {
        let envelope = VersionedValue::from_bytes(bytes)?;
        Ok(serde_json::from_slice(&envelope.payload)?)
    };
    let display = |key: &[u8]| -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut segments = decode_segments(key)?;
        segments.drain(..2);
        Ok(segments
            .into_iter()
            .map(|segment| {
                if segment
                    .iter()
                    .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
                {
                    String::from_utf8(segment)
                        .unwrap_or_else(|bytes| format!("invalid-utf8:{:?}", bytes.as_bytes()))
                } else {
                    segment.iter().fold(String::from("0x"), |mut output, byte| {
                        use std::fmt::Write;
                        let _ = write!(output, "{byte:02x}");
                        output
                    })
                }
            })
            .collect())
    };
    let mut changes = Vec::new();
    for key in keys {
        let left = old.get(&key);
        let right = new.get(&key);
        let change = match (left, right) {
            (Some(left), Some(right)) if left == right => None,
            (Some(left), Some(right)) => Some(GraphChange {
                record,
                change: ChangeKind::Changed,
                key: display(&key)?,
                old: Some(decode(left)?),
                new: Some(decode(right)?),
            }),
            (Some(left), None) => Some(GraphChange {
                record,
                change: ChangeKind::Removed,
                key: display(&key)?,
                old: Some(decode(left)?),
                new: None,
            }),
            (None, Some(right)) => Some(GraphChange {
                record,
                change: ChangeKind::Added,
                key: display(&key)?,
                old: None,
                new: Some(decode(right)?),
            }),
            (None, None) => None,
        };
        if let Some(change) = change {
            changes.push(change);
        }
    }
    Ok(changes)
}

fn sort_changes(changes: &mut [GraphChange]) -> Result<(), compass_history::HistoryError> {
    let mut keyed = changes
        .iter()
        .cloned()
        .map(|change| {
            Ok((
                compass_history::canonical_json_bytes(&serde_json::to_value(&change)?)?,
                change,
            ))
        })
        .collect::<Result<Vec<_>, compass_history::HistoryError>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    for (target, (_, change)) in changes.iter_mut().zip(keyed) {
        *target = change;
    }
    Ok(())
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
    let old_reader = history.reader(&old.id)?;
    let new_reader = history.reader(&new.id)?;
    let mut changes = VecSink::default();
    old_reader.diff(&new_reader, &mut changes)?;
    assert!(changes.0.iter().any(|change| {
        change.record == RecordKind::Node
            && change.change == ChangeKind::Added
            && change.key == ["c"]
    }));
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
    let mut topology_roots = VecSink::default();
    old_reader.diff_records(
        &new_reader,
        &[RecordKind::Node, RecordKind::Edge],
        &mut topology_roots,
    )?;
    assert!(
        topology_roots
            .0
            .iter()
            .all(|change| matches!(change.record, RecordKind::Node | RecordKind::Edge))
    );
    Ok(())
}

#[test]
fn structural_counts_collapse_graph_wide_source_coordinate_churn()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let nodes = |offset: usize| {
        (0..256)
            .map(|index| {
                json!({
                    "id": format!("node-{index:03}"),
                    "label": format!("Node {index}"),
                    "source_file": "src/service.rs",
                    "source_location": format!("L{}", index + offset),
                    "line_start": index + offset,
                    "line_end": index + offset,
                    "start_byte": index * 10 + offset,
                    "end_byte": index * 10 + offset + 5,
                })
            })
            .collect::<Vec<_>>()
    };
    let edges = |offset: usize| {
        (0..255)
            .map(|index| {
                json!({
                    "source": format!("node-{index:03}"),
                    "target": format!("node-{:03}", index + 1),
                    "relation": "calls",
                    "confidence": "EXTRACTED",
                    "source_file": "src/service.rs",
                    "source_location": format!("L{}", index + offset),
                    "start_byte": index * 10 + offset,
                    "end_byte": index * 10 + offset + 5,
                })
            })
            .collect::<Vec<_>>()
    };
    let old = history.publish(request('a', nodes(1), edges(1), Vec::new(), 1)?)?;
    let new = history.publish(request('b', nodes(2), edges(2), Vec::new(), 1)?)?;
    let old_reader = history.reader(&old.id)?;
    let new_reader = history.reader(&new.id)?;

    let mut exact = VecSink::default();
    old_reader.diff_records(
        &new_reader,
        &[RecordKind::Node, RecordKind::Edge],
        &mut exact,
    )?;
    assert_eq!(
        exact
            .0
            .iter()
            .filter(|change| change.record == RecordKind::Node)
            .count(),
        256
    );
    assert_eq!(
        exact
            .0
            .iter()
            .filter(|change| change.record == RecordKind::Edge)
            .count(),
        510
    );

    assert_eq!(
        old_reader.structural_change_counts(&new_reader)?,
        compass_history::StructuralChangeCounts::default()
    );
    let mut structural = VecSink::default();
    old_reader.structural_diff(&new_reader, &mut structural)?;
    assert!(structural.0.is_empty());
    Ok(())
}

#[test]
fn structural_counts_preserve_meaningful_node_edge_and_multiplicity_changes()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let old = history.publish(request(
        'a',
        vec![json!({"id":"a","label":"Old"}), json!({"id":"b"})],
        vec![json!({
            "source":"a",
            "target":"b",
            "relation":"calls",
            "confidence":"INFERRED"
        })],
        Vec::new(),
        1,
    )?)?;
    let new = history.publish(request(
        'b',
        vec![json!({"id":"a","label":"New"}), json!({"id":"b"})],
        vec![
            json!({
                "source":"a",
                "target":"b",
                "relation":"calls",
                "confidence":"EXTRACTED"
            }),
            json!({
                "source":"a",
                "target":"b",
                "relation":"calls",
                "confidence":"EXTRACTED",
                "context":"second occurrence"
            }),
        ],
        Vec::new(),
        1,
    )?)?;
    let old_reader = history.reader(&old.id)?;
    let new_reader = history.reader(&new.id)?;

    let counts = old_reader.structural_change_counts(&new_reader)?;
    assert_eq!(counts.nodes.changed, 1);
    assert_eq!(counts.edges.changed, 1);
    assert_eq!(counts.edges.added, 1);
    assert_eq!(counts.edges.removed, 0);
    Ok(())
}

#[test]
fn structural_diff_pairs_every_parallel_anchor_identity_change()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let edges = |confidence: &str, offset: u64| {
        ["first", "second"]
            .into_iter()
            .enumerate()
            .map(|(index, context)| {
                json!({
                    "source": "a",
                    "target": "b",
                    "relation": "calls",
                    "confidence": confidence,
                    "context": context,
                    "relationshipSite": {
                        "file": "src/lib.rs",
                        "startByte": offset + index as u64,
                        "endByte": offset + index as u64 + 1
                    }
                })
            })
            .collect::<Vec<_>>()
    };
    let nodes = vec![json!({"id":"a"}), json!({"id":"b"})];
    let old = history.publish(request(
        'a',
        nodes.clone(),
        edges("INFERRED", 10),
        Vec::new(),
        1,
    )?)?;
    let new = history.publish(request('b', nodes, edges("EXTRACTED", 20), Vec::new(), 1)?)?;
    let old_reader = history.reader(&old.id)?;
    let new_reader = history.reader(&new.id)?;

    let mut structural = VecSink::default();
    old_reader.structural_diff(&new_reader, &mut structural)?;
    assert_eq!(structural.0.len(), 2);
    assert!(structural.0.iter().all(|change| {
        change.record == RecordKind::Edge && change.change == ChangeKind::Changed
    }));
    assert!(structural.0.iter().all(|change| {
        change.old.as_ref().and_then(|value| value.get("context"))
            == change.new.as_ref().and_then(|value| value.get("context"))
    }));
    let counts = old_reader.structural_change_counts(&new_reader)?;
    assert_eq!(counts.edges.added, 0);
    assert_eq!(counts.edges.removed, 0);
    assert_eq!(counts.edges.changed, 2);
    Ok(())
}

#[test]
fn structural_diff_preserves_explicit_edge_identity_replacement()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let nodes = vec![json!({"id":"a"}), json!({"id":"b"})];
    let old = history.publish(request(
        'a',
        nodes.clone(),
        vec![json!({
            "source":"a", "target":"b", "relation":"calls", "key":"stable-old"
        })],
        Vec::new(),
        1,
    )?)?;
    let new = history.publish(request(
        'b',
        nodes,
        vec![json!({
            "source":"a", "target":"b", "relation":"calls", "key":"stable-new"
        })],
        Vec::new(),
        1,
    )?)?;
    let old_reader = history.reader(&old.id)?;
    let new_reader = history.reader(&new.id)?;

    let mut structural = VecSink::default();
    old_reader.structural_diff(&new_reader, &mut structural)?;
    assert_eq!(structural.0.len(), 2);
    assert_eq!(
        structural
            .0
            .iter()
            .filter(|change| change.change == ChangeKind::Added)
            .count(),
        1
    );
    assert_eq!(
        structural
            .0
            .iter()
            .filter(|change| change.change == ChangeKind::Removed)
            .count(),
        1
    );
    let counts = old_reader.structural_change_counts(&new_reader)?;
    assert_eq!(counts.edges.added, 1);
    assert_eq!(counts.edges.removed, 1);
    assert_eq!(counts.edges.changed, 0);
    Ok(())
}

#[test]
fn structural_diff_treats_relation_as_topology() -> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let nodes = vec![json!({"id":"a"}), json!({"id":"b"})];
    let old = history.publish(request(
        'a',
        nodes.clone(),
        vec![json!({"source":"a", "target":"b", "relation":"calls"})],
        Vec::new(),
        1,
    )?)?;
    let new = history.publish(request(
        'b',
        nodes,
        vec![json!({"source":"a", "target":"b", "relation":"uses"})],
        Vec::new(),
        1,
    )?)?;
    let old_reader = history.reader(&old.id)?;
    let new_reader = history.reader(&new.id)?;

    let counts = old_reader.structural_change_counts(&new_reader)?;
    assert_eq!(counts.edges.added, 1);
    assert_eq!(counts.edges.removed, 1);
    assert_eq!(counts.edges.changed, 0);
    Ok(())
}

#[test]
fn structural_counts_reject_different_build_profiles() -> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let old = history.publish(request('a', vec![json!({"id":"a"})], vec![], vec![], 1)?)?;
    let mut changed_profile = request('b', vec![json!({"id":"a"})], vec![], vec![], 1)?;
    changed_profile
        .profile
        .insert("extractor_version", "different")?;
    let new = history.publish(changed_profile)?;
    let old_reader = history.reader(&old.id)?;
    let new_reader = history.reader(&new.id)?;

    assert!(old_reader.structural_change_counts(&new_reader).is_err());
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
    let old_reader = history.reader(&old.id)?;
    let new_reader = history.reader(&new.id)?;
    let mut changes = VecSink::default();
    old_reader.diff(&new_reader, &mut changes)?;
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
    old_reader.diff(&old_reader, &mut equal)?;
    assert!(equal.0.is_empty());

    struct FailingSink(usize);
    impl ChangeSink for FailingSink {
        fn change(&mut self, _change: GraphChange) -> Result<(), HistoryError> {
            self.0 += 1;
            Err(HistoryError::Git("sink stopped".to_owned()))
        }
    }
    let mut failing = FailingSink(0);
    assert!(old_reader.diff(&new_reader, &mut failing).is_err());
    assert_eq!(failing.0, 1);
    Ok(())
}

#[test]
fn full_diff_includes_program_facts_while_topology_diff_skips_them()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    let mut old_request = request('e', vec![json!({"id":"a"})], vec![], vec![], 1)?;
    old_request.artifacts.program = Some(program(b"old")?);
    let old = history.publish(old_request)?;
    let mut new_request = request('f', vec![json!({"id":"a"})], vec![], vec![], 1)?;
    new_request.artifacts.program = Some(program(b"new")?);
    let new = history.publish(new_request)?;
    let old_reader = history.reader(&old.id)?;
    let new_reader = history.reader(&new.id)?;

    let mut full = VecSink::default();
    old_reader.diff(&new_reader, &mut full)?;
    assert!(
        full.0
            .iter()
            .any(|change| change.record == RecordKind::ProgramFact)
    );
    let mut topology = VecSink::default();
    old_reader.diff_records(
        &new_reader,
        &[RecordKind::Node, RecordKind::Edge],
        &mut topology,
    )?;
    assert!(topology.0.is_empty());
    Ok(())
}

#[test]
fn streamed_topology_diff_matches_naive_map_oracle_across_random_graphs()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let history = HistoryStore::create(&repository)?;
    for seed in 0..64_u64 {
        let mut rng = StdRng::seed_from_u64(seed);
        let graph = |rng: &mut StdRng| {
            let mut nodes = Vec::new();
            for index in 0..16 {
                if rng.gen_bool(0.65) {
                    nodes.push(json!({
                        "id": format!("node-{index:02}"),
                        "kind": if rng.gen_bool(0.5) { "function" } else { "class" },
                        "revision": rng.gen_range(0..4),
                    }));
                }
            }
            if nodes.is_empty() {
                nodes.push(json!({"id":"node-00","kind":"function","revision":0}));
            }
            let ids = nodes
                .iter()
                .filter_map(|node| node.get("id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let mut links = Vec::new();
            for index in 0..24 {
                if rng.gen_bool(0.55) {
                    let source = &ids[rng.gen_range(0..ids.len())];
                    let target = &ids[rng.gen_range(0..ids.len())];
                    links.push(json!({
                        "source": source,
                        "target": target,
                        "relation": if rng.gen_bool(0.5) { "calls" } else { "imports" },
                        "key": format!("edge-{index:02}"),
                        "weight": rng.gen_range(0..5),
                    }));
                }
            }
            (nodes, links)
        };
        let (old_nodes, old_links) = graph(&mut rng);
        let (new_nodes, new_links) = graph(&mut rng);
        let old_request = request('a', old_nodes, old_links, Vec::new(), rng.gen_range(0..8))?;
        let new_request = request('b', new_nodes, new_links, Vec::new(), rng.gen_range(0..8))?;
        let old_partition = old_request.artifacts.partition(&old_request.completion)?;
        let new_partition = new_request.artifacts.partition(&new_request.completion)?;
        let old = history.publish(old_request)?;
        let new = history.publish(new_request)?;
        let old_reader = history.reader(&old.id)?;
        let new_reader = history.reader(&new.id)?;
        let mut actual = VecSink::default();
        old_reader.diff_records(
            &new_reader,
            &[RecordKind::Node, RecordKind::Edge],
            &mut actual,
        )?;
        let mut expected =
            naive_changes(RecordKind::Node, &old_partition.nodes, &new_partition.nodes)?;
        expected.extend(naive_changes(
            RecordKind::Edge,
            &old_partition.edges,
            &new_partition.edges,
        )?);
        sort_changes(&mut actual.0)?;
        sort_changes(&mut expected)?;
        assert_eq!(actual.0, expected, "streaming diff diverged at seed {seed}");
    }
    Ok(())
}
