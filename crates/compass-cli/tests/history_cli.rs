use std::path::Path;
use std::process::{Command, Output};

use compass_history::{
    CompletionEvidence, ExtractionFingerprint, GraphArtifacts, HistoryConfig, HistoryQueue,
    HistoryStore, JobRequest, JobState, PublishRequest, Repository,
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
    let status_json = run(
        compass,
        directory.path(),
        &["history", "status", "HEAD", "--format=json"],
    )?;
    assert!(status_json.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&status_json.stdout)?["store"],
        false
    );
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
    let incompatible_json = run(
        compass,
        directory.path(),
        &["history", "status", "HEAD", "--format=json"],
    )?;
    assert_eq!(incompatible_json.status.code(), Some(1));
    let incompatible_json: serde_json::Value = serde_json::from_slice(&incompatible_json.stdout)?;
    assert_eq!(incompatible_json["compatible"], false);
    assert_eq!(incompatible_json["validation"]["valid"], false);
    Ok(())
}

#[test]
fn enable_disable_are_explicit_idempotent_and_invalid_profiles_roll_back()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("fixture.rs"), "pub struct Fixture;\n")?;
    git(directory.path(), &["add", "fixture.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;
    let compass = env!("CARGO_BIN_EXE_compass");
    let enabled = run(compass, directory.path(), &["history", "enable"])?;
    assert!(enabled.status.success());
    let config = directory.path().join(".git/compass/config.json");
    let before = std::fs::read(&config)?;
    let invalid = run(
        compass,
        directory.path(),
        &["history", "enable", "--code-only"],
    )?;
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(std::fs::read(&config)?, before);
    let status = run(
        compass,
        directory.path(),
        &["history", "status", "HEAD", "--format=json"],
    )?;
    let status: serde_json::Value = serde_json::from_slice(&status.stdout)?;
    assert_eq!(status["enabled"], true);
    assert!(status["profile_digest"].as_str().is_some());
    for _ in 0..2 {
        let disabled = run(compass, directory.path(), &["history", "disable"])?;
        assert!(disabled.status.success());
    }
    assert!(
        directory
            .path()
            .join(".git/compass/history.sqlite")
            .is_file()
    );
    Ok(())
}

#[test]
fn worker_drains_fifo_after_an_earlier_job_fails() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(
        directory.path().join("service.rs"),
        "pub struct DrainService;\n",
    )?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;

    let compass = env!("CARGO_BIN_EXE_compass");
    let enabled = run(compass, directory.path(), &["history", "enable"])?;
    assert!(enabled.status.success());
    let repository = Repository::discover(directory.path())?;
    let profile = HistoryConfig::load(&repository)?
        .profile
        .ok_or("enabled profile")?;
    let queue = HistoryQueue::for_repository(&repository)?;
    let mut invalid_profile_jobs = Vec::new();
    for (key, value) in [
        ("unsupported", "field"),
        ("compass_version", "incompatible"),
        ("gitignore", "maybe"),
        ("resolution", "NaN"),
        ("exclude_hubs", "NaN"),
        ("token_budget", "0"),
        ("semantic_mode", "invalid"),
        ("semantic_prompt_sha256", "invalid"),
        ("provider", "unsupported"),
        ("model", "model-without-provider"),
        ("provider_endpoint", "https://example.invalid"),
    ] {
        let mut invalid = profile.clone();
        invalid.insert(key, value)?;
        invalid_profile_jobs.push(queue.enqueue(JobRequest {
            commit: repository.resolve("HEAD")?,
            profile: invalid,
        })?);
    }
    for (key, value) in [
        ("provider_temperature", "NaN"),
        ("provider_max_output_tokens", "0"),
    ] {
        let mut invalid = profile.clone();
        invalid.insert("provider", "openai")?;
        invalid.insert("model", "fixture-model")?;
        invalid.insert(key, value)?;
        invalid_profile_jobs.push(queue.enqueue(JobRequest {
            commit: repository.resolve("HEAD")?,
            profile: invalid,
        })?);
    }
    let failed_id = queue.enqueue(JobRequest {
        commit: "ffffffffffffffffffffffffffffffffffffffff".parse()?,
        profile: profile.clone(),
    })?;
    let published_id = queue.enqueue(JobRequest {
        commit: repository.resolve("HEAD")?,
        profile,
    })?;

    let worker = run(compass, directory.path(), &["history-worker"])?;
    assert!(
        worker.status.success(),
        "{}",
        String::from_utf8_lossy(&worker.stderr)
    );
    assert_eq!(
        queue.get(&failed_id)?.ok_or("failed job")?.state,
        JobState::Failed
    );
    for job_id in invalid_profile_jobs {
        assert_eq!(
            queue.get(&job_id)?.ok_or("invalid profile job")?.state,
            JobState::Failed
        );
    }
    assert_eq!(
        queue.get(&published_id)?.ok_or("published job")?.state,
        JobState::Published
    );
    Ok(())
}

