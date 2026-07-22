use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use compass_history::{
    BuildProfile, CompletionEvidence, ExtractionFingerprint, GraphArtifacts, HistoryQueue,
    HistoryStore, JobRequest, JobState, PublishRequest, Repository,
};
use compass_model::GraphDocument;
use prolly::{Config, ManifestStoreScan, Prolly};
use prolly_store_sqlite::SqliteStore;
use serde_json::json;

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
    repository: Repository,
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
        std::fs::write(directory.path().join("fixture.rs"), "pub struct Fixture;\n")?;
        git(directory.path(), &["add", "fixture.rs"])?;
        git(directory.path(), &["commit", "--quiet", "-m", "fixture"])?;
        let path = directory.path().to_path_buf();
        let repository = Repository::discover(&path)?;
        Ok(Self {
            _directory: directory,
            path,
            repository,
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

fn request(fingerprint: char, label: &str) -> Result<PublishRequest, Box<dyn std::error::Error>> {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "nodes": [{"id": "fixture", "label": label}],
        "links": []
    }))?;
    Ok(PublishRequest {
        commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?,
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
    })
}

#[test]
fn pruning_is_explicit_checked_and_retains_the_store_format()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let history = HistoryStore::create(&fixture.repository)?;
    let first = history.publish(request('a', "First")?)?;
    let second = history.publish(request('b', "Second")?)?;
    let plan = history.plan_gc(true)?;
    assert_eq!(plan.prunable_realizations, 1);
    assert_eq!(
        plan.prunable_realization_ids.as_slice(),
        std::slice::from_ref(&first.id)
    );
    assert_eq!(plan.prunable_named_roots.len(), 6);
    let swept = history.sweep_gc(plan)?;
    assert_eq!(swept.deleted_named_roots, 6);
    assert!(history.get(&first.id).is_err());
    assert!(history.get(&second.id).is_ok());
    drop(history);
    assert!(HistoryStore::open_existing(&fixture.repository)?.is_some());
    Ok(())
}

#[test]
fn stale_plan_after_preferred_change_removes_nothing() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let history = HistoryStore::create(&fixture.repository)?;
    let first = history.publish(request('a', "First")?)?;
    let second = history.publish(request('b', "Second")?)?;
    let commit = first.version.git_commit.parse()?;
    let plan = history.plan_gc(true)?;
    assert!(history.compare_and_set_preferred(&commit, Some(&second.id), &first.id)?);
    let error = match history.sweep_gc(plan) {
        Ok(_) => return Err("stale plan unexpectedly swept".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("stale"));
    assert!(history.get(&first.id).is_ok());
    assert!(history.get(&second.id).is_ok());
    Ok(())
}

#[test]
fn cleanup_respects_age_terminal_state_and_live_leases() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let history = HistoryStore::create(&fixture.repository)?;
    let queue = HistoryQueue::for_repository(&fixture.repository)?;
    let profile = BuildProfile::default();
    let old_terminal = queue.enqueue(JobRequest {
        commit: "1111111111111111111111111111111111111111".parse()?,
        profile: profile.clone(),
    })?;
    let claimed = queue.claim_or_join(&old_terminal)?.ok_or("claim")?;
    queue.finish(&claimed, JobState::Failed, None, Some("old"))?;
    age_job(&queue, &old_terminal)?;
    let young_terminal = queue.enqueue(JobRequest {
        commit: "2222222222222222222222222222222222222222".parse()?,
        profile: profile.clone(),
    })?;
    let claimed = queue.claim_or_join(&young_terminal)?.ok_or("claim")?;
    queue.finish(&claimed, JobState::Failed, None, Some("young"))?;
    let active = queue.enqueue(JobRequest {
        commit: "3333333333333333333333333333333333333333".parse()?,
        profile,
    })?;
    let active_claim = queue.claim_or_join(&active)?.ok_or("active claim")?;

    let tmp = history.root().join("tmp");
    std::fs::create_dir_all(&tmp)?;
    let stale = tmp.join("worktree-stale");
    std::fs::create_dir(&stale)?;
    age_path(&stale)?;
    let plan = history.plan_gc(false)?;
    assert_eq!(plan.expired_job_records, [format!("{old_terminal}.json")]);
    assert!(plan.expired_temp_directories.is_empty());
    queue.finish(&active_claim, JobState::Failed, None, Some("done"))?;

    let plan = history.plan_gc(false)?;
    assert_eq!(plan.expired_temp_directories, ["worktree-stale"]);
    history.sweep_gc(plan)?;
    assert!(queue.get(&old_terminal)?.is_none());
    assert!(queue.get(&young_terminal)?.is_some());
    assert!(queue.get(&active)?.is_some());
    assert!(!stale.exists());
    Ok(())
}

