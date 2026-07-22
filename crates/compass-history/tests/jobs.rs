use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Barrier};

use compass_history::{
    BuildProfile, HistoryConfig, HistoryQueue, JobRequest, JobState, Repository,
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