#[test]
fn worker_reconciles_catalog_and_preferred_crash_windows() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("fixture.rs"), "pub struct Fixture;\n")?;
    git(directory.path(), &["add", "fixture.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let history = HistoryStore::create(&repository)?;
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {"name": "reconcile"},
        "nodes": [{"id": "fixture", "label": "Fixture", "community": 0}],
        "links": [],
        "built_at_commit": commit
    }))?;
    let candidate = history.publish(PublishRequest {
        commit: commit.clone(),
        parents: repository.parents(&commit)?,
        fingerprint: std::iter::repeat_n('d', 64)
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
        make_preferred: false,
    })?;
    let profile = compass_history::BuildProfile::default();
    let queue = HistoryQueue::for_repository(&repository)?;
    let first_id = queue.enqueue_rebuild(
        JobRequest {
            commit: commit.clone(),
            profile: profile.clone(),
        },
        false,
    )?;
    let first = queue.claim_or_join(&first_id)?.ok_or("first claim")?;
    queue.annotate(&first, None, Some(candidate.id.clone()), None)?;
    queue.transition(&first, JobState::Validating, None)?;
    expire_job_lease(&queue, &first)?;

    let compass = env!("CARGO_BIN_EXE_compass");
    let worker = run(compass, directory.path(), &["history-worker"])?;
    assert!(worker.status.success());
    let first = queue.get(&first_id)?.ok_or("first job")?;
    assert_eq!(first.state, JobState::Published);
    assert_eq!(first.preferred, Some(true));
    assert_eq!(
        history.preferred(&commit)?.ok_or("preferred")?.id,
        candidate.id
    );

    let second_id = queue.enqueue_rebuild(
        JobRequest {
            commit: commit.clone(),
            profile,
        },
        false,
    )?;
    let second = queue.claim_or_join(&second_id)?.ok_or("second claim")?;
    queue.annotate(
        &second,
        None,
        Some(candidate.id.clone()),
        Some(candidate.id.clone()),
    )?;
    queue.transition(&second, JobState::Validating, None)?;
    expire_job_lease(&queue, &second)?;
    let worker = run(compass, directory.path(), &["history-worker"])?;
    assert!(worker.status.success());
    let second = queue.get(&second_id)?.ok_or("second job")?;
    assert_eq!(second.state, JobState::Published);
    assert_eq!(second.preferred, Some(true));
    Ok(())
}

