use std::io::Write;

use compass_history::{
    ArtifactClass, ChangeKind, ChangeSink, GraphChange, HistoryError, HistoryStore, RealizationId,
    RecordKind, Repository,
};

use crate::{Frontend, Outcome};

pub(crate) fn help(frontend: Frontend) -> String {
    let prefix = if frontend == Frontend::Compass {
        "compass"
    } else {
        "graphify"
    };
    format!(
        "Usage: {prefix} history <command>\n\nCommands:\n  enable [build-profile options]\n  disable\n  status [REV] [--format text|json]\n  build REV [build-profile options] [--format text|json]\n  rebuild REV [--replace-corrupt] [--format text|json]\n  list [REV] [--format text|json]\n  show REALIZATION [--format text|json]\n  prefer REV REALIZATION [--format text|json]\n  export REV --format graph-json|graphify-out --output PATH\n  gc [--prune-non-preferred] [--yes] [--format text|json]"
    )
}

pub(crate) fn command(frontend: Frontend, args: &[String]) -> Outcome {
    if args.is_empty()
        || args
            .iter()
            .any(|arg| matches!(arg.as_str(), "-h" | "--help"))
    {
        return Outcome::success(help(frontend));
    }
    outcome(execute(frontend, args))
}

pub(crate) fn diff_help(frontend: Frontend) -> String {
    let prefix = match frontend {
        Frontend::Compass => "compass",
        Frontend::Graphify => "graphify",
    };
    format!("Usage: {prefix} diff OLD NEW [--detailed|--format text|json] [--topology-only]")
}

pub(crate) fn command_diff(frontend: Frontend, args: &[String]) -> Outcome {
    let mut bytes = Vec::new();
    let result = command_diff_to_writer(frontend, args, &mut bytes);
    if result.code != 0 {
        return result;
    }
    match String::from_utf8(bytes) {
        Ok(stdout) => Outcome::success_exact(stdout),
        Err(error) => Outcome::failure(format!("error: diff output was not UTF-8: {error}")),
    }
}

pub(crate) fn command_diff_to_writer(
    frontend: Frontend,
    args: &[String],
    writer: &mut dyn Write,
) -> Outcome {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return match writeln!(writer, "{}", diff_help(frontend)) {
            Ok(()) => Outcome::success_exact(String::new()),
            Err(error) => outcome(Err(runtime(output_error(error)))),
        };
    }
    outcome(execute_diff(frontend, args, writer).map(|()| String::new()))
}

