use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use time::{Month, OffsetDateTime};
use trail_files::write_text_atomic;

use crate::{Frontend, Outcome};

const OUTCOMES: [&str; 3] = ["useful", "dead_end", "corrected"];
const MAX_ANSWER_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Default)]
struct SaveResultOptions {
    question: Option<String>,
    answer: Option<String>,
    answer_file: Option<PathBuf>,
    query_type: Option<String>,
    source_nodes: Vec<String>,
    outcome: Option<String>,
    correction: Option<String>,
    memory_dir: Option<PathBuf>,
}

pub(super) fn command_save_result(frontend: Frontend, args: &[String]) -> Outcome {
    let options = match parse_options(frontend, args) {
        Ok(Some(options)) => options,
        Ok(None) => return Outcome::success(save_result_help(frontend)),
        Err(error) => return Outcome::failure(error),
    };
    let question = match options.question {
        Some(question) => question,
        None => {
            return Outcome::failure(
                "error: the following arguments are required: --question".to_owned(),
            );
        }
    };
    let answer = if let Some(path) = options.answer_file {
        match read_answer_file(&path) {
            Ok(answer) => answer,
            Err(error) => return Outcome::failure(error),
        }
    } else {
        match options.answer.filter(|answer| !answer.is_empty()) {
            Some(answer) => answer,
            None => {
                return Outcome::failure("error: --answer or --answer-file is required".to_owned());
            }
        }
    };
    let default_output =
        std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let memory_dir = options
        .memory_dir
        .unwrap_or_else(|| PathBuf::from(default_output).join("memory"));
    let now = OffsetDateTime::now_utc();
    match save_query_result(
        &question,
        &answer,
        &memory_dir,
        options.query_type.as_deref().unwrap_or("query"),
        &options.source_nodes,
        options.outcome.as_deref(),
        options.correction.as_deref(),
        now,
    ) {
        Ok(path) => Outcome::success(format!("Saved to {}", path.display())),
        Err(error) => Outcome::failure(error),
    }
}

fn parse_options(frontend: Frontend, args: &[String]) -> Result<Option<SaveResultOptions>, String> {
    let mut options = SaveResultOptions::default();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if matches!(argument.as_str(), "--help" | "-h") {
            return Ok(None);
        }
        if argument == "--nodes" {
            index += 1;
            while index < args.len() && !args[index].starts_with('-') {
                options.source_nodes.push(args[index].clone());
                index += 1;
            }
            continue;
        }
        let (name, inline) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(name, value)| {
                (name, Some(value))
            });
        let target = match name {
            "--question" => Some(&mut options.question),
            "--answer" => Some(&mut options.answer),
            "--type" => Some(&mut options.query_type),
            "--outcome" => Some(&mut options.outcome),
            "--correction" => Some(&mut options.correction),
            _ => None,
        };
        if let Some(target) = target {
            let value = option_value(args, &mut index, name, inline)?;
            *target = Some(value);
        } else if name == "--answer-file" {
            options.answer_file =
                Some(PathBuf::from(option_value(args, &mut index, name, inline)?));
        } else if name == "--memory-dir" {
            options.memory_dir = Some(PathBuf::from(option_value(args, &mut index, name, inline)?));
        } else {
            return Err(format!(
                "{}\nerror: unrecognized arguments: {argument}",
                save_result_help(frontend)
            ));
        }
        index += 1;
    }
    if let Some(outcome) = &options.outcome
        && !OUTCOMES.contains(&outcome.as_str())
    {
        return Err(format!(
            "error: argument --outcome: invalid choice: '{outcome}' (choose from 'useful', 'dead_end', 'corrected')"
        ));
    }
    Ok(Some(options))
}

fn option_value(
    args: &[String],
    index: &mut usize,
    name: &str,
    inline: Option<&str>,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value.to_owned());
    }
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("error: argument {name}: expected one argument"))
}

fn read_answer_file(path: &Path) -> Result<String, String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("error: could not inspect {}: {error}", path.display()))?;
    if metadata.len() > MAX_ANSWER_FILE_BYTES {
        return Err(format!(
            "error: answer file {} is {} bytes; maximum is {}",
            path.display(),
            metadata.len(),
            MAX_ANSWER_FILE_BYTES
        ));
    }
    fs::read_to_string(path)
        .map(|answer| answer.trim().to_owned())
        .map_err(|error| format!("error: could not read {}: {error}", path.display()))
}

