use std::path::Path;
use std::process::{Command, Output};

use compass_history::{
    CompletionEvidence, ExtractionFingerprint, GraphArtifacts, HistoryStore, PublishRequest,
    Repository,
};
use compass_model::GraphDocument;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{self, Write};

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

fn run(
    binary: &str,
    directory: &Path,
    arguments: &[&str],
) -> Result<Output, Box<dyn std::error::Error>> {
    Ok(Command::new(binary)
        .args(arguments)
        .current_dir(directory)
        .output()?)
}

#[test]
fn history_help_and_empty_status_are_actionable_and_non_mutating()
-> Result<(), Box<dyn std::error::Error>> {
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

    let compass = env!("CARGO_BIN_EXE_compass");
    let graphify = env!("CARGO_BIN_EXE_graphify");
    let help = run(compass, directory.path(), &["history", "--help"])?;
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("build REV"));
    let status = run(compass, directory.path(), &["history", "status", "HEAD"])?;
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("no store"));
    assert!(!directory.path().join(".git/compass").exists());
    let alias = run(graphify, directory.path(), &["history", "status", "HEAD"])?;
    assert_eq!(status.status.code(), alias.status.code());
    assert_eq!(status.stdout, alias.stdout);
    assert_eq!(status.stderr, alias.stderr);

    let repository = Repository::discover(directory.path())?;
    let history = HistoryStore::create(&repository)?;
    let database = history.database_path().to_path_buf();
    drop(history);
    std::fs::write(database, b"not a sqlite database")?;
    let incompatible = run(compass, directory.path(), &["history", "status", "HEAD"])?;
    assert_eq!(incompatible.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&incompatible.stdout).contains("store: incompatible"));
    assert!(String::from_utf8_lossy(&incompatible.stderr).contains("error:"));
    Ok(())
}

#[test]
fn history_commands_inspect_prefer_and_export_published_realizations()
-> Result<(), Box<dyn std::error::Error>> {
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
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let history = HistoryStore::create(&repository)?;
    let publish = |fingerprint: char,
                   label: &str,
                   make_preferred: bool|
     -> Result<compass_history::PublishedVersion, Box<dyn std::error::Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "directed": true,
            "multigraph": false,
            "graph":{"name":"fixture"},
            "nodes":[{"id":"a","label":label,"community":0}],
            "links":[],
            "built_at_commit":commit
        }))?;
        Ok(history.publish(PublishRequest {
            commit: commit.clone(),
            parents: repository.parents(&commit)?,
            fingerprint: std::iter::repeat_n(fingerprint, 64)
                .collect::<String>()
                .parse::<ExtractionFingerprint>()?,
            artifacts: GraphArtifacts {
                document,
                analysis: Some(json!({"communities":{"0":["a"]}})),
                labels: Some(json!({"0":"Core"})),
                manifest: Some(json!({"README.md":{"ast_hash":"abc"}})),
                authoritative_sidecars: BTreeMap::from([(
                    "semantic/facts.bin".to_owned(),
                    vec![0, 1, 255],
                )]),
            },
            completion: CompletionEvidence {
                extraction_succeeded: true,
                allow_partial: false,
                semantic_files_expected: 0,
                semantic_files_completed: 0,
                failed_chunks: 0,
            },
            make_preferred,
        })?)
    };
    let first = publish('a', "First", true)?;
    let second = publish('b', "Second", false)?;
    drop(history);

    let compass = env!("CARGO_BIN_EXE_compass");
    let graphify = env!("CARGO_BIN_EXE_graphify");
    let list = run(
        compass,
        directory.path(),
        &["history", "list", "HEAD", "--format", "json"],
    )?;
    assert!(list.status.success());
    let listed: serde_json::Value = serde_json::from_slice(&list.stdout)?;
    assert_eq!(listed.as_array().map(Vec::len), Some(2));
    let alias = run(
        graphify,
        directory.path(),
        &["history", "list", "HEAD", "--format", "json"],
    )?;
    assert_eq!(list.stdout, alias.stdout);

    let show = run(
        compass,
        directory.path(),
        &["history", "show", &second.id.as_hex(), "--format", "json"],
    )?;
    assert!(show.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&show.stdout)?["git_commit"],
        commit.as_str()
    );
    let preferred = run(
        compass,
        directory.path(),
        &["history", "prefer", "HEAD", &second.id.as_hex()],
    )?;
    assert!(preferred.status.success());

    let graph_json = directory.path().join("historical.json");
    let export = run(
        compass,
        directory.path(),
        &[
            "history",
            "export",
            "HEAD",
            "--format",
            "graph-json",
            "--output",
            graph_json.to_str().ok_or("path")?,
        ],
    )?;
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    let exported = GraphDocument::load_for_recluster_compatibility(&graph_json)?;
    assert_eq!(exported.nodes[0].label(), "Second");

    let bundle = directory.path().join("historical-out");
    let export = run(
        graphify,
        directory.path(),
        &[
            "history",
            "export",
            "HEAD",
            "--format",
            "graphify-out",
            "--output",
            bundle.to_str().ok_or("path")?,
        ],
    )?;
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(bundle.join("GRAPH_REPORT.md").is_file());
    assert!(bundle.join("graph.html").is_file());
    assert!(bundle.join("GRAPH_TREE.html").is_file());
    assert_eq!(
        std::fs::read(bundle.join("semantic/facts.bin"))?,
        vec![0, 1, 255]
    );
    assert_ne!(first.id, second.id);
    Ok(())
}

