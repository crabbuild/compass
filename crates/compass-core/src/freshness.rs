//! Advisory freshness over the selected graph's recorded source root.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use compass_files::read_bytes_bounded;
use compass_prs::{ProcessRunner, SystemRunner};
use serde::Serialize;

const FILTER_CONFIG_ARGUMENTS: &[&str] = &[
    "config",
    "--name-only",
    "--get-regexp",
    r"^filter\..*\.(clean|process)$",
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphFreshness {
    pub schema: &'static str,
    pub source_commit: Option<String>,
    pub head_commit: Option<String>,
    pub changed_files: Option<usize>,
    pub uncommitted_files: Option<usize>,
    pub status: FreshnessStatus,
    pub reason: Option<String>,
    pub update_command: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessStatus {
    Current,
    RevisionChanged,
    WorkingTreeChanges,
    Unknown,
}

impl GraphFreshness {
    #[must_use]
    pub fn warning(&self) -> Option<String> {
        if self.status == FreshnessStatus::Current {
            return None;
        }
        let built = self.source_commit.as_deref().unwrap_or("unknown revision");
        let built = built.chars().take(12).collect::<String>();
        Some(match self.status {
            FreshnessStatus::Unknown => format!(
                "Graph freshness unknown: {}. Run {}.",
                self.reason
                    .as_deref()
                    .unwrap_or("source revision cannot be checked"),
                self.update_command
            ),
            _ => format!(
                "Graph built at {built}; {} files differ; {} uncommitted files. Run {}.",
                self.changed_files.unwrap_or_default(),
                self.uncommitted_files.unwrap_or_default(),
                self.update_command
            ),
        })
    }
}

fn commit_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// No source root is guessed from the caller's current directory. Graphs
/// without a recorded root (including historical realizations) return None.
/// Git failures and limits are unknown freshness, never a claim of currency.
#[must_use]
pub fn graph_freshness(graph_path: &Path, source_commit: Option<&str>) -> Option<GraphFreshness> {
    graph_freshness_with_runner(graph_path, source_commit, &SystemRunner)
}

fn graph_freshness_with_runner(
    graph_path: &Path,
    source_commit: Option<&str>,
    runner: &impl ProcessRunner,
) -> Option<GraphFreshness> {
    let marker = graph_path.parent()?.join("source-root.txt");
    if !marker.exists() {
        return None;
    }
    let mut result = GraphFreshness {
        schema: "compass.graph-freshness/1",
        source_commit: source_commit
            .filter(|value| commit_id(value))
            .map(str::to_owned),
        head_commit: None,
        changed_files: None,
        uncommitted_files: None,
        status: FreshnessStatus::Unknown,
        reason: None,
        update_command: "compass update".into(),
    };
    let root = (|| -> Result<PathBuf, String> {
        let bytes = read_bytes_bounded(&marker, 4096)
            .map_err(|_| "source-root marker is unavailable or exceeds 4096 bytes")?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "source-root marker is not UTF-8")?
            .trim();
        let root = PathBuf::from(text);
        if !root.is_absolute() || text.chars().any(char::is_control) {
            return Err("source-root marker is not a valid absolute path".into());
        }
        std::fs::canonicalize(root).map_err(|_| "recorded source root is unavailable".into())
    })();
    let root = match root {
        Ok(root) => root,
        Err(reason) => {
            result.reason = Some(reason);
            return Some(result);
        }
    };
    let root_text = match root.to_str() {
        Some(text) => text,
        None => {
            result.reason = Some("recorded source root is not UTF-8".into());
            return Some(result);
        }
    };
    // Shell quoting is display only; this string is never executed.
    // The root may be outside the caller's current project.
    let mut output = compass_files::BuildGuard::output_container_for_artifact(graph_path);
    // A selected older managed generation must never be rebuilt in place.
    if let Some(snapshots) = graph_path.parent().and_then(Path::parent)
        && snapshots
            .file_name()
            .is_some_and(|name| name == "snapshots")
        && let Some(container) = snapshots.parent()
        && compass_files::BuildGuard::resolve_current_snapshot_directory(container).is_ok()
    {
        output = container.to_owned();
    }
    if !output.is_absolute() {
        output = std::env::current_dir()
            .unwrap_or_else(|_| root.clone())
            .join(output);
    }
    output = std::fs::canonicalize(&output).unwrap_or(output);
    if output == root {
        output = root.join("compass-out");
    }
    let output_text = match output.to_str() {
        Some(text) if !text.chars().any(char::is_control) => text,
        _ => {
            result.reason =
                Some("artifact output root cannot be represented in a recovery command".into());
            return Some(result);
        }
    };
    result.update_command = format!(
        "compass update {} --out {}",
        shell_quote(root_text),
        shell_quote(output_text)
    );
    let git = |args: &[&str]| -> Result<String, String> {
        let mut arguments = vec![
            "--no-optional-locks".to_owned(),
            "--no-lazy-fetch".into(),
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "core.quotePath=true".into(),
            "-c".into(),
            "status.renames=false".into(),
            "-C".into(),
            root_text.to_owned(),
        ];
        arguments.extend(args.iter().map(|arg| (*arg).to_owned()));
        let output = runner
            .run("git", &arguments, Duration::from_millis(500))
            .map_err(|_| "bounded Git freshness check unavailable")?;
        // Exit 1 means this exact config lookup found no matching keys.
        if output.code == 1 && args == FILTER_CONFIG_ARGUMENTS && output.stdout.is_empty() {
            return Ok(String::new());
        }
        if output.code != 0 {
            return Err(
                "Git cannot verify the graph's recorded revision in this source root".into(),
            );
        }
        if output.stdout.len() > 1024 * 1024 {
            return Err("Git freshness output exceeded 1 MiB".into());
        }
        Ok(output.stdout)
    };
    let checked = (|| -> Result<(), String> {
        let built = result
            .source_commit
            .as_deref()
            .ok_or("graph has no valid build commit")?;
        let head = git(&["rev-parse", "--verify", "HEAD"])?;
        let head = head.trim();
        if !commit_id(head) {
            return Err("Git returned an invalid HEAD revision".into());
        }
        result.head_commit = Some(head.to_owned());
        // Diff/status may invoke clean/process filters or child Git commands
        // in submodules. Do not inspect working bytes across those boundaries.
        if !git(FILTER_CONFIG_ARGUMENTS)?.is_empty() {
            return Err("Git conversion filters prevent an offline working-tree check".into());
        }
        let modes = git(&["ls-files", "--format=%(objectmode)", "-z"])?;
        if !modes.is_empty() && !modes.ends_with('\0') {
            return Err("Git returned malformed index modes".into());
        }
        if modes.split_terminator('\0').any(|mode| mode == "160000") {
            return Err("submodule state is outside the bounded freshness check".into());
        }
        let diff = git(&[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--name-only",
            "-z",
            built,
            "--",
        ])?;
        let status = git(&["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
        if diff.contains('\u{fffd}') || status.contains('\u{fffd}') {
            return Err("Git paths cannot be verified losslessly as UTF-8".into());
        }
        if !diff.is_empty() && !diff.ends_with('\0') {
            return Err("Git returned malformed changed-path output".into());
        }
        if !status.is_empty() && !status.ends_with('\0') {
            return Err("Git returned malformed working-tree status".into());
        }
        let mut changed = diff.split_terminator('\0').collect::<BTreeSet<_>>();
        let mut dirty = 0;
        for row in status.split_terminator('\0') {
            if row.len() < 4
                || row.as_bytes()[2] != b' '
                || !row.as_bytes()[..2].iter().all(u8::is_ascii)
            {
                return Err("Git returned malformed working-tree status".into());
            }
            dirty += 1;
            // Dirty tracked files already appear in diff unless their working
            // bytes equal the built commit; staged-only states remain visible.
            changed.insert(&row[3..]);
        }
        result.changed_files = Some(changed.len());
        result.uncommitted_files = Some(dirty);
        result.status = if built != head {
            FreshnessStatus::RevisionChanged
        } else if dirty > 0 || !changed.is_empty() {
            FreshnessStatus::WorkingTreeChanges
        } else {
            FreshnessStatus::Current
        };
        Ok(())
    })();
    if let Err(reason) = checked {
        result.reason = Some(reason);
        result.status = FreshnessStatus::Unknown;
    }
    Some(result)
}

fn shell_quote(text: &str) -> String {
    #[cfg(windows)]
    {
        format!("'{}'", text.replace('\'', "''"))
    }
    #[cfg(not(windows))]
    {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_prs::{ProcessOutput, PrsError};
    #[derive(Default)]
    struct Fake {
        head: String,
        status: &'static str,
        fail: bool,
        filters: bool,
        submodules: bool,
    }
    impl ProcessRunner for Fake {
        fn run(
            &self,
            _: &str,
            args: &[String],
            timeout: Duration,
        ) -> Result<ProcessOutput, PrsError> {
            assert_eq!(timeout, Duration::from_millis(500));
            assert!(args.iter().any(|arg| arg == "--no-lazy-fetch"));
            assert!(args.iter().any(|arg| arg == "core.fsmonitor=false"));
            assert!(args.iter().any(|arg| arg == "status.renames=false"));
            if args.iter().any(|arg| arg == "diff") {
                for flag in ["--no-ext-diff", "--no-textconv", "--no-renames"] {
                    assert!(args.iter().any(|arg| arg == flag));
                }
            }
            let stdout = if args.iter().any(|arg| arg == "rev-parse") {
                self.head.clone()
            } else if args.iter().any(|arg| arg == "config") {
                if self.filters {
                    "filter.spy.clean\n".into()
                } else {
                    String::new()
                }
            } else if args.iter().any(|arg| arg == "ls-files") {
                if self.submodules {
                    "160000\0".into()
                } else {
                    "100644\0".into()
                }
            } else if args.iter().any(|arg| arg == "status") {
                self.status.into()
            } else if self.status.is_empty() {
                String::new()
            } else {
                "app/a.py\0".into()
            };
            Ok(ProcessOutput {
                code: if self.fail {
                    7
                } else if args.iter().any(|arg| arg == "config") && !self.filters {
                    1
                } else {
                    0
                },
                stdout,
                stderr: String::new(),
            })
        }
    }
    #[test]
    fn unknown_revision_and_malformed_status_never_report_current()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let graph = directory.path().join("graph.json");
        let marker = directory.path().join("source-root.txt");
        std::fs::write(&marker, "relative/root")?;
        let commit = "a".repeat(40);
        let mut fake = Fake {
            head: commit.clone(),
            status: "",
            fail: false,
            ..Fake::default()
        };
        let invalid =
            graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(invalid.status, FreshnessStatus::Unknown);
        assert!(
            invalid
                .reason
                .is_some_and(|reason| reason.contains("absolute path"))
        );
        std::fs::write(&marker, directory.path().to_string_lossy().as_bytes())?;
        fake.head = "invalid".into();
        let invalid =
            graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(invalid.status, FreshnessStatus::Unknown);
        fake.head = commit.clone();
        fake.status = "bad";
        let invalid =
            graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(invalid.status, FreshnessStatus::Unknown);
        assert!(invalid.changed_files.is_none());
        assert!(
            invalid
                .reason
                .is_some_and(|reason| reason.contains("malformed"))
        );
        fake.status = " M app/a.py\0 D old.py\0?? new.py\0";
        let renamed =
            graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(renamed.status, FreshnessStatus::WorkingTreeChanges);
        assert_eq!(renamed.changed_files, Some(3));
        fake.status = " M app/a.py\0?? file with space.py\0?? name\nwith newline.py\0";
        let unusual =
            graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(unusual.changed_files, Some(3));
        assert_eq!(unusual.uncommitted_files, Some(3));
        assert!(shell_quote("has $dollars and 'quotes'").contains("$dollars"));
        Ok(())
    }

    #[test]
    fn freshness_is_bound_to_recorded_root_and_git_failures_stay_unknown()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let graph = directory.path().join("graph.json");
        let commit = "a".repeat(40);
        let mut fake = Fake {
            head: "b".repeat(40),
            status: " M app/a.py\0?? new.py\0",
            fail: false,
            ..Fake::default()
        };
        assert!(graph_freshness_with_runner(&graph, Some(&commit), &fake).is_none());
        std::fs::write(
            directory.path().join("source-root.txt"),
            directory.path().to_string_lossy().as_bytes(),
        )?;
        let stale = graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(stale.status, FreshnessStatus::RevisionChanged);
        assert_eq!(stale.changed_files, Some(2));
        assert_eq!(stale.uncommitted_files, Some(2));
        assert!(
            stale
                .warning()
                .is_some_and(|text| text.contains("compass update"))
        );
        fake.head = commit.clone();
        fake.status = "";
        let current =
            graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(current.status, FreshnessStatus::Current);
        assert!(current.warning().is_none());
        fake.filters = true;
        let filtered =
            graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(filtered.status, FreshnessStatus::Unknown);
        assert!(
            filtered
                .reason
                .is_some_and(|reason| reason.contains("conversion filters"))
        );
        fake.filters = false;
        fake.submodules = true;
        let submodules =
            graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(submodules.status, FreshnessStatus::Unknown);
        assert!(
            submodules
                .reason
                .is_some_and(|reason| reason.contains("submodule"))
        );
        fake.submodules = false;
        fake.fail = true;
        let unknown =
            graph_freshness_with_runner(&graph, Some(&commit), &fake).ok_or("freshness")?;
        assert_eq!(unknown.status, FreshnessStatus::Unknown);
        assert!(unknown.changed_files.is_none());
        Ok(())
    }
}
