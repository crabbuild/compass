use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use compass_analysis::{AnalysisBundle, analyze};
use compass_history::{
    CommitId, CompletedGraphArtifacts, CompletionEvidence, ExtractionFingerprint, GraphArtifacts,
    HistoryStore, MAX_JSON_DEPTH, PublishRequest, Repository,
};
use compass_ir::{ProgramBundle, ProviderDescriptor, ProviderKind, hex_sha256};
use compass_model::GraphDocument;
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, ExtractionStatus, FileRecord,
    GraphDocument as CodeGraphDocument, NodeKind, NodeRecord,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
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
    let mut profile = compass_history::BuildProfile::default();
    profile.insert("graph_schema", compass_history::HISTORY_GRAPH_SCHEMA)?;
    Ok(PublishRequest {
        commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?,
        parents: vec!["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".parse()?],
        profile,
        fingerprint: std::iter::repeat_n(fingerprint, 64)
            .collect::<String>()
            .parse::<ExtractionFingerprint>()?,
        artifacts: GraphArtifacts {
            document,
            program: None,
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

#[test]
fn publication_is_atomic_reopenable_and_content_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let mut publish = request('a', false)?;
    publish.profile.insert("provider", "none")?;
    let expected_profile = publish.profile.clone();
    let expected_artifacts = CompletedGraphArtifacts {
        artifacts: publish.artifacts.clone(),
        completion: publish.completion.clone(),
    };
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
    assert_eq!(reopened.artifacts(&first.id)?, expected_artifacts);
    assert_eq!(reopened.list(None)?.len(), 1);
    Ok(())
}

#[test]
fn realization_reader_returns_only_the_exact_trusted_typed_graph()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;

    let legacy = history.publish(request('a', false)?)?;
    let legacy_error = match history.reader(&legacy.id)?.graph_document() {
        Ok(_) => return Err("compatibility-only realization accepted a typed graph read".into()),
        Err(error) => error,
    };
    assert!(matches!(
        legacy_error,
        compass_history::HistoryError::TrustedGraphUnavailable { realization }
            if realization == legacy.id.to_string()
    ));

    let digest = format!("sha256:{}", "0".repeat(64));
    let mut document = CodeGraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: digest.clone(),
        source_tree_digest: digest.clone(),
        configuration_digest: digest.clone(),
        generation_id: digest,
        source_commit: None,
    });
    let anchor = SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 4,
    };
    let evidence = Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "history-test".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: None,
        anchors: vec![anchor.clone()],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    };
    document.graph.files.push(FileRecord {
        id: file_id("src/lib.rs"),
        path: "src/lib.rs".to_owned(),
        language: Some("rust".to_owned()),
        content_digest: format!("sha256:{}", "1".repeat(64)),
        byte_size: 4,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["history-test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    document.nodes = ["a", "b", "c"]
        .into_iter()
        .map(|id| NodeRecord {
            id: format!("n:{id}"),
            kind: NodeKind::Function,
            roles: Vec::new(),
            name: id.to_owned(),
            qualified_name: format!("fixture::{id}"),
            language: Some("rust".to_owned()),
            framework: None,
            source: Some(anchor.clone()),
            details: None,
            evidence: vec![evidence.clone()],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        })
        .collect();
    document.links = ["n:a", "n:b", "n:c"]
        .into_iter()
        .flat_map(|source| {
            ["n:a", "n:b", "n:c"]
                .into_iter()
                .filter(move |target| *target != source)
                .map(move |target| (source, target))
        })
        .map(|(source, target)| {
            let id = edge_id(source, EdgeKind::Calls, target, Some(&anchor), None);
            EdgeRecord {
                id: id.clone(),
                key: id,
                source: source.to_owned(),
                target: target.to_owned(),
                kind: EdgeKind::Calls,
                occurrence_rule: None,
                relationship_site: Some(anchor.clone()),
                details: None,
                evidence: vec![evidence.clone()],
                weight: None,
                context: None,
                deferred: false,
                diagnostics: Vec::new(),
            }
        })
        .collect();
    document.links.sort_by(|left, right| left.id.cmp(&right.id));
    let mut topology_order = document.links.clone();
    topology_order.sort_by(|left, right| {
        (
            left.source.as_str(),
            left.kind.as_str(),
            left.target.as_str(),
            left.key.as_str(),
        )
            .cmp(&(
                right.source.as_str(),
                right.kind.as_str(),
                right.target.as_str(),
                right.key.as_str(),
            ))
    });
    assert_ne!(document.links, topology_order);
    let typed_artifacts = GraphArtifacts::from_trusted(document.clone(), None, None, None)?;
    let expected_graph_bytes = typed_artifacts.graph_json_bytes()?;
    let expected_registry = typed_artifacts.artifact_registry()?;
    let mut typed_request = request('b', false)?;
    typed_request.artifacts = typed_artifacts;
    let typed = history.publish(typed_request)?;
    let reconstructed = history.artifacts(&typed.id)?;
    assert_eq!(
        reconstructed.artifacts.graph_json_bytes()?,
        expected_graph_bytes
    );
    assert_eq!(
        reconstructed.artifacts.artifact_registry()?,
        expected_registry
    );
    let reader = history.reader(&typed.id)?;
    assert_eq!(reader.version().id, typed.id);
    assert_eq!(reader.graph_document()?, document);
    assert_eq!(reader.version().id, typed.id);
    Ok(())
}

#[test]
fn publication_with_computed_floats_is_immediately_valid_and_reconstructable()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let mut publish = request('a', false)?;
    publish.artifacts.analysis = Some(json!({
        "cohesion": {"14": 0.10384068278805121_f64}
    }));

    let published = history.publish(publish)?;
    history.validate(&published.id)?;
    let reconstructed = history.artifacts(&published.id)?;
    assert_eq!(
        reconstructed
            .artifacts
            .analysis
            .as_ref()
            .and_then(|value| value.pointer("/cohesion/14"))
            .and_then(serde_json::Value::as_f64),
        Some(0.10384068278805121_f64)
    );
    Ok(())
}

#[test]
fn previous_graph_schema_profiles_are_rejected_at_publication()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let mut publish = request('a', false)?;
    publish
        .profile
        .insert("graph_schema", "networkx-node-link/v6")?;
    let error = match history.publish(publish) {
        Ok(_) => return Err("previous graph schema must not be published".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("networkx-node-link/v1"));
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
fn program_provider_identity_and_subtree_sharing_are_content_addressed()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let mut first_request = request('a', false)?;
    first_request.artifacts.program = Some(program(b"first")?);
    let first = history.publish(first_request.clone())?;

    let mut provider_changed = first_request.clone();
    provider_changed.artifacts.program = Some(program(b"second")?);
    let changed = history.publish(provider_changed)?;
    assert_ne!(changed.id, first.id);
    assert_ne!(
        changed.version.program_facts_root,
        first.version.program_facts_root
    );

    let mut graph_changed = first_request;
    graph_changed
        .artifacts
        .document
        .nodes
        .push(serde_json::from_value(json!({"id":"b","label":"B"}))?);
    let shared = history.publish(graph_changed)?;
    assert_ne!(shared.version.nodes_root, first.version.nodes_root);
    assert_eq!(
        shared.version.program_facts_root,
        first.version.program_facts_root
    );
    assert_eq!(
        shared.version.program_summaries_root,
        first.version.program_summaries_root
    );
    let encoded = serde_json::to_string(&shared.version)?;
    assert!(!encoded.contains(&fixture.path.to_string_lossy().into_owned()));
    Ok(())
}

#[test]
fn validation_rejects_missing_endpoints_before_catalog_publication()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let repository = Repository::discover(&fixture.path)?;
    let history = HistoryStore::create(&repository)?;
    let initial_nodes = history.plan_gc(false)?.candidate_nodes;
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

    let mut excessive_depth = request('a', false)?;
    let mut nested = serde_json::Value::Null;
    for _ in 0..MAX_JSON_DEPTH {
        nested = json!({"next": nested});
    }
    excessive_depth
        .artifacts
        .document
        .graph
        .insert("nested".to_owned(), nested);
    let error = match history.publish(excessive_depth) {
        Ok(_) => return Err("excessive JSON depth unexpectedly published".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("JSON depth"));

    assert_eq!(
        history.plan_gc(false)?.candidate_nodes,
        initial_nodes,
        "invalid in-memory partitions must not write orphaned tree nodes"
    );

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
