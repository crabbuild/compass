//! Request-local advisory messages collected while readers pin their graphs.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::Outcome;
use compass_core::{GraphFreshness, graph_freshness};

type Frame = BTreeMap<PathBuf, Option<GraphFreshness>>;
thread_local! { static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) }; }

pub(super) struct Scope {
    active: bool,
}
impl Scope {
    pub(super) fn begin() -> Self {
        let active = FRAMES.with(|frames| {
            let mut frames = frames.borrow_mut();
            if frames.len() >= 16 {
                false
            } else {
                frames.push(BTreeMap::new());
                true
            }
        });
        Self { active }
    }
    pub(super) fn finish(mut self, mut output: Outcome) -> Outcome {
        if self.active {
            let frame = FRAMES.with(|frames| frames.borrow_mut().pop());
            self.active = false;
            for warning in frame
                .into_iter()
                .flat_map(BTreeMap::into_values)
                .flatten()
                .filter_map(|value| value.warning())
            {
                if !output.stderr.is_empty() {
                    output.stderr.push('\n');
                }
                output.stderr.push_str(&warning);
                output.stderr_trailing_newline = true;
            }
        }
        output
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if self.active {
            FRAMES.with(|frames| {
                frames.borrow_mut().pop();
            });
        }
    }
}

pub(super) fn record(path: &Path, commit: Option<&str>) {
    record_with_mode(path, commit, false);
}

pub(super) fn record_program(path: &Path) {
    record_with_mode(path, None, true);
}

fn record_with_mode(path: &Path, commit: Option<&str>, program: bool) {
    let capture = FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        let Some(frame) = frames.last_mut() else {
            return false;
        };
        if frame.len() >= 8 || frame.contains_key(path) {
            return false;
        }
        frame.insert(path.to_owned(), None);
        true
    });
    if capture {
        let mut report = graph_freshness(path, commit);
        if program && let Some(report) = report.as_mut() {
            report.update_command.push_str(" --program");
            if report.reason.as_deref() == Some("graph has no valid build commit") {
                report.reason = Some("Program IR has no recorded build commit".into());
            }
        }
        FRAMES.with(|frames| {
            if let Some(frame) = frames.borrow_mut().last_mut() {
                frame.insert(path.to_owned(), report);
            }
        });
    }
}
