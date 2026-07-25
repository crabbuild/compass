use compass_analysis::FunctionSummary;
use compass_history::{
    ChangeKind, ChangeSink, ExtractionFingerprint, GraphChange, HistoryError, HistoryRecord,
    HistoryRecordKey, HistoryStore, RealizationId, RecordKind, Repository,
};
use compass_ir::{FunctionIr, ModuleIr};
use compass_model::NodeRecord;
use compass_semantic_diff::{
    ChangeDirection, DependencyDelta, EvidenceRef, GraphDelta, GraphEdgeDelta, GraphNodeDelta,
    SemanticDiffError, SemanticDiffInput, SnapshotIdentity, SnapshotReader, SnapshotSide,
    StaticTestEvidence, compare,
};

use crate::semantic_diff_render::{RenderOptions, render_html, render_json, render_text};
use crate::{Frontend, Outcome, history_commands};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Text,
    Json,
    Html,
}

struct Options {
    old: String,
    new: String,
    format: Format,
    all: bool,
    limit: Option<usize>,
    explain: Option<String>,
    fingerprint: Option<String>,
    output: Option<String>,
}

pub(crate) fn help(frontend: Frontend) -> String {
    let command = if frontend == Frontend::Compass {
        "compass"
    } else {
        "graphify"
    };
    format!(
        "Usage: {command} diff <OLD> <NEW> [OPTIONS]\n\nOptions:\n  --format <text|json|html>  Output format [default: text]\n  --output <PATH>            Write the self-contained HTML report (required with --format html)\n  --limit <N>                Show at most N findings per text section [default: 20]\n  --all                      Include routine churn and show every finding\n  --explain <FINDING_ID>     Expand one semantic finding\n  --fingerprint <SHA256>     Select one extraction fingerprint"
    )
}

pub(crate) fn command(frontend: Frontend, args: &[String]) -> Outcome {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return Outcome::success(help(frontend));
    }
    match execute(args) {
        Ok(output) => Outcome::success(output),
        Err(CommandError::Usage(message)) => {
            Outcome::failure_with_code(format!("error: {message}"), 2)
        }
        Err(CommandError::Runtime(message)) => Outcome::failure(format!("error: {message}")),
    }
}