fn age_job(queue: &HistoryQueue, id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = queue.root().join("jobs").join(format!("{id}.json"));
    let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    value["created_at_millis"] = 0_u64.into();
    value["updated_at_millis"] = 0_u64.into();
    std::fs::write(path, serde_json::to_vec(&value)?)?;
    Ok(())
}

fn age_path(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let old = SystemTime::now()
        .checked_sub(Duration::from_secs(2 * 24 * 60 * 60))
        .ok_or("old timestamp")?;
    std::fs::File::open(path)?.set_times(std::fs::FileTimes::new().set_modified(old))?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn cleanup_rejects_symlink_candidates() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new()?;
    let history = HistoryStore::create(&fixture.repository)?;
    let tmp = history.root().join("tmp");
    std::fs::create_dir_all(&tmp)?;
    symlink(&fixture.path, tmp.join("worktree-escape"))?;
    assert!(history.plan_gc(false).is_err());
    Ok(())
}

#[test]
fn gc_fails_closed_on_an_unknown_named_root() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let history = HistoryStore::create(&fixture.repository)?;
    let adapter = Arc::new(SqliteStore::open_existing(history.database_path())?);
    let prolly = Prolly::new(adapter, Config::default());
    let tree = prolly.put(&prolly.create(), b"key".to_vec(), b"value".to_vec())?;
    prolly.publish_named_root(b"unknown/root", &tree)?;
    let error = match history.plan_gc(false) {
        Ok(_) => return Err("unknown root unexpectedly accepted".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("named root"));
    Ok(())
}

#[test]
fn gc_rejects_incomplete_realization_root_sets() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let history = HistoryStore::create(&fixture.repository)?;
    history.publish(request('a', "First")?)?;
    let adapter = Arc::new(SqliteStore::open_existing(history.database_path())?);
    let prolly = Prolly::new(adapter.clone(), Config::default());
    let root = adapter
        .list_roots()?
        .into_iter()
        .find(|root| {
            prolly::decode_segments(&root.name)
                .is_ok_and(|segments| segments.len() == 5 && segments[2] == b"version")
        })
        .ok_or("version root")?;
    prolly.delete_named_root(&root.name)?;
    let error = match history.plan_gc(false) {
        Ok(_) => return Err("incomplete roots unexpectedly accepted".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("exactly six"));
    Ok(())
}

#[test]
fn interrupted_root_transaction_rolls_back_every_delete() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = Fixture::new()?;
    let history = HistoryStore::create(&fixture.repository)?;
    history.publish(request('a', "First")?)?;
    history.publish(request('b', "Second")?)?;
    let adapter = Arc::new(SqliteStore::open_existing(history.database_path())?);
    let prolly = Prolly::new(adapter.clone(), Config::default());
    let names = adapter
        .list_roots()?
        .into_iter()
        .filter(|root| {
            prolly::decode_segments(&root.name)
                .is_ok_and(|segments| segments.len() == 5 && segments[2] == b"version")
        })
        .map(|root| root.name)
        .collect::<Vec<_>>();
    let transaction = prolly.begin_transaction()?;
    for name in names.iter().take(3) {
        transaction.load_named_root(name)?.ok_or("root")?;
        transaction.delete_named_root(name)?;
    }
    transaction.rollback();
    assert_eq!(history.list(None)?.len(), 2);
    Ok(())
}

#[test]
fn maintenance_lock_child() -> Result<(), Box<dyn std::error::Error>> {
    let Some(repository_path) = std::env::var_os("COMPASS_TEST_LOCK_REPOSITORY") else {
        return Ok(());
    };
    let marker = PathBuf::from(std::env::var_os("COMPASS_TEST_LOCK_MARKER").ok_or("marker")?);
    let repository = Repository::discover(Path::new(&repository_path))?;
    let history = HistoryStore::create(&repository)?;
    let activity = history.activity()?;
    let unpublished =
        history.prepare_publish_with_activity(request('e', "Unpublished")?, &activity)?;
    std::fs::write(marker, b"ready")?;
    std::thread::sleep(Duration::from_secs(12));
    drop(unpublished);
    drop(activity);
    Ok(())
}

#[test]
fn active_builder_blocks_maintenance_in_a_separate_process()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let history = HistoryStore::create(&fixture.repository)?;
    let marker = fixture.path.join("lock-ready");
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--exact", "maintenance_lock_child", "--nocapture"])
        .env("COMPASS_TEST_LOCK_REPOSITORY", &fixture.path)
        .env("COMPASS_TEST_LOCK_MARKER", &marker)
        .spawn()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !marker.exists() {
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            return Err("child did not acquire activity lock".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let error = match history.plan_gc(false) {
        Ok(_) => return Err("maintenance unexpectedly acquired".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("timed out acquiring exclusive"));
    assert!(child.wait()?.success());
    let plan = history.plan_gc(false)?;
    assert!(plan.reclaimable_nodes > 0);
    Ok(())
}