fn outcome(result: Result<String, CommandFailure>) -> Outcome {
    match result {
        Ok(text) => Outcome::success(text),
        Err(CommandFailure {
            code,
            message,
            stdout: Some(stdout),
        }) => Outcome {
            code,
            stdout,
            stderr: format!("error: {message}"),
            stdout_trailing_newline: true,
            stderr_trailing_newline: true,
        },
        Err(error) if error.code == 2 => {
            Outcome::failure_with_code(format!("error: {}", error.message), 2)
        }
        Err(error) => Outcome::failure(format!("error: {}", error.message)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffOutput {
    Summary,
    Detailed,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DiffOptions {
    output: DiffOutput,
    topology_only: bool,
}

fn execute_diff(
    frontend: Frontend,
    args: &[String],
    writer: &mut dyn Write,
) -> Result<(), CommandFailure> {
    let (revisions, options) = parse_diff(args).map_err(usage)?;
    let repository =
        Repository::discover(&std::env::current_dir().map_err(runtime)?).map_err(runtime)?;
    let old_commit = repository.resolve(&revisions[0]).map_err(runtime)?;
    let new_commit = repository.resolve(&revisions[1]).map_err(runtime)?;
    let history = store(&repository)?;
    let old = history
        .preferred(&old_commit)
        .map_err(runtime)?
        .ok_or_else(|| missing_preferred(frontend, &revisions[0], &old_commit))?;
    let new = history
        .preferred(&new_commit)
        .map_err(runtime)?
        .ok_or_else(|| missing_preferred(frontend, &revisions[1], &new_commit))?;
    render_diff(&history, &old.id, &new.id, options, writer).map_err(runtime)
}

fn missing_preferred(
    frontend: Frontend,
    revision: &str,
    commit: &impl std::fmt::Display,
) -> CommandFailure {
    let prefix = match frontend {
        Frontend::Compass => "compass",
        Frontend::Graphify => "graphify",
    };
    runtime(format!(
        "commit {commit} ({revision}) has no preferred graph realization; run `{prefix} history build {revision}`"
    ))
}

fn parse_diff(args: &[String]) -> Result<(Vec<String>, DiffOptions), String> {
    let mut revisions = Vec::new();
    let mut format = None;
    let mut detailed = false;
    let mut topology_only = false;
    let mut options = true;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--" if options => options = false,
            "--detailed" if options => {
                if detailed {
                    return Err("duplicate --detailed".to_owned());
                }
                detailed = true;
            }
            "--topology-only" if options => {
                if topology_only {
                    return Err("duplicate --topology-only".to_owned());
                }
                topology_only = true;
            }
            "--format" if options => {
                index += 1;
                let value = args.get(index).ok_or("--format requires a value")?;
                if format.replace(value.clone()).is_some() {
                    return Err("duplicate --format".to_owned());
                }
            }
            value if options && value.starts_with("--format=") => {
                let value = &value[9..];
                if value.is_empty() {
                    return Err("--format requires a value".to_owned());
                }
                if format.replace(value.to_owned()).is_some() {
                    return Err("duplicate --format".to_owned());
                }
            }
            value if options && value.starts_with('-') => {
                return Err(format!("unknown option {value}"));
            }
            value => revisions.push(value.to_owned()),
        }
        index += 1;
    }
    if revisions.len() != 2 {
        return Err("diff requires exactly OLD and NEW revisions".to_owned());
    }
    let format = format.unwrap_or_else(|| "text".to_owned());
    if !matches!(format.as_str(), "text" | "json") {
        return Err("--format must be text or json".to_owned());
    }
    if detailed && format == "json" {
        return Err("--detailed cannot be combined with --format json".to_owned());
    }
    Ok((
        revisions,
        DiffOptions {
            output: if format == "json" {
                DiffOutput::Json
            } else if detailed {
                DiffOutput::Detailed
            } else {
                DiffOutput::Summary
            },
            topology_only,
        },
    ))
}

fn render_diff(
    history: &HistoryStore,
    old: &RealizationId,
    new: &RealizationId,
    options: DiffOptions,
    writer: &mut dyn Write,
) -> Result<(), HistoryError> {
    match options.output {
        DiffOutput::Summary => {
            let mut sink = SummarySink::new(options.topology_only);
            history.diff(old, new, &mut sink)?;
            sink.render(writer)
        }
        DiffOutput::Detailed => {
            let mut sink = DetailedSink::new(writer, options.topology_only);
            history.diff(old, new, &mut sink)?;
            sink.finish()
        }
        DiffOutput::Json => {
            writer.write_all(b"[").map_err(output_error)?;
            let mut sink = JsonSink::new(writer, options.topology_only);
            history.diff(old, new, &mut sink)?;
            sink.finish()
        }
    }
}

#[derive(Default)]
struct SummaryCell {
    count: u64,
    examples: Vec<String>,
}

struct SummarySink {
    cells: Vec<SummaryCell>,
    topology_only: bool,
}

impl SummarySink {
    fn new(topology_only: bool) -> Self {
        Self {
            cells: (0..15).map(|_| SummaryCell::default()).collect(),
            topology_only,
        }
    }

    fn render(&self, writer: &mut dyn Write) -> Result<(), HistoryError> {
        if self.cells.iter().all(|cell| cell.count == 0) {
            return writer
                .write_all(b"no graph changes\n")
                .map_err(output_error);
        }
        for record in RECORD_ORDER {
            for change in CHANGE_ORDER {
                let cell = &self.cells[summary_index(record, change)];
                if cell.count == 0 {
                    continue;
                }
                writeln!(
                    writer,
                    "{} {} {}",
                    cell.count,
                    record_name(record, cell.count),
                    change_name(change)
                )
                .map_err(output_error)?;
                for example in &cell.examples {
                    writeln!(writer, "  {example}").map_err(output_error)?;
                }
            }
        }
        Ok(())
    }
}

impl ChangeSink for SummarySink {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        if excluded(change.record, self.topology_only) {
            return Ok(());
        }
        let cell = &mut self.cells[summary_index(change.record, change.change)];
        cell.count = cell.count.saturating_add(1);
        if cell.examples.len() < 20 {
            cell.examples.push(change.key.join("/"));
        }
        Ok(())
    }
}

struct DetailedSink<'a> {
    writer: &'a mut dyn Write,
    topology_only: bool,
    count: u64,
}