fn expire_job_lease(
    queue: &HistoryQueue,
    job: &compass_history::ClaimedJob,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = queue
        .root()
        .join("leases")
        .join(format!("{}-{}.lease", job.commit, job.profile_digest));
    let mut lease: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    lease["expires_at_millis"] = 0_u64.into();
    std::fs::write(path, serde_json::to_vec(&lease)?)?;
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
    let list_text = run(compass, directory.path(), &["history", "list", "HEAD"])?;
    assert!(list_text.status.success());
    assert!(String::from_utf8_lossy(&list_text.stdout).contains("alternate"));
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
    let show_text = run(
        compass,
        directory.path(),
        &["history", "show", &first.id.as_hex()],
    )?;
    assert!(String::from_utf8_lossy(&show_text.stdout).contains("realization:"));
    let preferred = run(
        compass,
        directory.path(),
        &["history", "prefer", "HEAD", &second.id.as_hex()],
    )?;
    assert!(preferred.status.success());
    let preferred_json = run(
        compass,
        directory.path(),
        &[
            "history",
            "prefer",
            "HEAD",
            &first.id.as_hex(),
            "--format=json",
        ],
    )?;
    assert!(preferred_json.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&preferred_json.stdout)?["preferred"],
        first.id.as_hex()
    );

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
    assert_eq!(exported.nodes[0].label(), "First");

    let graph_directory = directory.path().join("not-a-graph-file");
    std::fs::create_dir(&graph_directory)?;
    let rejected = run(
        compass,
        directory.path(),
        &[
            "history",
            "export",
            "HEAD",
            "--format=graph-json",
            "--output",
            graph_directory.to_str().ok_or("path")?,
        ],
    )?;
    assert_eq!(rejected.status.code(), Some(1));

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
    let existing_bundle = run(
        compass,
        directory.path(),
        &[
            "history",
            "export",
            "HEAD",
            "--format=graphify-out",
            "--output",
            bundle.to_str().ok_or("path")?,
        ],
    )?;
    assert_eq!(existing_bundle.status.code(), Some(1));
    let invalid_format = run(
        compass,
        directory.path(),
        &[
            "history",
            "export",
            "HEAD",
            "--format=yaml",
            "--output=unused",
        ],
    )?;
    assert_eq!(invalid_format.status.code(), Some(2));
    let status = run(
        compass,
        directory.path(),
        &["history", "status", "HEAD", "--format=json"],
    )?;
    assert!(status.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&status.stdout)?["validation"]["valid"],
        true
    );
    assert_ne!(first.id, second.id);
    Ok(())
}

