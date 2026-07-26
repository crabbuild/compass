use compass_core::LoadedGraph;
use compass_core::{
    MaterializeError, MaterializeObserver, MaterializeRequest, MaterializeStage,
    materialize_history_with_observer,
};
use compass_history::{
    ArtifactClass, BuildProfile, ChangeKind, ChangeSink, ClaimedJob, CommitId,
    ExtractionFingerprint, GitTargetLimitation, GraphChange, HistoryConfig, HistoryError,
    HistoryQueue, HistoryStore, JobRequest, JobState, PublishedVersion, RealizationId, Repository,
};

use crate::history_build::{HistoryBuildOptions, parse_build_command, parse_enable_options};
use crate::{Frontend, Outcome};

pub(crate) fn help(_frontend: Frontend) -> String {
    let prefix = "compass";
    format!(
        "Usage: {prefix} history <command>\n\nCommands:\n  enable [build-profile options]\n  disable\n  timeline [--rev REV] [--limit N [--after CURSOR]] --format json\n  change-counts REV [--parent REV] --format json\n  status [REV] [--format text|json]\n  build REV [--all [--first-parent]] [build-profile options|--profile-from REV|REALIZATION] [--format text|json]\n  rebuild REV [build-profile options] [--replace-corrupt] [--format text|json]\n  list [REV] [--format text|json]\n  show REALIZATION [--format text|json]\n  prefer REV REALIZATION [--format text|json]\n  export REV --format graph-json|json|compass-out [--community ID] [--node-limit N] --output PATH\n  gc [--prune-non-preferred] [--yes] [--format text|json]\n\nBuild options:\n  --all                    Build every commit reachable from REV\n  --first-parent           With --all, build only the first-parent lineage\n\nBuild-profile options:\n  --code-only              Build a complete local AST/inferred realization without model credentials\n  --backend NAME           Build a semantic realization with the selected provider\n  --model NAME             Select the provider model\n  --exclude PATTERN        Exclude a committed path pattern (repeatable)\n  --cargo                   Include Cargo package metadata"
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

pub(crate) fn resolve_or_materialize(
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
    if let Some(profile) = config.profile {
        return HistoryBuildOptions::from_profile(profile).map_err(|error| error.to_string());
    }
    HistoryBuildOptions::defaults().map_err(|error| error.to_string())
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
            html_output: None,
        },
        Err(error) if error.code == 2 => {
            Outcome::failure_with_code(format!("error: {}", error.message), 2)
        }
        Err(error) => Outcome::failure(format!("error: {}", error.message)),
    }
}

pub(crate) struct ResolvedDiff {
    pub history: HistoryStore,
    pub old: PublishedVersion,
    pub new: PublishedVersion,
}

pub(crate) fn resolve_comparable_pair(
    repository: &Repository,
    old_commit: CommitId,
    new_commit: CommitId,
    required_fingerprint: Option<&str>,
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
            let (_, old) =
                resolve_or_materialize(repository, old_commit.clone(), &options, false, false)?;
            let (history, new) =
                resolve_or_materialize(repository, new_commit, &options, false, false)?;
            (history, old, new)
        }
    };
    if old.version.build_profile != new.version.build_profile {
        return Err(format!(
            "realizations are not semantically comparable\n\nOLD {} ({}) profile: {}\nNEW {} ({}) profile: {}\n\nBuild a comparable realization:\n  compass history build {} --profile-from {}",
            old.version.git_commit,
            old.id,
            old.version.profile_digest,
            new.version.git_commit,
            new.id,
            new.version.profile_digest,
            new.version.git_commit,
            old.version.git_commit,
        ));
    }
    Ok(ResolvedDiff { history, old, new })
}