impl<'a> DetailedSink<'a> {
    fn new(writer: &'a mut dyn Write, topology_only: bool) -> Self {
        Self {
            writer,
            topology_only,
            count: 0,
        }
    }

    fn finish(&mut self) -> Result<(), HistoryError> {
        if self.count == 0 {
            self.writer
                .write_all(b"no graph changes\n")
                .map_err(output_error)?;
        }
        Ok(())
    }
}

impl ChangeSink for DetailedSink<'_> {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        if excluded(change.record, self.topology_only) {
            return Ok(());
        }
        write!(
            self.writer,
            "{} {} {}",
            record_name(change.record, 1),
            change_name(change.change),
            change.key.join("/")
        )
        .map_err(output_error)?;
        if let Some(old) = &change.old {
            self.writer.write_all(b"\told=").map_err(output_error)?;
            serde_json::to_writer(&mut *self.writer, old).map_err(json_output_error)?;
        }
        if let Some(new) = &change.new {
            self.writer.write_all(b"\tnew=").map_err(output_error)?;
            serde_json::to_writer(&mut *self.writer, new).map_err(json_output_error)?;
        }
        self.writer.write_all(b"\n").map_err(output_error)?;
        self.count = self.count.saturating_add(1);
        Ok(())
    }
}

struct JsonSink<'a> {
    writer: &'a mut dyn Write,
    topology_only: bool,
    first: bool,
}

impl<'a> JsonSink<'a> {
    fn new(writer: &'a mut dyn Write, topology_only: bool) -> Self {
        Self {
            writer,
            topology_only,
            first: true,
        }
    }

    fn finish(&mut self) -> Result<(), HistoryError> {
        self.writer.write_all(b"]\n").map_err(output_error)
    }
}

impl ChangeSink for JsonSink<'_> {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        if excluded(change.record, self.topology_only) {
            return Ok(());
        }
        if self.first {
            self.first = false;
        } else {
            self.writer.write_all(b",").map_err(output_error)?;
        }
        serde_json::to_writer(&mut *self.writer, &change).map_err(json_output_error)
    }
}

const RECORD_ORDER: [RecordKind; 5] = [
    RecordKind::Node,
    RecordKind::Edge,
    RecordKind::Hyperedge,
    RecordKind::Analysis,
    RecordKind::Metadata,
];
const CHANGE_ORDER: [ChangeKind; 3] = [ChangeKind::Added, ChangeKind::Removed, ChangeKind::Changed];

fn summary_index(record: RecordKind, change: ChangeKind) -> usize {
    let record = match record {
        RecordKind::Node => 0,
        RecordKind::Edge => 1,
        RecordKind::Hyperedge => 2,
        RecordKind::Analysis => 3,
        RecordKind::Metadata => 4,
    };
    let change = match change {
        ChangeKind::Added => 0,
        ChangeKind::Removed => 1,
        ChangeKind::Changed => 2,
    };
    record * 3 + change
}

fn excluded(record: RecordKind, topology_only: bool) -> bool {
    topology_only && matches!(record, RecordKind::Analysis | RecordKind::Metadata)
}

