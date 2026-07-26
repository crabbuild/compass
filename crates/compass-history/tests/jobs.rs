use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Barrier};

use compass_history::{
    BuildProfile, CommitId, HistoryConfig, HistoryQueue, JobRequest, JobState, Repository,
};

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

fn repository() -> Result<(tempfile::TempDir, Repository), Box<dyn std::error::Error>> {
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
    Ok((directory, repository))
}

fn profile() -> Result<BuildProfile, Box<dyn std::error::Error>> {
    let mut profile = BuildProfile::default();
    profile.insert("pipeline", "test-v1")?;
    Ok(profile)
}

#[test]
fn jobs_follow_the_allowed_state_machine_and_survive_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let queue = HistoryQueue::open(directory.path())?;
    let commit = "1111111111111111111111111111111111111111".parse()?;
    let id = queue.enqueue(JobRequest {
        commit,
        profile: profile()?,
    })?;
    let claimed = queue.claim_next()?.ok_or("claim")?;
    assert_eq!(claimed.id, id);
    assert_eq!(claimed.state, JobState::Building);
    assert!(
        queue
            .transition(&claimed, JobState::Published, None)
            .is_err()
    );
    queue.transition(&claimed, JobState::Validating, None)?;
    queue.finish(&claimed, JobState::Published, Some(true), None)?;
    drop(queue);
    assert_eq!(
        HistoryQueue::open(directory.path())?
            .get(&id)?
            .ok_or("job")?
            .state,
        JobState::Published
    );
    Ok(())
}

#[test]
fn latest_jobs_are_loaded_directly_for_requested_commits() -> Result<(), Box<dyn std::error::Error>>
{
    let (_directory, repository) = repository()?;
    let queue = HistoryQueue::for_repository(&repository)?;
    let queue_root = queue.root().to_path_buf();
    let first: CommitId = "1111111111111111111111111111111111111111".parse()?;
    let second: CommitId = "2222222222222222222222222222222222222222".parse()?;
    let ignored: CommitId = "3333333333333333333333333333333333333333".parse()?;
    queue.enqueue(JobRequest {
        commit: first.clone(),
        profile: profile()?,
    })?;
    let second_id = queue.enqueue(JobRequest {
        commit: second.clone(),
        profile: profile()?,
    })?;
    queue.enqueue(JobRequest {
        commit: ignored,
        profile: profile()?,
    })?;
    drop(queue);

    // Queues created before the latest-job index was introduced have only job records.
    std::fs::remove_dir_all(queue_root.join("latest"))?;
    std::fs::remove_file(queue_root.join("latest-index.json"))?;
    let queue = HistoryQueue::open_existing(&repository)?.ok_or("queue")?;

    let latest = queue.latest_for_commits(&[second, first])?;

    assert_eq!(latest.len(), 2);
    assert_eq!(latest[0].id, second_id);
    assert!(!queue_root.join("latest").exists());
    assert!(!queue_root.join("latest-index.json").exists());
    drop(queue);

    // The next writer performs the one-time migration.
    let queue = HistoryQueue::for_repository(&repository)?;
    assert!(queue_root.join("latest").is_dir());
    assert!(queue_root.join("latest-index.json").is_file());
    assert_eq!(
        queue.latest_for_commits(&[latest[0].commit.clone()])?.len(),
        1
    );
    Ok(())
}

#[test]
fn derived_index_failure_does_not_fail_a_durable_job_transition()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let queue = HistoryQueue::open(directory.path())?;
    let id = queue.enqueue(JobRequest {
        commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?,
        profile: profile()?,
    })?;
    let claimed = queue.claim_next()?.ok_or("claim")?;
    queue.transition(&claimed, JobState::Validating, None)?;

    let latest = directory.path().join("latest");
    let saved_latest = directory.path().join("latest.saved");
    std::fs::rename(&latest, &saved_latest)?;
    std::fs::write(&latest, b"index unavailable")?;
    let finished = queue.finish(&claimed, JobState::Published, Some(true), None);
    std::fs::remove_file(&latest)?;
    std::fs::rename(&saved_latest, &latest)?;

    assert_eq!(finished?.state, JobState::Published);
    assert_eq!(queue.get(&id)?.ok_or("job")?.state, JobState::Published);
    let lease = directory.path().join("leases").join(format!(
        "{}-{}.lease",
        claimed.commit, claimed.profile_digest
    ));
    assert!(!lease.exists());
    Ok(())
}

