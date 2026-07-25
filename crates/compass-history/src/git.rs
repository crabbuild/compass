use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{CommitId, HistoryError, TimelineCommit};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SourceFileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SourceHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SourceFileDelta {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub status: SourceFileStatus,
    pub hunks: Vec<SourceHunk>,
}

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
    /// A configured checkout filter could execute arbitrary external code and is rejected.
    UnsupportedFilter(String),
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

    /// Return exact file statuses and zero-context source hunks without fetching or checking out.
    pub fn source_delta(
        &self,
        old: &CommitId,
        new: &CommitId,
    ) -> Result<Vec<SourceFileDelta>, HistoryError> {
        let status = git_output(
            &self.root,
            &[
                "diff",
                "--name-status",
                "-z",
                "--find-renames=50%",
                old.as_str(),
                new.as_str(),
                "--",
            ],
        )?;
        let mut deltas = parse_name_status(&status)?;
        let patch = git_output(
            &self.root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--find-renames=50%",
                "--unified=0",
                "--no-color",
                old.as_str(),
                new.as_str(),
                "--",
            ],
        )?;
        attach_hunks(&patch, &mut deltas)?;
        deltas.sort();
        Ok(deltas)
    }

    /// Return every commit reachable from `tip` in parent-before-child order.
    pub fn reachable_commits(
        &self,
        tip: &CommitId,
        first_parent: bool,
    ) -> Result<Vec<CommitId>, HistoryError> {
        let mut arguments = vec!["rev-list", "--reverse", "--topo-order"];
        if first_parent {
            arguments.push("--first-parent");
        }
        arguments.push("--end-of-options");
        arguments.push(tip.as_str());
        let output = git_output(&self.root, &arguments)?;
        std::str::from_utf8(&output)
            .map_err(|error| HistoryError::Git(format!("Git returned non-UTF-8 history: {error}")))?
            .lines()
            .map(|value| {
                value.parse().map_err(|_| {
                    HistoryError::Git(format!("Git returned invalid reachable commit ID {value}"))
                })
            })
            .collect()
    }

    /// Return every commit reachable from any local reference in parent-before-child order.
    pub fn all_reachable_commits(&self) -> Result<Vec<CommitId>, HistoryError> {
        let output = git_output(
            &self.root,
            &["rev-list", "--reverse", "--topo-order", "--all"],
        )?;
        std::str::from_utf8(&output)
            .map_err(|error| HistoryError::Git(format!("Git returned non-UTF-8 history: {error}")))?
            .lines()
            .map(|value| {
                value.parse().map_err(|_| {
                    HistoryError::Git(format!("Git returned invalid reachable commit ID {value}"))
                })
            })
            .collect()
    }

    /// Return presentation metadata for one exact commit without touching the worktree.
    pub fn timeline_commit(&self, commit: &CommitId) -> Result<TimelineCommit, HistoryError> {
        let output = git_output(
            &self.root,
            &[
                "show",
                "-s",
                "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s",
                "--end-of-options",
                commit.as_str(),
            ],
        )?;
        let text = std::str::from_utf8(&output).map_err(|error| {
            HistoryError::Git(format!("Git returned non-UTF-8 history: {error}"))
        })?;
        let fields = text.trim_end().split('\0').collect::<Vec<_>>();
        if fields.len() != 6 {
            return Err(HistoryError::Git(
                "Git returned malformed timeline metadata".to_owned(),
            ));
        }
        let commit = fields[0]
            .parse()
            .map_err(|_| HistoryError::Git("Git returned an invalid commit ID".to_owned()))?;
        let parents = if fields[1].is_empty() {
            Vec::new()
        } else {
            fields[1]
                .split_ascii_whitespace()
                .map(|value| {
                    value.parse().map_err(|_| {
                        HistoryError::Git(format!("Git returned invalid parent ID {value}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let authored_at_seconds = fields[4].parse().map_err(|_| {
            HistoryError::Git("Git returned an invalid author timestamp".to_owned())
        })?;
        Ok(TimelineCommit {
            commit,
            parents,
            author_name: fields[2].to_owned(),
            author_email: fields[3].to_owned(),
            authored_at_seconds,
            subject: fields[5].to_owned(),
        })
    }

    /// Inspect committed-tree and repository filter limitations without creating a worktree.
    pub fn target_limitations(
        &self,
        commit: &CommitId,
    ) -> Result<Vec<GitTargetLimitation>, HistoryError> {
        let mut limitations = Vec::new();
        match reject_unsupported_filters(&self.root) {
            Ok(()) => {}
            Err(HistoryError::UnsupportedGitFilter(filter)) => {
                limitations.push(GitTargetLimitation::UnsupportedFilter(filter));
            }
            Err(error) => return Err(error),
        }
        let listing = git_output(
            &self.root,
            &["ls-tree", "-r", "-z", "-l", "--full-tree", commit.as_str()],
        )?;
        for entry in listing
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
                continue;
            };
            let metadata = String::from_utf8_lossy(&entry[..tab]);
            let fields = metadata.split_ascii_whitespace().collect::<Vec<_>>();
            let path = String::from_utf8_lossy(&entry[tab + 1..]).into_owned();
            if fields.first() == Some(&"160000") {
                limitations.push(GitTargetLimitation::Gitlink(path));
                continue;
            }
            if fields.get(1) != Some(&"blob")
                || fields
                    .get(3)
                    .and_then(|size| size.parse::<u64>().ok())
                    .is_none_or(|size| size > 512)
            {
                continue;
            }
            let Some(object) = fields.get(2) else {
                continue;
            };
            let bytes = git_output(&self.root, &["cat-file", "blob", object])?;
            if bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n") {
                limitations.push(GitTargetLimitation::LfsPointer(path));
            }
        }
        limitations.sort();
        limitations.dedup();
        Ok(limitations)
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

fn parse_name_status(bytes: &[u8]) -> Result<Vec<SourceFileDelta>, HistoryError> {
    let fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status = std::str::from_utf8(fields[index])
            .map_err(|error| HistoryError::Git(format!("non-UTF-8 diff status: {error}")))?;
        index += 1;
        let code = status
            .as_bytes()
            .first()
            .copied()
            .ok_or_else(|| HistoryError::Git("empty diff status".to_owned()))?;
        let path = |field: &[u8]| -> Result<String, HistoryError> {
            let value = std::str::from_utf8(field)
                .map_err(|error| HistoryError::Git(format!("non-UTF-8 source path: {error}")))?;
            validate_source_path(value)?;
            Ok(value.replace('\\', "/"))
        };
        let delta = match code {
            b'A' | b'M' | b'D' => {
                let value = fields
                    .get(index)
                    .ok_or_else(|| HistoryError::Git("diff status has no path".to_owned()))?;
                index += 1;
                let value = path(value)?;
                SourceFileDelta {
                    old_path: (code != b'A').then(|| value.clone()),
                    new_path: (code != b'D').then_some(value),
                    status: match code {
                        b'A' => SourceFileStatus::Added,
                        b'D' => SourceFileStatus::Deleted,
                        _ => SourceFileStatus::Modified,
                    },
                    hunks: Vec::new(),
                }
            }
            b'R' => {
                let old = fields
                    .get(index)
                    .ok_or_else(|| HistoryError::Git("rename has no old path".to_owned()))?;
                let new = fields
                    .get(index + 1)
                    .ok_or_else(|| HistoryError::Git("rename has no new path".to_owned()))?;
                index += 2;
                SourceFileDelta {
                    old_path: Some(path(old)?),
                    new_path: Some(path(new)?),
                    status: SourceFileStatus::Renamed,
                    hunks: Vec::new(),
                }
            }
            b'C' => {
                return Err(HistoryError::Git(
                    "copy detection is not part of semantic source delta".to_owned(),
                ));
            }
            _ => {
                return Err(HistoryError::Git(format!(
                    "unsupported source diff status {status}"
                )));
            }
        };
        output.push(delta);
    }
    Ok(output)
}

fn validate_source_path(value: &str) -> Result<(), HistoryError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(HistoryError::Git(format!(
            "unsafe source delta path {value:?}"
        )));
    }
    Ok(())
}