fn select_existing(
    history: Option<&HistoryStore>,
    commit: &CommitId,
    required_fingerprint: Option<&str>,
) -> Result<Option<PublishedVersion>, String> {
    let Some(history) = history else {
        return Ok(None);
    };
    if let Some(fingerprint) = required_fingerprint {
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
        Ok(matches.pop())
    } else {
        history.preferred(commit).map_err(|error| error.to_string())
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
    if args[0] == "timeline" {
        return execute_timeline(&repository, &args[1..]);
    }
    if args[0] == "change-counts" {
        return execute_change_counts(&repository, &args[1..]);
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
    let ParsedCommonOptions {
        positionals,
        format,
        output,
        community,
        node_limit,
    } = parse(&args[1..]).map_err(usage)?;
    if args[0] != "export" && output.is_some() {
        return Err(usage("--output is only valid for history export"));
    }
    if args[0] != "export" && community.is_some() {
        return Err(usage("--community is only valid for history export"));
    }
    if args[0] != "export" && node_limit.is_some() {
        return Err(usage("--node-limit is only valid for history export"));
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
                    "history: {history_state}\nprofile: {}\nstore: present\ncommit: {commit}\nlimitations: {limitation_text}\npreferred: {}\nfingerprint: {}\nnodes: {}\nedges: {}\nprogram facts: {}\nprogram summaries: {}\nvalidation: valid",
                    config.profile_digest.as_deref().unwrap_or("none"),
                    value.id,
                    value.version.extraction_fingerprint,
                    value.version.node_count,
                    value.version.edge_count,
                    value.version.program_fact_count,
                    value.version.program_summary_count
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
                    "realization: {}\ncommit: {}\nfingerprint: {}\nnodes: {}\nedges: {}\nprogram facts: {}\nprogram summaries: {}",
                    value.id,
                    value.version.git_commit,
                    value.version.extraction_fingerprint,
                    value.version.node_count,
                    value.version.edge_count,
                    value.version.program_fact_count,
                    value.version.program_summary_count
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
                runtime(format!(
                    "cannot replace an unreadable preferred realization: {error}; run `compass history rebuild {} --replace-corrupt`",
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
            if matches!(format.as_str(), "json" | "viewer-json") {
                let node_limit = node_limit.unwrap_or(5_000);
                if node_limit < 1 {
                    return Err(usage("--node-limit must be a positive integer"));
                }
                let history = store(&repository)?;
                let preferred = history
                    .preferred(&commit)
                    .map_err(runtime)?
                    .ok_or_else(|| {
                        runtime(format!(
                            "revision {commit} is not materialized; build it explicitly first"
                        ))
                    })?;
                history.validate(&preferred.id).map_err(runtime)?;
                let artifacts = history.artifacts(&preferred.id).map_err(runtime)?;
                let communities = communities_from_document(&artifacts.artifacts.document);
                let labels = history_labels(artifacts.artifacts.labels.as_ref());
                let options = compass_output::HtmlOptions {
                    community_labels: (!labels.is_empty()).then_some(&labels),
                    member_counts: None,
                    node_limit: Some(node_limit),
                    learning_overlay: None,
                };
                let graph = if let Some(community) = community {
                    compass_output::graph_community_view_model_document(
                        &artifacts.artifacts.document,
                        &communities,
                        format!("{} @ {}", repository.root().display(), commit),
                        &options,
                        community,
                    )
                    .map_err(runtime)?
                } else {
                    compass_output::graph_view_model_document(
                        &artifacts.artifacts.document,
                        &communities,
                        format!("{} @ {}", repository.root().display(), commit),
                        &options,
                    )
                    .map_err(runtime)?
                    .ok_or_else(|| runtime("historical graph has no renderable overview"))?
                };
                let envelope = serde_json::json!({
                    "schema": "compass.history.viewer_graph/1",
                    "commit": commit,
                    "realization": preferred.id,
                    "fingerprint": preferred.version.extraction_fingerprint,
                    "graph": graph,
                });
                let bytes = serde_json::to_vec(&envelope).map_err(runtime)?;
                compass_files::write_bytes_atomic(&output, &bytes).map_err(runtime)?;
                return Ok(format!("exported {} to {}", preferred.id, output.display()));
            }
            if community.is_some() {
                return Err(usage(
                    "--community is only valid with history export --format json",
                ));
            }
            if node_limit.is_some() {
                return Err(usage(
                    "--node-limit is only valid with history export --format json",
                ));
            }
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
                let program = artifacts
                    .artifacts
                    .program
                    .as_ref()
                    .map(compass_analysis::AnalysisBundle::canonical_bytes)
                    .transpose()
                    .map_err(runtime)?;
                compass_output::publish_history_bundle(
                    &output,
                    &compass_output::HistoryBundleInput {
                        document: &artifacts.artifacts.document,
                        program: program.as_deref(),
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
                return Err(usage(
                    "export --format must be graph-json, json, or compass-out",
                ));
            }
            Ok(format!("exported {} to {}", preferred.id, output.display()))
        }
        other => Err(usage(format!("unknown history command {other}"))),
    }
}

#[derive(Default, serde::Serialize)]
struct RecordChangeCounts {
    added: u64,
    removed: u64,
    changed: u64,
}

#[derive(Default, serde::Serialize)]
struct ChangeCounts {
    nodes: RecordChangeCounts,
    edges: RecordChangeCounts,
    hyperedges: RecordChangeCounts,
}

impl ChangeSink for ChangeCounts {
    fn change(&mut self, change: GraphChange) -> Result<(), HistoryError> {
        let counts = match change.record {
            compass_history::RecordKind::Node => &mut self.nodes,
            compass_history::RecordKind::Edge => &mut self.edges,
            compass_history::RecordKind::Hyperedge => &mut self.hyperedges,
            _ => return Ok(()),
        };
        match change.change {
            ChangeKind::Added => counts.added += 1,
            ChangeKind::Removed => counts.removed += 1,
            ChangeKind::Changed => counts.changed += 1,
        }
        Ok(())
    }
}

fn execute_change_counts(
    repository: &Repository,
    args: &[String],
) -> Result<String, CommandFailure> {
    let mut revision = None;
    let mut parent_revision = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--parent" => {
                index += 1;
                parent_revision = Some(
                    args.get(index)
                        .ok_or_else(|| usage("--parent requires a revision"))?
                        .clone(),
                );
            }
            "--format" => {
                index += 1;
                if args.get(index).map(String::as_str) != Some("json") {
                    return Err(usage("history change-counts requires --format json"));
                }
            }
            value if value.starts_with("--parent=") => {
                parent_revision = Some(value[9..].to_owned());
            }
            value if value.starts_with("--format=") => {
                if &value[9..] != "json" {
                    return Err(usage("history change-counts requires --format json"));
                }
            }
            value if value.starts_with('-') => {
                return Err(usage(format!(
                    "unknown history change-counts option {value}"
                )));
            }
            value if revision.is_none() => revision = Some(value.to_owned()),
            value => {
                return Err(usage(format!(
                    "history change-counts accepts one revision, unexpected: {value}"
                )));
            }
        }
        index += 1;
    }
    let revision = revision.ok_or_else(|| usage("history change-counts requires REV"))?;
    let commit = repository.resolve(&revision).map_err(runtime)?;
    let parent = if let Some(parent) = parent_revision {
        repository.resolve(&parent).map_err(runtime)?
    } else {
        repository
            .parents(&commit)
            .map_err(runtime)?
            .into_iter()
            .next()
            .ok_or_else(|| runtime(format!("revision {commit} has no parent to compare")))?
    };
    let history = store(repository)?;
    let current = history
        .preferred(&commit)
        .map_err(runtime)?
        .ok_or_else(|| runtime(format!("revision {commit} is not materialized")))?;
    let previous = history
        .preferred(&parent)
        .map_err(runtime)?
        .ok_or_else(|| runtime(format!("parent revision {parent} is not materialized")))?;
    history.validate(&current.id).map_err(runtime)?;
    history.validate(&previous.id).map_err(runtime)?;
    let mut counts = ChangeCounts::default();
    history
        .diff_records(
            &previous.id,
            &current.id,
            &[
                compass_history::RecordKind::Node,
                compass_history::RecordKind::Edge,
                compass_history::RecordKind::Hyperedge,
            ],
            &mut counts,
        )
        .map_err(runtime)?;
    serde_json::to_string(&serde_json::json!({
        "schema": "compass.history.change_counts/1",
        "commit": commit,
        "parent": parent,
        "counts": counts,
    }))
    .map_err(runtime)
}

fn communities_from_document(
    document: &compass_model::GraphDocument,
) -> compass_graph::Communities {
    let mut communities = compass_graph::Communities::new();
    for node in &document.nodes {
        let community = node
            .attributes
            .get("community")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as usize;
        communities
            .entry(community)
            .or_default()
            .push(node.id.clone());
    }
    communities
}

fn history_labels(labels: Option<&serde_json::Value>) -> std::collections::BTreeMap<usize, String> {
    labels
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|labels| labels.iter())
        .filter_map(|(community, label)| {
            Some((community.parse().ok()?, label.as_str()?.to_owned()))
        })
        .collect()
}

