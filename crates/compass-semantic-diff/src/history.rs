use compass_analysis::FunctionSummary;
use compass_history::{
    ChangeKind, ChangeSink, GraphChange, HistoryError, HistoryRecord, HistoryRecordKey,
    HistoryStore, PublishedVersion, RealizationReader, RecordKind, SourceFileDelta,
};
use compass_ir::{FunctionIr, ModuleIr};
use compass_model::NodeRecord;

use crate::{
    ChangeDirection, DependencyDelta, EvidenceRef, GraphDelta, GraphEdgeDelta, GraphNodeDelta,
    SemanticDiffError, SemanticDiffInput, SemanticDiffReport, SnapshotIdentity, SnapshotReader,
    SnapshotSide, StaticTestEvidence, compare,
};

/// Compare two validated immutable realizations using the canonical semantic-diff engine.
pub fn compare_history_realizations(
    history: &HistoryStore,
    old: &PublishedVersion,
    new: &PublishedVersion,
    source_deltas: &[SourceFileDelta],
) -> Result<SemanticDiffReport, SemanticDiffError> {
    history.validate(&old.id).map_err(evidence_error)?;
    history.validate(&new.id).map_err(evidence_error)?;
    let mut direct = DirectChanges::default();
    let old_reader = history.reader(&old.id).map_err(evidence_error)?;
    let new_reader = history.reader(&new.id).map_err(evidence_error)?;
    old_reader
        .structural_diff(&new_reader, &mut direct)
        .map_err(evidence_error)?;
    direct.normalize_graph_delta();
    let snapshots = HistorySnapshots {
        old: old_reader,
        new: new_reader,
    };
    let test_evidence = StaticTestEvidence::new(&snapshots, SnapshotSide::New);
    compare(SemanticDiffInput {
        old: SnapshotIdentity {
            commit: old.version.git_commit.clone(),
            realization: old.id.to_string(),
            fingerprint: old.version.profile_digest.clone(),
        },
        new: SnapshotIdentity {
            commit: new.version.git_commit.clone(),
            realization: new.id.to_string(),
            fingerprint: new.version.profile_digest.clone(),
        },
        source_deltas,
        changed_node_ids: &direct.nodes,
        dependency_deltas: &direct.dependencies,
        graph_delta: &direct.graph_delta,
        snapshots: &snapshots,
        test_evidence: &test_evidence,
    })
}

struct HistorySnapshots<'a> {
    old: RealizationReader<'a>,
    new: RealizationReader<'a>,
}

impl HistorySnapshots<'_> {
    fn reader(&self, side: SnapshotSide) -> &RealizationReader<'_> {
        match side {
            SnapshotSide::Old => &self.old,
            SnapshotSide::New => &self.new,
        }
    }
}

impl SnapshotReader for HistorySnapshots<'_> {
    fn node(
        &self,
        side: SnapshotSide,
        node_id: &str,
    ) -> Result<Option<NodeRecord>, SemanticDiffError> {
        match self
            .reader(side)
            .read(HistoryRecordKey::Node(node_id))
            .map_err(evidence_error)?
        {
            Some(HistoryRecord::Node(node)) => Ok(Some(node)),
            None => Ok(None),
            Some(_) => Err(SemanticDiffError::Evidence(
                "node key returned another record type".to_owned(),
            )),
        }
    }

    fn module(
        &self,
        side: SnapshotSide,
        source_file: &str,
    ) -> Result<Option<ModuleIr>, SemanticDiffError> {
        match self
            .reader(side)
            .read(HistoryRecordKey::ProgramModule(source_file))
            .map_err(evidence_error)?
        {
            Some(HistoryRecord::ProgramModule(module)) => Ok(Some(module)),
            None => Ok(None),
            Some(_) => Err(SemanticDiffError::Evidence(
                "module key returned another record type".to_owned(),
            )),
        }
    }

    fn summary(
        &self,
        side: SnapshotSide,
        symbol_id: &str,
    ) -> Result<Option<FunctionSummary>, SemanticDiffError> {
        match self
            .reader(side)
            .read(HistoryRecordKey::ProgramSummary(symbol_id))
            .map_err(evidence_error)?
        {
            Some(HistoryRecord::ProgramSummary(summary)) => Ok(Some(summary)),
            None => Ok(None),
            Some(_) => Err(SemanticDiffError::Evidence(
                "summary key returned another record type".to_owned(),
            )),
        }
    }

    fn function(
        &self,
        side: SnapshotSide,
        symbol_id: &str,
    ) -> Result<Option<FunctionIr>, SemanticDiffError> {
        match self
            .reader(side)
            .read(HistoryRecordKey::ProgramFunction(symbol_id))
            .map_err(evidence_error)?
        {
            Some(HistoryRecord::ProgramFunction(function)) => Ok(Some(function)),
            None => Ok(None),
            Some(_) => Err(SemanticDiffError::Evidence(
                "function key returned another record type".to_owned(),
            )),
        }
    }

    fn reverse_callers(
        &self,
        side: SnapshotSide,
        symbol_id: &str,
    ) -> Result<Vec<String>, SemanticDiffError> {
        match self
            .reader(side)
            .read(HistoryRecordKey::ReverseCallers(symbol_id))
            .map_err(evidence_error)?
        {
            Some(HistoryRecord::ReverseCallers(callers)) => Ok(callers),
            None => Ok(Vec::new()),
            Some(_) => Err(SemanticDiffError::Evidence(
                "reverse-call key returned another record type".to_owned(),
            )),
        }
    }
}

