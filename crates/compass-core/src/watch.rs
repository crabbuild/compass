use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use compass_files::{DetectOptions, FileType, WatchPathFilter, classify_file, write_text_atomic};
use compass_languages::Registry;
use notify::event::EventKind;
use notify::{Config, Event, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};

use crate::watch_scheduler::{WatchScheduler, retry_delay};
use crate::{BuildOptions, BuildResult, CoreError, build_local_graph};

const DEFAULT_RECONCILIATION_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug)]
pub struct WatchOptions {
    pub build: BuildOptions,
    pub debounce: Duration,
    pub poll_interval: Duration,
    pub force_polling: bool,
    pub adaptive: bool,
    pub reconciliation_interval: Duration,
}

impl WatchOptions {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            build: BuildOptions::new(root),
            debounce: Duration::from_millis(150),
            poll_interval: Duration::from_millis(500),
            force_polling: false,
            adaptive: true,
            reconciliation_interval: DEFAULT_RECONCILIATION_INTERVAL,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchBackend {
    Native,
    Polling,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchBuildReason {
    Initial,
    Changes,
    Retry,
    Reconciliation,
}

#[derive(Clone, Debug)]
pub enum WatchStatus {
    Starting {
        root: PathBuf,
        includes: usize,
        excludes: usize,
        output: PathBuf,
    },
    Backend {
        backend: WatchBackend,
        fallback_error: Option<String>,
        poll_interval: Duration,
    },
    Watching {
        root: PathBuf,
        debounce: Duration,
    },
    Synchronizing,
    Settling {
        paths: usize,
        quiet_window: Duration,
        maximum_window: Duration,
    },
    Building {
        reason: WatchBuildReason,
    },
    Batch {
        paths: Vec<PathBuf>,
        deterministic: usize,
        semantic: usize,
    },
    Rebuilt(Box<BuildResult>),
    UpToDate {
        reason: WatchBuildReason,
    },
    FollowUpQueued {
        paths: usize,
    },
    RetryScheduled {
        delay: Duration,
        error: String,
        repeated: u32,
    },
    SemanticUpdateRequired {
        flag: PathBuf,
    },
    EventError(String),
    RebuildError(String),
    Finishing,
    Stopped,
}

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    File(#[from] compass_files::FileError),
    #[error("could not start filesystem watcher for {path}: {source}")]
    Start {
        path: PathBuf,
        #[source]
        source: notify::Error,
    },
    #[error("filesystem watcher channel disconnected")]
    Disconnected,
}

/// Watch a local corpus and rebuild its deterministic graph after coalesced
/// changes. The caller owns signal handling through `stop`, making this API
/// testable and safe to embed in other frontends.
pub fn watch_local_graph(
    options: &WatchOptions,
    stop: &AtomicBool,
    emit: impl FnMut(WatchStatus),
) -> Result<(), WatchError> {
    if options.adaptive {
        watch_adaptive(options, stop, emit)
    } else {
        watch_legacy(options, stop, emit)
    }
}

fn watch_context(
    options: &WatchOptions,
) -> Result<(PathBuf, WatchPathFilter, BTreeSet<PathBuf>), WatchError> {
    if !options.build.root.exists() {
        return Err(CoreError::MissingRoot(options.build.root.clone()).into());
    }
    let root =
        fs::canonicalize(&options.build.root).map_err(|source| compass_files::FileError::Io {
            path: options.build.root.clone(),
            source,
        })?;
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let filter = WatchPathFilter::new(
        &root,
        &DetectOptions {
            gitignore: options.build.gitignore,
            extra_excludes: options.build.extra_excludes.clone(),
            scope: options.build.scope.clone(),
            output_name,
            ..DetectOptions::default()
        },
    )?;
    let program_paths = program_watch_paths(&root, &options.build);
    Ok((root, filter, program_paths))
}

fn watch_legacy(
    options: &WatchOptions,
    stop: &AtomicBool,
    mut emit: impl FnMut(WatchStatus),
) -> Result<(), WatchError> {
    let (root, filter, program_paths) = watch_context(options)?;
    let (sender, receiver) = mpsc::channel();
    let handler = move |event| {
        let _result = sender.send(event);
    };
    let mut watcher: Box<dyn Watcher> = if options.force_polling {
        Box::new(
            PollWatcher::new(
                handler,
                Config::default()
                    .with_poll_interval(options.poll_interval)
                    // notify 8's polling backend truncates mtimes to whole
                    // seconds. Hashing prevents same-second editor saves from
                    // disappearing when users explicitly select `--poll`.
                    .with_compare_contents(true),
            )
            .map_err(|source| WatchError::Start {
                path: root.clone(),
                source,
            })?,
        )
    } else {
        Box::new(
            RecommendedWatcher::new(handler, Config::default()).map_err(|source| {
                WatchError::Start {
                    path: root.clone(),
                    source,
                }
            })?,
        )
    };
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|source| WatchError::Start {
            path: root.clone(),
            source,
        })?;
    attach_external_paths(&mut *watcher, &root, &program_paths)?;
    emit(WatchStatus::Watching {
        root: root.clone(),
        debounce: options.debounce,
    });

    let mut pending = BTreeSet::new();
    let mut last_change = None;
    while !stop.load(Ordering::Acquire) {
        let timeout = legacy_next_timeout(last_change, options.debounce, options.poll_interval);
        match receiver.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                if collect_event(&event, &filter, &program_paths, &mut pending) {
                    last_change = Some(Instant::now());
                }
            }
            Ok(Err(error)) => emit(WatchStatus::EventError(error.to_string())),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(WatchError::Disconnected),
        }
        if last_change.is_some_and(|last| last.elapsed() >= options.debounce) {
            let paths = std::mem::take(&mut pending).into_iter().collect::<Vec<_>>();
            last_change = None;
            if paths.is_empty() {
                continue;
            }
            process_batch(options, &root, &program_paths, paths, &mut emit)?;
        }
    }
    emit(WatchStatus::Stopped);
    Ok(())
}

