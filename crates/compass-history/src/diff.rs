use std::collections::{BTreeMap, BTreeSet};

use prolly::{Diff, VersionedValue, decode_segments};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::keys::{EDGE_KIND, HYPEREDGE_KIND, KEY_SCHEMA_V1, NODE_KIND};
use crate::{HistoryError, RealizationReader, StoredTree};

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

/// Bounded counts for one structural record family.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct RecordChangeCounts {
    pub added: u64,
    pub removed: u64,
    pub changed: u64,
}

/// Meaning-oriented graph counts that exclude source-coordinate and clustering churn.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct StructuralChangeCounts {
    pub nodes: RecordChangeCounts,
    pub edges: RecordChangeCounts,
    pub hyperedges: RecordChangeCounts,
}

impl RealizationReader<'_> {
    /// Stream graph-aware changes without materializing either complete graph.
    pub fn diff(
        &self,
        new: &RealizationReader<'_>,
        sink: &mut dyn ChangeSink,
    ) -> Result<(), HistoryError> {
        self.diff_records(
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
        new: &RealizationReader<'_>,
        records: &[RecordKind],
        sink: &mut dyn ChangeSink,
    ) -> Result<(), HistoryError> {
        if !std::ptr::eq(self.store, new.store) {
            return Err(HistoryError::OperationalState(
                "cannot diff readers from different history stores".to_owned(),
            ));
        }
        let old = &self.published;
        let new = &new.published;
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

    /// Count structural graph changes without treating relocated source anchors as topology.
    ///
    /// Unlike [`Self::diff_records`], this projection is intended for graph-version UI and
    /// summary queries. Exact record diffs remain available through `diff_records`. Both
    /// realizations must use the same build profile so extractor or configuration changes never
    /// masquerade as repository changes.
    pub fn structural_change_counts(
        &self,
        new: &RealizationReader<'_>,
    ) -> Result<StructuralChangeCounts, HistoryError> {
        let mut sink = StructuralCountSink::default();
        self.structural_diff(new, &mut sink)?;
        Ok(sink.counts)
    }

    /// Stream meaning-oriented graph changes from the Prolly roots.
    ///
    /// This is the canonical classification interface for versioned graph views. It preserves
    /// topology, direction, relation, multiplicity, provenance, and explicit compatibility edge
    /// keys while collapsing source-coordinate, clustering, presentation, and anchor-derived edge
    /// identity churn. Exact storage changes remain available through [`Self::diff_records`].
    pub fn structural_diff(
        &self,
        new: &RealizationReader<'_>,
        sink: &mut dyn ChangeSink,
    ) -> Result<(), HistoryError> {
        if !std::ptr::eq(self.store, new.store) {
            return Err(HistoryError::OperationalState(
                "cannot diff readers from different history stores".to_owned(),
            ));
        }
        if self.published.version.build_profile != new.published.version.build_profile {
            return Err(HistoryError::OperationalState(
                "cannot compare structural graphs for different history build profiles".to_owned(),
            ));
        }
        let mut projection = StructuralChangeSink::new(sink);
        for (record, old, new) in [
            (
                RecordKind::Node,
                &self.published.version.nodes_root,
                &new.published.version.nodes_root,
            ),
            (
                RecordKind::Edge,
                &self.published.version.edges_root,
                &new.published.version.edges_root,
            ),
            (
                RecordKind::Hyperedge,
                &self.published.version.hyperedges_root,
                &new.published.version.hyperedges_root,
            ),
        ] {
            self.diff_structural_root(record, old, new, &mut projection)?;
        }
        projection.finish()
    }

    fn diff_structural_root(
        &self,
        record: RecordKind,
        old: &StoredTree,
        new: &StoredTree,
        sink: &mut StructuralChangeSink<'_>,
    ) -> Result<(), HistoryError> {
        if old == new {
            return Ok(());
        }
        for difference in self.prolly.stream_diff(&self.tree(old), &self.tree(new))? {
            let difference = difference?;
            let (change, raw_key, old, new, identity) = match difference {
                Diff::Added { key, val } => {
                    let value = decode_stored_value(&val)?;
                    let identity = edge_identity(record, &key, &value.schema)?;
                    (ChangeKind::Added, key, None, Some(value.payload), identity)
                }
                Diff::Removed { key, val } => {
                    let value = decode_stored_value(&val)?;
                    let identity = edge_identity(record, &key, &value.schema)?;
                    (
                        ChangeKind::Removed,
                        key,
                        Some(value.payload),
                        None,
                        identity,
                    )
                }
                Diff::Changed { key, old, new } => (
                    ChangeKind::Changed,
                    key,
                    Some(decode_value(&old)?),
                    Some(decode_value(&new)?),
                    EdgeIdentity::Derived,
                ),
            };
            sink.change(
                GraphChange {
                    record,
                    change,
                    key: display_key(record, &raw_key)?,
                    old,
                    new,
                },
                identity,
            )?;
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
        for difference in self.prolly.stream_diff(&self.tree(old), &self.tree(new))? {
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

type EdgeTopology = (String, String, String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EdgeIdentity {
    Derived,
    ExplicitCompatibility,
}

#[derive(Default)]
struct EdgeIdentityChanges {
    added: BTreeMap<Vec<u8>, Vec<GraphChange>>,
    removed: BTreeMap<Vec<u8>, Vec<GraphChange>>,
}

struct ProjectedIdentityChange {
    projection: Vec<u8>,
    change: GraphChange,
}

struct StructuralChangeSink<'a> {
    sink: &'a mut dyn ChangeSink,
    // Prolly edge keys are ordered by topology then occurrence, so only one topology group needs
    // to be retained while the exact diff streams.
    edge_identity_changes: Option<(EdgeTopology, EdgeIdentityChanges)>,
}

impl<'a> StructuralChangeSink<'a> {
    fn new(sink: &'a mut dyn ChangeSink) -> Self {
        Self {
            sink,
            edge_identity_changes: None,
        }
    }

    fn finish(mut self) -> Result<(), HistoryError> {
        self.flush_edge_identity_changes()
    }

    fn flush_edge_identity_changes(&mut self) -> Result<(), HistoryError> {
        let Some((_, changes)) = self.edge_identity_changes.take() else {
            return Ok(());
        };
        let projections = changes
            .added
            .keys()
            .chain(changes.removed.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut added = changes.added;
        let mut removed = changes.removed;
        let mut unmatched_added = Vec::new();
        let mut unmatched_removed = Vec::new();
        for projection in projections {
            let mut added_records = added.remove(&projection).unwrap_or_default();
            let mut removed_records = removed.remove(&projection).unwrap_or_default();
            sort_identity_changes(&mut added_records);
            sort_identity_changes(&mut removed_records);
            let unchanged = added_records.len().min(removed_records.len());
            unmatched_added.extend(added_records.into_iter().skip(unchanged).map(|change| {
                ProjectedIdentityChange {
                    projection: projection.clone(),
                    change,
                }
            }));
            unmatched_removed.extend(removed_records.into_iter().skip(unchanged).map(|change| {
                ProjectedIdentityChange {
                    projection: projection.clone(),
                    change,
                }
            }));
        }
        sort_projected_identity_changes(&mut unmatched_added);
        sort_projected_identity_changes(&mut unmatched_removed);
        let changed = unmatched_added.len().min(unmatched_removed.len());
        for (added, removed) in unmatched_added
            .iter()
            .take(changed)
            .zip(unmatched_removed.iter().take(changed))
        {
            self.sink.change(GraphChange {
                record: RecordKind::Edge,
                change: ChangeKind::Changed,
                key: added.change.key.clone(),
                old: removed.change.old.clone(),
                new: added.change.new.clone(),
            })?;
        }
        for change in unmatched_added.into_iter().skip(changed) {
            self.sink.change(change.change)?;
        }
        for change in unmatched_removed.into_iter().skip(changed) {
            self.sink.change(change.change)?;
        }
        Ok(())
    }

    fn edge_identity_change(
        &mut self,
        change: &GraphChange,
        identity: EdgeIdentity,
    ) -> Result<(), HistoryError> {
        let [source, target, relation, ..] = change.key.as_slice() else {
            return Err(HistoryError::InvalidKey(
                "edge change has no source, target, and relation".to_owned(),
            ));
        };
        let value = match change.change {
            ChangeKind::Added => change.new.as_ref(),
            ChangeKind::Removed => change.old.as_ref(),
            ChangeKind::Changed => None,
        }
        .ok_or_else(|| {
            HistoryError::InvalidArtifacts("edge identity change has no record value".to_owned())
        })?;
        if identity == EdgeIdentity::ExplicitCompatibility {
            return self.sink.change(change.clone());
        }
        let projection = structural_projection(RecordKind::Edge, value)?;
        let topology = (source.clone(), target.clone(), relation.clone());
        if self
            .edge_identity_changes
            .as_ref()
            .is_some_and(|(current, _)| current != &topology)
        {
            self.flush_edge_identity_changes()?;
        }
        let (_, changes) = self
            .edge_identity_changes
            .get_or_insert_with(|| (topology, EdgeIdentityChanges::default()));
        let counts = if change.change == ChangeKind::Added {
            &mut changes.added
        } else {
            &mut changes.removed
        };
        counts.entry(projection).or_default().push(change.clone());
        Ok(())
    }

    fn change(&mut self, change: GraphChange, identity: EdgeIdentity) -> Result<(), HistoryError> {
        match (change.record, change.change) {
            (RecordKind::Node | RecordKind::Hyperedge, ChangeKind::Added | ChangeKind::Removed) => {
                self.sink.change(change)?;
            }
            (RecordKind::Edge, ChangeKind::Added | ChangeKind::Removed) => {
                self.edge_identity_change(&change, identity)?;
            }
            (RecordKind::Node | RecordKind::Edge | RecordKind::Hyperedge, ChangeKind::Changed)
                if meaningful_record_changed(&change)? =>
            {
                self.sink.change(change)?;
            }
            _ => {}
        }
        Ok(())
    }
}

fn sort_identity_changes(changes: &mut [GraphChange]) {
    changes.sort_by(|left, right| left.key.cmp(&right.key));
}

fn sort_projected_identity_changes(changes: &mut [ProjectedIdentityChange]) {
    changes.sort_by(|left, right| {
        left.projection
            .cmp(&right.projection)
            .then_with(|| left.change.key.cmp(&right.change.key))
    });
}

fn edge_identity(
    record: RecordKind,
    raw_key: &[u8],
    schema: &str,
) -> Result<EdgeIdentity, HistoryError> {
    if record != RecordKind::Edge || schema != "compass.edge" {
        return Ok(EdgeIdentity::Derived);
    }
    let segments =
        decode_segments(raw_key).map_err(|error| HistoryError::InvalidKey(error.to_string()))?;
    Ok(
        if segments.get(5).and_then(|segment| segment.first()) == Some(&1) {
            EdgeIdentity::ExplicitCompatibility
        } else {
            EdgeIdentity::Derived
        },
    )
}

#[derive(Default)]
struct StructuralCountSink {
    counts: StructuralChangeCounts,
}

impl ChangeSink for StructuralCountSink {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        let counts = match change.record {
            RecordKind::Node => &mut self.counts.nodes,
            RecordKind::Edge => &mut self.counts.edges,
            RecordKind::Hyperedge => &mut self.counts.hyperedges,
            _ => return Ok(()),
        };
        let count = match change.change {
            ChangeKind::Added => &mut counts.added,
            ChangeKind::Removed => &mut counts.removed,
            ChangeKind::Changed => &mut counts.changed,
        };
        *count = count.saturating_add(1);
        Ok(())
    }
}

fn meaningful_record_changed(change: &GraphChange) -> Result<bool, HistoryError> {
    let old = change.old.as_ref().ok_or_else(|| {
        HistoryError::InvalidArtifacts("changed graph record has no old value".to_owned())
    })?;
    let new = change.new.as_ref().ok_or_else(|| {
        HistoryError::InvalidArtifacts("changed graph record has no new value".to_owned())
    })?;
    Ok(structural_projection(change.record, old)? != structural_projection(change.record, new)?)
}

fn structural_projection(record: RecordKind, value: &Value) -> Result<Vec<u8>, HistoryError> {
    let projected = structural_graph_projection(record, value);
    crate::canonical_json_bytes(&projected)
}

/// Remove source-coordinate, clustering, and presentation fields from a graph record.
///
/// This is the shared meaning projection used by structural history summaries. It does not alter
/// the stored record or the exact diff contract.
pub fn structural_graph_projection(record: RecordKind, value: &Value) -> Value {
    project_value(record, value, true)
}

fn project_value(record: RecordKind, value: &Value, root: bool) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| project_value(record, value, false))
                .collect(),
        ),
        Value::Object(values) => Value::Object(project_object(record, values, root)),
        _ => value.clone(),
    }
}

fn project_object(
    record: RecordKind,
    values: &Map<String, Value>,
    root: bool,
) -> Map<String, Value> {
    values
        .iter()
        .filter(|(key, _)| !ignored_structural_field(record, key, root))
        .map(|(key, value)| (key.clone(), project_value(record, value, false)))
        .collect()
}

fn ignored_structural_field(record: RecordKind, field: &str, root: bool) -> bool {
    matches!(
        field,
        "anchor"
            | "anchors"
            | "color"
            | "community"
            | "communityName"
            | "community_name"
            | "endByte"
            | "endColumn"
            | "endLine"
            | "end_byte"
            | "end_column"
            | "line_end"
            | "line_start"
            | "norm_label"
            | "relationshipSite"
            | "relationship_site"
            | "sourceDigest"
            | "source_file"
            | "source_hash"
            | "source_location"
            | "startByte"
            | "startColumn"
            | "startLine"
            | "start_byte"
            | "start_column"
            | "wiringSite"
            | "wiring_site"
    ) || (root && record == RecordKind::Node && field == "source")
        || (root && record == RecordKind::Edge && matches!(field, "id" | "key"))
}

struct StoredValue {
    schema: String,
    payload: Value,
}

fn decode_stored_value(bytes: &[u8]) -> Result<StoredValue, HistoryError> {
    let envelope = VersionedValue::from_bytes(bytes)?;
    Ok(StoredValue {
        schema: envelope.schema,
        payload: serde_json::from_slice(&envelope.payload)?,
    })
}

fn decode_value(bytes: &[u8]) -> Result<Value, HistoryError> {
    Ok(decode_stored_value(bytes)?.payload)
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

    #[test]
    fn edge_identity_uses_the_value_schema_not_discriminator_shape() -> Result<(), HistoryError> {
        let key = edge_key("source", "target", "calls", true, Some(&[1, b'k']));
        assert_eq!(
            edge_identity(RecordKind::Edge, &key, "compass.edge")?,
            EdgeIdentity::ExplicitCompatibility
        );
        assert_eq!(
            edge_identity(RecordKind::Edge, &key, "compass.graph.edge.v1")?,
            EdgeIdentity::Derived
        );
        Ok(())
    }
}