fn execute(args: &[String]) -> Result<String, CommandError> {
    let options = parse(args).map_err(CommandError::Usage)?;
    let current = std::env::current_dir().map_err(runtime)?;
    let repository = Repository::discover(&current).map_err(runtime)?;
    let old_commit = repository.resolve(&options.old).map_err(runtime)?;
    let new_commit = repository.resolve(&options.new).map_err(runtime)?;
    let source_deltas = repository
        .source_delta(&old_commit, &new_commit)
        .map_err(runtime)?;
    let resolved = history_commands::resolve_comparable_pair(
        &repository,
        old_commit,
        new_commit,
        options.fingerprint.as_deref(),
    )
    .map_err(CommandError::Runtime)?;
    let mut direct = DirectChanges::default();
    resolved
        .history
        .diff_records(
            &resolved.old.id,
            &resolved.new.id,
            &[RecordKind::Node, RecordKind::Edge],
            &mut direct,
        )
        .map_err(runtime)?;
    direct.cancel_attribute_only_dependency_churn();
    direct.normalize_graph_delta();
    let snapshots = HistorySnapshots {
        store: &resolved.history,
        old: &resolved.old.id,
        new: &resolved.new.id,
    };
    let test_evidence = StaticTestEvidence::new(&snapshots, SnapshotSide::New);
    let report = compare(SemanticDiffInput {
        old: SnapshotIdentity {
            commit: resolved.old.version.git_commit.clone(),
            realization: resolved.old.id.to_string(),
            fingerprint: resolved.old.version.profile_digest.clone(),
        },
        new: SnapshotIdentity {
            commit: resolved.new.version.git_commit.clone(),
            realization: resolved.new.id.to_string(),
            fingerprint: resolved.new.version.profile_digest.clone(),
        },
        source_deltas: &source_deltas,
        changed_node_ids: &direct.nodes,
        dependency_deltas: &direct.dependencies,
        graph_delta: &direct.graph_delta,
        snapshots: &snapshots,
        test_evidence: &test_evidence,
    })
    .map_err(runtime)?;
    let render = RenderOptions {
        include_routine: options.all,
        max_findings_per_section: options.limit.or((!options.all).then_some(20)),
        explain: options.explain.as_deref(),
    };
    match options.format {
        Format::Text => render_text(&report, &render).map_err(runtime),
        Format::Json => render_json(&report, &render).map_err(runtime),
        Format::Html => {
            let html = render_html(&report, &render).map_err(runtime)?;
            let output = options
                .output
                .as_deref()
                .ok_or_else(|| CommandError::Usage("--output is required".to_owned()))?;
            std::fs::write(output, html).map_err(runtime)?;
            Ok(format!("semantic diff HTML written to {output}"))
        }
    }
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut revisions = Vec::new();
    let mut format = Format::Text;
    let mut format_set = false;
    let mut all = false;
    let mut limit = None;
    let mut explain = None;
    let mut fingerprint = None;
    let mut output = None;
    let mut parse_options = true;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--" if parse_options => parse_options = false,
            "--all" if parse_options => {
                if all {
                    return Err("duplicate --all".to_owned());
                }
                all = true;
            }
            "--limit" if parse_options => {
                index += 1;
                let value = args.get(index).ok_or("--limit requires a value")?;
                if limit.replace(parse_limit(value)?).is_some() {
                    return Err("duplicate --limit".to_owned());
                }
            }
            value if parse_options && value.starts_with("--limit=") => {
                if limit.replace(parse_limit(&value[8..])?).is_some() {
                    return Err("duplicate --limit".to_owned());
                }
            }
            "--format" if parse_options => {
                index += 1;
                let value = args.get(index).ok_or("--format requires a value")?;
                if format_set {
                    return Err("duplicate --format".to_owned());
                }
                format_set = true;
                format = parse_format(value)?;
            }
            value if parse_options && value.starts_with("--format=") => {
                if format_set {
                    return Err("duplicate --format".to_owned());
                }
                format_set = true;
                format = parse_format(&value[9..])?;
            }
            "--output" if parse_options => {
                index += 1;
                let value = args.get(index).ok_or("--output requires a value")?;
                if value.is_empty() || output.replace(value.clone()).is_some() {
                    return Err("--output requires one path".to_owned());
                }
            }
            value if parse_options && value.starts_with("--output=") => {
                let value = &value[9..];
                if value.is_empty() || output.replace(value.to_owned()).is_some() {
                    return Err("--output requires one path".to_owned());
                }
            }
            "--explain" if parse_options => {
                index += 1;
                let value = args.get(index).ok_or("--explain requires a value")?;
                if explain.replace(value.clone()).is_some() {
                    return Err("duplicate --explain".to_owned());
                }
            }
            value if parse_options && value.starts_with("--explain=") => {
                let value = &value[10..];
                if value.is_empty() || explain.replace(value.to_owned()).is_some() {
                    return Err("--explain requires one finding ID".to_owned());
                }
            }
            "--fingerprint" if parse_options => {
                index += 1;
                let value = args.get(index).ok_or("--fingerprint requires a value")?;
                if fingerprint.replace(value.clone()).is_some() {
                    return Err("duplicate --fingerprint".to_owned());
                }
            }
            value if parse_options && value.starts_with("--fingerprint=") => {
                let value = &value[14..];
                if value.is_empty() || fingerprint.replace(value.to_owned()).is_some() {
                    return Err("--fingerprint requires one value".to_owned());
                }
            }
            value if parse_options && value.starts_with('-') => {
                return Err(format!("unknown option {value}"));
            }
            value => revisions.push(value.to_owned()),
        }
        index += 1;
    }
    if revisions.len() != 2 {
        return Err("diff requires exactly OLD and NEW revisions".to_owned());
    }
    if all && limit.is_some() {
        return Err("--all conflicts with --limit; --all is intentionally exhaustive".to_owned());
    }
    match (format, output.as_ref()) {
        (Format::Html, None) => {
            return Err("--output is required with --format html".to_owned());
        }
        (Format::Html, Some(_)) => {}
        (_, Some(_)) => {
            return Err("--output is only valid with --format html".to_owned());
        }
        (_, None) => {}
    }
    if let Some(value) = &fingerprint {
        value
            .parse::<ExtractionFingerprint>()
            .map_err(|_| "--fingerprint must be a lowercase SHA-256 digest".to_owned())?;
    }
    Ok(Options {
        old: revisions.remove(0),
        new: revisions.remove(0),
        format,
        all,
        limit,
        explain,
        fingerprint,
        output,
    })
}