fn attach_hunks(bytes: &[u8], deltas: &mut [SourceFileDelta]) -> Result<(), HistoryError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| HistoryError::Git(format!("non-UTF-8 source patch: {error}")))?;
    let mut file_index = None;
    let mut files_seen = 0_usize;
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            if files_seen >= deltas.len() {
                return Err(HistoryError::Git(
                    "source patch has more files than status output".to_owned(),
                ));
            }
            file_index = Some(files_seen);
            files_seen += 1;
        } else if line.starts_with("@@ ") {
            let index = file_index
                .ok_or_else(|| HistoryError::Git("source hunk has no file".to_owned()))?;
            deltas[index].hunks.push(parse_hunk_header(line)?);
        }
    }
    if files_seen != deltas.len() {
        return Err(HistoryError::Git(format!(
            "source patch described {files_seen} files but status described {}",
            deltas.len()
        )));
    }
    for delta in deltas {
        delta.hunks.sort();
        delta.hunks.dedup();
    }
    Ok(())
}

fn parse_hunk_header(line: &str) -> Result<SourceHunk, HistoryError> {
    let body = line
        .strip_prefix("@@ -")
        .and_then(|line| line.split_once(" @@").map(|(body, _)| body))
        .ok_or_else(|| HistoryError::Git(format!("malformed source hunk {line:?}")))?;
    let (old, new) = body
        .split_once(" +")
        .ok_or_else(|| HistoryError::Git(format!("malformed source hunk {line:?}")))?;
    let parse_range = |value: &str| -> Result<(u32, u32), HistoryError> {
        let (start, lines) = value.split_once(',').unwrap_or((value, "1"));
        let start = start
            .parse()
            .map_err(|_| HistoryError::Git(format!("invalid hunk start {start:?}")))?;
        let lines = lines
            .parse()
            .map_err(|_| HistoryError::Git(format!("invalid hunk length {lines:?}")))?;
        Ok((start, lines))
    };
    let (old_start, old_lines) = parse_range(old)?;
    let (new_start, new_lines) = parse_range(new)?;
    Ok(SourceHunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
    })
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
