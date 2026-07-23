use std::fs::File;
use std::io::{Seek, SeekFrom, Write};

use compass_core::LoadedGraph;
use compass_core::{
    MaterializeError, MaterializeObserver, MaterializeRequest, MaterializeStage,
    materialize_history_with_observer,
};
use compass_history::{
    ArtifactClass, BuildProfile, ChangeKind, ChangeSink, ClaimedJob, CommitId,
    ExtractionFingerprint, GitTargetLimitation, GraphChange, HistoryConfig, HistoryError,
    HistoryQueue, HistoryStore, JobRequest, JobState, PublishedVersion, RealizationId, RecordKind,
    Repository,
};

use crate::history_build::{HistoryBuildOptions, parse_build_command, parse_enable_options};
use crate::{Frontend, Outcome};

pub(crate) fn help(frontend: Frontend) -> String {
    let prefix = if frontend == Frontend::Compass {
        "compass"
    } else {
        "graphify"
    };
    format!(
        "Usage: {prefix} history <command>\n\nCommands:\n  enable [build-profile options]\n  disable\n  status [REV] [--format text|json]\n  build REV [build-profile options|--profile-from REV|REALIZATION] [--format text|json]\n  rebuild REV [build-profile options] [--replace-corrupt] [--format text|json]\n  list [REV] [--format text|json]\n  show REALIZATION [--format text|json]\n  prefer REV REALIZATION [--format text|json]\n  export REV --format graph-json|compass-out --output PATH\n  gc [--prune-non-preferred] [--yes] [--format text|json]\n\nBuild-profile options:\n  --code-only              Build a complete local AST/inferred realization without model credentials\n  --backend NAME           Build a semantic realization with the selected provider\n  --model NAME             Select the provider model\n  --exclude PATTERN        Exclude a committed path pattern (repeatable)\n  --cargo                   Include Cargo package metadata"
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

pub(crate) fn command_worker(_frontend: Frontend, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Outcome::failure_with_code(
            "error: history-worker accepts no arguments".to_owned(),
            2,
        );
    }
    outcome(run_worker().map(|()| String::new()))
}

pub(crate) fn diff_help(frontend: Frontend) -> String {
    let prefix = match frontend {
        Frontend::Compass => "compass",
        Frontend::Graphify => "graphify",
    };
    format!(
        "Usage: {prefix} diff OLD NEW [--detailed] [--format text|json] [--topology-only] \
[--include-locations] [--include-analysis] [--include-metadata] [--fingerprint SHA] \
[--allow-profile-mismatch]"
    )
}

pub(crate) fn load_graph_at(
    _frontend: Frontend,
    revision: &str,
    force_directed: bool,
) -> Result<LoadedGraph, String> {
    let repository =
        Repository::discover(&std::env::current_dir().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let commit = repository
        .resolve(revision)
        .map_err(|error| error.to_string())?;
    let options = configured_build_options(&repository)?;
    let (history, preferred) = resolve_or_materialize(&repository, commit, &options, false, false)?;
    let activity = history.activity().map_err(|error| error.to_string())?;
    // `artifacts` performs full realization validation before reconstruction.
    let artifacts = history
        .artifacts_with_activity(&preferred.id, &activity)
        .map_err(|error| error.to_string())?;
    LoadedGraph::from_document(artifacts.artifacts.document, force_directed)
        .map_err(|error| error.to_string())
}

fn resolve_or_materialize(
    repository: &Repository,
    commit: CommitId,
    options: &HistoryBuildOptions,
    rebuild: bool,
    replace_corrupt: bool,
) -> Result<(HistoryStore, PublishedVersion), String> {
    let existing = HistoryStore::open_existing(repository).map_err(|error| error.to_string())?;
    if !rebuild && let Some(history) = existing {
        match history.preferred(&commit) {
            Ok(Some(preferred)) => {
                history
                    .validate(&preferred.id)
                    .map_err(|error| error.to_string())?;
                return Ok((history, preferred));
            }
            Ok(None) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    let history =
        match HistoryStore::open_existing(repository).map_err(|error| error.to_string())? {
            Some(history) => history,
            None => HistoryStore::create(repository).map_err(|error| error.to_string())?,
        };
    let queue = HistoryQueue::for_repository(repository).map_err(|error| error.to_string())?;
    let request = JobRequest {
        commit: commit.clone(),
        profile: options.profile(),
    };
    let job_id = if rebuild {
        queue.enqueue_rebuild(request, replace_corrupt)
    } else {
        queue.enqueue(request)
    }
    .map_err(|error| error.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(150);
    loop {
        if !rebuild {
            match history.preferred(&commit) {
                Ok(Some(preferred)) if history.validate(&preferred.id).is_ok() => {
                    return Ok((history, preferred));
                }
                Ok(_) => {}
                Err(error) if error.is_catalog_corruption() => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        let job = queue
            .get(&job_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "joined history job disappeared".to_owned())?;
        if job.state.terminal() {
            if job.state == JobState::Published {
                if let Some(candidate) = &job.candidate_realization {
                    let mut published =
                        history.get(candidate).map_err(|error| error.to_string())?;
                    published.preferred = job.preferred.unwrap_or(false);
                    return Ok((history, published));
                }
                if let Some(preferred) = history
                    .preferred(&commit)
                    .map_err(|error| error.to_string())?
                {
                    return Ok((history, preferred));
                }
            }
            return Err(job
                .diagnostic
                .unwrap_or_else(|| format!("history materialization ended in {:?}", job.state)));
        }
        if let Some(claimed) = queue
            .claim_or_join(&job_id)
            .map_err(|error| error.to_string())?
        {
            run_claimed_job(repository, &history, &queue, &claimed, true)
                .map_err(|error| error.message)?;
            continue;
        }
        if std::time::Instant::now() >= deadline {
            return Err("timed out joining the live history materialization lease".to_owned());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn configured_build_options(repository: &Repository) -> Result<HistoryBuildOptions, String> {
    let config = HistoryConfig::load(repository).map_err(|error| error.to_string())?;
    if config.enabled
        && let Some(profile) = config.profile
    {
        return HistoryBuildOptions::from_profile(profile).map_err(|error| error.to_string());
    }
    HistoryBuildOptions::defaults().map_err(|error| error.to_string())
}

pub(crate) fn command_diff(frontend: Frontend, args: &[String]) -> Outcome {
    let mut bytes = Vec::new();
    let mut result = command_diff_to_writer(frontend, args, &mut bytes);
    if result.code != 0 {
        return result;
    }
    match String::from_utf8(bytes) {
        Ok(stdout) => {
            result.stdout = stdout;
            result
        }
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
    match execute_diff(frontend, args, writer) {
        Ok(execution) => Outcome {
            code: 0,
            stdout: String::new(),
            stderr: execution.warning.unwrap_or_default(),
            stdout_trailing_newline: false,
            stderr_trailing_newline: true,
        },
        Err(error) => outcome(Err(error)),
    }
}

struct DiffExecution {
    warning: Option<String>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct DiffOptions {
    output: DiffOutput,
    topology_only: bool,
    include_locations: bool,
    include_analysis: bool,
    include_metadata: bool,
    fingerprint: Option<String>,
    allow_profile_mismatch: bool,
}

fn execute_diff(
    _frontend: Frontend,
    args: &[String],
    writer: &mut dyn Write,
) -> Result<DiffExecution, CommandFailure> {
    let (revisions, diff_options) = parse_diff(args).map_err(usage)?;
    let repository =
        Repository::discover(&std::env::current_dir().map_err(runtime)?).map_err(runtime)?;
    let old_commit = repository.resolve(&revisions[0]).map_err(runtime)?;
    let new_commit = repository.resolve(&revisions[1]).map_err(runtime)?;
    let resolved = resolve_comparable_pair(
        &repository,
        old_commit,
        new_commit,
        diff_options.fingerprint.as_deref(),
        diff_options.allow_profile_mismatch,
    )
    .map_err(runtime)?;
    render_diff(
        &resolved.history,
        &resolved.old,
        &resolved.new,
        resolved.profile_mismatch,
        &diff_options,
        writer,
    )
    .map_err(runtime)?;
    Ok(DiffExecution {
        warning: (resolved.profile_mismatch && diff_options.output != DiffOutput::Json).then(|| {
            format!(
                "warning: comparing realizations with different extraction fingerprints ({} != {}); results may reflect profile differences",
                resolved.old.version.extraction_fingerprint,
                resolved.new.version.extraction_fingerprint
            )
        }),
    })
}

struct ResolvedDiff {
    history: HistoryStore,
    old: PublishedVersion,
    new: PublishedVersion,
    profile_mismatch: bool,
}

fn resolve_comparable_pair(
    repository: &Repository,
    old_commit: CommitId,
    new_commit: CommitId,
    required_fingerprint: Option<&str>,
    allow_profile_mismatch: bool,
) -> Result<ResolvedDiff, String> {
    let existing = HistoryStore::open_existing(repository).map_err(|error| error.to_string())?;
    let old = select_existing(existing.as_ref(), &old_commit, required_fingerprint)?;
    let new = select_existing(existing.as_ref(), &new_commit, required_fingerprint)?;
    if required_fingerprint.is_some() && (old.is_none() || new.is_none()) {
        return Err("the requested fingerprint is not materialized at both commits".to_owned());
    }

    let (history, old, new) = match (old, new) {
        (Some(old), Some(new)) => (
            existing.ok_or_else(|| "history store disappeared".to_owned())?,
            old,
            new,
        ),
        (Some(old), None) => {
            let options = HistoryBuildOptions::from_profile(old.version.build_profile.clone())
                .map_err(|error| error.to_string())?;
            let (history, new) =
                resolve_or_materialize(repository, new_commit, &options, false, false)?;
            (history, old, new)
        }
        (None, Some(new)) => {
            let options = HistoryBuildOptions::from_profile(new.version.build_profile.clone())
                .map_err(|error| error.to_string())?;
            let (history, old) =
                resolve_or_materialize(repository, old_commit, &options, false, false)?;
            (history, old, new)
        }
        (None, None) => {
            let options = configured_build_options(repository)?;
            let (_, old) = resolve_or_materialize(repository, old_commit, &options, false, false)?;
            let (history, new) =
                resolve_or_materialize(repository, new_commit, &options, false, false)?;
            (history, old, new)
        }
    };
    let profile_mismatch = old.version.extraction_fingerprint != new.version.extraction_fingerprint;
    if profile_mismatch && !allow_profile_mismatch {
        return Err(format!(
            "realizations are not semantically comparable\n\nOLD {} ({}) fingerprint: {}\nNEW {} ({}) fingerprint: {}\n\nBuild a comparable realization:\n  compass history build {} --profile-from {}\n\nOr inspect intentionally:\n  compass diff {} {} --allow-profile-mismatch",
            old.version.git_commit,
            old.id,
            old.version.extraction_fingerprint,
            new.version.git_commit,
            new.id,
            new.version.extraction_fingerprint,
            new.version.git_commit,
            old.version.git_commit,
            old.version.git_commit,
            new.version.git_commit,
        ));
    }
    Ok(ResolvedDiff {
        history,
        old,
        new,
        profile_mismatch,
    })
}

fn select_existing(
    history: Option<&HistoryStore>,
    commit: &CommitId,
    required_fingerprint: Option<&str>,
) -> Result<Option<PublishedVersion>, String> {
    let Some(history) = history else {
        return Ok(None);
    };
    let selected = if let Some(fingerprint) = required_fingerprint {
        let mut matches = history
            .list(Some(commit))
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|version| version.version.extraction_fingerprint == fingerprint)
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(format!(
                "multiple realizations at {commit} have fingerprint {fingerprint}"
            ));
        }
        matches.pop()
    } else {
        history
            .preferred(commit)
            .map_err(|error| error.to_string())?
    };
    // Publication validates the complete immutable realization before making
    // it visible, while `preferred`/`list` authenticate the manifest and its
    // direct roots. The diff traversal then verifies every Prolly node it
    // actually reads. Re-scanning and reconstructing all five trees here made
    // even a topology-only diff pay the full graph-validation cost twice and
    // defeated structural sharing.
    Ok(selected)
}

fn parse_diff(args: &[String]) -> Result<(Vec<String>, DiffOptions), String> {
    let mut revisions = Vec::new();
    let mut format = None;
    let mut detailed = false;
    let mut topology_only = false;
    let mut include_locations = false;
    let mut include_analysis = false;
    let mut include_metadata = false;
    let mut fingerprint = None;
    let mut allow_profile_mismatch = false;
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
            "--include-locations" if options => {
                if include_locations {
                    return Err("duplicate --include-locations".to_owned());
                }
                include_locations = true;
            }
            "--include-analysis" if options => {
                if include_analysis {
                    return Err("duplicate --include-analysis".to_owned());
                }
                include_analysis = true;
            }
            "--include-metadata" if options => {
                if include_metadata {
                    return Err("duplicate --include-metadata".to_owned());
                }
                include_metadata = true;
            }
            "--allow-profile-mismatch" if options => {
                if allow_profile_mismatch {
                    return Err("duplicate --allow-profile-mismatch".to_owned());
                }
                allow_profile_mismatch = true;
            }
            "--fingerprint" if options => {
                index += 1;
                let value = args.get(index).ok_or("--fingerprint requires a value")?;
                if fingerprint.replace(value.clone()).is_some() {
                    return Err("duplicate --fingerprint".to_owned());
                }
            }
            value if options && value.starts_with("--fingerprint=") => {
                let value = &value[14..];
                if value.is_empty() {
                    return Err("--fingerprint requires a value".to_owned());
                }
                if fingerprint.replace(value.to_owned()).is_some() {
                    return Err("duplicate --fingerprint".to_owned());
                }
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
    if let Some(value) = &fingerprint {
        value
            .parse::<ExtractionFingerprint>()
            .map_err(|_| "--fingerprint must be a lowercase SHA-256 digest".to_owned())?;
    }
    if fingerprint.is_some() && allow_profile_mismatch {
        return Err("--fingerprint cannot be combined with --allow-profile-mismatch".to_owned());
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
            include_locations,
            include_analysis,
            include_metadata,
            fingerprint,
            allow_profile_mismatch,
        },
    ))
}

fn render_diff(
    history: &HistoryStore,
    old: &PublishedVersion,
    new: &PublishedVersion,
    profile_mismatch: bool,
    options: &DiffOptions,
    writer: &mut dyn Write,
) -> Result<(), HistoryError> {
    match options.output {
        DiffOutput::Summary | DiffOutput::Detailed => {
            let mut sink = TextSink::new(options)?;
            stream_diff(history, old, new, options.topology_only, &mut sink)?;
            sink.finish(writer)
        }
        DiffOutput::Json => {
            writer
                .write_all(b"{\"schema_version\":2,\"comparison\":")
                .map_err(output_error)?;
            serde_json::to_writer(
                &mut *writer,
                &serde_json::json!({
                    "old_commit": old.version.git_commit,
                    "new_commit": new.version.git_commit,
                    "old_realization": old.id,
                    "new_realization": new.id,
                    "old_fingerprint": old.version.extraction_fingerprint,
                    "new_fingerprint": new.version.extraction_fingerprint,
                    "profile_mismatch": profile_mismatch,
                }),
            )
            .map_err(json_output_error)?;
            writer.write_all(b",\"changes\":[").map_err(output_error)?;
            let mut sink = JsonSink::new(writer, options.topology_only);
            stream_diff(history, old, new, options.topology_only, &mut sink)?;
            sink.finish()
        }
    }
}

fn stream_diff(
    history: &HistoryStore,
    old: &PublishedVersion,
    new: &PublishedVersion,
    topology_only: bool,
    sink: &mut dyn ChangeSink,
) -> Result<(), HistoryError> {
    if topology_only {
        history.diff_records(
            &old.id,
            &new.id,
            &[RecordKind::Node, RecordKind::Edge],
            sink,
        )
    } else {
        history.diff(&old.id, &new.id, sink)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChangeCategory {
    Semantic,
    Textual,
    Location,
    Analysis,
    Metadata,
}

impl ChangeCategory {
    fn index(self) -> usize {
        match self {
            Self::Semantic => 0,
            Self::Textual => 1,
            Self::Location => 2,
            Self::Analysis => 3,
            Self::Metadata => 4,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Textual => "textual",
            Self::Location => "location",
            Self::Analysis => "analysis",
            Self::Metadata => "metadata",
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Self::Semantic => "Semantic graph changes",
            Self::Textual => "Textual changes",
            Self::Location => "Location changes",
            Self::Analysis => "Analysis changes",
            Self::Metadata => "Metadata changes",
        }
    }
}

const CATEGORY_ORDER: [ChangeCategory; 5] = [
    ChangeCategory::Semantic,
    ChangeCategory::Textual,
    ChangeCategory::Analysis,
    ChangeCategory::Location,
    ChangeCategory::Metadata,
];

struct TextSink {
    counts: [[u64; 15]; 5],
    examples: Vec<Vec<String>>,
    details: Vec<File>,
    output: DiffOutput,
    topology_only: bool,
    include_locations: bool,
    include_analysis: bool,
    include_metadata: bool,
}

impl TextSink {
    fn new(options: &DiffOptions) -> Result<Self, HistoryError> {
        let mut details = Vec::with_capacity(5);
        for _ in 0..5 {
            details.push(tempfile::tempfile().map_err(output_error)?);
        }
        Ok(Self {
            counts: [[0; 15]; 5],
            examples: (0..75).map(|_| Vec::new()).collect(),
            details,
            output: options.output,
            topology_only: options.topology_only,
            include_locations: options.include_locations,
            include_analysis: options.include_analysis,
            include_metadata: options.include_metadata,
        })
    }

    fn expanded(&self, category: ChangeCategory) -> bool {
        match category {
            ChangeCategory::Semantic | ChangeCategory::Textual => {
                self.output == DiffOutput::Detailed
            }
            ChangeCategory::Location => self.include_locations,
            ChangeCategory::Analysis => self.include_analysis,
            ChangeCategory::Metadata => self.include_metadata,
        }
    }

    fn finish(&mut self, writer: &mut dyn Write) -> Result<(), HistoryError> {
        let total = self.counts.iter().flatten().copied().sum::<u64>();
        if total == 0 {
            let message = if self.topology_only {
                b"no topology changes\n".as_slice()
            } else {
                b"no graph changes\n".as_slice()
            };
            return writer.write_all(message).map_err(output_error);
        }
        let mut first_section = true;
        for category in CATEGORY_ORDER {
            let category_counts = &self.counts[category.index()];
            let category_total = category_counts.iter().copied().sum::<u64>();
            if category_total == 0 {
                continue;
            }
            if !first_section {
                writer.write_all(b"\n").map_err(output_error)?;
            }
            first_section = false;
            writeln!(writer, "{}", category.heading()).map_err(output_error)?;
            for record in RECORD_ORDER {
                for change in CHANGE_ORDER {
                    let index = summary_index(record, change);
                    let count = category_counts[index];
                    if count == 0 {
                        continue;
                    }
                    writeln!(
                        writer,
                        "  {count} {} {}",
                        record_name(record, count),
                        change_name(change)
                    )
                    .map_err(output_error)?;
                    if !self.expanded(category)
                        && matches!(category, ChangeCategory::Semantic | ChangeCategory::Textual)
                    {
                        for example in &self.examples[category.index() * 15 + index] {
                            writeln!(writer, "    {example}").map_err(output_error)?;
                        }
                    }
                }
            }
            if self.expanded(category) {
                let detail = &mut self.details[category.index()];
                detail.seek(SeekFrom::Start(0)).map_err(output_error)?;
                std::io::copy(detail, writer).map_err(output_error)?;
            } else if matches!(
                category,
                ChangeCategory::Location | ChangeCategory::Analysis | ChangeCategory::Metadata
            ) {
                writeln!(
                    writer,
                    "  (collapsed; use --include-{} to expand)",
                    match category {
                        ChangeCategory::Location => "locations",
                        ChangeCategory::Analysis => "analysis",
                        ChangeCategory::Metadata => "metadata",
                        _ => unreachable!(),
                    }
                )
                .map_err(output_error)?;
            }
        }
        Ok(())
    }
}

impl ChangeSink for TextSink {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        if topology_excluded(&change, self.topology_only) {
            return Ok(());
        }
        let category = classify_change(&change);
        let index = summary_index(change.record, change.change);
        self.counts[category.index()][index] =
            self.counts[category.index()][index].saturating_add(1);
        let examples = &mut self.examples[category.index() * 15 + index];
        if examples.len() < 20 {
            examples.push(change.key.join("/"));
        }
        if self.expanded(category) {
            write_change_line(&mut self.details[category.index()], &change)?;
        }
        Ok(())
    }
}

fn write_change_line(writer: &mut dyn Write, change: &GraphChange) -> Result<(), HistoryError> {
    write!(
        writer,
        "  {} {} {}",
        record_name(change.record, 1),
        change_name(change.change),
        change.key.join("/")
    )
    .map_err(output_error)?;
    if let Some(old) = &change.old {
        writer.write_all(b"\told=").map_err(output_error)?;
        serde_json::to_writer(&mut *writer, old).map_err(json_output_error)?;
    }
    if let Some(new) = &change.new {
        writer.write_all(b"\tnew=").map_err(output_error)?;
        serde_json::to_writer(&mut *writer, new).map_err(json_output_error)?;
    }
    writer.write_all(b"\n").map_err(output_error)
}

struct JsonSink<'a> {
    writer: &'a mut dyn Write,
    topology_only: bool,
    first: bool,
    summary: [u64; 5],
}

impl<'a> JsonSink<'a> {
    fn new(writer: &'a mut dyn Write, topology_only: bool) -> Self {
        Self {
            writer,
            topology_only,
            first: true,
            summary: [0; 5],
        }
    }

    fn finish(&mut self) -> Result<(), HistoryError> {
        writeln!(
            self.writer,
            "],\"summary\":{{\"semantic\":{},\"textual\":{},\"location\":{},\"analysis\":{},\"metadata\":{}}}}}",
            self.summary[ChangeCategory::Semantic.index()],
            self.summary[ChangeCategory::Textual.index()],
            self.summary[ChangeCategory::Location.index()],
            self.summary[ChangeCategory::Analysis.index()],
            self.summary[ChangeCategory::Metadata.index()],
        )
        .map_err(output_error)
    }
}

impl ChangeSink for JsonSink<'_> {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        if topology_excluded(&change, self.topology_only) {
            return Ok(());
        }
        let category = classify_change(&change);
        self.summary[category.index()] = self.summary[category.index()].saturating_add(1);
        if self.first {
            self.first = false;
        } else {
            self.writer.write_all(b",").map_err(output_error)?;
        }
        self.writer
            .write_all(b"{\"category\":")
            .map_err(output_error)?;
        serde_json::to_writer(&mut *self.writer, category.name()).map_err(json_output_error)?;
        self.writer
            .write_all(b",\"record\":")
            .map_err(output_error)?;
        serde_json::to_writer(&mut *self.writer, &change.record).map_err(json_output_error)?;
        self.writer
            .write_all(b",\"change\":")
            .map_err(output_error)?;
        serde_json::to_writer(&mut *self.writer, &change.change).map_err(json_output_error)?;
        self.writer.write_all(b",\"key\":").map_err(output_error)?;
        serde_json::to_writer(&mut *self.writer, &change.key).map_err(json_output_error)?;
        if let Some(old) = &change.old {
            self.writer.write_all(b",\"old\":").map_err(output_error)?;
            serde_json::to_writer(&mut *self.writer, old).map_err(json_output_error)?;
        }
        if let Some(new) = &change.new {
            self.writer.write_all(b",\"new\":").map_err(output_error)?;
            serde_json::to_writer(&mut *self.writer, new).map_err(json_output_error)?;
        }
        self.writer.write_all(b"}").map_err(output_error)
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

fn classify_change(change: &GraphChange) -> ChangeCategory {
    match change.record {
        RecordKind::Analysis => return ChangeCategory::Analysis,
        RecordKind::Metadata => return ChangeCategory::Metadata,
        RecordKind::Node | RecordKind::Edge | RecordKind::Hyperedge => {}
    }
    if change.change != ChangeKind::Changed {
        return ChangeCategory::Semantic;
    }
    let (Some(old), Some(new)) = (&change.old, &change.new) else {
        return ChangeCategory::Semantic;
    };
    if equal_without_location(old, new) {
        return ChangeCategory::Location;
    }
    if exact_source_only_changed(old, new) {
        return ChangeCategory::Textual;
    }
    ChangeCategory::Semantic
}

fn equal_without_location(old: &serde_json::Value, new: &serde_json::Value) -> bool {
    stripped(old, StripMode::Location) == stripped(new, StripMode::Location)
}

fn exact_source_only_changed(old: &serde_json::Value, new: &serde_json::Value) -> bool {
    let old_source = old.get("source_hash");
    let new_source = new.get("source_hash");
    if old_source.is_none() || new_source.is_none() || old_source == new_source {
        return false;
    }
    if old.get("signature_hash") != new.get("signature_hash")
        || old.get("implementation_hash") != new.get("implementation_hash")
    {
        return false;
    }
    stripped(old, StripMode::LocationAndSourceHash)
        == stripped(new, StripMode::LocationAndSourceHash)
}

#[derive(Clone, Copy)]
enum StripMode {
    Location,
    LocationAndSourceHash,
}

fn stripped(value: &serde_json::Value, mode: StripMode) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .filter(|(key, _)| {
                    !location_field(key)
                        && !(matches!(mode, StripMode::LocationAndSourceHash)
                            && key.as_str() == "source_hash")
                })
                .map(|(key, value)| (key.clone(), stripped(value, mode)))
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(|value| stripped(value, mode)).collect())
        }
        value => value.clone(),
    }
}

fn location_field(key: &str) -> bool {
    matches!(
        key,
        "source_file"
            | "source_location"
            | "location"
            | "span"
            | "line"
            | "column"
            | "start_line"
            | "start_column"
            | "end_line"
            | "end_column"
            | "start_position"
            | "end_position"
    )
}

fn topology_excluded(change: &GraphChange, topology_only: bool) -> bool {
    topology_only
        && (change.change == ChangeKind::Changed
            || !matches!(change.record, RecordKind::Node | RecordKind::Edge))
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
    if matches!(args[0].as_str(), "build" | "rebuild") {
        return execute_build(&repository, &args[0], &args[1..]);
    }
    if args[0] == "gc" {
        return execute_gc(&repository, &args[1..]);
    }
    if args[0] == "enable" {
        let options = parse_enable_options(&args[1..]).map_err(usage)?;
        HistoryStore::create(&repository).map_err(runtime)?;
        crate::hook_commands::install_managed(frontend).map_err(runtime)?;
        let config = HistoryConfig::enable(&repository, options.profile()).map_err(runtime)?;
        return Ok(format!(
            "history: enabled\nprofile: {}",
            config.profile_digest.as_deref().unwrap_or("none")
        ));
    }
    if args[0] == "disable" {
        if args.len() != 1 {
            return Err(usage("history disable accepts no arguments"));
        }
        HistoryConfig::disable(&repository).map_err(runtime)?;
        return Ok("history: disabled".to_owned());
    }
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
            let config = HistoryConfig::load(&repository).map_err(runtime)?;
            let history_state = if config.enabled {
                "enabled"
            } else {
                "disabled"
            };
            let limitations = repository
                .target_limitations(&commit)
                .map_err(runtime)?
                .into_iter()
                .map(render_limitation)
                .collect::<Vec<_>>();
            let limitation_text = if limitations.is_empty() {
                "none".to_owned()
            } else {
                limitations.join(", ")
            };
            let history = match HistoryStore::open_existing(&repository) {
                Ok(Some(history)) => history,
                Ok(None) => {
                    return Ok(if format == "json" {
                        serde_json::json!({
                            "enabled":config.enabled,
                            "profile_digest":config.profile_digest,
                            "store":false,
                            "commit":commit,
                            "limitations":limitations
                        })
                        .to_string()
                    } else {
                        format!(
                            "history: {history_state}\nprofile: {}\nstore: no store\ncommit: {commit}\nlimitations: {limitation_text}",
                            config.profile_digest.as_deref().unwrap_or("none")
                        )
                    });
                }
                Err(error) => {
                    let report = if format == "json" {
                        serde_json::json!({
                            "enabled":config.enabled,
                            "profile_digest":config.profile_digest,
                            "store":true,
                            "compatible":false,
                            "commit":commit,
                            "limitations":limitations,
                            "validation":{"valid":false,"error":error.to_string()}
                        })
                        .to_string()
                    } else {
                        format!(
                            "history: {history_state}\nprofile: {}\nstore: incompatible\ncommit: {commit}\nlimitations: {limitation_text}\nvalidation: invalid",
                            config.profile_digest.as_deref().unwrap_or("none")
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
                            "enabled":config.enabled,
                            "profile_digest":config.profile_digest,
                            "store":true,
                            "commit":commit,
                            "limitations":limitations,
                            "preferred":serde_json::Value::Null,
                            "validation":{"valid":false,"error":error.to_string()}
                        })
                        .to_string()
                    } else {
                        format!(
                            "history: {history_state}\nprofile: {}\nstore: present\ncommit: {commit}\nlimitations: {limitation_text}\npreferred: unreadable\nvalidation: invalid",
                            config.profile_digest.as_deref().unwrap_or("none")
                        )
                    };
                    return Err(report_failure(report, error));
                }
            };
            if format == "json" {
                let job = newest_job(&repository, &commit).map_err(runtime)?;
                let validation = preferred
                    .as_ref()
                    .map(|value| history.validate(&value.id))
                    .transpose();
                let report = serde_json::json!({
                    "enabled":config.enabled,
                    "profile_digest":config.profile_digest,
                    "store":true,
                    "commit":commit,
                    "limitations":limitations,
                    "preferred":preferred.as_ref().map(|v|v.id.as_hex()),
                    "version":preferred.as_ref().map(|v|&v.version),
                    "job":job,
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
                let mut prefix = format!(
                    "history: {history_state}\nprofile: {}\nstore: present\ncommit: {commit}\nlimitations: {limitation_text}\npreferred: {}\nfingerprint: {}\nnodes: {}\nedges: {}\nvalidation: valid",
                    config.profile_digest.as_deref().unwrap_or("none"),
                    value.id,
                    value.version.extraction_fingerprint,
                    value.version.node_count,
                    value.version.edge_count
                );
                if let Some(job) = newest_job(&repository, &commit).map_err(runtime)?
                    && matches!(job.state, JobState::Failed | JobState::Incomplete)
                {
                    prefix.push_str(&format!(
                        "\nlatest failed attempt: {}\nattempts: {}",
                        job_state_name(job.state),
                        job.attempts
                    ));
                    if let Some(diagnostic) = job.diagnostic {
                        prefix.push_str(&format!("\ndiagnostic: {diagnostic}"));
                    }
                }
                match history.validate(&value.id) {
                    Ok(_) => Ok(prefix),
                    Err(error) => Err(report_failure(
                        prefix.replacen("validation: valid", "validation: invalid", 1),
                        error,
                    )),
                }
            } else {
                let mut report = format!(
                    "history: {history_state}\nprofile: {}\nstore: present\ncommit: {commit}\nlimitations: {limitation_text}\npreferred: none",
                    config.profile_digest.as_deref().unwrap_or("none")
                );
                if let Some(job) = newest_job(&repository, &commit).map_err(runtime)? {
                    report.push_str(&format!(
                        "\njob: {}\nattempts: {}",
                        job_state_name(job.state),
                        job.attempts
                    ));
                    if let Some(diagnostic) = job.diagnostic {
                        report.push_str(&format!("\ndiagnostic: {diagnostic}"));
                    }
                }
                Ok(report)
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
            let build_options = configured_build_options(&repository).map_err(runtime)?;
            let (history, preferred) =
                resolve_or_materialize(&repository, commit, &build_options, false, false)
                    .map_err(runtime)?;
            let artifacts = history.artifacts(&preferred.id).map_err(runtime)?;
            if format == "graph-json" {
                if output.is_dir() {
                    return Err(runtime("graph-json output must be a file"));
                }
                let value = serde_json::to_value(&artifacts.artifacts.document).map_err(runtime)?;
                let bytes = compass_history::canonical_json_bytes(&value).map_err(runtime)?;
                compass_files::write_bytes_atomic(&output, &bytes).map_err(runtime)?;
            } else if format == "compass-out" {
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
                return Err(usage("export --format must be graph-json or compass-out"));
            }
            Ok(format!("exported {} to {}", preferred.id, output.display()))
        }
        other => Err(usage(format!("unknown history command {other}"))),
    }
}

fn execute_gc(repository: &Repository, args: &[String]) -> Result<String, CommandFailure> {
    let mut prune_non_preferred = false;
    let mut yes = false;
    let mut format = "text";
    let mut format_seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--prune-non-preferred" if !prune_non_preferred => prune_non_preferred = true,
            "--yes" if !yes => yes = true,
            "--format" => {
                if format_seen {
                    return Err(usage("duplicate --format"));
                }
                format_seen = true;
                index += 1;
                format = args
                    .get(index)
                    .ok_or_else(|| usage("--format requires a value"))?;
            }
            value if value.starts_with("--format=") => {
                if format_seen {
                    return Err(usage("duplicate --format"));
                }
                format_seen = true;
                format = &value[9..];
            }
            value => return Err(usage(format!("unknown history gc argument {value}"))),
        }
        index += 1;
    }
    if yes && !prune_non_preferred {
        return Err(usage("history gc --yes requires --prune-non-preferred"));
    }
    if !matches!(format, "text" | "json") {
        return Err(usage("history gc --format must be text or json"));
    }
    let history = store(repository)?;
    let plan = history.plan_gc(prune_non_preferred).map_err(runtime)?;
    if prune_non_preferred && !yes {
        if format == "json" {
            return serde_json::to_string(&serde_json::json!({
                "applied": false,
                "confirmation_required": true,
                "plan": plan
            }))
            .map_err(runtime);
        }
        let ids = plan
            .prunable_realization_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(format!(
            "GC plan (not applied)\nprunable realizations: {}\n{}\nreclaimable SQLite node rows: {}\nreclaimable logical bytes: {}\nrerun with --yes to apply",
            plan.prunable_realizations, ids, plan.reclaimable_nodes, plan.reclaimable_bytes
        ));
    }
    let sweep = history.sweep_gc(plan).map_err(runtime)?;
    if format == "json" {
        serde_json::to_string(&serde_json::json!({
            "applied": true,
            "result": sweep
        }))
        .map_err(runtime)
    } else {
        Ok(format!(
            "GC applied\ndeleted SQLite node rows: {}\nreclaimed logical bytes: {}\ndeleted named roots: {}\ndeleted job records: {}\ndeleted temporary directories: {}\nSQLite file size: unchanged or reusable internally (not compacted)",
            sweep.deleted_nodes,
            sweep.deleted_bytes,
            sweep.deleted_named_roots,
            sweep.deleted_job_records,
            sweep.deleted_temp_directories
        ))
    }
}

fn newest_job(
    repository: &Repository,
    commit: &CommitId,
) -> Result<Option<compass_history::JobRecord>, HistoryError> {
    let Some(queue) = HistoryQueue::open_existing(repository)? else {
        return Ok(None);
    };
    Ok(queue
        .list()?
        .into_iter()
        .filter(|job| &job.commit == commit)
        .max_by_key(|job| (job.updated_at_millis, job.id.clone())))
}

fn render_limitation(limitation: GitTargetLimitation) -> String {
    match limitation {
        GitTargetLimitation::LfsPointer(path) => format!("lfs-pointer:{path}"),
        GitTargetLimitation::Gitlink(path) => format!("gitlink:{path}"),
        GitTargetLimitation::UnsupportedFilter(filter) => {
            format!("unsupported-filter:{filter}")
        }
    }
}

fn job_state_name(state: JobState) -> &'static str {
    match state {
        JobState::Queued => "queued",
        JobState::Building => "building",
        JobState::Validating => "validating",
        JobState::Published => "published",
        JobState::Failed => "failed",
        JobState::Incomplete => "incomplete",
    }
}

fn run_worker() -> Result<(), CommandFailure> {
    let repository =
        Repository::discover(&std::env::current_dir().map_err(runtime)?).map_err(runtime)?;
    let queue = HistoryQueue::for_repository(&repository).map_err(runtime)?;
    let history = HistoryStore::create(&repository).map_err(runtime)?;
    while let Some(claimed) = queue.claim_next().map_err(runtime)? {
        run_claimed_job(&repository, &history, &queue, &claimed, false)?;
    }
    Ok(())
}

fn run_claimed_job(
    repository: &Repository,
    history: &HistoryStore,
    queue: &HistoryQueue,
    claimed: &ClaimedJob,
    progress: bool,
) -> Result<(), CommandFailure> {
    if let Some(candidate) = &claimed.candidate_realization
        && history.get(candidate).is_ok()
    {
        let became_preferred = match history.preferred(&claimed.commit) {
            Ok(preferred) if preferred.as_ref().map(|value| &value.id) == Some(candidate) => true,
            Ok(preferred)
                if preferred.as_ref().map(|value| &value.id)
                    == claimed.observed_preferred.as_ref() =>
            {
                history
                    .compare_and_set_preferred(
                        &claimed.commit,
                        claimed.observed_preferred.as_ref(),
                        candidate,
                    )
                    .map_err(runtime)?
            }
            Ok(_) => false,
            Err(error) if error.is_catalog_corruption() && claimed.replace_corrupt => {
                let token = history
                    .corrupt_preferred_token(&claimed.commit)
                    .map_err(runtime)?;
                let activity = history.activity().map_err(runtime)?;
                history
                    .recover_corrupt_preferred_with_activity(
                        &claimed.commit,
                        &token,
                        candidate,
                        &activity,
                    )
                    .map_err(runtime)?
            }
            Err(error) if error.is_catalog_corruption() => false,
            Err(error) => return Err(runtime(error)),
        };
        queue
            .transition(claimed, JobState::Validating, None)
            .map_err(runtime)?;
        queue
            .finish(claimed, JobState::Published, Some(became_preferred), None)
            .map_err(runtime)?;
        return Ok(());
    }
    let options = match HistoryBuildOptions::from_profile(claimed.profile.clone()) {
        Ok(options) => options,
        Err(error) => {
            queue
                .finish(claimed, JobState::Failed, None, Some(&error.to_string()))
                .map_err(runtime)?;
            return Ok(());
        }
    };
    let executable = std::env::current_exe().map_err(runtime)?;
    let builder = options.builder(executable);
    let heartbeat_job = claimed.clone();
    let heartbeat_root = queue.root().to_path_buf();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel();
    let heartbeat = std::thread::spawn(move || {
        let Ok(queue) = HistoryQueue::open(&heartbeat_root) else {
            return;
        };
        loop {
            match stop_rx.recv_timeout(std::time::Duration::from_millis(
                compass_history::LEASE_HEARTBEAT_MILLIS,
            )) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if queue.heartbeat(&heartbeat_job).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let mut observer = DurableJobObserver {
        queue,
        claimed,
        validating: false,
        progress,
    };
    let result = materialize_history_with_observer(
        history,
        &builder,
        MaterializeRequest {
            repository: repository.clone(),
            commit: claimed.commit.clone(),
            profile: claimed.profile.clone(),
            rebuild: claimed.rebuild,
            replace_corrupt: claimed.replace_corrupt,
        },
        &mut observer,
    );
    let _stopped = stop_tx.send(());
    let _joined = heartbeat.join();
    match result {
        Ok(published) => {
            if !observer.validating {
                queue
                    .transition(claimed, JobState::Validating, None)
                    .map_err(runtime)?;
            }
            queue
                .finish(
                    claimed,
                    JobState::Published,
                    Some(published.preferred),
                    None,
                )
                .map_err(runtime)?;
        }
        Err(error) => {
            let state = if matches!(error, MaterializeError::Incomplete(_)) {
                JobState::Incomplete
            } else {
                JobState::Failed
            };
            queue
                .finish(claimed, state, None, Some(&error.to_string()))
                .map_err(runtime)?;
        }
    }
    Ok(())
}

struct DurableJobObserver<'a> {
    queue: &'a HistoryQueue,
    claimed: &'a ClaimedJob,
    validating: bool,
    progress: bool,
}

impl MaterializeObserver for DurableJobObserver<'_> {
    fn entered(&mut self, stage: MaterializeStage) -> Result<(), MaterializeError> {
        if self.progress {
            let message = match stage {
                MaterializeStage::Building => "building complete graph",
                MaterializeStage::Validating => "validating complete graph",
                MaterializeStage::Publishing => "publishing immutable realization",
            };
            eprintln!("[graph history] {message}");
        }
        if stage == MaterializeStage::Validating {
            self.queue
                .transition(self.claimed, JobState::Validating, None)
                .map_err(|error| MaterializeError::Observer(error.to_string()))?;
            self.validating = true;
        }
        Ok(())
    }

    fn resolved(&mut self, fingerprint: &ExtractionFingerprint) -> Result<(), MaterializeError> {
        self.queue
            .annotate(self.claimed, Some(fingerprint.as_hex()), None, None)
            .map(|_| ())
            .map_err(|error| MaterializeError::Observer(error.to_string()))
    }

    fn candidate(
        &mut self,
        candidate: &RealizationId,
        observed_preferred: Option<&RealizationId>,
    ) -> Result<(), MaterializeError> {
        self.queue
            .annotate(
                self.claimed,
                None,
                Some(candidate.clone()),
                observed_preferred.cloned(),
            )
            .map(|_| ())
            .map_err(|error| MaterializeError::Observer(error.to_string()))
    }
}

fn execute_build(
    repository: &Repository,
    command: &str,
    args: &[String],
) -> Result<String, CommandFailure> {
    let parsed = parse_build_command(command, args).map_err(usage)?;
    let commit = repository.resolve(&parsed.revision).map_err(runtime)?;
    let options = if let Some(source) = &parsed.profile_from {
        HistoryBuildOptions::from_profile(stored_profile(repository, source).map_err(runtime)?)
            .map_err(runtime)?
    } else {
        parsed.options
    };
    let profile_rebuild = if parsed.profile_from.is_some() {
        match HistoryStore::open_existing(repository).map_err(runtime)? {
            Some(history) => history
                .preferred(&commit)
                .map_err(runtime)?
                .is_some_and(|preferred| preferred.version.build_profile != options.profile()),
            None => false,
        }
    } else {
        false
    };
    let (_history, published) = resolve_or_materialize(
        repository,
        commit,
        &options,
        command == "rebuild" || profile_rebuild,
        parsed.replace_corrupt,
    )
    .map_err(runtime)?;
    if parsed.format == "json" {
        Ok(serde_json::json!({
            "commit": published.version.git_commit,
            "realization": published.id,
            "fingerprint": published.version.extraction_fingerprint,
            "nodes": published.version.node_count,
            "edges": published.version.edge_count,
            "hyperedges": published.version.hyperedge_count,
            "analysis_records": published.version.analysis_count,
            "metadata_records": published.version.metadata_count,
            "preferred": published.preferred
        })
        .to_string())
    } else {
        Ok(format!(
            "commit: {}\nrealization: {}\nfingerprint: {}\nnodes: {}\nedges: {}\nhyperedges: {}\nanalysis records: {}\nmetadata records: {}\npreferred: {}",
            published.version.git_commit,
            published.id,
            published.version.extraction_fingerprint,
            published.version.node_count,
            published.version.edge_count,
            published.version.hyperedge_count,
            published.version.analysis_count,
            published.version.metadata_count,
            published.preferred
        ))
    }
}

fn stored_profile(repository: &Repository, source: &str) -> Result<BuildProfile, String> {
    let history = HistoryStore::open_existing(repository)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "no graph history is materialized".to_owned())?;
    if let Ok(commit) = repository.resolve(source) {
        let version = history
            .preferred(&commit)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("no preferred realization exists for {source}"))?;
        history
            .validate(&version.id)
            .map_err(|error| error.to_string())?;
        return Ok(version.version.build_profile);
    }
    let id = source
        .parse::<RealizationId>()
        .map_err(|_| format!("--profile-from must name a revision or realization, got {source}"))?;
    let version = history.get(&id).map_err(|error| error.to_string())?;
    history
        .validate(&version.id)
        .map_err(|error| error.to_string())?;
    Ok(version.version.build_profile)
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

    fn change(
        record: RecordKind,
        kind: ChangeKind,
        old: Option<serde_json::Value>,
        new: Option<serde_json::Value>,
    ) -> GraphChange {
        GraphChange {
            record,
            change: kind,
            key: vec![format!("{record:?}"), format!("{kind:?}")],
            old,
            new,
        }
    }

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
        let fingerprint = "a".repeat(64);
        let parsed = parse_diff(&[
            "old".to_owned(),
            "new".to_owned(),
            "--include-locations".to_owned(),
            "--include-analysis".to_owned(),
            "--include-metadata".to_owned(),
            format!("--fingerprint={fingerprint}"),
        ]);
        let Ok(parsed) = parsed else {
            assert!(parsed.is_ok());
            return;
        };
        assert!(parsed.1.include_locations);
        assert!(parsed.1.include_analysis);
        assert!(parsed.1.include_metadata);
        assert_eq!(parsed.1.fingerprint.as_deref(), Some(fingerprint.as_str()));
        assert!(
            parse_diff(&[
                "old".to_owned(),
                "new".to_owned(),
                format!("--fingerprint={fingerprint}"),
                "--allow-profile-mismatch".to_owned(),
            ])
            .is_err()
        );
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
        for arguments in [
            vec!["old", "new", "--detailed", "--detailed"],
            vec!["old", "new", "--topology-only", "--topology-only"],
            vec!["old", "new", "--format"],
            vec!["old", "new", "--format="],
            vec!["old", "new", "--format=text", "--format", "json"],
            vec!["old", "new", "--unknown"],
        ] {
            assert!(
                parse_diff(&arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err()
            );
        }
        let parsed = parse_diff(&["--".to_owned(), "-old".to_owned(), "-new".to_owned()]);
        assert_eq!(
            parsed.map(|value| value.0),
            Ok(vec!["-old".to_owned(), "-new".to_owned()])
        );
    }

    #[test]
    fn graph_changes_are_classified_by_meaning_before_location_churn() {
        let location = change(
            RecordKind::Node,
            ChangeKind::Changed,
            Some(json!({"id":"n","implementation_hash":"a","source_location":"L1"})),
            Some(json!({"id":"n","implementation_hash":"a","source_location":"L9"})),
        );
        assert_eq!(classify_change(&location), ChangeCategory::Location);

        let textual = change(
            RecordKind::Node,
            ChangeKind::Changed,
            Some(json!({
                "id":"n","signature_hash":"s","implementation_hash":"i",
                "source_hash":"old","source_location":"L1"
            })),
            Some(json!({
                "id":"n","signature_hash":"s","implementation_hash":"i",
                "source_hash":"new","source_location":"L2"
            })),
        );
        assert_eq!(classify_change(&textual), ChangeCategory::Textual);

        let semantic = change(
            RecordKind::Node,
            ChangeKind::Changed,
            Some(json!({"id":"n","implementation_hash":"old","source_hash":"old"})),
            Some(json!({"id":"n","implementation_hash":"new","source_hash":"new"})),
        );
        assert_eq!(classify_change(&semantic), ChangeCategory::Semantic);
        assert_eq!(
            classify_change(&change(
                RecordKind::Analysis,
                ChangeKind::Added,
                None,
                Some(json!({"community":1})),
            )),
            ChangeCategory::Analysis
        );
    }

    #[test]
    fn help_failures_and_common_argument_boundaries_are_total() {
        assert!(help(Frontend::Compass).starts_with("Usage: compass history"));
        assert!(diff_help(Frontend::Compass).starts_with("Usage: compass diff"));
        assert_eq!(command(Frontend::Compass, &[]).code, 0);
        assert_eq!(
            command_worker(Frontend::Compass, &["extra".to_owned()]).code,
            2
        );

        let reported = outcome(Err(report_failure("partial".to_owned(), "failed")));
        assert_eq!(reported.code, 1);
        assert_eq!(reported.stdout, "partial");
        assert_eq!(reported.stderr, "error: failed");
        assert_eq!(outcome(Err(usage("bad"))).code, 2);
        assert_eq!(outcome(Err(runtime("bad"))).code, 1);
        assert_eq!(outcome(Ok("ok".to_owned())).stdout, "ok");

        assert!(exact(&["one".to_owned()], 1, "bad").is_ok());
        assert!(exact(&[], 1, "bad").is_err());
        assert!(one_or_zero(&[], "status").is_ok());
        assert!(one_or_zero(&["one".to_owned()], "status").is_ok());
        assert!(one_or_zero(&["one".to_owned(), "two".to_owned()], "status").is_err());

        for arguments in [
            vec!["--format"],
            vec!["--output"],
            vec!["--format="],
            vec!["--output="],
            vec!["--output=a", "--output", "b"],
        ] {
            assert!(parse(&arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
    }

    #[test]
    fn diff_renderers_cover_every_record_change_and_topology_filter()
    -> Result<(), Box<dyn std::error::Error>> {
        let records = [
            RecordKind::Node,
            RecordKind::Edge,
            RecordKind::Hyperedge,
            RecordKind::Analysis,
            RecordKind::Metadata,
        ];
        let changes = [ChangeKind::Added, ChangeKind::Removed, ChangeKind::Changed];
        let summary_options = DiffOptions {
            output: DiffOutput::Summary,
            topology_only: false,
            include_locations: false,
            include_analysis: false,
            include_metadata: false,
            fingerprint: None,
            allow_profile_mismatch: false,
        };
        let mut summary = TextSink::new(&summary_options)?;
        for record in records {
            for kind in changes {
                summary.change(change(record, kind, None, None))?;
            }
        }
        for index in 0..25 {
            summary.change(GraphChange {
                record: RecordKind::Node,
                change: ChangeKind::Added,
                key: vec![format!("node-{index}")],
                old: None,
                new: None,
            })?;
        }
        let mut rendered = Vec::new();
        summary.finish(&mut rendered)?;
        let rendered = String::from_utf8(rendered)?;
        assert!(rendered.contains("26 nodes added"));
        assert!(rendered.contains("analysis record"));
        assert!(rendered.contains("metadata record"));

        for record in records {
            assert!(!record_name(record, 1).is_empty());
            assert!(!record_name(record, 2).is_empty());
        }
        for kind in changes {
            assert!(!change_name(kind).is_empty());
        }

        let mut empty = Vec::new();
        let topology_options = DiffOptions {
            topology_only: true,
            ..summary_options.clone()
        };
        TextSink::new(&topology_options)?.finish(&mut empty)?;
        assert_eq!(empty, b"no topology changes\n");
        let mut topology = TextSink::new(&topology_options)?;
        topology.change(change(
            RecordKind::Analysis,
            ChangeKind::Changed,
            None,
            None,
        ))?;
        let mut filtered = Vec::new();
        topology.finish(&mut filtered)?;
        assert_eq!(filtered, b"no topology changes\n");

        let mut detailed_bytes = Vec::new();
        let detailed_options = DiffOptions {
            output: DiffOutput::Detailed,
            ..summary_options.clone()
        };
        let mut detailed = TextSink::new(&detailed_options)?;
        detailed.change(change(
            RecordKind::Edge,
            ChangeKind::Changed,
            Some(json!({"confidence": 0.5})),
            Some(json!({"confidence": 0.9})),
        ))?;
        detailed.change(change(RecordKind::Node, ChangeKind::Added, None, None))?;
        detailed.finish(&mut detailed_bytes)?;
        assert!(String::from_utf8(detailed_bytes)?.contains("old="));

        let mut no_details = Vec::new();
        TextSink::new(&topology_options)?.finish(&mut no_details)?;
        assert_eq!(no_details, b"no topology changes\n");

        let mut json_bytes = b"{\"changes\":[".to_vec();
        let mut json_sink = JsonSink::new(&mut json_bytes, true);
        json_sink.change(change(
            RecordKind::Metadata,
            ChangeKind::Changed,
            None,
            None,
        ))?;
        json_sink.change(change(RecordKind::Node, ChangeKind::Added, None, None))?;
        json_sink.change(change(RecordKind::Edge, ChangeKind::Removed, None, None))?;
        json_sink.finish()?;
        let decoded: serde_json::Value = serde_json::from_slice(&json_bytes)?;
        assert_eq!(
            decoded
                .get("changes")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        Ok(())
    }

    #[test]
    fn history_labels_and_non_io_json_errors_are_stable() {
        assert_eq!(
            render_limitation(GitTargetLimitation::LfsPointer("a".to_owned())),
            "lfs-pointer:a"
        );
        assert_eq!(
            render_limitation(GitTargetLimitation::Gitlink("b".to_owned())),
            "gitlink:b"
        );
        assert_eq!(
            render_limitation(GitTargetLimitation::UnsupportedFilter("crypt".to_owned())),
            "unsupported-filter:crypt"
        );
        for (state, name) in [
            (JobState::Queued, "queued"),
            (JobState::Building, "building"),
            (JobState::Validating, "validating"),
            (JobState::Published, "published"),
            (JobState::Failed, "failed"),
            (JobState::Incomplete, "incomplete"),
        ] {
            assert_eq!(job_state_name(state), name);
        }
        let parsed = serde_json::from_str::<serde_json::Value>("{");
        assert!(parsed.is_err());
        if let Err(syntax) = parsed {
            assert!(matches!(json_output_error(syntax), HistoryError::Json(_)));
        }
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
        short.write_all(b"{\"changes\":[")?;
        let mut sink = JsonSink::new(&mut short, false);
        sink.change(change.clone())?;
        sink.finish()?;
        let parsed: serde_json::Value = serde_json::from_slice(&short.0)?;
        assert_eq!(parsed["changes"].as_array().map(Vec::len), Some(1));

        let mut broken = BrokenWriter;
        let mut sink = JsonSink::new(&mut broken, false);
        assert!(sink.change(change).is_err());
        Ok(())
    }
}