fn execute_timeline(repository: &Repository, args: &[String]) -> Result<String, CommandFailure> {
    let mut revision = "HEAD".to_owned();
    let mut revision_selected = false;
    let mut format = "json".to_owned();
    let mut limit = None;
    let mut after = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--rev" => {
                index += 1;
                revision = args
                    .get(index)
                    .ok_or_else(|| usage("--rev requires a revision"))?
                    .clone();
                revision_selected = true;
            }
            "--format" => {
                index += 1;
                format = args
                    .get(index)
                    .ok_or_else(|| usage("--format requires json"))?
                    .clone();
            }
            value if value.starts_with("--rev=") => {
                revision = value[6..].to_owned();
                revision_selected = true;
            }
            value if value.starts_with("--format=") => format = value[9..].to_owned(),
            "--limit" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| usage("--limit requires a value"))?;
                limit = Some(parse_timeline_limit(value)?);
            }
            value if value.starts_with("--limit=") => {
                limit = Some(parse_timeline_limit(&value[8..])?);
            }
            "--after" => {
                index += 1;
                after = Some(
                    args.get(index)
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| usage("--after requires a cursor"))?
                        .clone(),
                );
            }
            value if value.starts_with("--after=") && value.len() > 8 => {
                after = Some(value[8..].to_owned());
            }
            value => return Err(usage(format!("unknown history timeline option {value}"))),
        }
        index += 1;
    }
    if format != "json" {
        return Err(usage("history timeline requires --format json"));
    }
    if after.is_some() && limit.is_none() {
        return Err(usage("history timeline --after requires --limit"));
    }
    let head = repository.resolve(&revision).map_err(runtime)?;
    let snapshot = if revision_selected {
        format!("revision-{}", head.as_str())
    } else {
        repository.reference_snapshot().map_err(runtime)?
    };
    let start = after
        .as_deref()
        .map(parse_timeline_cursor)
        .transpose()?
        .map(|cursor| {
            if cursor.snapshot != snapshot {
                Err(usage(
                    "history timeline snapshot changed; reload the timeline",
                ))
            } else {
                Ok(cursor.offset)
            }
        })
        .transpose()?
        .unwrap_or(0);
    let (commits, has_more) = if let Some(limit) = limit {
        let request_limit = limit.saturating_add(1);
        let mut page = if revision_selected {
            repository
                .reachable_commit_page(&head, start, request_limit)
                .map_err(runtime)?
        } else {
            repository
                .all_reachable_commit_page(start, request_limit)
                .map_err(runtime)?
        };
        let has_more = page.len() > limit;
        page.truncate(limit);
        (page, has_more)
    } else {
        let mut commits = if revision_selected {
            repository
                .reachable_commits(&head, false)
                .map_err(runtime)?
        } else {
            repository.all_reachable_commits().map_err(runtime)?
        };
        commits.reverse();
        (commits, false)
    };
    let end = start.saturating_add(commits.len());
    let total_entries = (!has_more).then_some(end);
    let next_cursor = has_more.then(|| timeline_cursor(&snapshot, end));
    let history = HistoryStore::open_existing(repository).map_err(runtime)?;
    let versions = history
        .as_ref()
        .map(|store| store.preferred_many(&commits))
        .transpose()
        .map_err(runtime)?
        .unwrap_or_default();
    let preferred = versions
        .into_iter()
        .map(|version| (version.version.git_commit.clone(), version))
        .collect::<std::collections::BTreeMap<_, _>>();
    let jobs = HistoryQueue::open_existing(repository)
        .map_err(runtime)?
        .map(|queue| queue.latest_for_commits(&commits))
        .transpose()
        .map_err(runtime)?
        .unwrap_or_default()
        .into_iter()
        .fold(
            std::collections::HashMap::<String, compass_history::JobRecord>::new(),
            |mut latest, job| {
                let replace = latest
                    .get(job.commit.as_str())
                    .is_none_or(|current| current.updated_at_millis < job.updated_at_millis);
                if replace {
                    latest.insert(job.commit.to_string(), job);
                }
                latest
            },
        );
    let metadata = repository.timeline_commits(&commits).map_err(runtime)?;
    let entries = commits
        .into_iter()
        .zip(metadata)
        .map(|(commit, metadata)| {
            let version = preferred.get(commit.as_str());
            let job = jobs.get(commit.as_str());
            let graph_state = if version.is_some() {
                "graph_available"
            } else {
                match job.map(|job| job.state) {
                    Some(JobState::Queued | JobState::Building | JobState::Validating) => "building",
                    Some(JobState::Failed | JobState::Incomplete) => "failed",
                    Some(JobState::Published) | None => "not_materialized",
                }
            };
            Ok(serde_json::json!({
                "commit": metadata.commit,
                "parents": metadata.parents,
                "authorName": metadata.author_name,
                "authorEmail": metadata.author_email,
                "authoredAtSeconds": metadata.authored_at_seconds,
                "subject": metadata.subject,
                "graphState": graph_state,
                "presentationAvailable": version.is_some(),
                "realization": version.map(|version| version.id.as_hex()),
                "fingerprint": version.map(|version| version.version.extraction_fingerprint.as_str()),
                "job": job,
            }))
        })
        .collect::<Result<Vec<_>, CommandFailure>>()?;
    let config = HistoryConfig::load(repository).map_err(runtime)?;
    serde_json::to_string(&serde_json::json!({
        "schema": compass_history::HISTORY_TIMELINE_SCHEMA,
        "repositoryId": repository.common_dir().to_string_lossy(),
        "selectedHead": head,
        "historyEnabled": config.enabled,
        "totalEntries": total_entries,
        "hasMore": has_more,
        "nextCursor": next_cursor,
        "entries": entries,
    }))
    .map_err(runtime)
}