#[derive(Default)]
struct DirectChanges {
    nodes: Vec<String>,
    dependencies: Vec<DependencyDelta>,
    graph_delta: GraphDelta,
}

impl DirectChanges {
    fn normalize_graph_delta(&mut self) {
        let sort_nodes = |nodes: &mut Vec<GraphNodeDelta>| {
            for node in nodes.iter_mut() {
                node.changed_fields.sort();
                node.changed_fields.dedup();
            }
            nodes.sort_by(|left, right| left.id.as_bytes().cmp(right.id.as_bytes()));
            nodes.dedup_by(|left, right| left.id == right.id);
        };
        sort_nodes(&mut self.graph_delta.added_nodes);
        sort_nodes(&mut self.graph_delta.removed_nodes);
        sort_nodes(&mut self.graph_delta.changed_nodes);
        let sort_edges = |edges: &mut Vec<GraphEdgeDelta>| {
            for edge in edges.iter_mut() {
                edge.changed_fields.sort();
                edge.changed_fields.dedup();
            }
            edges.sort_by(|left, right| {
                (
                    left.source.as_bytes(),
                    left.relation.as_bytes(),
                    left.target.as_bytes(),
                    left.key.as_bytes(),
                )
                    .cmp(&(
                        right.source.as_bytes(),
                        right.relation.as_bytes(),
                        right.target.as_bytes(),
                        right.key.as_bytes(),
                    ))
            });
            edges.dedup_by(|left, right| {
                left.source == right.source
                    && left.target == right.target
                    && left.relation == right.relation
                    && left.key == right.key
                    && left.changed_fields == right.changed_fields
            });
        };
        sort_edges(&mut self.graph_delta.added_edges);
        sort_edges(&mut self.graph_delta.removed_edges);
        sort_edges(&mut self.graph_delta.changed_edges);
    }
}

impl ChangeSink for DirectChanges {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        match change.record {
            RecordKind::Node => self.node_change(&change),
            RecordKind::Edge => self.edge_change(&change),
            _ => {}
        }
        Ok(())
    }
}

impl DirectChanges {
    fn node_change(&mut self, change: &GraphChange) {
        let Some(node_id) = change.key.first() else {
            return;
        };
        match change.change {
            ChangeKind::Added => {
                self.nodes.push(node_id.clone());
                self.graph_delta.added_nodes.push(graph_node_delta(
                    node_id,
                    change.new.as_ref(),
                    Vec::new(),
                ));
            }
            ChangeKind::Removed => {
                self.nodes.push(node_id.clone());
                self.graph_delta.removed_nodes.push(graph_node_delta(
                    node_id,
                    change.old.as_ref(),
                    Vec::new(),
                ));
            }
            ChangeKind::Changed => {
                let fields = meaningful_graph_fields(
                    RecordKind::Node,
                    change.old.as_ref(),
                    change.new.as_ref(),
                    &mut self.graph_delta.collapsed_attribute_changes,
                );
                if !fields.is_empty() {
                    self.nodes.push(node_id.clone());
                    self.graph_delta.changed_nodes.push(graph_node_delta(
                        node_id,
                        change.new.as_ref().or(change.old.as_ref()),
                        fields,
                    ));
                }
            }
        }
    }

    fn edge_change(&mut self, change: &GraphChange) {
        let Some((source, target, relation)) = edge_key(change) else {
            return;
        };
        let key = change.key.get(3).map(String::as_str).unwrap_or_default();
        let value = change.new.as_ref().or(change.old.as_ref());
        match change.change {
            ChangeKind::Added => self.graph_delta.added_edges.push(graph_edge_delta(
                source,
                target,
                relation,
                key,
                value,
                Vec::new(),
            )),
            ChangeKind::Removed => self.graph_delta.removed_edges.push(graph_edge_delta(
                source,
                target,
                relation,
                key,
                value,
                Vec::new(),
            )),
            ChangeKind::Changed => {
                let fields = meaningful_graph_fields(
                    RecordKind::Edge,
                    change.old.as_ref(),
                    change.new.as_ref(),
                    &mut self.graph_delta.collapsed_attribute_changes,
                );
                if !fields.is_empty() {
                    self.graph_delta.changed_edges.push(graph_edge_delta(
                        source, target, relation, key, value, fields,
                    ));
                }
                return;
            }
        }
        if !matches!(
            relation,
            "calls" | "imports" | "imports_from" | "depends_on" | "uses" | "references"
        ) {
            return;
        }
        self.dependencies.push(DependencyDelta {
            source: source.to_owned(),
            target: target.to_owned(),
            relation: relation.to_owned(),
            change: if change.change == ChangeKind::Added {
                ChangeDirection::Added
            } else {
                ChangeDirection::Removed
            },
            evidence: vec![EvidenceRef {
                source_file: value.map(graph_edge_source_file).unwrap_or_default(),
                start_byte: None,
                end_byte: None,
                record_key: Some(change.key.join("/")),
                capability: "dependencies".to_owned(),
            }],
        });
    }
}

