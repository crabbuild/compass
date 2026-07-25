use std::ffi::OsString;
use std::io::{self, Write};

use serde::Serialize;

use crate::Outcome;

pub const CAPABILITY_SCHEMA: &str = "compass.ide.capabilities/1";
pub const PROGRESS_SCHEMA: &str = "compass.ide.progress/1";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressState {
    Started,
    Running,
    Retrying,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Serialize)]
pub struct ProgressEvent<'a> {
    pub schema: &'static str,
    pub operation_id: &'a str,
    pub operation: &'a str,
    pub state: ProgressState,
    pub phase: &'a str,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub message: &'a str,
    pub terminal: bool,
}

pub struct ProgressWriter<W> {
    writer: W,
    terminal_written: bool,
}

impl<W: Write> ProgressWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            terminal_written: false,
        }
    }

    pub fn write(&mut self, event: &ProgressEvent<'_>) -> io::Result<()> {
        if self.terminal_written {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a terminal progress event was already written",
            ));
        }
        serde_json::to_writer(&mut self.writer, event)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        self.terminal_written = event.terminal;
        Ok(())
    }
}

pub fn take_jsonl_events(arguments: &mut Vec<OsString>) -> Result<bool, String> {
    let mut enabled = false;
    let mut index = 0;
    while index < arguments.len() {
        let value = arguments[index].to_string_lossy();
        if value == "--events" {
            let format = arguments
                .get(index + 1)
                .ok_or_else(|| "error: --events requires jsonl".to_owned())?
                .to_string_lossy();
            if format != "jsonl" {
                return Err(format!(
                    "error: unsupported event format {format:?}; expected jsonl"
                ));
            }
            arguments.drain(index..=index + 1);
            enabled = true;
            continue;
        }
        if let Some(format) = value.strip_prefix("--events=") {
            if format != "jsonl" {
                return Err(format!(
                    "error: unsupported event format {format:?}; expected jsonl"
                ));
            }
            arguments.remove(index);
            enabled = true;
            continue;
        }
        index += 1;
    }
    Ok(enabled)
}

#[must_use]
pub fn progress_outcome(operation: &str, outcome: Outcome) -> Outcome {
    let operation_id = format!("{operation}-{}", std::process::id());
    let started = ProgressEvent {
        schema: PROGRESS_SCHEMA,
        operation_id: &operation_id,
        operation,
        state: ProgressState::Started,
        phase: "starting",
        current: None,
        total: None,
        message: "Compass operation started",
        terminal: false,
    };
    let succeeded = outcome.code == 0;
    let terminal = ProgressEvent {
        schema: PROGRESS_SCHEMA,
        operation_id: &operation_id,
        operation,
        state: if succeeded {
            ProgressState::Succeeded
        } else {
            ProgressState::Failed
        },
        phase: if succeeded { "complete" } else { "failed" },
        current: None,
        total: None,
        message: if succeeded {
            "Compass operation completed"
        } else {
            "Compass operation failed"
        },
        terminal: true,
    };
    let stdout = [started, terminal]
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_or_else(
            |error| format!(r#"{{"schema":"{PROGRESS_SCHEMA}","state":"failed","terminal":true,"message":"could not serialize progress: {error}"}}"#),
            |events| events.join("\n"),
        );
    let human_output = [outcome.stdout, outcome.stderr]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Outcome {
        code: outcome.code,
        stdout,
        stderr: human_output,
        stdout_trailing_newline: true,
        stderr_trailing_newline: true,
        html_output: None,
    }
}