fn parse_timeline_limit(value: &str) -> Result<usize, CommandFailure> {
    let limit = value
        .parse::<usize>()
        .map_err(|_| usage("history timeline --limit must be a positive integer"))?;
    if !(1..=1000).contains(&limit) {
        return Err(usage("history timeline --limit must be between 1 and 1000"));
    }
    Ok(limit)
}

struct TimelineCursor {
    snapshot: String,
    offset: usize,
}

fn timeline_cursor(snapshot: &str, offset: usize) -> String {
    format!("v1:{snapshot}:{offset}")
}

fn parse_timeline_cursor(value: &str) -> Result<TimelineCursor, CommandFailure> {
    let mut fields = value.split(':');
    let version = fields.next();
    let snapshot = fields.next();
    let offset = fields.next();
    if version != Some("v1")
        || snapshot.is_none_or(|snapshot| {
            snapshot.is_empty()
                || snapshot.len() > 80
                || !snapshot
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        || fields.next().is_some()
    {
        return Err(usage(
            "history timeline cursor is invalid; reload the timeline",
        ));
    }
    let offset = offset
        .and_then(|offset| offset.parse::<usize>().ok())
        .ok_or_else(|| usage("history timeline cursor is invalid; reload the timeline"))?;
    Ok(TimelineCursor {
        snapshot: snapshot.unwrap_or_default().to_owned(),
        offset,
    })
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
    } else if parsed.use_repository_profile {
        configured_build_options(repository).map_err(runtime)?
    } else {
        parsed.options
    };
    if parsed.all {
        let commits = repository
            .reachable_commits(&commit, parsed.first_parent)
            .map_err(runtime)?;
        let batch = crate::history_batch::execute(
            repository,
            &parsed.revision,
            commit,
            commits,
            &options,
            parsed.first_parent,
            &parsed.format,
        )
        .map_err(runtime)?;
        return if batch.failed {
            Err(report_failure(
                batch.stdout,
                "one or more history builds failed",
            ))
        } else {
            Ok(batch.stdout)
        };
    }
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
            "program_fact_records": published.version.program_fact_count,
            "program_summary_records": published.version.program_summary_count,
            "preferred": published.preferred
        })
        .to_string())
    } else {
        Ok(format!(
            "commit: {}\nrealization: {}\nfingerprint: {}\nnodes: {}\nedges: {}\nhyperedges: {}\nanalysis records: {}\nmetadata records: {}\nprogram fact records: {}\nprogram summary records: {}\npreferred: {}",
            published.version.git_commit,
            published.id,
            published.version.extraction_fingerprint,
            published.version.node_count,
            published.version.edge_count,
            published.version.hyperedge_count,
            published.version.analysis_count,
            published.version.metadata_count,
            published.version.program_fact_count,
            published.version.program_summary_count,
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

struct ParsedCommonOptions {
    positionals: Vec<String>,
    format: String,
    output: Option<std::path::PathBuf>,
    community: Option<usize>,
    node_limit: Option<isize>,
}

fn parse(args: &[String]) -> Result<ParsedCommonOptions, String> {
    let mut p = Vec::new();
    let mut f = None;
    let mut o = None;
    let mut c = None;
    let mut n = None;
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
            "--community" if options => {
                i += 1;
                let value = args.get(i).ok_or("--community requires an id")?;
                let value = value
                    .parse::<usize>()
                    .map_err(|_| "--community must be a non-negative integer")?;
                if c.replace(value).is_some() {
                    return Err("duplicate --community".into());
                }
            }
            "--node-limit" if options => {
                i += 1;
                let value = args.get(i).ok_or("--node-limit requires a value")?;
                let value = value
                    .parse::<isize>()
                    .map_err(|_| "--node-limit must be an integer")?;
                if n.replace(value).is_some() {
                    return Err("duplicate --node-limit".into());
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
            v if options && v.starts_with("--community=") => {
                let value = &v[12..];
                if value.is_empty() {
                    return Err("--community requires an id".to_owned());
                }
                let value = value
                    .parse::<usize>()
                    .map_err(|_| "--community must be a non-negative integer")?;
                if c.replace(value).is_some() {
                    return Err("duplicate --community".into());
                }
            }
            v if options && v.starts_with("--node-limit=") => {
                let value = &v[13..];
                if value.is_empty() {
                    return Err("--node-limit requires a value".to_owned());
                }
                let value = value
                    .parse::<isize>()
                    .map_err(|_| "--node-limit must be an integer")?;
                if n.replace(value).is_some() {
                    return Err("duplicate --node-limit".into());
                }
            }
            v if options && v.starts_with('-') => return Err(format!("unknown option {v}")),
            v => p.push(v.into()),
        }
        i += 1;
    }
    Ok(ParsedCommonOptions {
        positionals: p,
        format: f.unwrap_or_else(|| "text".into()),
        output: o,
        community: c,
        node_limit: n,
    })
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
    use super::*;

    #[test]
    fn common_options_support_equals_end_marker_and_reject_duplicates() {
        let result = parse(&[
            "--format=json".to_owned(),
            "--output=result".to_owned(),
            "--".to_owned(),
            "-revision".to_owned(),
        ]);
        let Ok((positionals, format, output, community, node_limit)) = result else {
            assert!(result.is_ok());
            return;
        };
        assert_eq!(positionals, ["-revision"]);
        assert_eq!(format, "json");
        assert_eq!(output.as_deref(), Some(std::path::Path::new("result")));
        assert_eq!(community, None);
        assert_eq!(node_limit, None);
        assert!(
            parse(&[
                "--format=json".to_owned(),
                "--format".to_owned(),
                "text".to_owned(),
            ])
            .is_err()
        );
        assert!(parse(&["--unknown".to_owned()]).is_err());
    }

    #[test]
    fn help_failures_and_common_argument_boundaries_are_total() {
        assert!(help(Frontend::Compass).starts_with("Usage: compass history"));
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
    }
}
