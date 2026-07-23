use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

const BACKUP_ARTIFACTS: &[&str] = &[
    "graph.json",
    "program.json",
    "GRAPH_REPORT.md",
    ".compass_labels.json",
    ".compass_analysis.json",
    "manifest.json",
    ".compass_semantic_marker",
    "cost.json",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackupResult {
    pub path: Option<PathBuf>,
    pub message: Option<String>,
    pub warning: Option<String>,
}

/// Snapshot non-regenerable graph artifacts before an overwrite.
///
/// Failures are reported but deliberately never block the graph write, matching
/// Graphify's recovery contract.
#[must_use]
pub fn backup_if_protected(output_dir: &Path) -> BackupResult {
    if std::env::var_os("GRAPHIFY_NO_BACKUP").is_some_and(|value| !value.is_empty()) {
        return BackupResult::default();
    }
    let graph_path = output_dir.join("graph.json");
    if !graph_path.is_file() {
        return BackupResult::default();
    }
    let semantic = output_dir.join(".compass_semantic_marker").exists();
    let curated = labels_are_curated(&output_dir.join(".compass_labels.json"));
    if !semantic && !curated {
        return BackupResult::default();
    }
    let reason = match (semantic, curated) {
        (true, true) => "semantic+curated",
        (true, false) => "semantic",
        (false, true) => "curated",
        (false, false) => return BackupResult::default(),
    };
    let date = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .date()
        .to_string();
    let backup_dir = output_dir.join(&date);
    let backup_graph = backup_dir.join("graph.json");
    if backup_graph.is_file()
        && file_digest(&graph_path).is_some_and(|digest| file_digest(&backup_graph) == Some(digest))
    {
        return BackupResult {
            path: Some(backup_dir),
            ..BackupResult::default()
        };
    }
    if let Err(error) = fs::create_dir_all(&backup_dir) {
        return BackupResult {
            warning: Some(format!(
                "[graphify] warning: backup failed ({error}) - continuing with overwrite"
            )),
            ..BackupResult::default()
        };
    }
    let copied = BACKUP_ARTIFACTS
        .iter()
        .filter(|artifact| {
            let source = output_dir.join(artifact);
            source.is_file() && fs::copy(&source, backup_dir.join(artifact)).is_ok()
        })
        .count();
    BackupResult {
        path: Some(backup_dir),
        message: (copied > 0)
            .then(|| format!("[graphify] backed up {reason} graph ({copied} files) -> {date}/")),
        warning: None,
    }
}

fn labels_are_curated(path: &Path) -> bool {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.as_object().cloned())
        .is_some_and(|labels| {
            labels.iter().any(|(community, label)| {
                label
                    .as_str()
                    .is_none_or(|label| label != format!("Community {community}"))
            })
        })
}

fn file_digest(path: &Path) -> Option<[u8; 32]> {
    let bytes = fs::read(path).ok()?;
    Some(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_backup_is_dated_deduplicated_and_complete() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("graph.json"), "graph")?;
        fs::write(directory.path().join("program.json"), "program")?;
        fs::write(directory.path().join("GRAPH_REPORT.md"), "report")?;
        fs::write(
            directory.path().join(".compass_labels.json"),
            r#"{"0":"Orders"}"#,
        )?;
        let first = backup_if_protected(directory.path());
        assert!(
            first
                .message
                .as_deref()
                .is_some_and(|message| message.contains("4 files"))
        );
        let backup = first.path.ok_or("backup path missing")?;
        assert_eq!(fs::read_to_string(backup.join("graph.json"))?, "graph");
        assert_eq!(fs::read_to_string(backup.join("program.json"))?, "program");
        let second = backup_if_protected(directory.path());
        assert!(second.message.is_none());
        Ok(())
    }
}