fn parse_limit(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "--limit must be a positive integer".to_owned())
}

fn parse_format(value: &str) -> Result<Format, String> {
    match value {
        "text" => Ok(Format::Text),
        "json" => Ok(Format::Json),
        "html" => Ok(Format::Html),
        _ => Err("--format must be text, json, or html".to_owned()),
    }
}

struct HistorySnapshots<'a> {
    store: &'a HistoryStore,
    old: &'a RealizationId,
    new: &'a RealizationId,
}

impl HistorySnapshots<'_> {
    fn realization(&self, side: SnapshotSide) -> &RealizationId {
        match side {
            SnapshotSide::Old => self.old,
            SnapshotSide::New => self.new,
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
            .store
            .read_record(self.realization(side), HistoryRecordKey::Node(node_id))
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
            .store
            .read_record(
                self.realization(side),
                HistoryRecordKey::ProgramModule(source_file),
            )
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
            .store
            .read_record(
                self.realization(side),
                HistoryRecordKey::ProgramSummary(symbol_id),
            )
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
            .store
            .read_record(
                self.realization(side),
                HistoryRecordKey::ProgramFunction(symbol_id),
            )
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
            .store
            .read_record(
                self.realization(side),
                HistoryRecordKey::ReverseCallers(symbol_id),
            )
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
    fn cancel_attribute_only_dependency_churn(&mut self) {
        let mut directions = std::collections::BTreeMap::<(String, String, String), i64>::new();
        for delta in &self.dependencies {
            let balance = directions
                .entry((
                    delta.source.clone(),
                    delta.target.clone(),
                    delta.relation.clone(),
                ))
                .or_default();
            match delta.change {
                ChangeDirection::Added => *balance += 1,
                ChangeDirection::Removed => *balance -= 1,
            }
        }
        self.dependencies.retain(|delta| {
            let key = (
                delta.source.clone(),
                delta.target.clone(),
                delta.relation.clone(),
            );
            let Some(balance) = directions.get_mut(&key) else {
                return true;
            };
            match delta.change {
                ChangeDirection::Added if *balance > 0 => {
                    *balance -= 1;
                    true
                }
                ChangeDirection::Removed if *balance < 0 => {
                    *balance += 1;
                    true
                }
                _ => false,
            }
        });
    }

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
        let mut balances = std::collections::BTreeMap::<(String, String, String), i64>::new();
        for edge in &self.graph_delta.added_edges {
            *balances
                .entry((
                    edge.source.clone(),
                    edge.relation.clone(),
                    edge.target.clone(),
                ))
                .or_default() += 1;
        }
        for edge in &self.graph_delta.removed_edges {
            *balances
                .entry((
                    edge.source.clone(),
                    edge.relation.clone(),
                    edge.target.clone(),
                ))
                .or_default() -= 1;
        }
        let removed_before = self.graph_delta.removed_edges.len();
        self.graph_delta.added_edges.retain(|edge| {
            let key = (
                edge.source.clone(),
                edge.relation.clone(),
                edge.target.clone(),
            );
            let Some(balance) = balances.get_mut(&key) else {
                return true;
            };
            if *balance > 0 {
                *balance -= 1;
                true
            } else {
                false
            }
        });
        self.graph_delta.removed_edges.retain(|edge| {
            let key = (
                edge.source.clone(),
                edge.relation.clone(),
                edge.target.clone(),
            );
            let Some(balance) = balances.get_mut(&key) else {
                return true;
            };
            if *balance < 0 {
                *balance += 1;
                true
            } else {
                false
            }
        });
        let cancelled = removed_before.saturating_sub(self.graph_delta.removed_edges.len());
        if cancelled > 0 {
            *self
                .graph_delta
                .collapsed_attribute_changes
                .entry("edge_identity".to_owned())
                .or_default() += cancelled;
        }
    }
}

