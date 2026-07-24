use compass_analysis::FunctionSummary;
use compass_history::{
    ChangeKind, ChangeSink, ExtractionFingerprint, GraphChange, HistoryError, HistoryRecord,
    HistoryRecordKey, HistoryStore, RealizationId, RecordKind, Repository,
};
use compass_ir::{FunctionIr, ModuleIr};
use compass_model::NodeRecord;
use compass_semantic_diff::{
    ChangeDirection, DependencyDelta, EvidenceRef, SemanticDiffError, SemanticDiffInput,
    SnapshotIdentity, SnapshotReader, SnapshotSide, StaticTestEvidence, compare,
};

use crate::semantic_diff_render::{RenderOptions, render_json, render_text};
use crate::{Frontend, Outcome, history_commands};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Text,
    Json,
}

struct Options {
    old: String,
    new: String,
    format: Format,
    all: bool,
    explain: Option<String>,
    fingerprint: Option<String>,
}

pub(crate) fn help(frontend: Frontend) -> String {
    let command = if frontend == Frontend::Compass {
        "compass"
    } else {
        "graphify"
    };
    format!(
        "Usage: {command} diff <OLD> <NEW> [OPTIONS]\n\nOptions:\n  --format <text|json>       Output format [default: text]\n  --all                      Include routine symbol churn\n  --explain <FINDING_ID>     Expand one semantic finding\n  --fingerprint <SHA256>     Select one extraction fingerprint"
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
        snapshots: &snapshots,
        test_evidence: &test_evidence,
    })
    .map_err(runtime)?;
    let render = RenderOptions {
        include_routine: options.all,
        explain: options.explain.as_deref(),
    };
    match options.format {
        Format::Text => render_text(&report, &render).map_err(runtime),
        Format::Json => render_json(&report, &render).map_err(runtime),
    }
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut revisions = Vec::new();
    let mut format = Format::Text;
    let mut format_set = false;
    let mut all = false;
    let mut explain = None;
    let mut fingerprint = None;
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
        explain,
        fingerprint,
    })
}

fn parse_format(value: &str) -> Result<Format, String> {
    match value {
        "text" => Ok(Format::Text),
        "json" => Ok(Format::Json),
        _ => Err("--format must be text or json".to_owned()),
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
}

impl ChangeSink for DirectChanges {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        match change.record {
            RecordKind::Node => {
                if let Some(node_id) = change.key.first() {
                    self.nodes.push(node_id.clone());
                }
            }
            RecordKind::Edge if change.change != ChangeKind::Changed => {
                let Some((source, target, relation)) = edge_key(&change) else {
                    return Ok(());
                };
                if !matches!(
                    relation,
                    "calls" | "imports" | "imports_from" | "depends_on" | "uses" | "references"
                ) {
                    return Ok(());
                }
                let value = change.new.as_ref().or(change.old.as_ref());
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
        assert_eq!(
            parsed.explain.as_deref(),
            Some("sd1-0123456789abcdef01234567")
        );
        assert_eq!(parsed.fingerprint.as_deref(), Some(fingerprint.as_str()));

        assert!(parse(&["old".to_owned()]).is_err());
        assert!(
            parse(&[
                "old".to_owned(),
                "new".to_owned(),
                "--format=yaml".to_owned(),
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
        };
        changes.cancel_attribute_only_dependency_churn();
        assert_eq!(changes.dependencies.len(), 1);
        assert_eq!(changes.dependencies[0].target, "new-target");
    }
}