type EventMessage = Result<Event, notify::Error>;

struct EventSource {
    _watcher: Box<dyn Watcher>,
    receiver: Receiver<EventMessage>,
    backend: WatchBackend,
    fallback_error: Option<String>,
}

#[derive(Clone, Debug)]
enum AdaptiveWork {
    Initial,
    Changes(Vec<PathBuf>),
    Reconciliation,
}

struct RetryState {
    work: AdaptiveWork,
    failures: u32,
    at: Instant,
    error: String,
    repeated: u32,
}

enum AdaptiveOutcome {
    Succeeded(Option<Box<BuildResult>>),
    Failed(String),
}

fn watch_adaptive(
    options: &WatchOptions,
    stop: &AtomicBool,
    mut emit: impl FnMut(WatchStatus),
) -> Result<(), WatchError> {
    let (root, filter, program_paths) = watch_context(options)?;
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let output = options
        .build
        .output_root
        .as_deref()
        .unwrap_or(&root)
        .join(output_name);
    emit(WatchStatus::Starting {
        root: root.clone(),
        includes: options.build.scope.include.len(),
        excludes: options.build.scope.exclude.len(),
        output,
    });
    let mut source = start_adaptive_event_source(options, &root, &program_paths)?;
    emit(WatchStatus::Backend {
        backend: source.backend,
        fallback_error: source.fallback_error.take(),
        poll_interval: options.poll_interval,
    });

    let mut scheduler = WatchScheduler::new(
        Instant::now(),
        options.debounce,
        options.reconciliation_interval,
    );
    let mut initial = true;
    let mut retry: Option<RetryState> = None;
    let mut event_errors = RepeatedError::default();
    let mut watching = false;

    loop {
        if stop.load(Ordering::Acquire) {
            emit(WatchStatus::Stopped);
            return Ok(());
        }

        let now = Instant::now();
        let selected = if initial {
            initial = false;
            Some((AdaptiveWork::Initial, 0, None, 0))
        } else if retry.as_ref().is_some_and(|state| now >= state.at) {
            retry.take().map(|state| {
                (
                    state.work,
                    state.failures,
                    Some(state.error),
                    state.repeated,
                )
            })
        } else if retry.is_none() && scheduler.is_batch_ready(now) {
            Some((AdaptiveWork::Changes(scheduler.take_batch()), 0, None, 0))
        } else if retry.is_none() && scheduler.reconciliation_due(now) {
            Some((AdaptiveWork::Reconciliation, 0, None, 0))
        } else {
            None
        };

        if let Some((mut work, failures, previous_error, previous_repeated)) = selected {
            if let AdaptiveWork::Changes(paths) = &mut work {
                paths.extend(scheduler.take_batch());
                paths.sort();
                paths.dedup();
            } else {
                // A full build observes every change already present before it
                // starts. Only events arriving during the build need a
                // follow-up.
                scheduler.take_batch();
            }
            let reason = if failures > 0 {
                WatchBuildReason::Retry
            } else {
                match work {
                    AdaptiveWork::Initial => WatchBuildReason::Initial,
                    AdaptiveWork::Changes(_) => WatchBuildReason::Changes,
                    AdaptiveWork::Reconciliation => WatchBuildReason::Reconciliation,
                }
            };
            if matches!(work, AdaptiveWork::Initial) {
                emit(WatchStatus::Synchronizing);
            }
            emit(WatchStatus::Building { reason });
            let outcome = run_adaptive_work_observed(
                options,
                &root,
                &program_paths,
                &work,
                stop,
                &mut source,
                &filter,
                &mut scheduler,
                &mut event_errors,
                &mut emit,
            )?;
            match outcome {
                AdaptiveOutcome::Succeeded(result) => {
                    scheduler.mark_build_succeeded(Instant::now());
                    event_errors.clear();
                    if let Some(result) = result {
                        if result.outputs_changed {
                            emit(WatchStatus::Rebuilt(result));
                        } else {
                            emit(WatchStatus::UpToDate { reason });
                        }
                    }
                    if matches!(work, AdaptiveWork::Initial) && !watching {
                        emit(WatchStatus::Watching {
                            root: root.clone(),
                            debounce: options.debounce,
                        });
                        watching = true;
                    }
                    if scheduler.pending_len() > 0 {
                        emit(WatchStatus::FollowUpQueued {
                            paths: scheduler.pending_len(),
                        });
                    }
                    retry = None;
                }
                AdaptiveOutcome::Failed(error) => {
                    if stop.load(Ordering::Acquire) {
                        emit(WatchStatus::RebuildError(error));
                        continue;
                    }
                    if let AdaptiveWork::Changes(paths) = &mut work {
                        paths.extend(scheduler.take_batch());
                        paths.sort();
                        paths.dedup();
                    }
                    let failures = failures.saturating_add(1);
                    let repeated = if previous_error.as_deref() == Some(error.as_str()) {
                        previous_repeated.saturating_add(1)
                    } else {
                        1
                    };
                    let delay = retry_delay(failures);
                    emit(WatchStatus::RetryScheduled {
                        delay,
                        error: error.clone(),
                        repeated,
                    });
                    retry = Some(RetryState {
                        work,
                        failures,
                        at: Instant::now() + delay,
                        error,
                        repeated,
                    });
                }
            }
            continue;
        }

        let retry_at = retry.as_ref().map(|state| state.at);
        let timeout = scheduler.next_timeout(now, options.poll_interval, retry_at);
        match source.receiver.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                let was_empty = scheduler.pending_len() == 0;
                record_adaptive_event(
                    &event,
                    &filter,
                    &program_paths,
                    &mut scheduler,
                    Instant::now(),
                );
                if was_empty && scheduler.pending_len() > 0 {
                    emit(WatchStatus::Settling {
                        paths: scheduler.pending_len(),
                        quiet_window: options.debounce,
                        maximum_window: scheduler.maximum_window(),
                    });
                }
            }
            Ok(Err(error)) => {
                if let Some(message) = event_errors.observe(error.to_string()) {
                    emit(WatchStatus::EventError(message));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) if source.backend == WatchBackend::Native => {
                source = start_single_event_source(
                    options,
                    &root,
                    &program_paths,
                    WatchBackend::Polling,
                )?;
                emit(WatchStatus::Backend {
                    backend: WatchBackend::Polling,
                    fallback_error: Some("native filesystem event channel disconnected".to_owned()),
                    poll_interval: options.poll_interval,
                });
            }
            Err(RecvTimeoutError::Disconnected) => return Err(WatchError::Disconnected),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_adaptive_work_observed(
    options: &WatchOptions,
    root: &Path,
    program_paths: &BTreeSet<PathBuf>,
    work: &AdaptiveWork,
    stop: &AtomicBool,
    source: &mut EventSource,
    filter: &WatchPathFilter,
    scheduler: &mut WatchScheduler,
    event_errors: &mut RepeatedError,
    emit: &mut impl FnMut(WatchStatus),
) -> Result<AdaptiveOutcome, WatchError> {
    let worker_options = options.clone();
    let worker_root = root.to_path_buf();
    let worker_program_paths = program_paths.clone();
    let worker_work = work.clone();
    let (outcome_sender, outcome_receiver) = mpsc::channel();
    let (status_sender, status_receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let outcome = run_adaptive_work(
            &worker_options,
            &worker_root,
            &worker_program_paths,
            &worker_work,
            &mut |status| {
                let _result = status_sender.send(status);
            },
        );
        let _result = outcome_sender.send(outcome);
    });
    let mut finishing = false;
    let mut disconnected_backend = None;
    let outcome = loop {
        while let Ok(status) = status_receiver.try_recv() {
            emit(status);
        }
        match outcome_receiver.try_recv() {
            Ok(outcome) => break outcome,
            Err(TryRecvError::Disconnected) => {
                break AdaptiveOutcome::Failed(
                    "adaptive watch build worker stopped unexpectedly".to_owned(),
                );
            }
            Err(TryRecvError::Empty) => {}
        }
        if stop.load(Ordering::Acquire) && !finishing {
            emit(WatchStatus::Finishing);
            finishing = true;
        }
        if disconnected_backend.is_some() {
            thread::sleep(Duration::from_millis(25));
            continue;
        }
        match source.receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(Ok(event)) => {
                record_adaptive_event(&event, filter, program_paths, scheduler, Instant::now())
            }
            Ok(Err(error)) => {
                if let Some(message) = event_errors.observe(error.to_string()) {
                    emit(WatchStatus::EventError(message));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                disconnected_backend = Some(source.backend);
            }
        }
    };
    while let Ok(status) = status_receiver.try_recv() {
        emit(status);
    }
    if worker.join().is_err() {
        return Ok(AdaptiveOutcome::Failed(
            "adaptive watch build worker panicked".to_owned(),
        ));
    }
    match disconnected_backend {
        Some(WatchBackend::Native) => {
            *source =
                start_single_event_source(options, root, program_paths, WatchBackend::Polling)?;
            emit(WatchStatus::Backend {
                backend: WatchBackend::Polling,
                fallback_error: Some("native filesystem event channel disconnected".to_owned()),
                poll_interval: options.poll_interval,
            });
        }
        Some(WatchBackend::Polling) => return Err(WatchError::Disconnected),
        None => {}
    }
    Ok(outcome)
}

fn run_adaptive_work(
    options: &WatchOptions,
    root: &Path,
    program_paths: &BTreeSet<PathBuf>,
    work: &AdaptiveWork,
    emit: &mut impl FnMut(WatchStatus),
) -> AdaptiveOutcome {
    match work {
        AdaptiveWork::Initial | AdaptiveWork::Reconciliation => {
            match build_local_graph(&options.build) {
                Ok(result) => AdaptiveOutcome::Succeeded(Some(Box::new(result))),
                Err(error) => AdaptiveOutcome::Failed(error.to_string()),
            }
        }
        AdaptiveWork::Changes(paths) => {
            let deterministic = paths
                .iter()
                .filter(|path| is_deterministic(path, program_paths))
                .count();
            let semantic = paths.len().saturating_sub(deterministic);
            emit(WatchStatus::Batch {
                paths: paths.clone(),
                deterministic,
                semantic,
            });
            if semantic > 0 {
                let output_root = options.build.output_root.as_deref().unwrap_or(root);
                let output_name =
                    std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
                let flag = output_root.join(output_name).join("needs_update");
                if let Err(error) = write_text_atomic(&flag, "1") {
                    return AdaptiveOutcome::Failed(error.to_string());
                }
                emit(WatchStatus::SemanticUpdateRequired { flag });
            }
            if deterministic == 0 {
                AdaptiveOutcome::Succeeded(None)
            } else {
                match build_local_graph(&options.build) {
                    Ok(result) => AdaptiveOutcome::Succeeded(Some(Box::new(result))),
                    Err(error) => AdaptiveOutcome::Failed(error.to_string()),
                }
            }
        }
    }
}

fn start_adaptive_event_source(
    options: &WatchOptions,
    root: &Path,
    program_paths: &BTreeSet<PathBuf>,
) -> Result<EventSource, WatchError> {
    if options.force_polling {
        return start_single_event_source(options, root, program_paths, WatchBackend::Polling);
    }
    match start_single_event_source(options, root, program_paths, WatchBackend::Native) {
        Ok(source) => Ok(source),
        Err(native_error) => {
            let mut source =
                start_single_event_source(options, root, program_paths, WatchBackend::Polling)?;
            source.fallback_error = Some(native_error.to_string());
            Ok(source)
        }
    }
}

fn start_single_event_source(
    options: &WatchOptions,
    root: &Path,
    program_paths: &BTreeSet<PathBuf>,
    backend: WatchBackend,
) -> Result<EventSource, WatchError> {
    let (sender, receiver) = mpsc::channel();
    let mut watcher: Box<dyn Watcher> = match backend {
        WatchBackend::Native => Box::new(
            RecommendedWatcher::new(event_handler(sender), Config::default()).map_err(
                |source| WatchError::Start {
                    path: root.to_path_buf(),
                    source,
                },
            )?,
        ),
        WatchBackend::Polling => Box::new(
            PollWatcher::new(
                event_handler(sender),
                Config::default()
                    .with_poll_interval(options.poll_interval)
                    .with_compare_contents(true),
            )
            .map_err(|source| WatchError::Start {
                path: root.to_path_buf(),
                source,
            })?,
        ),
    };
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|source| WatchError::Start {
            path: root.to_path_buf(),
            source,
        })?;
    attach_external_paths(&mut *watcher, root, program_paths)?;
    Ok(EventSource {
        _watcher: watcher,
        receiver,
        backend,
        fallback_error: None,
    })
}

