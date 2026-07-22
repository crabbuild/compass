use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{CommitId, HistoryError};

/// Canonical paths that identify a Git repository and its shared common directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Repository {
    root: PathBuf,
    common_dir: PathBuf,
}

/// A committed-tree feature that a historical build must report explicitly.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum GitTargetLimitation {
    /// A tracked file is a Git LFS pointer; offline checkout intentionally did not smudge it.
    LfsPointer(String),
    /// A tree entry is a gitlink/submodule commit rather than ordinary file content.
    Gitlink(String),
}

/// An exact detached checkout below the repository's protected Compass temporary directory.
pub struct WorktreeGuard {
    repository_root: PathBuf,
    tmp_root: PathBuf,
    base: PathBuf,
    base_name: std::ffi::OsString,
    path: PathBuf,
    output_root: PathBuf,
    limitations: Vec<GitTargetLimitation>,
    registered: bool,
    closed: bool,
}

impl Repository {
    /// Discover a repository without assuming that `.git` is a directory.
    pub fn discover(current_dir: &Path) -> Result<Self, HistoryError> {
        let root = git_path(current_dir, &["rev-parse", "--show-toplevel"])?;
        let common = git_path(current_dir, &["rev-parse", "--git-common-dir"])?;
        let common_dir = if common.is_absolute() {
            common
        } else {
            root.join(common)
        };
        let root = root
            .canonicalize()
            .map_err(|source| crate::error::io_error(&root, source))?;
        let common_dir = common_dir
            .canonicalize()
            .map_err(|source| crate::error::io_error(&common_dir, source))?;
        Ok(Self { root, common_dir })
    }

    /// Return the canonical repository worktree root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the canonical Git common directory shared by linked worktrees.
    #[must_use]
    pub fn common_dir(&self) -> &Path {
        &self.common_dir
    }

    /// Resolve one revision to a full commit object ID without option ambiguity.
    pub fn resolve(&self, revision: &str) -> Result<CommitId, HistoryError> {
        let expression = format!("{revision}^{{commit}}");
        let value = git_line(
            &self.root,
            &["rev-parse", "--verify", "--end-of-options", &expression],
        )?;
        value
            .parse()
            .map_err(|_| HistoryError::Git(format!("revision {revision:?} is not a commit")))
    }

    /// Return the exact ordered parents recorded by a commit.
    pub fn parents(&self, commit: &CommitId) -> Result<Vec<CommitId>, HistoryError> {
        let value = git_line_allow_empty(
            &self.root,
            &[
                "show",
                "-s",
                "--format=%P",
                "--end-of-options",
                commit.as_str(),
            ],
        )?;
        if value.is_empty() {
            return Ok(Vec::new());
        }
        value
            .split_ascii_whitespace()
            .map(|parent| {
                parent.parse().map_err(|_| {
                    HistoryError::Git(format!("Git returned invalid parent ID {parent}"))
                })
            })
            .collect()
    }

    /// Resolve a revision from a specific checkout.
    pub fn resolve_at(&self, checkout: &Path, revision: &str) -> Result<CommitId, HistoryError> {
        let expression = format!("{revision}^{{commit}}");
        git_line(
            checkout,
            &["rev-parse", "--verify", "--end-of-options", &expression],
        )?
        .parse()
        .map_err(|_| HistoryError::Git(format!("revision {revision:?} is not a commit")))
    }

    /// Return first-parent ancestors nearest-first, excluding the target commit itself.
    pub fn first_parent_ancestors(&self, commit: &CommitId) -> Result<Vec<CommitId>, HistoryError> {
        let output = git_output(&self.root, &["rev-list", "--first-parent", commit.as_str()])?;
        std::str::from_utf8(&output)
            .map_err(|error| HistoryError::Git(format!("Git returned non-UTF-8 history: {error}")))?
            .lines()
            .skip(1)
            .map(|value| {
                value.parse().map_err(|_| {
                    HistoryError::Git(format!("Git returned invalid ancestor ID {value}"))
                })
            })
            .collect()
    }