fn record_name(record: RecordKind, count: u64) -> &'static str {
    match (record, count == 1) {
        (RecordKind::Node, true) => "node",
        (RecordKind::Node, false) => "nodes",
        (RecordKind::Edge, true) => "edge",
        (RecordKind::Edge, false) => "edges",
        (RecordKind::Hyperedge, true) => "hyperedge",
        (RecordKind::Hyperedge, false) => "hyperedges",
        (RecordKind::Analysis, true) => "analysis record",
        (RecordKind::Analysis, false) => "analysis records",
        (RecordKind::Metadata, true) => "metadata record",
        (RecordKind::Metadata, false) => "metadata records",
    }
}

fn change_name(change: ChangeKind) -> &'static str {
    match change {
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Changed => "changed",
    }
}

fn output_error(source: std::io::Error) -> HistoryError {
    HistoryError::Io {
        path: std::path::PathBuf::from("<stdout>"),
        source,
    }
}

fn json_output_error(source: serde_json::Error) -> HistoryError {
    if source.is_io() {
        output_error(std::io::Error::other(source))
    } else {
        HistoryError::Json(source)
    }
}

struct CommandFailure {
    code: u8,
    message: String,
    stdout: Option<String>,
}

fn execute(frontend: Frontend, args: &[String]) -> Result<String, CommandFailure> {
    let repository =
        Repository::discover(&std::env::current_dir().map_err(runtime)?).map_err(runtime)?;
    let (positionals, format, output) = parse(&args[1..]).map_err(usage)?;
    if args[0] != "export" && output.is_some() {
        return Err(usage("--output is only valid for history export"));
    }
    if args[0] != "export" && !matches!(format.as_str(), "text" | "json") {
        return Err(usage("--format must be text or json"));
    }
    match args[0].as_str() {
        "status" => {
            one_or_zero(&positionals, "status")?;
            let commit = repository
                .resolve(positionals.first().map(String::as_str).unwrap_or("HEAD"))
                .map_err(runtime)?;
            let history = match HistoryStore::open_existing(&repository) {
                Ok(Some(history)) => history,
                Ok(None) => {
                    return Ok(if format == "json" {
                        serde_json::json!({"enabled":false,"store":false,"commit":commit})
                            .to_string()
                    } else {
                        format!("history: disabled\nstore: no store\ncommit: {commit}")
                    });
                }
                Err(error) => {
                    let report = if format == "json" {
                        serde_json::json!({
                            "enabled":false,
                            "store":true,
                            "compatible":false,
                            "commit":commit,
                            "validation":{"valid":false,"error":error.to_string()}
                        })
                        .to_string()
                    } else {
                        format!(
                            "history: disabled\nstore: incompatible\ncommit: {commit}\nvalidation: invalid"
                        )
                    };
                    return Err(report_failure(report, error));
                }
            };
            let preferred = match history.preferred(&commit) {
                Ok(preferred) => preferred,
                Err(error) => {
                    let report = if format == "json" {
                        serde_json::json!({
                            "enabled":false,
                            "store":true,
                            "commit":commit,
                            "preferred":serde_json::Value::Null,
                            "validation":{"valid":false,"error":error.to_string()}
                        })
                        .to_string()
                    } else {
                        format!(
                            "history: disabled\nstore: present\ncommit: {commit}\npreferred: unreadable\nvalidation: invalid"
                        )
                    };
                    return Err(report_failure(report, error));
                }
            };
            if format == "json" {
                let validation = preferred
                    .as_ref()
                    .map(|value| history.validate(&value.id))
                    .transpose();
                let report = serde_json::json!({
                    "enabled":false,
                    "store":true,
                    "commit":commit,
                    "preferred":preferred.as_ref().map(|v|v.id.as_hex()),
                    "version":preferred.as_ref().map(|v|&v.version),
                    "validation": match &validation {
                        Ok(Some(_)) => serde_json::json!({"valid":true}),
                        Ok(None) => serde_json::Value::Null,
                        Err(error) => serde_json::json!({"valid":false,"error":error.to_string()}),
                    }
                })
                .to_string();
                match validation {
                    Ok(_) => Ok(report),
                    Err(error) => Err(report_failure(report, error)),
                }
            } else if let Some(value) = preferred {
                let prefix = format!(
                    "history: disabled\nstore: present\ncommit: {commit}\npreferred: {}\nfingerprint: {}\nnodes: {}\nedges: {}\nvalidation: valid",
                    value.id,
                    value.version.extraction_fingerprint,
                    value.version.node_count,
                    value.version.edge_count
                );
                match history.validate(&value.id) {
                    Ok(_) => Ok(prefix),
                    Err(error) => Err(report_failure(
                        prefix.replacen("validation: valid", "validation: invalid", 1),
                        error,
                    )),
                }
            } else {
                Ok(format!(
                    "history: disabled\nstore: present\ncommit: {commit}\npreferred: none"
                ))
            }
        }
        "list" => {
            one_or_zero(&positionals, "list")?;
            let commit = positionals
                .first()
                .map(|rev| repository.resolve(rev))
                .transpose()
                .map_err(runtime)?;
            let Some(history) = HistoryStore::open_existing(&repository).map_err(runtime)? else {
                return Ok(if format == "json" { "[]" } else { "" }.to_owned());
            };
            let values = history.list(commit.as_ref()).map_err(runtime)?;
            if format == "json" {
                serde_json::to_string(&values.iter().map(|v|serde_json::json!({"id":v.id,"preferred":v.preferred,"version":v.version})).collect::<Vec<_>>()).map_err(runtime)
            } else {
                Ok(values
                    .into_iter()
                    .map(|v| {
                        format!(
                            "{}\t{}\t{}\t{}",
                            v.version.git_commit,
                            v.id,
                            v.version.extraction_fingerprint,
                            if v.preferred {
                                "preferred"
                            } else {
                                "alternate"
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        "show" => {
            exact(&positionals, 1, "show requires REALIZATION")?;
            let id: RealizationId = positionals[0].parse().map_err(runtime)?;
            let value = store(&repository)?.get(&id).map_err(runtime)?;
            if format == "json" {
                serde_json::to_string(&value.version).map_err(runtime)
            } else {
                Ok(format!(
                    "realization: {}\ncommit: {}\nfingerprint: {}\nnodes: {}\nedges: {}",
                    value.id,
                    value.version.git_commit,
                    value.version.extraction_fingerprint,
                    value.version.node_count,
                    value.version.edge_count
                ))
            }
        }
        "prefer" => {
            exact(&positionals, 2, "prefer requires REV REALIZATION")?;
            let commit = repository.resolve(&positionals[0]).map_err(runtime)?;
            let id: RealizationId = positionals[1].parse().map_err(runtime)?;
            let history = store(&repository)?;
            history.validate(&id).map_err(runtime)?;
            let rebuild_error = |error: &dyn std::fmt::Display| {
                let prefix = match frontend {
                    Frontend::Compass => "compass",
                    Frontend::Graphify => "graphify",
                };
                runtime(format!(
                    "cannot replace an unreadable preferred realization: {error}; run `{prefix} history rebuild {} --replace-corrupt`",
                    positionals[0]
                ))
            };
            let current = history
                .preferred(&commit)
                .map_err(|error| rebuild_error(&error))?;
            if let Some(current) = &current {
                history
                    .validate(&current.id)
                    .map_err(|error| rebuild_error(&error))?;
            }
            if !history
                .compare_and_set_preferred(&commit, current.as_ref().map(|v| &v.id), &id)
                .map_err(runtime)?
            {
                return Err(runtime("preferred realization changed concurrently"));
            }
            Ok(if format == "json" {
                serde_json::json!({"commit":commit,"preferred":id}).to_string()
            } else {
                format!("preferred {id} for {commit}")
            })
        }
        "export" => {
            exact(&positionals, 1, "export requires REV")?;
            let output = output.ok_or_else(|| usage("export requires --output PATH"))?;
            let commit = repository.resolve(&positionals[0]).map_err(runtime)?;
            let history = store(&repository)?;
            let preferred = history
                .preferred(&commit)
                .map_err(runtime)?
                .ok_or_else(|| runtime("no preferred realization"))?;
            let artifacts = history.artifacts(&preferred.id).map_err(runtime)?;
            if format == "graph-json" {
                if output.is_dir() {
                    return Err(runtime("graph-json output must be a file"));
                }
                let value = serde_json::to_value(&artifacts.artifacts.document).map_err(runtime)?;
                let bytes = compass_history::canonical_json_bytes(&value).map_err(runtime)?;
                compass_files::write_bytes_atomic(&output, &bytes).map_err(runtime)?;
            } else if format == "graphify-out" {
                if output.exists() {
                    return Err(runtime("bundle output already exists"));
                }
                let derived = artifacts
                    .artifacts
                    .artifact_registry()
                    .map_err(runtime)?
                    .into_iter()
                    .filter(|entry| entry.class == ArtifactClass::Derived)
                    .map(|entry| {
                        Ok(compass_output::DerivedArtifactRequest {
                            relative_path: entry.relative_path,
                            regeneration_version: entry.regeneration_version.ok_or_else(|| {
                                runtime("derived artifact has no regeneration version")
                            })?,
                        })
                    })
                    .collect::<Result<Vec<_>, CommandFailure>>()?;
                let marker = serde_json::json!({
                    "schema": "compass.history.completion",
                    "schema_version": 1,
                    "extraction_succeeded": artifacts.completion.extraction_succeeded,
                    "allow_partial": artifacts.completion.allow_partial,
                    "semantic_files_expected": artifacts.completion.semantic_files_expected,
                    "semantic_files_completed": artifacts.completion.semantic_files_completed,
                    "failed_chunks": artifacts.completion.failed_chunks
                });
                compass_output::publish_history_bundle(
                    &output,
                    &compass_output::HistoryBundleInput {
                        document: &artifacts.artifacts.document,
                        analysis: artifacts.artifacts.analysis.as_ref(),
                        labels: artifacts.artifacts.labels.as_ref(),
                        manifest: artifacts.artifacts.manifest.as_ref(),
                        authoritative_sidecars: &artifacts.artifacts.authoritative_sidecars,
                        semantic_marker: &marker,
                        derived: &derived,
                    },
                )
                .map_err(runtime)?;
            } else {
                return Err(usage("export --format must be graph-json or graphify-out"));
            }
            Ok(format!("exported {} to {}", preferred.id, output.display()))
        }
        "enable" | "disable" | "build" | "rebuild" | "gc" => {
            Err(runtime(format!("history {} is not available yet", args[0])))
        }
        other => Err(usage(format!("unknown history command {other}"))),
    }
}

fn parse(args: &[String]) -> Result<(Vec<String>, String, Option<std::path::PathBuf>), String> {
    let mut p = Vec::new();
    let mut f = None;
    let mut o = None;
    let mut i = 0;
    let mut options = true;
    while i < args.len() {
        match args[i].as_str() {
            "--" if options => options = false,
            "--format" if options => {
                i += 1;
                let v = args.get(i).ok_or("--format requires a value")?;
                if f.replace(v.clone()).is_some() {
                    return Err("duplicate --format".into());
                }
            }
            "--output" if options => {
                i += 1;
                let v = args.get(i).ok_or("--output requires a path")?;
                if o.replace(v.into()).is_some() {
                    return Err("duplicate --output".into());
                }
            }
            v if options && v.starts_with("--format=") => {
                let value = &v[9..];
                if value.is_empty() {
                    return Err("--format requires a value".to_owned());
                }
                if f.replace(value.to_owned()).is_some() {
                    return Err("duplicate --format".into());
                }
            }
            v if options && v.starts_with("--output=") => {
                let value = &v[9..];
                if value.is_empty() {
                    return Err("--output requires a path".to_owned());
                }
                if o.replace(value.into()).is_some() {
                    return Err("duplicate --output".into());
                }
            }
            v if options && v.starts_with('-') => return Err(format!("unknown option {v}")),
            v => p.push(v.into()),
        }
        i += 1;
    }
    Ok((p, f.unwrap_or_else(|| "text".into()), o))
}
fn store(r: &Repository) -> Result<HistoryStore, CommandFailure> {
    HistoryStore::open_existing(r)
        .map_err(runtime)?
        .ok_or_else(|| runtime("graph history has no store"))
}
fn exact(p: &[String], n: usize, m: &str) -> Result<(), CommandFailure> {
    if p.len() == n { Ok(()) } else { Err(usage(m)) }
}
fn one_or_zero(p: &[String], m: &str) -> Result<(), CommandFailure> {
    if p.len() <= 1 {
        Ok(())
    } else {
        Err(usage(format!("{m} accepts at most one revision")))
    }
}
fn runtime(e: impl ToString) -> CommandFailure {
    CommandFailure {
        code: 1,
        message: e.to_string(),
        stdout: None,
    }
}
fn usage(e: impl ToString) -> CommandFailure {
    CommandFailure {
        code: 2,
        message: e.to_string(),
        stdout: None,
    }
}
fn report_failure(stdout: String, e: impl ToString) -> CommandFailure {
    CommandFailure {
        code: 1,
        message: e.to_string(),
        stdout: Some(stdout),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use serde_json::json;

    use super::*;

    #[test]
    fn common_options_support_equals_end_marker_and_reject_duplicates() {
        let result = parse(&[
            "--format=json".to_owned(),
            "--output=result".to_owned(),
            "--".to_owned(),
            "-revision".to_owned(),
        ]);
        let Ok((positionals, format, output)) = result else {
            assert!(result.is_ok());
            return;
        };
        assert_eq!(positionals, ["-revision"]);
        assert_eq!(format, "json");
        assert_eq!(output.as_deref(), Some(std::path::Path::new("result")));
        assert!(
            parse(&[
                "--format=json".to_owned(),
                "--format".to_owned(),
                "text".to_owned()
            ])
            .is_err()
        );
        assert!(parse(&["--unknown".to_owned()]).is_err());
    }

    #[test]
    fn diff_options_are_total_and_mutually_exclusive() {
        let parsed = parse_diff(&[
            "old".to_owned(),
            "new".to_owned(),
            "--topology-only".to_owned(),
            "--format=json".to_owned(),
        ]);
        let Ok((revisions, options)) = parsed else {
            assert!(parsed.is_ok());
            return;
        };
        assert_eq!(revisions, ["old", "new"]);
        assert_eq!(options.output, DiffOutput::Json);
        assert!(options.topology_only);
        assert!(
            parse_diff(&[
                "old".to_owned(),
                "new".to_owned(),
                "--detailed".to_owned(),
                "--format=json".to_owned(),
            ])
            .is_err()
        );
        assert!(parse_diff(&["old".to_owned()]).is_err());
        assert!(
            parse_diff(&[
                "old".to_owned(),
                "new".to_owned(),
                "--format=yaml".to_owned(),
            ])
            .is_err()
        );
    }

    #[test]
    fn json_diff_sink_handles_short_writes_and_propagates_broken_pipes()
    -> Result<(), Box<dyn std::error::Error>> {
        struct ShortWriter(Vec<u8>);
        impl Write for ShortWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                let length = bytes.len().min(1);
                self.0.extend_from_slice(&bytes[..length]);
                Ok(length)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        struct BrokenWriter;
        impl Write for BrokenWriter {
            fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let change = GraphChange {
            record: RecordKind::Edge,
            change: ChangeKind::Changed,
            key: vec!["edge".to_owned()],
            old: Some(json!({"confidence":0.5})),
            new: Some(json!({"confidence":0.9})),
        };

        let mut short = ShortWriter(Vec::new());
        short.write_all(b"[")?;
        let mut sink = JsonSink::new(&mut short, false);
        sink.change(change.clone())?;
        sink.finish()?;
        let parsed: Vec<serde_json::Value> = serde_json::from_slice(&short.0)?;
        assert_eq!(parsed.len(), 1);

        let mut broken = BrokenWriter;
        let mut sink = JsonSink::new(&mut broken, false);
        assert!(sink.change(change).is_err());
        Ok(())
    }
}