fn event_handler(sender: Sender<EventMessage>) -> impl FnMut(EventMessage) + Send + 'static {
    move |event| {
        let _result = sender.send(event);
    }
}

fn attach_external_paths(
    watcher: &mut dyn Watcher,
    root: &Path,
    program_paths: &BTreeSet<PathBuf>,
) -> Result<(), WatchError> {
    for parent in program_paths
        .iter()
        .filter(|path| !path.starts_with(root))
        .filter_map(|path| path.parent())
        .collect::<BTreeSet<_>>()
    {
        watcher
            .watch(parent, RecursiveMode::NonRecursive)
            .map_err(|source| WatchError::Start {
                path: parent.to_path_buf(),
                source,
            })?;
    }
    Ok(())
}

fn record_adaptive_event(
    event: &Event,
    filter: &WatchPathFilter,
    program_paths: &BTreeSet<PathBuf>,
    scheduler: &mut WatchScheduler,
    now: Instant,
) {
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    for path in event
        .paths
        .iter()
        .filter(|path| filter.allows(path) || program_paths.contains(*path))
    {
        scheduler.record(path.clone(), now);
    }
}

#[derive(Default)]
struct RepeatedError {
    last: Option<String>,
    count: u32,
}

impl RepeatedError {
    fn observe(&mut self, error: String) -> Option<String> {
        if self.last.as_deref() == Some(error.as_str()) {
            self.count = self.count.saturating_add(1);
            if self.count.is_power_of_two() {
                return Some(format!("{error} (repeated {} times)", self.count));
            }
            return None;
        }
        self.last = Some(error.clone());
        self.count = 1;
        Some(error)
    }