    /// Create an exact detached worktree without running hooks, prompting, fetching, or smudging
    /// LFS content.
    pub fn detached_worktree(&self, commit: &CommitId) -> Result<WorktreeGuard, HistoryError> {
        reject_unsupported_filters(&self.root)?;
        let compass_root = self.common_dir.join("compass");
        crate::store::create_owner_dir(&compass_root)?;
        let tmp_root = compass_root.join("tmp");
        crate::store::create_owner_dir(&tmp_root)?;
        let tmp_root = tmp_root
            .canonicalize()
            .map_err(|source| crate::error::io_error(&tmp_root, source))?;
        let temporary = tempfile::Builder::new()
            .prefix("worktree-")
            .tempdir_in(&tmp_root)
            .map_err(|source| crate::error::io_error(&tmp_root, source))?;
        let base = temporary.keep();
        let base_name = base
            .file_name()
            .ok_or_else(|| HistoryError::UnsafePath {
                path: base.clone(),
                reason: "temporary worktree has no basename".to_owned(),
            })?
            .to_os_string();
        let path = base.join("checkout");
        let hooks = base.join("empty-hooks");
        crate::store::create_owner_dir(&hooks)?;
        let output_root = base.join("output");
        crate::store::create_owner_dir(&output_root)?;
        let mut guard = WorktreeGuard {
            repository_root: self.root.clone(),
            tmp_root,
            base,
            base_name,
            path,
            output_root,
            limitations: Vec::new(),
            registered: false,
            closed: false,
        };
        if let Err(error) = add_worktree(&guard.repository_root, &hooks, &guard.path, commit) {
            let _cleanup = guard.cleanup();
            return Err(error);
        }
        guard.registered = true;
        let actual = self.resolve_at(&guard.path, "HEAD")?;
        if &actual != commit {
            let _cleanup = guard.cleanup();
            return Err(HistoryError::Git(format!(
                "detached worktree resolved to {actual}, expected {commit}"
            )));
        }
        guard.limitations = target_limitations(&guard.path)?;
        Ok(guard)
    }
}

impl WorktreeGuard {
    /// Return the exact checkout root.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return offline target limitations detected in the committed tree.
    #[must_use]
    pub fn limitations(&self) -> &[GitTargetLimitation] {
        &self.limitations
    }

    /// Return an attempt-local output root outside the checked-out source tree.
    #[must_use]
    pub fn output_root(&self) -> &Path {
        &self.output_root
    }

    /// Explicitly remove the Git worktree and its protected temporary directory.
    pub fn close(mut self) -> Result<(), HistoryError> {
        self.cleanup()
    }

    fn cleanup(&mut self) -> Result<(), HistoryError> {
        if self.closed {
            return Ok(());
        }
        self.validate_cleanup_target(self.path.exists())?;
        if self.registered {
            let output = Command::new("git")
                .args(["-C"])
                .arg(&self.repository_root)
                .args(["worktree", "remove", "--force", "--"])
                .arg(&self.path)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GCM_INTERACTIVE", "never")
                .output()
                .map_err(|error| HistoryError::WorktreeCleanup(error.to_string()))?;
            if !output.status.success() {
                return Err(HistoryError::WorktreeCleanup(
                    String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                ));
            }
            self.registered = false;
        }
        self.validate_cleanup_target(false)?;
        if self.base.exists() {
            fs::remove_dir_all(&self.base)
                .map_err(|source| crate::error::io_error(&self.base, source))?;
        }
        self.closed = true;
        Ok(())
    }

    fn validate_cleanup_target(&self, checkout_must_exist: bool) -> Result<(), HistoryError> {
        crate::store::reject_directory(&self.tmp_root)?;
        let canonical_tmp = self
            .tmp_root
            .canonicalize()
            .map_err(|source| crate::error::io_error(&self.tmp_root, source))?;
        if canonical_tmp != self.tmp_root
            || self.base.parent() != Some(self.tmp_root.as_path())
            || self.base.file_name() != Some(self.base_name.as_os_str())
            || !self.base_name.to_string_lossy().starts_with("worktree-")
        {
            return Err(HistoryError::UnsafePath {
                path: self.base.clone(),
                reason: "temporary worktree escaped its protected root".to_owned(),
            });
        }
        crate::store::reject_symlink(&self.base, false)?;
        crate::store::reject_directory(&self.base)?;
        let canonical_base = self
            .base
            .canonicalize()
            .map_err(|source| crate::error::io_error(&self.base, source))?;
        if canonical_base.parent() != Some(self.tmp_root.as_path())
            || self.path != self.base.join("checkout")
        {
            return Err(HistoryError::UnsafePath {
                path: self.base.clone(),
                reason: "temporary worktree identity changed".to_owned(),
            });
        }
        crate::store::reject_symlink(&self.path, !checkout_must_exist)?;
        if checkout_must_exist {
            crate::store::reject_directory(&self.path)?;
        }
        Ok(())
    }
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        let _cleanup = self.cleanup();
    }
}

