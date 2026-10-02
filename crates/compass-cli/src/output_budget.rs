//! A common budget for completed outputs, including commands without a native pager.
use crate::Outcome;
use compass_files::{read_bytes_bounded, write_json_atomic_new};
use compass_output::render_budgeted_text;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SAVED_OUTPUTS: usize = 16;
const SCHEMA: &str = "compass.saved-output/1";

#[derive(Default)]
pub(crate) struct Controls {
    pub budget: Option<usize>,
    pub machine: bool,
}

pub(crate) fn take(args: &mut Vec<String>) -> Result<Controls, String> {
    let mut controls = Controls {
        machine: machine_requested(args),
        ..Controls::default()
    };
    let mut index = 0;
    while index < args.len() {
        let (value, count) = if args[index] == "--budget" {
            (
                args.get(index + 1)
                    .ok_or("--budget requires a positive integer")?
                    .as_str(),
                2,
            )
        } else if let Some(value) = args[index].strip_prefix("--budget=") {
            (value, 1)
        } else {
            index += 1;
            continue;
        };
        if controls.budget.is_some() {
            return Err("--budget may be supplied only once".to_owned());
        }
        let budget = value
            .parse::<usize>()
            .map_err(|_| "--budget requires a positive integer")?;
        if !(32..=65_536).contains(&budget) {
            return Err("--budget must be between 32 and 65536".to_owned());
        }
        controls.budget = Some(budget);
        args.drain(index..index + count);
    }
    Ok(controls)
}

pub(crate) fn machine_requested(args: &[String]) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        let value = arg.strip_prefix("--format=").or_else(|| {
            (arg == "--format")
                .then(|| args.get(index + 1).map(String::as_str))
                .flatten()
        });
        value.is_some_and(|value| matches!(value, "json" | "agent-json" | "jsonl"))
            || matches!(arg.as_str(), "--json" | "--events=jsonl")
    })
}