    fn clear(&mut self) {
        self.last = None;
        self.count = 0;
    }
}

fn legacy_next_timeout(last: Option<Instant>, debounce: Duration, poll: Duration) -> Duration {
    last.map_or(poll, |instant| {
        debounce.saturating_sub(instant.elapsed()).min(poll)
    })
}

fn collect_event(
    event: &Event,
    filter: &WatchPathFilter,
    program_paths: &BTreeSet<PathBuf>,
    pending: &mut BTreeSet<PathBuf>,
) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    let before = pending.len();
    pending.extend(
        event
            .paths
            .iter()
            .filter(|path| filter.allows(path) || program_paths.contains(*path))
            .cloned(),
    );
    pending.len() != before
}

fn process_batch(
    options: &WatchOptions,
    root: &Path,
    program_paths: &BTreeSet<PathBuf>,
    paths: Vec<PathBuf>,
    emit: &mut impl FnMut(WatchStatus),
) -> Result<(), WatchError> {
    let deterministic = paths
        .iter()
        .filter(|path| is_deterministic(path, program_paths))
        .count();
    let semantic = paths.len().saturating_sub(deterministic);
    emit(WatchStatus::Batch {
        paths,
        deterministic,
        semantic,
    });
    if deterministic > 0 {
        match build_local_graph(&options.build) {
            Ok(result) => emit(WatchStatus::Rebuilt(Box::new(result))),
            Err(error) => emit(WatchStatus::RebuildError(error.to_string())),
        }
    }
    if semantic > 0 {
        let output_root = options.build.output_root.as_deref().unwrap_or(root);
        let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
        let flag = output_root.join(output_name).join("needs_update");
        write_text_atomic(&flag, "1")?;
        emit(WatchStatus::SemanticUpdateRequired { flag });
    }
    Ok(())
}