#[test]
fn enqueue_joins_concurrently_and_expired_generation_rejects_late_worker()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().to_path_buf();
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let root = root.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            let queue = HistoryQueue::open(&root)?;
            let commit = "2222222222222222222222222222222222222222".parse()?;
            let mut profile = BuildProfile::default();
            profile.insert("pipeline", "test-v1")?;
            barrier.wait();
            queue.enqueue(JobRequest { commit, profile })
        }));
    }
    barrier.wait();
    let mut ids = Vec::new();
    for thread in threads {
        ids.push(thread.join().map_err(|_| "thread panicked")??);
    }
    assert_eq!(ids[0], ids[1]);

    let queue = HistoryQueue::open(&root)?;
    let first = queue.claim_next()?.ok_or("first claim")?;
    queue.heartbeat(&first)?;
    let lease = root
        .join("leases")
        .join(format!("{}-{}.lease", first.commit, first.profile_digest));
    std::fs::write(
        &lease,
        format!(
            "{{\"owner\":\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\",\"generation\":{},\"expires_at_millis\":0}}",
            first.lease_generation
        ),
    )?;
    let reclaimed = queue.claim_next()?.ok_or("reclaimed")?;
    assert!(reclaimed.lease_generation > first.lease_generation);
    assert!(
        queue
            .transition(&first, JobState::Validating, None)
            .is_err()
    );
    queue.finish(
        &reclaimed,
        JobState::Failed,
        None,
        Some("failed without credentials"),
    )?;
    assert!(
        queue
            .transition(&reclaimed, JobState::Building, None)
            .is_err()
    );
    Ok(())
}

#[test]
fn configuration_is_non_mutating_idempotent_and_rolls_forward_atomically()
-> Result<(), Box<dyn std::error::Error>> {
    let (_directory, repository) = repository()?;
    let absent = HistoryConfig::load(&repository)?;
    assert!(!absent.enabled);
    assert!(!absent.configured());
    assert!(!repository.common_dir().join("compass").exists());

    let enabled = HistoryConfig::enable(&repository, profile()?)?;
    assert!(enabled.enabled);
    assert!(enabled.profile_digest.is_some());
    let disabled = HistoryConfig::disable(&repository)?;
    assert!(!disabled.enabled);
    assert!(disabled.configured());
    assert_eq!(
        HistoryConfig::disable(&repository)?.profile_digest,
        disabled.profile_digest
    );
    assert_eq!(
        HistoryConfig::load(&repository)?,
        HistoryConfig::disable(&repository)?
    );
    Ok(())
}

#[test]
fn claims_are_fifo_and_a_terminal_failure_does_not_block_the_next_job()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let queue = HistoryQueue::open(directory.path())?;
    let first_id = queue.enqueue(JobRequest {
        commit: "3333333333333333333333333333333333333333".parse()?,
        profile: profile()?,
    })?;
    let second_id = queue.enqueue(JobRequest {
        commit: "4444444444444444444444444444444444444444".parse()?,
        profile: profile()?,
    })?;

    let first = queue.claim_next()?.ok_or("first job")?;
    assert_eq!(first.id, first_id);
    queue.finish(&first, JobState::Failed, None, Some("expected failure"))?;
    let second = queue.claim_next()?.ok_or("second job")?;
    assert_eq!(second.id, second_id);
    queue.transition(&second, JobState::Validating, None)?;
    queue.finish(&second, JobState::Published, Some(true), None)?;
    assert!(queue.claim_next()?.is_none());
    Ok(())
}