fn edge_key(change: &GraphChange) -> Option<(&str, &str, &str)> {
    Some((
        change.key.first()?.as_str(),
        change.key.get(1)?.as_str(),
        change.key.get(2)?.as_str(),
    ))
}

fn graph_node_delta(
    id: &str,
    value: Option<&serde_json::Value>,
    changed_fields: Vec<String>,
) -> GraphNodeDelta {
    GraphNodeDelta {
        id: id.to_owned(),
        label: graph_string(value, "label")
            .or_else(|| graph_string(value, "name"))
            .unwrap_or_default(),
        kind: graph_string(value, "kind").unwrap_or_default(),
        source_file: value.map(graph_node_source_file).unwrap_or_default(),
        changed_fields,
    }
}

fn graph_edge_delta(
    source: &str,
    target: &str,
    relation: &str,
    key: &str,
    value: Option<&serde_json::Value>,
    changed_fields: Vec<String>,
) -> GraphEdgeDelta {
    GraphEdgeDelta {
        source: source.to_owned(),
        target: target.to_owned(),
        relation: relation.to_owned(),
        key: key.to_owned(),
        source_file: value.map(graph_edge_source_file).unwrap_or_default(),
        changed_fields,
    }
}

fn graph_string(value: Option<&serde_json::Value>, key: &str) -> Option<String> {
    value
        .and_then(|value| value.get(key))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            value
                .and_then(|value| value.get("attributes"))
                .and_then(|attributes| attributes.get(key))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_owned)
}

fn graph_node_source_file(value: &serde_json::Value) -> String {
    graph_string(Some(value), "source_file")
        .or_else(|| nested_string(value, &["source", "file"]))
        .unwrap_or_default()
}

fn graph_edge_source_file(value: &serde_json::Value) -> String {
    graph_string(Some(value), "source_file")
        .or_else(|| nested_string(value, &["relationshipSite", "file"]))
        .or_else(|| nested_string(value, &["relationship_site", "file"]))
        .or_else(|| {
            value
                .get("evidence")
                .and_then(serde_json::Value::as_array)
                .and_then(|items| items.iter().find_map(evidence_source_file))
        })
        .unwrap_or_default()
}

fn evidence_source_file(value: &serde_json::Value) -> Option<String> {
    nested_string(value, &["wiringSite", "file"])
        .or_else(|| nested_string(value, &["wiring_site", "file"]))
        .or_else(|| {
            value
                .get("anchors")
                .and_then(serde_json::Value::as_array)
                .and_then(|anchors| anchors.first())
                .and_then(|anchor| nested_string(anchor, &["file"]))
        })
}

fn nested_string(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn meaningful_graph_fields(
    record: RecordKind,
    old: Option<&serde_json::Value>,
    new: Option<&serde_json::Value>,
    collapsed: &mut std::collections::BTreeMap<String, usize>,
) -> Vec<String> {
    let projected_old =
        old.map(|value| compass_history::structural_graph_projection(record, value));
    let projected_new =
        new.map(|value| compass_history::structural_graph_projection(record, value));
    let mut keys = std::collections::BTreeSet::new();
    if let Some(object) = old.and_then(serde_json::Value::as_object) {
        keys.extend(object.keys().cloned());
    }
    if let Some(object) = new.and_then(serde_json::Value::as_object) {
        keys.extend(object.keys().cloned());
    }
    let mut meaningful = Vec::new();
    for key in keys {
        if old.and_then(|value| value.get(&key)) == new.and_then(|value| value.get(&key)) {
            continue;
        }
        let projected_old_field = projected_old.as_ref().and_then(|value| value.get(&key));
        let projected_new_field = projected_new.as_ref().and_then(|value| value.get(&key));
        if projected_old_field == projected_new_field {
            *collapsed.entry(key).or_default() += 1;
        } else {
            meaningful.push(key);
        }
    }
    meaningful
}

fn evidence_error(error: HistoryError) -> SemanticDiffError {
    SemanticDiffError::Evidence(error.to_string())
}