fn is_deterministic(path: &Path, program_paths: &BTreeSet<PathBuf>) -> bool {
    program_paths.contains(path)
        || classify_file(path).is_some_and(|kind| {
            kind == FileType::Code
                || (kind == FileType::Document && Registry::resolve(path).is_some())
        })
}

fn program_watch_paths(root: &Path, options: &BuildOptions) -> BTreeSet<PathBuf> {
    if !options.program_analysis {
        return BTreeSet::new();
    }
    let mut artifacts = vec![root.join("index.scip")];
    artifacts.extend(options.program_artifacts.iter().map(|path| {
        if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        }
    }));
    artifacts
        .into_iter()
        .flat_map(|artifact| {
            let mut companion_name = artifact
                .file_name()
                .map_or_else(std::ffi::OsString::new, std::ffi::OsString::from);
            companion_name.push(".compass-manifest.json");
            let companion = artifact.with_file_name(companion_name);
            [artifact, companion]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use compass_ir::hex_sha256;
    use protobuf::{EnumOrUnknown, Message, MessageField};
    use scip::types::{Index, Metadata, TextEncoding, ToolInfo};

    use super::*;

    #[test]
    fn watch_rebuilds_code_and_flags_semantic_changes() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().to_path_buf();
        fs::write(root.join("main.py"), "def before():\n    return 1\n")?;
        let mut initial = BuildOptions::new(&root);
        initial.no_viz = true;
        build_local_graph(&initial)?;

        let stop = Arc::new(AtomicBool::new(false));
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_statuses = Arc::clone(&statuses);
        let mut options = WatchOptions::new(&root);
        options.build.no_viz = true;
        options.debounce = Duration::from_millis(100);
        options.poll_interval = Duration::from_millis(50);
        options.force_polling = true;
        options.adaptive = false;
        let handle = thread::spawn(move || {
            watch_local_graph(&options, &thread_stop, |status| {
                if let Ok(mut values) = thread_statuses.lock() {
                    values.push(status);
                }
            })
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if statuses.lock().is_ok_and(|values| {
                values
                    .iter()
                    .any(|status| matches!(status, WatchStatus::Watching { .. }))
            }) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(statuses.lock().is_ok_and(|values| {
            values
                .iter()
                .any(|status| matches!(status, WatchStatus::Watching { .. }))
        }));
        // PollWatcher establishes its first metadata snapshot asynchronously.
        // Wait past two poll intervals so the writes are guaranteed to be
        // compared against that baseline on slow CI hosts.
        thread::sleep(Duration::from_millis(150));
        fs::write(
            root.join("main.py"),
            "def after_change():\n    return 200\n",
        )?;
        fs::write(root.join("paper.pdf"), b"%PDF-1.4\n")?;

        let mut complete = false;
        while Instant::now() < deadline {
            complete = statuses.lock().is_ok_and(|values| {
                values
                    .iter()
                    .any(|status| matches!(status, WatchStatus::Rebuilt(_)))
                    && values
                        .iter()
                        .any(|status| matches!(status, WatchStatus::SemanticUpdateRequired { .. }))
            });
            if complete {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        stop.store(true, Ordering::Release);
        let result = handle.join().map_err(|_| "watch thread panicked")?;
        result?;

        assert!(complete, "watch statuses: {:?}", statuses.lock());
        assert!(root.join("compass-out/needs_update").is_file());
        let graph_path =
            compass_files::BuildGuard::resolve_artifact(&root.join("compass-out"), "graph.json")?;
        let graph = compass_model::code_graph::GraphDocument::load(&graph_path)?;
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.label() == "after_change()"),
            "labels: {:?}",
            graph
                .nodes
                .iter()
                .map(|node| node.label())
                .collect::<Vec<_>>()
        );
        let statuses = statuses.lock().map_err(|_| "status mutex poisoned")?;
        let saw_deterministic = statuses.iter().any(|status| {
            matches!(
                status,
                WatchStatus::Batch { deterministic, .. } if *deterministic > 0
            )
        });
        let saw_semantic = statuses.iter().any(|status| {
            matches!(
                status,
                WatchStatus::Batch { semantic, .. } if *semantic > 0
            )
        });
        assert!(
            saw_deterministic && saw_semantic,
            "watch statuses: {statuses:?}"
        );
        Ok(())
    }

    #[test]
    fn watch_rebuilds_for_external_scip_and_companion_manifest_changes()
    -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("root");
        let artifacts = directory.path().join("artifacts");
        fs::create_dir_all(root.join("src"))?;
        fs::create_dir(&artifacts)?;
        fs::write(root.join("src/lib.rs"), "pub fn run() {}\n")?;
        let artifact = artifacts.join("index.scip");
        write_watch_scip(&artifact, "1.0", false)?;

        let mut initial = BuildOptions::new(&root);
        initial.no_cluster = true;
        initial.no_viz = true;
        initial.program_analysis = true;
        initial.program_artifacts = vec![artifact.clone()];
        build_local_graph(&initial)?;

        let stop = Arc::new(AtomicBool::new(false));
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_statuses = Arc::clone(&statuses);
        let mut options = WatchOptions::new(&root);
        options.build = initial;
        options.debounce = Duration::from_millis(100);
        options.poll_interval = Duration::from_millis(50);
        options.force_polling = true;
        options.adaptive = false;
        let handle = thread::spawn(move || {
            watch_local_graph(&options, &thread_stop, |status| {
                if let Ok(mut values) = thread_statuses.lock() {
                    values.push(status);
                }
            })
        });

        wait_for_status(&statuses, |status| {
            matches!(status, WatchStatus::Watching { .. })
        })?;
        thread::sleep(Duration::from_millis(150));
        write_watch_scip(&artifact, "2.0", false)?;
        wait_for_rebuild_count(&statuses, 1)?;

        let companion = watch_companion_path(&artifact);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&companion)?)?;
        fs::write(&companion, serde_json::to_string_pretty(&value)?)?;
        wait_for_rebuild_count(&statuses, 2)?;

        stop.store(true, Ordering::Release);
        handle.join().map_err(|_| "watch thread panicked")??;
        let statuses = statuses.lock().map_err(|_| "status mutex poisoned")?;
        assert!(
            statuses
                .iter()
                .filter(|status| matches!(
                    status,
                    WatchStatus::Batch {
                        deterministic,
                        semantic: 0,
                        ..
                    } if *deterministic > 0
                ))
                .count()
                >= 2,
            "watch statuses: {statuses:?}"
        );
        Ok(())
    }

    #[test]
    fn adaptive_watch_synchronizes_before_waiting_for_changes() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().to_path_buf();
        fs::write(root.join("main.rs"), "fn synchronized() {}\n")?;

        let stop = Arc::new(AtomicBool::new(false));
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_statuses = Arc::clone(&statuses);
        let mut options = WatchOptions::new(&root);
        options.build.no_cluster = true;
        options.build.no_viz = true;
        options.poll_interval = Duration::from_millis(50);
        options.force_polling = true;
        let handle = thread::spawn(move || {
            watch_local_graph(&options, &thread_stop, |status| {
                if let Ok(mut values) = thread_statuses.lock() {
                    values.push(status);
                }
            })
        });

        wait_for_status(&statuses, |status| {
            matches!(status, WatchStatus::Rebuilt(_))
        })?;
        stop.store(true, Ordering::Release);
        handle.join().map_err(|_| "watch thread panicked")??;

        let graph_path =
            compass_files::BuildGuard::resolve_artifact(&root.join("compass-out"), "graph.json")?;
        let graph = compass_model::code_graph::GraphDocument::load(&graph_path)?;
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.label() == "synchronized()")
        );
        let statuses = statuses.lock().map_err(|_| "status mutex poisoned")?;
        assert!(
            statuses
                .iter()
                .any(|status| matches!(status, WatchStatus::Synchronizing))
        );
        assert!(statuses.iter().any(|status| matches!(
            status,
            WatchStatus::Backend {
                backend: WatchBackend::Polling,
                fallback_error: None,
                ..
            }
        )));
        Ok(())
    }

    #[test]
    fn adaptive_watch_retries_a_transient_initial_build_failure() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().to_path_buf();
        fs::write(root.join("main.rs"), "fn recovered() {}\n")?;
        fs::write(root.join("compass-out"), "temporarily obstructed\n")?;

        let stop = Arc::new(AtomicBool::new(false));
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let thread_stop = Arc::clone(&stop);
        let thread_statuses = Arc::clone(&statuses);
        let mut options = WatchOptions::new(&root);
        options.build.no_cluster = true;
        options.build.no_viz = true;
        options.poll_interval = Duration::from_millis(50);
        options.force_polling = true;
        let handle = thread::spawn(move || {
            watch_local_graph(&options, &thread_stop, |status| {
                if let Ok(mut values) = thread_statuses.lock() {
                    values.push(status);
                }
            })
        });

        wait_for_status(&statuses, |status| {
            matches!(status, WatchStatus::RetryScheduled { .. })
        })?;
        fs::remove_file(root.join("compass-out"))?;
        wait_for_status(&statuses, |status| {
            matches!(status, WatchStatus::Rebuilt(_))
        })?;
        stop.store(true, Ordering::Release);
        handle.join().map_err(|_| "watch thread panicked")??;

        assert!(
            compass_files::BuildGuard::resolve_artifact(&root.join("compass-out"), "graph.json")?
                .is_file()
        );
        let statuses = statuses.lock().map_err(|_| "status mutex poisoned")?;
        assert!(statuses.iter().any(|status| matches!(
            status,
            WatchStatus::Building {
                reason: WatchBuildReason::Retry
            }
        )));
        Ok(())
    }

    fn write_watch_scip(
        artifact: &Path,
        version: &str,
        pretty_manifest: bool,
    ) -> Result<(), Box<dyn Error>> {
        let mut tool = ToolInfo::new();
        tool.name = "watch-fixture".to_owned();
        tool.version = version.to_owned();
        let mut metadata = Metadata::new();
        metadata.tool_info = MessageField::some(tool);
        metadata.text_document_encoding = EnumOrUnknown::new(TextEncoding::UTF8);
        let mut index = Index::new();
        index.metadata = MessageField::some(metadata);
        let bytes = index.write_to_bytes()?;
        fs::write(artifact, &bytes)?;
        let manifest = serde_json::json!({
            "schema": "compass.scip-manifest/1",
            "index_sha256": hex_sha256(&bytes),
            "documents": {},
        });
        fs::write(
            watch_companion_path(artifact),
            if pretty_manifest {
                serde_json::to_string_pretty(&manifest)?
            } else {
                serde_json::to_string(&manifest)?
            },
        )?;
        Ok(())
    }

    fn watch_companion_path(artifact: &Path) -> PathBuf {
        let mut name = artifact
            .file_name()
            .map_or_else(std::ffi::OsString::new, std::ffi::OsString::from);
        name.push(".compass-manifest.json");
        artifact.with_file_name(name)
    }

    fn wait_for_status(
        statuses: &Arc<Mutex<Vec<WatchStatus>>>,
        predicate: impl Fn(&WatchStatus) -> bool,
    ) -> Result<(), Box<dyn Error>> {
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            if statuses
                .lock()
                .is_ok_and(|values| values.iter().any(&predicate))
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(25));
        }
        Err(format!("timed out waiting for watch status: {:?}", statuses.lock()).into())
    }

    fn wait_for_rebuild_count(
        statuses: &Arc<Mutex<Vec<WatchStatus>>>,
        expected: usize,
    ) -> Result<(), Box<dyn Error>> {
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            if statuses.lock().is_ok_and(|values| {
                values
                    .iter()
                    .filter(|status| matches!(status, WatchStatus::Rebuilt(_)))
                    .count()
                    >= expected
            }) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(25));
        }
        Err(format!(
            "timed out waiting for {expected} rebuilds: {:?}",
            statuses.lock()
        )
        .into())
    }
}
