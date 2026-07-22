use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{CommitId, HistoryError};

/// Canonical paths that identify a Git repository and its shared common directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Repository {
    root: PathBuf,
    common_dir: PathBuf,
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
        let value = git_line(
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
}

fn git_path(current_dir: &Path, arguments: &[&str]) -> Result<PathBuf, HistoryError> {
    git_line(current_dir, arguments).map(PathBuf::from)
}

fn git_line(current_dir: &Path, arguments: &[&str]) -> Result<String, HistoryError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(current_dir)
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
    if output.stdout.contains(&0) {
        return Err(HistoryError::Git(
            "Git returned a NUL byte in a path".to_owned(),
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|error| HistoryError::Git(format!("Git returned a non-UTF-8 path: {error}")))?;
    let mut lines = text.lines();
    let value = lines
        .next()
        .filter(|line| !line.is_empty())
        .ok_or_else(|| HistoryError::Git("Git returned an empty path".to_owned()))?;
    if lines.next().is_some() {
        return Err(HistoryError::Git("Git returned multiple paths".to_owned()));
    }
    Ok(value.to_owned())
}
