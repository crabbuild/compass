use std::path::PathBuf;

use compass_analysis::FunctionSummary;
use compass_history::{
    ChangeKind, ChangeSink, DerivedCacheNamespace, ExtractionFingerprint, GraphChange,
    HistoryError, HistoryRecord, HistoryRecordKey, RealizationReader, RecordKind, Repository,
    canonical_json_bytes,
};
use compass_ir::{FunctionIr, ModuleIr};
use compass_model::NodeRecord;
use compass_semantic_diff::{
    ChangeDirection, DependencyDelta, EvidenceRef, GraphDelta, GraphEdgeDelta, GraphNodeDelta,
    SemanticDiffError, SemanticDiffInput, SemanticDiffReport, SnapshotIdentity, SnapshotReader,
    SnapshotSide, StaticTestEvidence, compare,
};
use sha2::{Digest, Sha256};

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

struct CommandOutput {
    message: String,
    html_output: Option<PathBuf>,
}

impl CommandOutput {
    fn text(message: String) -> Self {
        Self {
            message,
            html_output: None,
        }
    }
}

pub(crate) fn help(_frontend: Frontend) -> String {
    let command = "compass";
    let browser_note =
        "\n\nWhen run interactively, Compass asks before opening generated HTML in your browser.";
    format!(
        "Usage: {command} diff <OLD> <NEW> [OPTIONS]\n\nOptions:\n  --format <text|json|html>  Output format [default: text]\n  --output <PATH>            Write the self-contained HTML report (required with --format html)\n  --limit <N>                Show at most N findings per text section [default: 20]\n  --all                      Include routine churn and show every finding\n  --explain <FINDING_ID>     Expand one semantic finding\n  --fingerprint <SHA256>     Select one extraction fingerprint{browser_note}"
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
        Ok(output) => {
            let outcome = Outcome::success(output.message);
            match output.html_output {
                Some(path) => outcome.with_html_output(path),
                None => outcome,
            }
        }
        Err(CommandError::Usage(message)) => {
            Outcome::failure_with_code(format!("error: {message}"), 2)
        }
        Err(CommandError::Runtime(message)) => Outcome::failure(format!("error: {message}")),
    }
}