#[test]
fn completed_outcomes_handle_short_and_broken_writers() {
    struct ShortWriter(Vec<u8>);
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let length = bytes.len().min(2);
            self.0.extend_from_slice(&bytes[..length]);
            Ok(length)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    struct BrokenWriter;
    impl Write for BrokenWriter {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let outcome = compass_cli::run(
        compass_cli::Frontend::Compass,
        [std::ffi::OsString::from("--version")],
    );
    let mut short = ShortWriter(Vec::new());
    let mut diagnostics = Vec::new();
    assert_eq!(
        compass_cli::write_outcome(&outcome, &mut short, &mut diagnostics),
        0
    );
    assert_eq!(short.0, format!("{}\n", outcome.stdout).as_bytes());
    assert!(diagnostics.is_empty());

    let mut diagnostics = Vec::new();
    assert_eq!(
        compass_cli::write_outcome(&outcome, &mut BrokenWriter, &mut diagnostics),
        1
    );
    assert!(String::from_utf8_lossy(&diagnostics).contains("failed to write stdout"));
}

#[test]
fn diff_supports_summary_details_streaming_json_and_topology_filtering()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("README.md"), "old\n")?;
    git(directory.path(), &["add", "README.md"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "old"])?;
    let repository = Repository::discover(directory.path())?;
    let old_commit = repository.resolve("HEAD")?;
    let history = HistoryStore::create(&repository)?;
    let old_document: GraphDocument = serde_json::from_value(json!({
        "directed":true,
        "multigraph":true,
        "nodes":[
            {"id":"a","label":"A","community":0},
            {"id":"b","label":"B","community":0}
        ],
        "links":[
            {"source":"a","target":"b","relation":"calls","key":"relation"},
            {"source":"a","target":"a","relation":"references","key":"confidence","confidence":0.5}
        ],
        "built_at_commit":old_commit
    }))?;
    history.publish(PublishRequest {
        commit: old_commit.clone(),
        parents: repository.parents(&old_commit)?,
        fingerprint: std::iter::repeat_n('c', 64)
            .collect::<String>()
            .parse::<ExtractionFingerprint>()?,
        artifacts: GraphArtifacts {
            document: old_document,
            analysis: Some(json!({"communities":{"0":["a","b"]}})),
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
        make_preferred: true,
    })?;

    std::fs::write(directory.path().join("README.md"), "new\n")?;
    git(directory.path(), &["add", "README.md"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "new"])?;
    let new_commit = repository.resolve("HEAD")?;
    let new_document: GraphDocument = serde_json::from_value(json!({
        "directed":true,
        "multigraph":true,
        "nodes":[
            {"id":"a","label":"A","community":1},
            {"id":"b","label":"B","community":0},
            {"id":"c","label":"C","community":1}
        ],
        "links":[
            {"source":"a","target":"b","relation":"imports","key":"relation"},
            {"source":"a","target":"a","relation":"references","key":"confidence","confidence":0.9}
        ],
        "built_at_commit":new_commit
    }))?;
    history.publish(PublishRequest {
        commit: new_commit.clone(),
        parents: repository.parents(&new_commit)?,
        fingerprint: std::iter::repeat_n('d', 64)
            .collect::<String>()
            .parse::<ExtractionFingerprint>()?,
        artifacts: GraphArtifacts {
            document: new_document,
            analysis: Some(json!({"communities":{"0":["b"],"1":["a","c"]}})),
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
        make_preferred: true,
    })?;
    drop(history);

    let compass = env!("CARGO_BIN_EXE_compass");
    let graphify = env!("CARGO_BIN_EXE_graphify");
    let summary = run(compass, directory.path(), &["diff", "HEAD~1", "HEAD"])?;
    assert!(
        summary.status.success(),
        "{}",
        String::from_utf8_lossy(&summary.stderr)
    );
    assert!(String::from_utf8_lossy(&summary.stdout).contains("1 node added"));

    let json_output = run(
        compass,
        directory.path(),
        &["diff", "HEAD~1", "HEAD", "--format", "json"],
    )?;
    assert!(json_output.status.success());
    let changes: Vec<serde_json::Value> = serde_json::from_slice(&json_output.stdout)?;
    assert!(changes.iter().any(|change| change["record"] == "edge"));
    assert!(
        changes
            .iter()
            .any(|change| { change["record"] == "edge" && change["change"] == "changed" })
    );
    let order = |record: &str| match record {
        "node" => 0,
        "edge" => 1,
        "hyperedge" => 2,
        "analysis" => 3,
        "metadata" => 4,
        _ => 5,
    };
    assert!(changes.windows(2).all(|pair| {
        order(pair[0]["record"].as_str().unwrap_or_default())
            <= order(pair[1]["record"].as_str().unwrap_or_default())
    }));

    let topology = run(
        compass,
        directory.path(),
        &["diff", "HEAD~1", "HEAD", "--format=json", "--topology-only"],
    )?;
    let topology_changes: Vec<serde_json::Value> = serde_json::from_slice(&topology.stdout)?;
    assert!(
        topology_changes
            .iter()
            .all(|change| { !matches!(change["record"].as_str(), Some("analysis" | "metadata")) })
    );

    let detailed = run(
        compass,
        directory.path(),
        &["diff", "HEAD~1", "HEAD", "--detailed"],
    )?;
    assert!(detailed.status.success());
    assert!(String::from_utf8_lossy(&detailed.stdout).contains("edge changed"));

    let empty = run(compass, directory.path(), &["diff", "HEAD", "HEAD"])?;
    assert_eq!(String::from_utf8_lossy(&empty.stdout), "no graph changes\n");
    let alias = run(
        graphify,
        directory.path(),
        &["diff", "HEAD~1", "HEAD", "--format", "json"],
    )?;
    assert_eq!(json_output.status.code(), alias.status.code());
    assert_eq!(json_output.stdout, alias.stdout);
    assert_eq!(json_output.stderr, alias.stderr);
    Ok(())
}

#[test]
fn query_path_and_explain_read_the_selected_materialized_commit()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("service.txt"), "legacy\n")?;
    git(directory.path(), &["add", "service.txt"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "legacy"])?;
    let repository = Repository::discover(directory.path())?;
    let legacy_commit = repository.resolve("HEAD")?;
    let history = HistoryStore::create(&repository)?;
    let publish = |commit: compass_history::CommitId,
                   fingerprint: char,
                   service_id: &str,
                   service_label: &str|
     -> Result<(), Box<dyn std::error::Error>> {
        let document: GraphDocument = serde_json::from_value(json!({
            "directed":true,
            "multigraph":false,
            "nodes":[
                {"id":service_id,"label":service_label,"source_file":"service.rs"},
                {"id":"database","label":"Database","source_file":"database.rs"}
            ],
            "links":[
                {"source":service_id,"target":"database","relation":"calls","confidence":"EXTRACTED"}
            ],
            "built_at_commit":commit
        }))?;
        history.publish(PublishRequest {
            parents: repository.parents(&commit)?,
            commit,
            fingerprint: std::iter::repeat_n(fingerprint, 64)
                .collect::<String>()
                .parse::<ExtractionFingerprint>()?,
            artifacts: GraphArtifacts {
                document,
                analysis: None,
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
            make_preferred: true,
        })?;
        Ok(())
    };
    publish(legacy_commit, 'e', "legacy", "LegacyService")?;

    std::fs::write(directory.path().join("service.txt"), "replacement\n")?;
    git(directory.path(), &["add", "service.txt"])?;
    git(
        directory.path(),
        &["commit", "--quiet", "-m", "replacement"],
    )?;
    let replacement_commit = repository.resolve("HEAD")?;
    publish(replacement_commit, 'f', "replacement", "ReplacementService")?;
    drop(history);

    let compass = env!("CARGO_BIN_EXE_compass");
    let graphify = env!("CARGO_BIN_EXE_graphify");
    let query = run(
        compass,
        directory.path(),
        &["query", "legacy service", "--at", "HEAD~1"],
    )?;
    assert!(
        query.status.success(),
        "{}",
        String::from_utf8_lossy(&query.stderr)
    );
    let query_text = String::from_utf8_lossy(&query.stdout);
    assert!(query_text.contains("LegacyService"));
    assert!(!query_text.contains("ReplacementService"));

    let path = run(
        compass,
        directory.path(),
        &["path", "LegacyService", "Database", "--at=HEAD~1"],
    )?;
    assert!(
        path.status.success(),
        "{}",
        String::from_utf8_lossy(&path.stderr)
    );
    assert!(String::from_utf8_lossy(&path.stdout).contains("LegacyService"));

    let explain = run(
        compass,
        directory.path(),
        &["explain", "LegacyService", "--at", "HEAD~1"],
    )?;
    assert!(
        explain.status.success(),
        "{}",
        String::from_utf8_lossy(&explain.stderr)
    );
    assert!(String::from_utf8_lossy(&explain.stdout).contains("Database"));

    let alias = run(
        graphify,
        directory.path(),
        &["query", "legacy service", "--at", "HEAD~1"],
    )?;
    assert_eq!(query.status.code(), alias.status.code());
    assert_eq!(query.stdout, alias.stdout);

    std::fs::write(directory.path().join("service.txt"), "unbuilt\n")?;
    git(directory.path(), &["add", "service.txt"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "unbuilt"])?;
    let missing = run(
        compass,
        directory.path(),
        &["query", "service", "--at", "HEAD"],
    )?;
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("compass history build HEAD"));
    Ok(())
}