impl ChangeSink for DirectChanges {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        match change.record {
            RecordKind::Node => {
                if let Some(node_id) = change.key.first() {
                    self.nodes.push(node_id.clone());
                    match change.change {
                        ChangeKind::Added => self.graph_delta.added_nodes.push(graph_node_delta(
                            node_id,
                            change.new.as_ref(),
                            Vec::new(),
                        )),
                        ChangeKind::Removed => {
                            self.graph_delta.removed_nodes.push(graph_node_delta(
                                node_id,
                                change.old.as_ref(),
                                Vec::new(),
                            ));
                        }
                        ChangeKind::Changed => {
                            let fields = meaningful_graph_fields(
                                change.old.as_ref(),
                                change.new.as_ref(),
                                &mut self.graph_delta.collapsed_attribute_changes,
                            );
                            if !fields.is_empty() {
                                self.graph_delta.changed_nodes.push(graph_node_delta(
                                    node_id,
                                    change.new.as_ref().or(change.old.as_ref()),
                                    fields,
                                ));
                            }
                        }
                    }
                }
            }
            RecordKind::Edge => {
                let Some((source, target, relation)) = edge_key(&change) else {
                    return Ok(());
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
                            change.old.as_ref(),
                            change.new.as_ref(),
                            &mut self.graph_delta.collapsed_attribute_changes,
                        );
                        if !fields.is_empty() {
                            self.graph_delta.changed_edges.push(graph_edge_delta(
                                source, target, relation, key, value, fields,
                            ));
                        }
                        return Ok(());
                    }
                }
                if !matches!(
                    relation,
                    "calls" | "imports" | "imports_from" | "depends_on" | "uses" | "references"
                ) {
                    return Ok(());
                }
                let source_file = value
                    .and_then(|value| value.get("source_file"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
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
                        source_file,
                        start_byte: None,
                        end_byte: None,
                        record_key: Some(change.key.join("/")),
                        capability: "dependencies".to_owned(),
                    }],
                });
            }
            _ => {}
        }
        Ok(())
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
        label: graph_string(value, "label"),
        kind: graph_string(value, "kind"),
        source_file: graph_string(value, "source_file"),
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
        source_file: graph_string(value, "source_file"),
        changed_fields,
    }
}

fn graph_string(value: Option<&serde_json::Value>, key: &str) -> String {
    value
        .and_then(|value| value.get(key))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn meaningful_graph_fields(
    old: Option<&serde_json::Value>,
    new: Option<&serde_json::Value>,
    collapsed: &mut std::collections::BTreeMap<String, usize>,
) -> Vec<String> {
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
        if is_graph_churn_field(&key) {
            *collapsed.entry(key).or_default() += 1;
        } else {
            meaningful.push(key);
        }
    }
    meaningful
}