#[test]
fn gc_requires_explicit_confirmation_for_non_preferred_realizations()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("fixture.rs"), "pub struct Fixture;\n")?;
    git(directory.path(), &["add", "fixture.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;
    let repository = Repository::discover(directory.path())?;
    let commit = repository.resolve("HEAD")?;
    let history = HistoryStore::create(&repository)?;
    for (fingerprint, label) in [('a', "First"), ('b', "Second")] {
        let document: GraphDocument = serde_json::from_value(json!({
            "directed": true,
            "multigraph": false,
            "nodes": [{"id": "fixture", "label": label}],
            "links": [],
            "built_at_commit": commit
        }))?;
        history.publish(PublishRequest {
            commit: commit.clone(),
            parents: Vec::new(),
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
    }
    drop(history);

    let compass = env!("CARGO_BIN_EXE_compass");
    let dry = run(
        compass,
        directory.path(),
        &["history", "gc", "--prune-non-preferred", "--format=json"],
    )?;
    assert!(dry.status.success());
    let dry: serde_json::Value = serde_json::from_slice(&dry.stdout)?;
    assert_eq!(dry["applied"], false);
    assert_eq!(dry["plan"]["prunable_realizations"], 1);
    assert_eq!(
        HistoryStore::open_existing(&repository)?
            .ok_or("store")?
            .list(None)?
            .len(),
        2
    );

    let dry_text = run(
        compass,
        directory.path(),
        &["history", "gc", "--prune-non-preferred"],
    )?;
    assert!(String::from_utf8_lossy(&dry_text.stdout).contains("GC plan (not applied)"));

    let applied = run(
        compass,
        directory.path(),
        &["history", "gc", "--prune-non-preferred", "--yes"],
    )?;
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert!(String::from_utf8_lossy(&applied.stdout).contains("not compacted"));
    assert_eq!(
        HistoryStore::open_existing(&repository)?
            .ok_or("store")?
            .list(None)?
            .len(),
        1
    );

    let json_sweep = run(
        compass,
        directory.path(),
        &["history", "gc", "--format=json"],
    )?;
    assert!(json_sweep.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&json_sweep.stdout)?["applied"],
        true
    );

    for arguments in [
        vec!["history", "gc", directory.path().to_str().ok_or("path")?],
        vec!["history", "gc", "--yes"],
        vec!["history", "gc", "--format"],
        vec!["history", "gc", "--format=yaml"],
        vec!["history", "gc", "--format=json", "--format", "text"],
    ] {
        assert_eq!(
            run(compass, directory.path(), &arguments)?.status.code(),
            Some(2)
        );
    }
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

    let cql = run(
        compass,
        directory.path(),
        &[
            "query",
            "--cql",
            "MATCH (a)-[:CALLS]->(b) RETURN a.id AS caller",
            "--at",
            "HEAD~1",
            "--format=json",
        ],
    )?;
    assert!(
        cql.status.success(),
        "{}",
        String::from_utf8_lossy(&cql.stderr)
    );
    let cql: serde_json::Value = serde_json::from_slice(&cql.stdout)?;
    assert_eq!(cql["rows"][0]["caller"]["value"], "legacy");

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

    Ok(())
}

#[test]
fn missing_code_only_commit_is_built_on_first_query() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(
        directory.path().join("service.rs"),
        "pub struct OldService;\nimpl OldService { pub fn run(&self) {} }\n",
    )?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "old"])?;
    std::fs::write(
        directory.path().join("service.rs"),
        "pub struct NewService;\nimpl NewService { pub fn run(&self) {} }\n",
    )?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "new"])?;

    let compass = env!("CARGO_BIN_EXE_compass");
    let query = run(
        compass,
        directory.path(),
        &["query", "OldService", "--at", "HEAD~1"],
    )?;
    assert!(
        query.status.success(),
        "{}",
        String::from_utf8_lossy(&query.stderr)
    );
    assert!(String::from_utf8_lossy(&query.stdout).contains("OldService"));
    assert!(!directory.path().join("graphify-out").exists());

    let status = run(compass, directory.path(), &["history", "status", "HEAD~1"])?;
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("validation: valid"));
    Ok(())
}

