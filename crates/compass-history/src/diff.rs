use prolly::{Diff, VersionedValue, decode_segments};
use serde::Serialize;
use serde_json::Value;

use crate::{HistoryError, HistoryStore, RealizationId, StoredTree};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Node,
    Edge,
    Hyperedge,
    Analysis,
    Metadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphChange {
    pub record: RecordKind,
    pub change: ChangeKind,
    pub key: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new: Option<Value>,
}

pub trait ChangeSink {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError>;
}

impl HistoryStore {
    /// Stream graph-aware changes without materializing either complete graph.
    pub fn diff(
        &self,
        old: &RealizationId,
        new: &RealizationId,
        sink: &mut dyn ChangeSink,
    ) -> Result<(), HistoryError> {
        let _activity = self.activity()?;
        let old = self.get(old)?;
        let new = self.get(new)?;
        for (kind, left, right) in [
            (
                RecordKind::Node,
                &old.version.nodes_root,
                &new.version.nodes_root,
            ),
            (
                RecordKind::Edge,
                &old.version.edges_root,
                &new.version.edges_root,
            ),
            (
                RecordKind::Hyperedge,
                &old.version.hyperedges_root,
                &new.version.hyperedges_root,
            ),
            (
                RecordKind::Analysis,
                &old.version.analysis_root,
                &new.version.analysis_root,
            ),
            (
                RecordKind::Metadata,
                &old.version.metadata_root,
                &new.version.metadata_root,
            ),
        ] {
            self.diff_root(kind, left, right, sink)?;
        }
        Ok(())
    }

    fn diff_root(
        &self,
        record: RecordKind,
        old: &StoredTree,
        new: &StoredTree,
        sink: &mut dyn ChangeSink,
    ) -> Result<(), HistoryError> {
        if old == new {
            return Ok(());
        }
        for difference in self.prolly.stream_diff(&old.to_tree(), &new.to_tree())? {
            let difference = difference?;
            let (change, key, old, new) = match difference {
                Diff::Added { key, val } => {
                    (ChangeKind::Added, key, None, Some(decode_value(&val)?))
                }
                Diff::Removed { key, val } => {
                    (ChangeKind::Removed, key, Some(decode_value(&val)?), None)
                }
                Diff::Changed { key, old, new } => (
                    ChangeKind::Changed,
                    key,
                    Some(decode_value(&old)?),
                    Some(decode_value(&new)?),
                ),
            };
            sink.change(GraphChange {
                record,
                change,
                key: display_key(&key)?,
                old,
                new,
            })?;
        }
        Ok(())
    }
}

fn decode_value(bytes: &[u8]) -> Result<Value, HistoryError> {
    let envelope = VersionedValue::from_bytes(bytes)?;
    serde_json::from_slice(&envelope.payload).map_err(HistoryError::from)
}

fn display_key(key: &[u8]) -> Result<Vec<String>, HistoryError> {
    decode_segments(key)
        .map_err(|error| HistoryError::InvalidKey(error.to_string()))?
        .into_iter()
        .map(|segment| {
            if segment
                .iter()
                .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
            {
                String::from_utf8(segment)
                    .map_err(|error| HistoryError::InvalidKey(error.to_string()))
            } else {
                Ok(format!("0x{}", hex(&segment)))
            }
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}
