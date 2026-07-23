use prolly::{Diff, VersionedValue, decode_segments};
use serde::Serialize;
use serde_json::Value;

use crate::keys::{EDGE_KIND, HYPEREDGE_KIND, KEY_SCHEMA_V1, NODE_KIND};
use crate::{HistoryError, HistoryStore, RealizationId, StoredTree};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Node,
    Edge,
    Hyperedge,
    Analysis,
    Metadata,
    ProgramFact,
    ProgramSummary,
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
        self.diff_records(
            old,
            new,
            &[
                RecordKind::Node,
                RecordKind::Edge,
                RecordKind::Hyperedge,
                RecordKind::Analysis,
                RecordKind::Metadata,
                RecordKind::ProgramFact,
                RecordKind::ProgramSummary,
            ],
            sink,
        )
    }

    /// Stream only the requested record roots.
    ///
    /// This is more than a presentation filter: omitted Prolly roots are never
    /// opened or traversed. Callers such as topology-only diff can therefore
    /// avoid decoding analysis and reconstruction metadata entirely.
    pub fn diff_records(
        &self,
        old: &RealizationId,
        new: &RealizationId,
        records: &[RecordKind],
        sink: &mut dyn ChangeSink,
    ) -> Result<(), HistoryError> {
        let activity = self.activity()?;
        let old = self.get_with_activity(old, &activity)?;
        let new = self.get_with_activity(new, &activity)?;
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
            (
                RecordKind::ProgramFact,
                &old.version.program_facts_root,
                &new.version.program_facts_root,
            ),
            (
                RecordKind::ProgramSummary,
                &old.version.program_summaries_root,
                &new.version.program_summaries_root,
            ),
        ] {
            if records.contains(&kind) {
                self.diff_root(kind, left, right, sink)?;
            }
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
                key: display_key(record, &key)?,
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

fn display_key(record: RecordKind, key: &[u8]) -> Result<Vec<String>, HistoryError> {
    let mut segments =
        decode_segments(key).map_err(|error| HistoryError::InvalidKey(error.to_string()))?;
    if let Some(kind) = match record {
        RecordKind::Node => Some(NODE_KIND),
        RecordKind::Edge => Some(EDGE_KIND),
        RecordKind::Hyperedge => Some(HYPEREDGE_KIND),
        RecordKind::Analysis
        | RecordKind::Metadata
        | RecordKind::ProgramFact
        | RecordKind::ProgramSummary => None,
    } {
        if segments.first().map(Vec::as_slice) != Some(KEY_SCHEMA_V1)
            || segments.get(1).map(Vec::as_slice) != Some(kind)
        {
            return Err(HistoryError::InvalidKey(format!(
                "{record:?} key has an invalid typed prefix"
            )));
        }
        segments.drain(..2);
    }
    segments
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

#[cfg(test)]
mod tests {
    use prolly::KeyBuilder;

    use super::*;
    use crate::{edge_key, hyperedge_key, node_key};

    #[test]
    fn display_keys_hide_valid_internal_typed_prefixes() -> Result<(), HistoryError> {
        assert_eq!(
            display_key(RecordKind::Node, &node_key("node-id"))?,
            ["node-id"]
        );
        assert_eq!(
            display_key(
                RecordKind::Edge,
                &edge_key("source", "target", "calls", true, None),
            )?,
            ["source", "target", "calls"]
        );
        assert_eq!(
            display_key(RecordKind::Hyperedge, &hyperedge_key(b"identity", None))?,
            ["identity"]
        );
        Ok(())
    }

    #[test]
    fn display_keys_reject_a_mismatched_internal_typed_prefix() {
        let edge = KeyBuilder::new()
            .push_segment(KEY_SCHEMA_V1)
            .push_segment(EDGE_KIND)
            .push_str("node-id")
            .finish();
        assert!(display_key(RecordKind::Node, &edge).is_err());
    }
}
