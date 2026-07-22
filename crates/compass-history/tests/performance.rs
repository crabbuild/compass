use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use compass_history::{
    ChangeSink, CompletionEvidence, ExtractionFingerprint, GraphArtifacts, GraphChange,
    HistoryStore, PublishRequest, Repository,
};
use compass_model::GraphDocument;
use serde_json::{Value, json};

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

fn commit(directory: &Path, message: &str) -> Result<(), Box<dyn std::error::Error>> {
    git(directory, &["add", "fixture.rs"])?;
    git(directory, &["commit", "--quiet", "-m", message])
}

fn graph(commit: &str, changed: bool) -> Result<GraphDocument, Box<dyn std::error::Error>> {
    let nodes = (0..2_000)
        .map(|index| {
            json!({
                "id": format!("node-{index:04}"),
                "label": if changed && index == 1_000 {
                    "ChangedService".to_owned()
                } else {
                    format!("Service{index:04}")
                },
                "community": index % 16,
                "source_file": format!("src/module_{:04}.rs", index / 4),
                "source_location": format!("L{}", index * 3 + 1),
                "kind": "function",
                "summary": format!("Deterministic fixture service number {index}")
            })
        })
        .collect::<Vec<Value>>();
    let links = (0..1_999)
        .map(|index| {
            json!({
                "source": format!("node-{index:04}"),
                "target": format!("node-{:04}", index + 1),
                "relation": "calls",
                "confidence": "EXTRACTED"
            })
        })
        .collect::<Vec<Value>>();
    Ok(serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {"name": "performance-fixture"},
        "nodes": nodes,
        "links": links,
        "built_at_commit": commit
    }))?)
}

fn request(
    repository: &Repository,
    changed: bool,
) -> Result<PublishRequest, Box<dyn std::error::Error>> {
    let commit = repository.resolve("HEAD")?;
    Ok(PublishRequest {
        parents: repository.parents(&commit)?,
        artifacts: GraphArtifacts {
            document: graph(commit.as_str(), changed)?,
            analysis: Some(json!({"communities": 16, "fixture": true})),
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
        fingerprint: std::iter::repeat_n(if changed { 'b' } else { 'a' }, 64)
            .collect::<String>()
            .parse::<ExtractionFingerprint>()?,
        commit,
        make_preferred: true,
    })
}

#[derive(Default)]
struct CountingSink {
    changes: usize,
    peak_buffered_records: usize,
}

impl ChangeSink for CountingSink {
    fn change(&mut self, _change: GraphChange) -> Result<(), compass_history::HistoryError> {
        self.changes += 1;
        // The callback consumes each record before the stream advances.
        self.peak_buffered_records = 1;
        Ok(())
    }
}

#[test]
fn performance_maintenance_child() -> Result<(), Box<dyn std::error::Error>> {
    let Some(repository) = std::env::var_os("COMPASS_PERF_LOCK_REPOSITORY") else {
        return Ok(());
    };
    let marker =
        PathBuf::from(std::env::var_os("COMPASS_PERF_LOCK_MARKER").ok_or("missing lock marker")?);
    let repository = Repository::discover(Path::new(&repository))?;
    let history = HistoryStore::create(&repository)?;
    let _maintenance = history.maintenance()?;
    std::fs::write(marker, b"ready")?;
    std::thread::sleep(Duration::from_millis(300));
    Ok(())
}

#[test]
#[ignore = "performance evidence; run explicitly"]
fn small_change_reuses_content_addressed_nodes() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    git(directory.path(), &["init", "--quiet"])?;
    git(directory.path(), &["config", "user.name", "Compass Test"])?;
    git(
        directory.path(),
        &["config", "user.email", "compass@example.invalid"],
    )?;
    std::fs::write(directory.path().join("fixture.rs"), "pub struct Base;\n")?;
    commit(directory.path(), "base")?;

    let repository = Repository::discover(directory.path())?;
    let history = HistoryStore::create(&repository)?;
    let initial_nodes = history.plan_gc(false)?.candidate_nodes;
    let started = Instant::now();
    let first = history.publish(request(&repository, false)?)?;
    let cold_publish = started.elapsed();
    let after_first = history.plan_gc(false)?.candidate_nodes;

    std::fs::write(directory.path().join("fixture.rs"), "pub struct Changed;\n")?;
    commit(directory.path(), "one-file-change")?;

    let marker = directory.path().join("maintenance-ready");
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--exact", "performance_maintenance_child", "--nocapture"])
        .env("COMPASS_PERF_LOCK_REPOSITORY", directory.path())
        .env("COMPASS_PERF_LOCK_MARKER", &marker)
        .spawn()?;
    let marker_deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() {
        if Instant::now() >= marker_deadline {
            let _ = child.kill();
            return Err("contention child did not acquire maintenance lock".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let started = Instant::now();
    let second = history.publish(request(&repository, true)?)?;
    let seeded_publish_with_contention = started.elapsed();
    if !child.wait()?.success() {
        return Err("contention child failed".into());
    }

    let after_second = history.plan_gc(false)?.candidate_nodes;
    let sharing = history.structural_sharing(&first.id, &second.id)?;
    assert!(sharing.shared_nodes > 0);
    assert!(after_second - after_first < sharing.second_total_nodes);

    let mut sink = CountingSink::default();
    let started = Instant::now();
    history.diff(&first.id, &second.id, &mut sink)?;
    let diff_latency = started.elapsed();
    assert!(sink.changes > 0);

    let started = Instant::now();
    let reconstructed = history.artifacts(&second.id)?;
    let query_latency = started.elapsed();
    let query_memory_bytes = serde_json::to_vec(&reconstructed.artifacts.document)?.len();

    let started = Instant::now();
    let plan = history.plan_gc(false)?;
    let gc_plan_latency = started.elapsed();
    let started = Instant::now();
    let sweep = history.sweep_gc(plan)?;
    let gc_sweep_latency = started.elapsed();

    println!(
        "cold_publish={cold_publish:?} seeded_publish_with_multi_process_contention={seeded_publish_with_contention:?}"
    );
    println!(
        "logical_nodes_initial={initial_nodes} after_first={after_first} after_second={after_second} second_growth={} reusable_pages={:?} reusable_bytes={:?}",
        after_second - after_first,
        sweep.reusable_pages,
        sweep.reusable_bytes
    );
    println!("structural_sharing={}", serde_json::to_string(&sharing)?);
    println!(
        "diff_latency={diff_latency:?} changes={} peak_buffered_records={}",
        sink.changes, sink.peak_buffered_records
    );
    println!("query_latency={query_latency:?} reconstructed_graph_bytes={query_memory_bytes}");
    println!(
        "gc_plan_latency={gc_plan_latency:?} gc_sweep_latency={gc_sweep_latency:?} deleted_nodes={} deleted_bytes={}",
        sweep.deleted_nodes, sweep.deleted_bytes
    );
    Ok(())
}
