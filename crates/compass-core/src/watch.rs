use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use compass_files::{DetectOptions, FileType, WatchPathFilter, classify_file, write_text_atomic};
use compass_languages::Registry;
use notify::event::EventKind;
use notify::{Config, Event, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{BuildOptions, BuildResult, CoreError, build_local_graph};

#[derive(Clone, Debug)]
pub struct WatchOptions {
    pub build: BuildOptions,
    pub debounce: Duration,
    pub poll_interval: Duration,
    pub force_polling: bool,
    pub graphify_compatibility: bool,
}

impl WatchOptions {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            build: BuildOptions::new(root),
            debounce: Duration::from_secs(3),
            poll_interval: Duration::from_millis(500),
            force_polling: false,
            graphify_compatibility: false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum WatchStatus {
    Watching {
        root: PathBuf,
        debounce: Duration,
    },
    Batch {
        paths: Vec<PathBuf>,
        deterministic: usize,
        semantic: usize,
    },
    Rebuilt(Box<BuildResult>),
    SemanticUpdateRequired {
        flag: PathBuf,
    },
    EventError(String),
    RebuildError(String),
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
    mut emit: impl FnMut(WatchStatus),
) -> Result<(), WatchError> {
    if !options.build.root.exists() {
        return Err(CoreError::MissingRoot(options.build.root.clone()).into());
    }
    let root =
        fs::canonicalize(&options.build.root).map_err(|source| compass_files::FileError::Io {
            path: options.build.root.clone(),
            source,
        })?;
    let output_name = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let filter = WatchPathFilter::new(
        &root,
        &DetectOptions {
            gitignore: options.build.gitignore,
            extra_excludes: options.build.extra_excludes.clone(),
            output_name,
            ..DetectOptions::default()
        },
    )?;
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
    emit(WatchStatus::Watching {
        root: root.clone(),
        debounce: options.debounce,
    });

    let mut pending = BTreeSet::new();
    let mut last_change = None;
    while !stop.load(Ordering::Acquire) {
        let timeout = next_timeout(last_change, options.debounce, options.poll_interval);
        match receiver.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                if collect_event(&event, &filter, &mut pending) {
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
            process_batch(options, &root, paths, &mut emit)?;
        }
    }
    emit(WatchStatus::Stopped);
    Ok(())
}

fn next_timeout(last: Option<Instant>, debounce: Duration, poll: Duration) -> Duration {
    last.map_or(poll, |instant| {
        debounce.saturating_sub(instant.elapsed()).min(poll)
    })
}

fn collect_event(event: &Event, filter: &WatchPathFilter, pending: &mut BTreeSet<PathBuf>) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    let before = pending.len();
    pending.extend(
        event
            .paths
            .iter()
            .filter(|path| filter.allows(path))
            .cloned(),
    );
    pending.len() != before
}

fn process_batch(
    options: &WatchOptions,
    root: &Path,
    paths: Vec<PathBuf>,
    emit: &mut impl FnMut(WatchStatus),
) -> Result<(), WatchError> {
    let deterministic = paths
        .iter()
        .filter(|path| is_deterministic(path, options.graphify_compatibility))
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
        let output_name =
            std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
        let flag = output_root.join(output_name).join("needs_update");
        write_text_atomic(&flag, "1")?;
        emit(WatchStatus::SemanticUpdateRequired { flag });
    }
    Ok(())
}

fn is_deterministic(path: &Path, graphify_compatibility: bool) -> bool {
    classify_file(path).is_some_and(|kind| {
        kind == FileType::Code
            || (!graphify_compatibility
                && kind == FileType::Document
                && Registry::resolve(path).is_some())
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::{Arc, Mutex};
    use std::thread;

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
        assert!(root.join("graphify-out/needs_update").is_file());
        let graph = compass_model::GraphDocument::load(&root.join("graphify-out/graph.json"))?;
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
        assert!(statuses.iter().any(|status| {
            matches!(
                status,
                WatchStatus::Batch {
                    deterministic: 1,
                    semantic: 1,
                    ..
                }
            )
        }));
        Ok(())
    }
}
