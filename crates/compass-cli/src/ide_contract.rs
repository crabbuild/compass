use std::io::{self, Write};

use serde::Serialize;

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