fn execute(args: &[String]) -> Result<CommandOutput, CommandError> {
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
    let source_delta_bytes =
        canonical_json_bytes(&serde_json::to_value(&source_deltas).map_err(runtime)?)
            .map_err(runtime)?;
    let cache_key = serde_json::json!({
        "schema": "compass.semantic_diff.cache_key/1",
        "engine_version": compass_semantic_diff::ENGINE_VERSION,
        "old_realization": resolved.old.id.to_string(),
        "new_realization": resolved.new.id.to_string(),
        "source_delta_sha256": format!("{:x}", Sha256::digest(&source_delta_bytes)),
    });
    let cache = resolved.history.cache().ok();
    let cached_report = cache
        .as_ref()
        .and_then(|cache| {
            cache
                .read(
                    DerivedCacheNamespace::SemanticDiff,
                    &cache_key,
                    128 * 1024 * 1024,
                )
                .ok()
                .flatten()
        })
        .and_then(|bytes| serde_json::from_slice::<SemanticDiffReport>(&bytes).ok());
    let report = match cached_report {
        Some(report) => report,
        None => {
            let mut direct = DirectChanges::default();
            let old_reader = resolved.history.reader(&resolved.old.id).map_err(runtime)?;
            let new_reader = resolved.history.reader(&resolved.new.id).map_err(runtime)?;
            old_reader
                .diff_records(
                    &new_reader,
                    &[RecordKind::Node, RecordKind::Edge],
                    &mut direct,
                )
                .map_err(runtime)?;
            direct.cancel_attribute_only_dependency_churn();
            direct.normalize_graph_delta().map_err(runtime)?;
            let snapshots = HistorySnapshots {
                old: old_reader,
                new: new_reader,
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
            let payload = canonical_json_bytes(&serde_json::to_value(&report).map_err(runtime)?)
                .map_err(runtime)?;
            if let Some(cache) = &cache {
                let _ = cache.write(DerivedCacheNamespace::SemanticDiff, &cache_key, &payload);
            }
            serde_json::from_slice(&payload).map_err(runtime)?
        }
    };
    let render = RenderOptions {
        include_routine: options.all,
        max_findings_per_section: options.limit.or((!options.all).then_some(20)),
        explain: options.explain.as_deref(),
    };
    match options.format {
        Format::Text => render_text(&report, &render)
            .map(CommandOutput::text)
            .map_err(runtime),
        Format::Json => render_json(&report, &render)
            .map(CommandOutput::text)
            .map_err(runtime),
        Format::Html => {
            let html = render_html(&report, &render).map_err(runtime)?;
            let output = options
                .output
                .as_deref()
                .ok_or_else(|| CommandError::Usage("--output is required".to_owned()))?;
            std::fs::write(output, html).map_err(runtime)?;
            Ok(CommandOutput {
                message: format!("semantic diff HTML written to {output}"),
                html_output: Some(PathBuf::from(output)),
            })
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
    edge_identity_changes: Vec<EdgeIdentityChange>,
}

struct EdgeIdentityChange {
    change: ChangeKind,
    delta: GraphEdgeDelta,
    value: serde_json::Value,
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

    fn normalize_graph_delta(&mut self) -> Result<(), HistoryError> {
        self.normalize_edge_identity_changes()?;
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
        Ok(())
    }

    fn normalize_edge_identity_changes(&mut self) -> Result<(), HistoryError> {
        type Topology = (String, String, String);
        type ProjectionGroups = std::collections::BTreeMap<Vec<u8>, Vec<EdgeIdentityChange>>;
        let mut groups =
            std::collections::BTreeMap::<Topology, (ProjectionGroups, ProjectionGroups)>::new();
        for change in self.edge_identity_changes.drain(..) {
            let topology = (
                change.delta.source.clone(),
                change.delta.relation.clone(),
                change.delta.target.clone(),
            );
            let projected =
                compass_history::structural_graph_projection(RecordKind::Edge, &change.value);
            let projection = canonical_json_bytes(&projected)?;
            let projections = groups.entry(topology).or_default();
            let target = if change.change == ChangeKind::Added {
                &mut projections.0
            } else {
                &mut projections.1
            };
            target.entry(projection).or_default().push(change);
        }
        for (mut added, mut removed) in groups.into_values() {
            let projections = added
                .keys()
                .chain(removed.keys())
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let mut unmatched_added = Vec::new();
            let mut unmatched_removed = Vec::new();
            for projection in projections {
                let mut added_records = added.remove(&projection).unwrap_or_default();
                let mut removed_records = removed.remove(&projection).unwrap_or_default();
                added_records.sort_by(|left, right| left.delta.key.cmp(&right.delta.key));
                removed_records.sort_by(|left, right| left.delta.key.cmp(&right.delta.key));
                let unchanged = added_records.len().min(removed_records.len());
                if unchanged > 0 {
                    *self
                        .graph_delta
                        .collapsed_attribute_changes
                        .entry("edge_identity".to_owned())
                        .or_default() += unchanged;
                }
                unmatched_added.extend(added_records.into_iter().skip(unchanged));
                unmatched_removed.extend(removed_records.into_iter().skip(unchanged));
            }
            unmatched_added.sort_by(|left, right| left.delta.key.cmp(&right.delta.key));
            unmatched_removed.sort_by(|left, right| left.delta.key.cmp(&right.delta.key));
            let changed = usize::from(unmatched_added.len() == 1 && unmatched_removed.len() == 1);
            for (added, removed) in unmatched_added
                .iter()
                .take(changed)
                .zip(unmatched_removed.iter().take(changed))
            {
                let fields = meaningful_graph_fields(
                    RecordKind::Edge,
                    Some(&removed.value),
                    Some(&added.value),
                    &mut self.graph_delta.collapsed_attribute_changes,
                );
                let mut delta = added.delta.clone();
                delta.changed_fields = fields;
                self.graph_delta.changed_edges.push(delta);
            }
            self.graph_delta.added_edges.extend(
                unmatched_added
                    .into_iter()
                    .skip(changed)
                    .map(|change| change.delta),
            );
            self.graph_delta.removed_edges.extend(
                unmatched_removed
                    .into_iter()
                    .skip(changed)
                    .map(|change| change.delta),
            );
        }
        Ok(())
    }
}

impl ChangeSink for DirectChanges {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        match change.record {
            RecordKind::Node => {
                if let Some(node_id) = change.key.first() {
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
            }
            RecordKind::Edge => {
                let Some((source, target, relation)) = edge_key(&change) else {
                    return Ok(());
                };
                let key = change.key.get(3).map(String::as_str).unwrap_or_default();
                let value = change.new.as_ref().or(change.old.as_ref());
                match change.change {
                    ChangeKind::Added | ChangeKind::Removed => {
                        let Some(value) = value.cloned() else {
                            return Err(HistoryError::InvalidArtifacts(
                                "edge identity change has no record value".to_owned(),
                            ));
                        };
                        self.edge_identity_changes.push(EdgeIdentityChange {
                            change: change.change,
                            delta: graph_edge_delta(
                                source,
                                target,
                                relation,
                                key,
                                Some(&value),
                                Vec::new(),
                            ),
                            value,
                        });
                    }
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
                        return Ok(());
                    }
                }
                if !matches!(
                    relation,
                    "calls" | "imports" | "imports_from" | "depends_on" | "uses" | "references"
                ) {
                    return Ok(());
                }
                let source_file = value.map(graph_edge_source_file).unwrap_or_default();
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
        ]);
        let Ok(html) = html else {
            assert!(html.is_ok());
            return;
        };
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
    fn typed_graph_records_decode_into_flat_semantic_deltas() {
        let node = serde_json::json!({
            "id": "node",
            "kind": "function",
            "name": "serve",
            "qualifiedName": "api::serve",
            "source": {
                "file": "src/api.rs",
                "startByte": 10,
                "endByte": 20,
                "startLine": 2,
                "startColumn": 0,
                "endLine": 2,
                "endColumn": 10
            }
        });
        let edge = serde_json::json!({
            "source": "route",
            "target": "node",
            "kind": "routes_to",
            "relationshipSite": {
                "file": "src/routes.rs",
                "startByte": 1,
                "endByte": 5,
                "startLine": 1,
                "startColumn": 1,
                "endLine": 1,
                "endColumn": 5
            }
        });

        let node_delta = graph_node_delta("node", Some(&node), Vec::new());
        assert_eq!(node_delta.label, "serve");
        assert_eq!(node_delta.kind, "function");
        assert_eq!(node_delta.source_file, "src/api.rs");
        let edge_delta = graph_edge_delta(
            "route",
            "node",
            "routes_to",
            "edge",
            Some(&edge),
            Vec::new(),
        );
        assert_eq!(edge_delta.source_file, "src/routes.rs");
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
            edge_identity_changes: Vec::new(),
        };
        changes.cancel_attribute_only_dependency_churn();
        assert_eq!(changes.dependencies.len(), 1);
        assert_eq!(changes.dependencies[0].target, "new-target");
    }

    #[test]
    fn graph_edge_identity_churn_preserves_only_net_multigraph_multiplicity()
    -> Result<(), HistoryError> {
        let mut changes = DirectChanges::default();
        for (change, key) in [
            (ChangeKind::Removed, "old-1"),
            (ChangeKind::Added, "new-1"),
            (ChangeKind::Added, "new-2"),
        ] {
            let value = serde_json::json!({
                "source": "caller",
                "target": "target",
                "relation": "calls",
                "confidence": "extracted",
                "relationshipSite": {"file": "example.rs", "startLine": 1}
            });
            changes.change(GraphChange {
                record: RecordKind::Edge,
                change,
                key: vec![
                    "caller".to_owned(),
                    "target".to_owned(),
                    "calls".to_owned(),
                    key.to_owned(),
                ],
                old: (change == ChangeKind::Removed).then_some(value.clone()),
                new: (change == ChangeKind::Added).then_some(value),
            })?;
        }
        changes.normalize_graph_delta()?;
        assert_eq!(changes.graph_delta.added_edges.len(), 1);
        assert!(changes.graph_delta.removed_edges.is_empty());
        assert_eq!(
            changes
                .graph_delta
                .collapsed_attribute_changes
                .get("edge_identity"),
            Some(&1)
        );
        Ok(())
    }

    #[test]
    fn edge_attribute_change_survives_anchor_derived_identity_churn() -> Result<(), HistoryError> {
        let mut changes = DirectChanges::default();
        for (change, key, confidence, line) in [
            (ChangeKind::Removed, "old", "inferred", 10),
            (ChangeKind::Added, "new", "extracted", 11),
        ] {
            let value = serde_json::json!({
                "source": "caller",
                "target": "target",
                "relation": "calls",
                "confidence": confidence,
                "relationshipSite": {"file": "example.rs", "startLine": line}
            });
            changes.change(GraphChange {
                record: RecordKind::Edge,
                change,
                key: vec![
                    "caller".to_owned(),
                    "target".to_owned(),
                    "calls".to_owned(),
                    key.to_owned(),
                ],
                old: (change == ChangeKind::Removed).then_some(value.clone()),
                new: (change == ChangeKind::Added).then_some(value),
            })?;
        }
        changes.normalize_graph_delta()?;

        assert!(changes.graph_delta.added_edges.is_empty());
        assert!(changes.graph_delta.removed_edges.is_empty());
        assert_eq!(changes.graph_delta.changed_edges.len(), 1);
        assert_eq!(
            changes.graph_delta.changed_edges[0].changed_fields,
            ["confidence"]
        );
        Ok(())
    }

    #[test]
    fn nested_source_coordinate_churn_does_not_mark_a_node_changed() -> Result<(), HistoryError> {
        let mut changes = DirectChanges::default();
        changes.change(GraphChange {
            record: RecordKind::Node,
            change: ChangeKind::Changed,
            key: vec!["api::serve".to_owned()],
            old: Some(serde_json::json!({
                "id": "api::serve",
                "kind": "function",
                "name": "serve",
                "source": {"file": "src/api.rs", "startLine": 10, "endLine": 12}
            })),
            new: Some(serde_json::json!({
                "id": "api::serve",
                "kind": "function",
                "name": "serve",
                "source": {"file": "src/api.rs", "startLine": 11, "endLine": 13}
            })),
        })?;

        assert!(changes.nodes.is_empty());
        assert!(changes.graph_delta.changed_nodes.is_empty());
        assert_eq!(
            changes
                .graph_delta
                .collapsed_attribute_changes
                .get("source"),
            Some(&1)
        );
        Ok(())
    }
}