fn add_worktree(
    repository_root: &Path,
    hooks: &Path,
    path: &Path,
    commit: &CommitId,
) -> Result<(), HistoryError> {
    let output = Command::new("git")
        .arg("-c")
        .arg(format!("core.hooksPath={}", hooks.display()))
        .args(["-c", "credential.helper=", "-C"])
        .arg(repository_root)
        .args(["worktree", "add", "--quiet", "--detach"])
        .arg(path)
        .arg(commit.as_str())
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_ASKPASS", "false")
        .env("SSH_ASKPASS", "false")
        .output()
        .map_err(|error| HistoryError::Git(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(HistoryError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

fn reject_unsupported_filters(repository_root: &Path) -> Result<(), HistoryError> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(repository_root)
        .args(["config", "--get-regexp", r"^filter\..*\.(smudge|process)$"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| HistoryError::Git(error.to_string()))?;
    if !output.status.success() {
        if output.status.code() == Some(1) && output.stderr.is_empty() {
            return Ok(());
        }
        return Err(HistoryError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|error| HistoryError::Git(format!("Git returned non-UTF-8 filters: {error}")))?;
    for line in text.lines() {
        let (name, command) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let command = command.trim_start();
        if !matches!(command, "git-lfs" | "git lfs")
            && !command.starts_with("git-lfs ")
            && !command.starts_with("git lfs ")
        {
            return Err(HistoryError::UnsupportedGitFilter(format!(
                "{name}={}",
                command.trim()
            )));
        }
    }
    Ok(())
}

fn target_limitations(checkout: &Path) -> Result<Vec<GitTargetLimitation>, HistoryError> {
    let mut limitations = Vec::new();
    let index = git_output(checkout, &["ls-files", "--stage", "-z"])?;
    for entry in index
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            continue;
        };
        if entry[..tab].starts_with(b"160000 ") {
            limitations.push(GitTargetLimitation::Gitlink(
                String::from_utf8_lossy(&entry[tab + 1..]).into_owned(),
            ));
        }
    }
    find_lfs_pointers(checkout, checkout, &mut limitations)?;
    limitations.sort();
    limitations.dedup();
    Ok(limitations)
}

fn find_lfs_pointers(
    root: &Path,
    directory: &Path,
    limitations: &mut Vec<GitTargetLimitation>,
) -> Result<(), HistoryError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| crate::error::io_error(directory, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| crate::error::io_error(directory, source))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path == root.join(".git") {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|source| crate::error::io_error(&path, source))?;
        if file_type.is_dir() {
            find_lfs_pointers(root, &path, limitations)?;
        } else if file_type.is_file() {
            let mut bytes = Vec::with_capacity(128);
            fs::File::open(&path)
                .map_err(|source| crate::error::io_error(&path, source))?
                .take(256)
                .read_to_end(&mut bytes)
                .map_err(|source| crate::error::io_error(&path, source))?;
            if bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n") {
                limitations.push(GitTargetLimitation::LfsPointer(
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn git_path(current_dir: &Path, arguments: &[&str]) -> Result<PathBuf, HistoryError> {
    git_line(current_dir, arguments).map(PathBuf::from)
}

fn git_output(current_dir: &Path, arguments: &[&str]) -> Result<Vec<u8>, HistoryError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(current_dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .output()
        .map_err(|error| HistoryError::Git(error.to_string()))?;
    if !output.status.success() {
        let diagnostic = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(HistoryError::Git(if diagnostic.is_empty() {
            format!("git {} exited with {}", arguments.join(" "), output.status)
        } else {
            diagnostic
        }));
    }
    if !output.stderr.is_empty() {
        return Err(HistoryError::Git(format!(
            "Git wrote an unexpected diagnostic: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn git_line(current_dir: &Path, arguments: &[&str]) -> Result<String, HistoryError> {
    let value = git_line_allow_empty(current_dir, arguments)?;
    if value.is_empty() {
        Err(HistoryError::Git("Git returned an empty value".to_owned()))
    } else {
        Ok(value)
    }
}

fn git_line_allow_empty(current_dir: &Path, arguments: &[&str]) -> Result<String, HistoryError> {
    let output = git_output(current_dir, arguments)?;
    if output.contains(&0) {
        return Err(HistoryError::Git(
            "Git returned a NUL byte in a path".to_owned(),
        ));
    }
    let text = std::str::from_utf8(&output)
        .map_err(|error| HistoryError::Git(format!("Git returned a non-UTF-8 path: {error}")))?;
    let value = text.strip_suffix('\n').unwrap_or(text);
    let value = value.strip_suffix('\r').unwrap_or(value);
    if value.contains(['\r', '\n']) {
        return Err(HistoryError::Git("Git returned multiple lines".to_owned()));
    }
    Ok(value.to_owned())
}