#[allow(clippy::too_many_arguments)]
fn save_query_result(
    question: &str,
    answer: &str,
    memory_dir: &Path,
    query_type: &str,
    source_nodes: &[String],
    outcome: Option<&str>,
    correction: Option<&str>,
    now: OffsetDateTime,
) -> Result<PathBuf, String> {
    fs::create_dir_all(memory_dir)
        .map_err(|error| format!("error: could not create {}: {error}", memory_dir.display()))?;
    let slug_pattern = Regex::new(r"[^\w]").map_err(|error| error.to_string())?;
    let lowered = question.to_lowercase();
    let replaced = slug_pattern.replace_all(&lowered, "_");
    let slug = replaced
        .chars()
        .take(50)
        .collect::<String>()
        .trim_matches('_')
        .to_owned();
    let filename = format!("query_{}_{}.md", filename_timestamp(now), slug);
    let mut frontmatter = vec![
        "---".to_owned(),
        format!("type: \"{}\"", yaml_string(query_type)),
        format!("date: \"{}\"", iso_timestamp(now)),
        format!("question: \"{}\"", yaml_string(question)),
        "contributor: \"graphify\"".to_owned(),
    ];
    if let Some(outcome) = outcome.filter(|value| !value.is_empty()) {
        frontmatter.push(format!("outcome: \"{}\"", yaml_string(outcome)));
    }
    if let Some(correction) = correction.filter(|value| !value.is_empty()) {
        frontmatter.push(format!("correction: \"{}\"", yaml_string(correction)));
    }
    if !source_nodes.is_empty() {
        let nodes = source_nodes
            .iter()
            .take(10)
            .map(|node| format!("\"{}\"", yaml_string(node)))
            .collect::<Vec<_>>()
            .join(", ");
        frontmatter.push(format!("source_nodes: [{nodes}]"));
    }
    frontmatter.push("---".to_owned());
    let mut body = vec![
        String::new(),
        format!("# Q: {question}"),
        String::new(),
        "## Answer".to_owned(),
        String::new(),
        answer.to_owned(),
    ];
    if outcome.is_some_and(|value| !value.is_empty())
        || correction.is_some_and(|value| !value.is_empty())
    {
        body.extend([String::new(), "## Outcome".to_owned(), String::new()]);
        if let Some(outcome) = outcome.filter(|value| !value.is_empty()) {
            body.push(format!("- Signal: {outcome}"));
        }
        if let Some(correction) = correction.filter(|value| !value.is_empty()) {
            body.push(format!("- Correction: {correction}"));
        }
    }
    if !source_nodes.is_empty() {
        body.extend([String::new(), "## Source Nodes".to_owned(), String::new()]);
        body.extend(source_nodes.iter().map(|node| format!("- {node}")));
    }
    frontmatter.extend(body);
    let output = memory_dir.join(filename);
    write_text_atomic(&output, &frontmatter.join("\n"))
        .map_err(|error| format!("error: could not write {}: {error}", output.display()))?;
    Ok(output)
}

fn filename_timestamp(now: OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        now.year(),
        month_number(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn iso_timestamp(now: OffsetDateTime) -> String {
    let base = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        now.year(),
        month_number(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let micros = now.microsecond();
    if micros == 0 {
        format!("{base}+00:00")
    } else {
        format!("{base}.{micros:06}+00:00")
    }
}

fn month_number(month: Month) -> u8 {
    month as u8
}

fn yaml_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\0' => output.push_str("\\0"),
            '\u{2028}' => output.push_str("\\L"),
            '\u{2029}' => output.push_str("\\P"),
            control if u32::from(control) < 0x20 || control == '\u{7f}' => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\x{:02x}", u32::from(control));
            }
            other => output.push(other),
        }
    }
    output
}

pub(super) fn save_result_help(frontend: Frontend) -> String {
    let prefix = match frontend {
        Frontend::Trail => "trail graph save-result",
        Frontend::Graphify => "graphify save-result",
    };
    format!(
        "Usage: {prefix} --question Q (--answer A | --answer-file PATH) [--type T] [--nodes N1 N2 ...] [--outcome useful|dead_end|corrected] [--correction TEXT] [--memory-dir DIR]"
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use time::Date;

    use super::*;

    #[test]
    fn saved_result_matches_python_memory_format() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let date = Date::from_calendar_date(2026, Month::June, 1)?;
        let now = date.with_hms_micro(12, 34, 56, 123_456)?.assume_utc();
        let path = save_query_result(
            "path is C:\\Users and a \"quote\"",
            "line one\nline two",
            directory.path(),
            "explain",
            &["Node\"With\\Quote".to_owned()],
            Some("corrected"),
            Some("line1\nline2"),
            now,
        )?;
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("query_20260601_123456_path_is_c__users_and_a__quote.md")
        );
        assert_eq!(
            fs::read_to_string(path)?,
            "---\ntype: \"explain\"\ndate: \"2026-06-01T12:34:56.123456+00:00\"\nquestion: \"path is C:\\\\Users and a \\\"quote\\\"\"\ncontributor: \"graphify\"\noutcome: \"corrected\"\ncorrection: \"line1\\nline2\"\nsource_nodes: [\"Node\\\"With\\\\Quote\"]\n---\n\n# Q: path is C:\\Users and a \"quote\"\n\n## Answer\n\nline one\nline two\n\n## Outcome\n\n- Signal: corrected\n- Correction: line1\nline2\n\n## Source Nodes\n\n- Node\"With\\Quote"
        );
        Ok(())
    }
}
