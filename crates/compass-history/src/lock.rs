use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::HistoryError;

const LOCK_WAIT: Duration = Duration::from_secs(10);
const LOCK_RETRY: Duration = Duration::from_millis(10);

/// Shared guard held by readers, builders, and publishers.
#[derive(Debug)]
pub struct ActivityGuard {
    file: File,
}

/// Exclusive guard held by garbage collection and format maintenance.
#[derive(Debug)]
pub struct MaintenanceGuard {
    file: File,
}

impl ActivityGuard {
    pub(crate) fn acquire(path: &Path, create: bool) -> Result<Self, HistoryError> {
        let file = open_lock(path, create)?;
        acquire_until(&file, path, "shared", File::try_lock_shared)?;
        Ok(Self { file })
    }
}

impl MaintenanceGuard {
    pub(crate) fn acquire(path: &Path, create: bool) -> Result<Self, HistoryError> {
        let file = open_lock(path, create)?;
        acquire_until(&file, path, "exclusive", File::try_lock)?;
        Ok(Self { file })
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn open_lock(path: &Path, create: bool) -> Result<File, HistoryError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| crate::error::io_error(path, source))
}

fn acquire_until(
    file: &File,
    path: &Path,
    kind: &'static str,
    attempt: fn(&File) -> Result<(), std::fs::TryLockError>,
) -> Result<(), HistoryError> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match attempt(file) {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(HistoryError::LockTimeout {
                        kind,
                        path: PathBuf::from(path),
                    });
                }
                thread::sleep(LOCK_RETRY);
            }
            Err(std::fs::TryLockError::Error(source)) => {
                return Err(crate::error::io_error(path, source));
            }
        }
    }
}