fn bounded_error(message: &str, budget: usize) -> Outcome {
    let limit = budget.saturating_mul(4).saturating_sub(1);
    let text = format!("error: {message}");
    if text.len() <= limit {
        return Outcome::failure(text);
    }
    Outcome::failure("error: output unavailable; rerun with --budget 256".to_owned())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedOutput {
    schema: String,
    code: u8,
    text: String,
}

fn directory() -> Result<PathBuf, String> {
    std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map(|path| path.join(".compass/cache/output"))
        .map_err(|error| error.to_string())
}
fn identity(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn is_identity(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn save(directory: &Path, bytes: &[u8]) -> Result<String, String> {
    // Saved output may contain source excerpts. Never follow a redirected
    // cache directory or a redirected record while publishing it.
    for ancestor in directory.ancestors().take(3) {
        if ancestor.is_symlink() {
            return Err("saved-output directory is a symlink".to_owned());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let id = identity(bytes);
    let path = directory.join(format!("{id}.json"));
    if path.is_symlink() {
        return Err("saved-output record is a symlink".to_owned());
    }
    if path.exists() {
        let existing =
            read_bytes_bounded(&path, MAX_OUTPUT_BYTES).map_err(|error| error.to_string())?;
        let record: SavedOutput =
            serde_json::from_slice(&existing).map_err(|error| error.to_string())?;
        if serde_json::to_vec(&record).map_err(|error| error.to_string())? != bytes {
            return Err("saved-output digest mismatch".to_owned());
        }
        return Ok(id);
    }
    let mut entries = Vec::new();
    for (scanned, entry) in std::fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .take(129)
        .enumerate()
    {
        let entry = entry.map_err(|error| error.to_string())?;
        if scanned >= 128 {
            return Err("saved-output directory exceeds its scan bound".to_owned());
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.strip_suffix(".json").is_some_and(is_identity) {
            let metadata = entry.metadata().map_err(|error| error.to_string())?;
            entries.push((metadata.modified().ok(), entry.path()));
        }
    }
    entries.sort();
    let remove = entries
        .len()
        .saturating_add(1)
        .saturating_sub(MAX_SAVED_OUTPUTS);
    for (_, path) in entries.into_iter().take(remove) {
        std::fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    let record: SavedOutput = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    write_json_atomic_new(&path, &record, false).map_err(|error| error.to_string())?;
    Ok(id)
}

pub(crate) fn finish(mut outcome: Outcome, controls: &Controls) -> Outcome {
    let Some(budget) = controls.budget else {
        return outcome;
    };
    let bytes = outcome.stdout.len()
        + outcome.stderr.len()
        + usize::from(!outcome.stdout.is_empty() && outcome.stdout_trailing_newline)
        + usize::from(!outcome.stderr.is_empty() && outcome.stderr_trailing_newline);
    if bytes <= budget.saturating_mul(4) {
        return outcome;
    }
    if bytes as u64 > MAX_OUTPUT_BYTES {
        return bounded_error(
            "output exceeds 4 MiB; narrow the query or write it to a file",
            budget,
        );
    }
    let text = if outcome.stdout.is_empty() {
        outcome.stderr.clone()
    } else if outcome.stderr.is_empty() {
        outcome.stdout.clone()
    } else {
        format!("{}\nSTDERR:\n{}", outcome.stdout, outcome.stderr)
    };
    let record = SavedOutput {
        schema: SCHEMA.to_owned(),
        code: outcome.code,
        text,
    };
    let result = (|| {
        let mut capture = Capture::default();
        serde_json::to_writer(&mut capture, &record).map_err(|_| "saved output exceeds 4 MiB")?;
        let bytes = capture.bytes;
        if bytes.len() as u64 > MAX_OUTPUT_BYTES {
            return Err("output exceeds the 4 MiB saved-output bound".to_owned());
        }
        let id = save(&directory()?, &bytes)?;
        if controls.machine || serde_json::from_str::<serde_json::Value>(&outcome.stdout).is_ok() {
            return Ok((format!("error: budget exceeded; compass output {id}"), true));
        }
        page(&record, &id, 0, budget).map(|text| (text, false))
    })();
    match result {
        Ok((text, machine)) => {
            if machine {
                return bounded_error(text.strip_prefix("error: ").unwrap_or(&text), budget);
            }
            if outcome.code == 0 {
                outcome.stdout = text;
                outcome.stderr.clear();
            } else {
                outcome.stderr = text;
                outcome.stdout.clear();
            }
            outcome
        }
        Err(error) => bounded_error(&error, budget),
    }
}

fn page(record: &SavedOutput, id: &str, start: usize, budget: usize) -> Result<String, String> {
    render_budgeted_text(&record.text, start, budget, |end| {
        format!("More: compass output {id} --offset {end} --budget {budget}")
    })
    .map(|page| page.text)
    .map_err(|error| error.to_string())
}

pub(crate) fn command(args: &[String], budget: Option<usize>) -> Outcome {
    let result = (|| {
        let id = args
            .first()
            .filter(|value| is_identity(value))
            .ok_or("output requires the saved answer ID")?;
        let offset = match &args[1..] {
            [] => 0,
            [flag, value] if flag == "--offset" => value
                .parse::<usize>()
                .map_err(|_| "--offset requires a byte offset")?,
            _ => return Err("usage: compass output ID [--offset N] [--budget N]".to_owned()),
        };
        let root = directory()?;
        for ancestor in root.ancestors().take(3) {
            if ancestor.is_symlink() {
                return Err("saved-output directory is a symlink".to_owned());
            }
        }
        let path = root.join(format!("{id}.json"));
        if path.is_symlink() {
            return Err("saved-output record is a symlink".to_owned());
        }
        let bytes = read_bytes_bounded(&path, MAX_OUTPUT_BYTES).map_err(|_| {
            "saved output missing or unreadable; rerun the original command".to_owned()
        })?;
        let record: SavedOutput =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if record.schema != SCHEMA {
            return Err("unsupported saved-output schema".to_owned());
        }
        let canonical = serde_json::to_vec(&record).map_err(|error| error.to_string())?;
        if identity(&canonical) != *id {
            return Err("saved-output digest mismatch".to_owned());
        }
        let text = page(&record, id, offset, budget.unwrap_or(2_000))?;
        Ok(Outcome {
            code: record.code,
            stdout: text,
            stderr: String::new(),
            stdout_trailing_newline: true,
            stderr_trailing_newline: true,
            html_output: None,
        })
    })();
    result.unwrap_or_else(|error| bounded_error(&error, budget.unwrap_or(2_000)))
}

/// Capture a completed initializer without allowing its output buffer to grow.
#[derive(Default)]
pub(crate) struct Capture {
    pub bytes: Vec<u8>,
    pub exceeded: bool,
}
impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > MAX_OUTPUT_BYTES as usize {
            self.exceeded = true;
            return Err(std::io::Error::other("completed output exceeds 4 MiB"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_records_are_immutable_bounded_and_detect_corruption()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let record = SavedOutput {
            schema: SCHEMA.to_owned(),
            code: 0,
            text: "é東京🙂".repeat(200),
        };
        let bytes = serde_json::to_vec(&record)?;
        let id = save(root.path(), &bytes)?;
        assert_eq!(save(root.path(), &bytes)?, id);
        let mut start = 0;
        while start < record.text.len() {
            let rendered = render_budgeted_text(&record.text, start, 32, |end| {
                format!("More: compass output {id} --offset {end} --budget 32")
            })?;
            assert!(rendered.text.len() < 128);
            assert!(rendered.end > start);
            start = rendered.end;
        }
        std::fs::write(root.path().join(format!("{id}.json")), b"{}")?;
        assert!(save(root.path(), &bytes).is_err());
        assert!(!is_identity("../outside"));
        Ok(())
    }

    #[test]
    fn retention_and_machine_budget_controls_are_bounded() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempfile::tempdir()?;
        for index in 0..MAX_SAVED_OUTPUTS + 3 {
            save(
                root.path(),
                &serde_json::to_vec(&SavedOutput {
                    schema: SCHEMA.to_owned(),
                    code: 0,
                    text: index.to_string(),
                })?,
            )?;
        }
        assert_eq!(std::fs::read_dir(root.path())?.count(), MAX_SAVED_OUTPUTS);
        let mut args = ["query", "json", "--format", "jsonl", "--budget=32"]
            .map(str::to_owned)
            .to_vec();
        let controls = take(&mut args)?;
        assert!(controls.machine);
        assert_eq!(controls.budget, Some(32));
        assert!(!machine_requested(&[
            "search".to_owned(),
            "json".to_owned()
        ]));
        let error = bounded_error(&"x".repeat(1000), 32);
        assert!(error.stderr.len() < 128);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn redirected_cache_directories_and_records_are_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        let redirected = root.path().join("redirected");
        std::os::unix::fs::symlink(outside.path(), &redirected)?;
        let bytes = serde_json::to_vec(&SavedOutput {
            schema: SCHEMA.to_owned(),
            code: 0,
            text: "source".to_owned(),
        })?;
        assert!(save(&redirected, &bytes).is_err());
        std::os::unix::fs::symlink(
            outside.path().join("missing"),
            root.path().join(format!("{}.json", identity(&bytes))),
        )?;
        assert!(save(root.path(), &bytes).is_err());
        Ok(())
    }
}