#[test]
fn build_rebuild_and_unseen_diff_publish_complete_realizations()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(
        directory.path().join("service.rs"),
        "pub struct FirstService;\n",
    )?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "first"])?;
    std::fs::write(
        directory.path().join("service.rs"),
        "pub struct SecondService;\n",
    )?;
    git(directory.path(), &["add", "service.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "second"])?;

    let compass = env!("CARGO_BIN_EXE_compass");
    let diff = run(
        compass,
        directory.path(),
        &["diff", "HEAD~1", "HEAD", "--format=json"],
    )?;
    assert!(
        diff.status.success(),
        "{}",
        String::from_utf8_lossy(&diff.stderr)
    );
    let _: Vec<serde_json::Value> = serde_json::from_slice(&diff.stdout)?;
    let progress = String::from_utf8_lossy(&diff.stderr);
    assert!(progress.contains("building complete graph"));
    assert!(progress.contains("publishing immutable realization"));

    let first = run(
        compass,
        directory.path(),
        &["history", "build", "HEAD", "--format=json"],
    )?;
    assert!(first.status.success());
    let first: serde_json::Value = serde_json::from_slice(&first.stdout)?;
    for field in [
        "commit",
        "realization",
        "fingerprint",
        "nodes",
        "edges",
        "hyperedges",
        "analysis_records",
        "metadata_records",
        "preferred",
    ] {
        assert!(first.get(field).is_some(), "missing {field}");
    }
    assert!(first["preferred"].as_bool().unwrap_or(false));

    let rebuilt = run(
        compass,
        directory.path(),
        &[
            "history",
            "rebuild",
            "HEAD",
            "--resolution=2",
            "--format=json",
        ],
    )?;
    assert!(
        rebuilt.status.success(),
        "{}",
        String::from_utf8_lossy(&rebuilt.stderr)
    );
    let rebuilt: serde_json::Value = serde_json::from_slice(&rebuilt.stdout)?;
    assert_ne!(first["realization"], rebuilt["realization"]);
    let old = run(
        compass,
        directory.path(),
        &[
            "history",
            "show",
            first["realization"].as_str().ok_or("realization")?,
            "--format=json",
        ],
    )?;
    assert!(old.status.success());
    Ok(())
}

#[test]
fn historical_build_uses_only_the_exact_commit_tree_and_historical_ignore_policy()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(
        directory.path().join(".gitignore"),
        "committed_ignored.rs\n",
    )?;
    std::fs::write(
        directory.path().join("committed_ignored.rs"),
        "pub struct CommittedIgnored;\n",
    )?;
    std::fs::write(
        directory.path().join("explicit.rs"),
        "pub struct ExplicitlyExcluded;\n",
    )?;
    std::fs::write(
        directory.path().join("local.rs"),
        "pub struct LocalIgnoreMustNotApply;\n",
    )?;
    git(
        directory.path(),
        &["add", ".gitignore", "explicit.rs", "local.rs"],
    )?;
    git(directory.path(), &["add", "-f", "committed_ignored.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;
    std::fs::write(directory.path().join(".git/info/exclude"), "local.rs\n")?;
    std::fs::write(
        directory.path().join("uncommitted.rs"),
        "pub struct UncommittedMustNotAppear;\n",
    )?;

    let compass = env!("CARGO_BIN_EXE_compass");
    let built = run(
        compass,
        directory.path(),
        &[
            "history",
            "build",
            "HEAD",
            "--exclude",
            "explicit.rs",
            "--format=json",
        ],
    )?;
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let query = run(
        compass,
        directory.path(),
        &["query", "LocalIgnoreMustNotApply", "--at", "HEAD"],
    )?;
    assert!(query.status.success());
    assert!(String::from_utf8_lossy(&query.stdout).contains("LocalIgnoreMustNotApply"));
    for absent in [
        "CommittedIgnored",
        "ExplicitlyExcluded",
        "UncommittedMustNotAppear",
    ] {
        let query = run(
            compass,
            directory.path(),
            &["query", absent, "--at", "HEAD"],
        )?;
        assert!(!String::from_utf8_lossy(&query.stdout).contains(absent));
    }
    assert!(!directory.path().join("graphify-out").exists());
    Ok(())
}

#[test]
fn provider_failure_leaves_no_preferred_realization() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("document.md"), "# Semantic fixture\n")?;
    git(directory.path(), &["add", "document.md"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;

    let mut command = Command::new(env!("CARGO_BIN_EXE_compass"));
    command
        .args([
            "history",
            "build",
            "HEAD",
            "--backend",
            "openai",
            "--model",
            "history-test-model",
        ])
        .current_dir(directory.path());
    for key in [
        "OPENAI_API_KEY",
        "OPENAI_KEY",
        "AZURE_OPENAI_API_KEY",
        "DEEPSEEK_API_KEY",
    ] {
        command.env_remove(key);
    }
    let failed = command.output()?;
    assert_eq!(failed.status.code(), Some(1));
    let status = run(
        env!("CARGO_BIN_EXE_compass"),
        directory.path(),
        &["history", "status", "HEAD"],
    )?;
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("preferred: none"));
    Ok(())
}

#[test]
fn merge_commit_materialization_reads_the_exact_merge_tree()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("base.rs"), "pub struct Base;\n")?;
    git(directory.path(), &["add", "base.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "base"])?;
    git(directory.path(), &["checkout", "--quiet", "-b", "feature"])?;
    std::fs::write(
        directory.path().join("feature.rs"),
        "pub struct FeatureSide;\n",
    )?;
    git(directory.path(), &["add", "feature.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "feature"])?;
    git(directory.path(), &["checkout", "--quiet", "-"])?;
    std::fs::write(directory.path().join("main.rs"), "pub struct MainSide;\n")?;
    git(directory.path(), &["add", "main.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "main"])?;
    git(
        directory.path(),
        &["merge", "--quiet", "--no-ff", "feature", "-m", "merge"],
    )?;

    let compass = env!("CARGO_BIN_EXE_compass");
    let built = run(compass, directory.path(), &["history", "build", "HEAD"])?;
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    for symbol in ["FeatureSide", "MainSide"] {
        let query = run(
            compass,
            directory.path(),
            &["query", symbol, "--at", "HEAD"],
        )?;
        assert!(query.status.success());
        assert!(String::from_utf8_lossy(&query.stdout).contains(symbol));
    }
    Ok(())
}

#[test]
fn normal_graph_export_and_historical_queries_are_semantically_identical()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(
        directory.path().join("auth.rs"),
        "pub fn authenticate() { validate_session(); }\npub fn validate_session() {}\n",
    )?;
    git(directory.path(), &["add", "auth.rs"])?;
    git(directory.path(), &["commit", "--quiet", "-m", "auth graph"])?;

    let compass = env!("CARGO_BIN_EXE_compass");
    let extracted = run(compass, directory.path(), &["extract", ".", "--no-viz"])?;
    assert!(
        extracted.status.success(),
        "{}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    let graph_path = directory.path().join("graphify-out/graph.json");
    let original: serde_json::Value = serde_json::from_slice(&std::fs::read(&graph_path)?)?;

    let built = run(compass, directory.path(), &["history", "build", "HEAD"])?;
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let exported_path = directory.path().join("historical-graph.json");
    let exported = run(
        compass,
        directory.path(),
        &[
            "history",
            "export",
            "HEAD",
            "--format",
            "graph-json",
            "--output",
            exported_path.to_str().ok_or("export path")?,
        ],
    )?;
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let reconstructed: serde_json::Value = serde_json::from_slice(&std::fs::read(&exported_path)?)?;
    assert_eq!(
        normalize_graph(original.clone()),
        normalize_graph(reconstructed)
    );

    let first_link = original
        .get("links")
        .and_then(serde_json::Value::as_array)
        .and_then(|links| links.first())
        .ok_or("normal Rust graph has no edge")?;
    let source = first_link["source"].as_str().ok_or("edge source")?;
    let target = first_link["target"].as_str().ok_or("edge target")?;
    let graph = graph_path.to_str().ok_or("graph path")?;
    for (file_args, history_args) in [
        (
            vec!["query", source, "--graph", graph],
            vec!["query", source, "--at", "HEAD"],
        ),
        (
            vec!["path", source, target, "--graph", graph],
            vec!["path", source, target, "--at", "HEAD"],
        ),
        (
            vec!["explain", source, "--graph", graph],
            vec!["explain", source, "--at", "HEAD"],
        ),
    ] {
        let from_file = run(compass, directory.path(), &file_args)?;
        let from_history = run(compass, directory.path(), &history_args)?;
        assert!(
            from_file.status.success(),
            "{}",
            String::from_utf8_lossy(&from_file.stderr)
        );
        assert_eq!(from_file.status.code(), from_history.status.code());
        assert_eq!(from_file.stdout, from_history.stdout, "{file_args:?}");
        assert_eq!(from_file.stderr, from_history.stderr, "{file_args:?}");
    }
    Ok(())
}

fn normalize_graph(mut graph: serde_json::Value) -> serde_json::Value {
    for field in ["nodes", "links", "edges"] {
        if let Some(records) = graph
            .get_mut(field)
            .and_then(serde_json::Value::as_array_mut)
        {
            records.sort_by_key(serde_json::Value::to_string);
        }
    }
    graph
}