fn is_graph_churn_field(field: &str) -> bool {
    matches!(
        field,
        "community"
            | "line_start"
            | "line_end"
            | "source_location"
            | "source_hash"
            | "start_byte"
            | "end_byte"
            | "column"
    )
}

enum CommandError {
    Usage(String),
    Runtime(String),
}

fn runtime(error: impl ToString) -> CommandError {
    CommandError::Runtime(error.to_string())
}

fn evidence_error(error: HistoryError) -> SemanticDiffError {
    SemanticDiffError::Evidence(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_diff_options_are_strict() {
        let fingerprint = "a".repeat(64);
        let parsed = parse(&[
            "old".to_owned(),
            "new".to_owned(),
            "--format=json".to_owned(),
            "--all".to_owned(),
            "--explain=sd1-0123456789abcdef01234567".to_owned(),
            format!("--fingerprint={fingerprint}"),
        ]);
        let Ok(parsed) = parsed else {
            assert!(parsed.is_ok());
            return;
        };
        assert_eq!(parsed.format, Format::Json);
        assert!(parsed.all);
        assert_eq!(parsed.limit, None);
        assert_eq!(
            parsed.explain.as_deref(),
            Some("sd1-0123456789abcdef01234567")
        );
        assert_eq!(parsed.fingerprint.as_deref(), Some(fingerprint.as_str()));
        assert_eq!(parsed.output, None);

        let html = parse(&[
            "old".to_owned(),
            "new".to_owned(),
            "--format=html".to_owned(),
            "--output=review.html".to_owned(),
        ])
        .expect("HTML output options");
        assert_eq!(html.format, Format::Html);
        assert_eq!(html.output.as_deref(), Some("review.html"));

        assert!(parse(&["old".to_owned()]).is_err());
        assert!(
            parse(&[
                "old".to_owned(),
                "new".to_owned(),
                "--format=yaml".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "old".to_owned(),
                "new".to_owned(),
                "--all".to_owned(),
                "--limit=10".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "old".to_owned(),
                "new".to_owned(),
                "--format=html".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "old".to_owned(),
                "new".to_owned(),
                "--output=review.html".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn dependency_attribute_churn_is_not_reported_as_a_semantic_change() {
        let delta = |change| DependencyDelta {
            source: "caller".to_owned(),
            target: "target".to_owned(),
            relation: "calls".to_owned(),
            change,
            evidence: Vec::new(),
        };
        let mut changes = DirectChanges {
            nodes: Vec::new(),
            dependencies: vec![
                delta(ChangeDirection::Removed),
                delta(ChangeDirection::Added),
                DependencyDelta {
                    target: "new-target".to_owned(),
                    ..delta(ChangeDirection::Added)
                },
            ],
            graph_delta: GraphDelta::default(),
        };
        changes.cancel_attribute_only_dependency_churn();
        assert_eq!(changes.dependencies.len(), 1);
        assert_eq!(changes.dependencies[0].target, "new-target");
    }

    #[test]
    fn graph_edge_identity_churn_preserves_only_net_multigraph_multiplicity() {
        let edge = |key: &str| GraphEdgeDelta {
            source: "caller".to_owned(),
            target: "target".to_owned(),
            relation: "calls".to_owned(),
            key: key.to_owned(),
            source_file: "example.rs".to_owned(),
            changed_fields: Vec::new(),
        };
        let mut changes = DirectChanges {
            nodes: Vec::new(),
            dependencies: Vec::new(),
            graph_delta: GraphDelta {
                added_edges: vec![edge("new-1"), edge("new-2")],
                removed_edges: vec![edge("old-1")],
                ..GraphDelta::default()
            },
        };
        changes.normalize_graph_delta();
        assert_eq!(changes.graph_delta.added_edges.len(), 1);
        assert!(changes.graph_delta.removed_edges.is_empty());
        assert_eq!(
            changes
                .graph_delta
                .collapsed_attribute_changes
                .get("edge_identity"),
            Some(&1)
        );
    }
}
