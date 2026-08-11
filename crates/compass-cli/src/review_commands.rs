use std::path::PathBuf;

use compass_history::{ExtractionFingerprint, Repository};
use compass_pr_intelligence::{Completeness, MergeOutcome, RepositoryIdentity};
use compass_prs::{
    ChangeRequestSource, GithubChangeRequestSource, LocalGitChangeRequestSource, SystemRunner,
    detect_repository_identity,
};

use crate::{Outcome, history_commands};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Format {
    Text,
    Json,
    Markdown,
    Sarif,
}

struct Options {
    base: Option<String>,
    head: Option<String>,
    pull_request: Option<u64>,
    report_pull_request: Option<u64>,
    repository: Option<(String, String)>,
    host: String,
    fingerprint: Option<String>,
    format: Format,
    output: Option<PathBuf>,
    max_findings: Option<usize>,
    max_output_bytes: Option<usize>,
}

pub(crate) fn command(args: &[String]) -> Outcome {
    if args
        .iter()
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
    {
        return crate::help::request(
            &["review".to_owned(), "--help".to_owned()],
            crate::help::HelpStyle::Plain,
        )
        .unwrap_or_else(|| Outcome::failure("error: review help is unavailable".to_owned()));
    }
    match execute(args) {
        Ok(message) => Outcome::success(message),
        Err(ReviewCommandError::Usage(message)) => {
            Outcome::failure_with_code(format!("error: {message}"), 2)
        }
        Err(ReviewCommandError::Runtime(message)) => Outcome::failure(format!("error: {message}")),
    }
}

fn execute(args: &[String]) -> Result<String, ReviewCommandError> {
    let options = parse(args).map_err(ReviewCommandError::Usage)?;
    let current = std::env::current_dir().map_err(runtime)?;
    let repository = Repository::discover(&current).map_err(runtime)?;
    let request = if let Some(number) = options.pull_request {
        let (owner, name) = options.repository.as_ref().ok_or_else(|| {
            ReviewCommandError::Usage("--pr requires --repo OWNER/REPO".to_owned())
        })?;
        GithubChangeRequestSource::new(
            &SystemRunner,
            repository.root(),
            &options.host,
            owner,
            name,
            number,
        )
        .capture()
        .map_err(runtime)?
    } else {
        let base = options.base.as_deref().ok_or_else(|| {
            ReviewCommandError::Usage("local review requires --base REV".to_owned())
        })?;
        let head = options.head.as_deref().ok_or_else(|| {
            ReviewCommandError::Usage("local review requires --head REV".to_owned())
        })?;
        let identity = match options.repository.as_ref() {
            Some((owner, name)) => RepositoryIdentity {
                forge: "github".to_owned(),
                host: options.host.clone(),
                owner: owner.clone(),
                name: name.clone(),
            },
            None => {
                detect_repository_identity(&SystemRunner, repository.root()).map_err(runtime)?
            }
        };
        let source = LocalGitChangeRequestSource::new(
            &SystemRunner,
            repository.root(),
            identity,
            base,
            head,
        );
        match options.report_pull_request {
            Some(number) => source.with_pull_request_number(number).capture(),
            None => source.capture(),
        }
        .map_err(runtime)?
    };
    let old = repository
        .resolve(&request.revisions.target_head)
        .map_err(runtime)?;
    let comparison_revision = request
        .revisions
        .merge_result
        .object_id()
        .unwrap_or(&request.revisions.pull_request_head);
    let new = repository.resolve(comparison_revision).map_err(runtime)?;
    let resolved = history_commands::resolve_comparable_pair(
        &repository,
        old,
        new,
        options.fingerprint.as_deref(),
    )
    .map_err(ReviewCommandError::Runtime)?;
    let report = compass_core::review_change_request_exact(
        &repository,
        &resolved.history,
        &request,
        &resolved.old,
        &resolved.new,
        Completeness::LocalExact,
    )
    .map_err(runtime)?;
    let rendered = match options.format {
        Format::Text => compass_output::render_review_text(&report).map_err(runtime)?,
        Format::Json => compass_output::render_review_json(&report).map_err(runtime)?,
        Format::Markdown => {
            compass_output::render_review_markdown_bounded(
                &report,
                options.max_findings.unwrap_or(report.findings.len()),
                options
                    .max_output_bytes
                    .unwrap_or(compass_output::MAX_REVIEW_RENDER_BYTES),
            )
            .map_err(runtime)?
            .content
        }
        Format::Sarif => compass_output::render_review_sarif(&report).map_err(runtime)?,
    };
    if let Some(path) = options.output {
        compass_files::write_text_atomic(&path, &rendered).map_err(runtime)?;
        Ok(format!("PR review written to {}", path.display()))
    } else {
        Ok(rendered)
    }
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut base = None;
    let mut head = None;
    let mut pull_request = None;
    let mut report_pull_request = None;
    let mut repository = None;
    let mut host = "github.com".to_owned();
    let mut host_set = false;
    let mut fingerprint = None;
    let mut format = Format::Text;
    let mut format_set = false;
    let mut output = None;
    let mut max_findings = None;
    let mut max_output_bytes = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        let (name, inline) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(name, value)| {
                (name, Some(value))
            });
        match name {
            "--base" => set_string(&mut base, "--base", inline, args, &mut index)?,
            "--head" => set_string(&mut head, "--head", inline, args, &mut index)?,
            "--pr" => {
                let value = value("--pr", inline, args, &mut index)?;
                let number = value
                    .parse::<u64>()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or_else(|| "--pr must be a positive integer".to_owned())?;
                if pull_request.replace(number).is_some() {
                    return Err("duplicate --pr".to_owned());
                }
            }
            "--pull-request-number" => {
                let value = value("--pull-request-number", inline, args, &mut index)?;
                let number = value
                    .parse::<u64>()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or_else(|| "--pull-request-number must be a positive integer".to_owned())?;
                if report_pull_request.replace(number).is_some() {
                    return Err("duplicate --pull-request-number".to_owned());
                }
            }
            "--repo" | "-R" => {
                let raw = value(name, inline, args, &mut index)?;
                if repository.replace(parse_repository(&raw)?).is_some() {
                    return Err("duplicate --repo".to_owned());
                }
            }
            "--host" => {
                let raw = value("--host", inline, args, &mut index)?;
                if host_set {
                    return Err("duplicate --host".to_owned());
                }
                validate_host(&raw)?;
                host = raw;
                host_set = true;
            }
            "--fingerprint" => {
                let raw = value("--fingerprint", inline, args, &mut index)?;
                raw.parse::<ExtractionFingerprint>()
                    .map_err(|_| "--fingerprint must be a lowercase SHA-256 digest".to_owned())?;
                if fingerprint.replace(raw).is_some() {
                    return Err("duplicate --fingerprint".to_owned());
                }
            }
            "--format" => {
                let raw = value("--format", inline, args, &mut index)?;
                if format_set {
                    return Err("duplicate --format".to_owned());
                }
                format = parse_format(&raw)?;
                format_set = true;
            }
            "--output" => {
                let raw = value("--output", inline, args, &mut index)?;
                if output.replace(PathBuf::from(raw)).is_some() {
                    return Err("duplicate --output".to_owned());
                }
            }
            "--max-findings" => {
                let raw = value("--max-findings", inline, args, &mut index)?;
                let parsed = parse_positive(&raw, "--max-findings")?;
                if max_findings.replace(parsed).is_some() {
                    return Err("duplicate --max-findings".to_owned());
                }
            }
            "--max-output-bytes" => {
                let raw = value("--max-output-bytes", inline, args, &mut index)?;
                let parsed = parse_positive(&raw, "--max-output-bytes")?;
                if parsed > compass_output::MAX_REVIEW_RENDER_BYTES {
                    return Err(format!(
                        "--max-output-bytes must not exceed {}",
                        compass_output::MAX_REVIEW_RENDER_BYTES
                    ));
                }
                if max_output_bytes.replace(parsed).is_some() {
                    return Err("duplicate --max-output-bytes".to_owned());
                }
            }
            value if value.starts_with('-') => return Err(format!("unknown option {value}")),
            value => return Err(format!("unexpected positional argument {value:?}")),
        }
        index += 1;
    }
    let local = base.is_some() || head.is_some();
    let github = pull_request.is_some();
    if local && github {
        return Err("--pr conflicts with --base/--head".to_owned());
    }
    if local && (base.is_none() || head.is_none()) {
        return Err("local review requires both --base and --head".to_owned());
    }
    if github && repository.is_none() {
        return Err("GitHub review requires --repo OWNER/REPO".to_owned());
    }
    if !local && !github {
        return Err("review requires --base/--head or --pr/--repo".to_owned());
    }
    if report_pull_request.is_some() && !local {
        return Err("--pull-request-number is only valid with --base/--head".to_owned());
    }
    if host_set && repository.is_none() {
        return Err("--host requires --repo".to_owned());
    }
    if output
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err("--output requires a non-empty path".to_owned());
    }
    if format != Format::Markdown && (max_findings.is_some() || max_output_bytes.is_some()) {
        return Err(
            "--max-findings and --max-output-bytes are only valid with --format markdown"
                .to_owned(),
        );
    }
    Ok(Options {
        base,
        head,
        pull_request,
        report_pull_request,
        repository,
        host,
        fingerprint,
        format,
        output,
        max_findings,
        max_output_bytes,
    })
}

fn value(
    name: &str,
    inline: Option<&str>,
    args: &[String],
    index: &mut usize,
) -> Result<String, String> {
    let raw = match inline {
        Some(value) => value,
        None => {
            *index += 1;
            args.get(*index)
                .map(String::as_str)
                .ok_or_else(|| format!("{name} requires a value"))?
        }
    };
    if raw.is_empty() {
        Err(format!("{name} requires a non-empty value"))
    } else {
        Ok(raw.to_owned())
    }
}

fn set_string(
    slot: &mut Option<String>,
    name: &str,
    inline: Option<&str>,
    args: &[String],
    index: &mut usize,
) -> Result<(), String> {
    let raw = value(name, inline, args, index)?;
    if slot.replace(raw).is_some() {
        Err(format!("duplicate {name}"))
    } else {
        Ok(())
    }
}

fn parse_repository(value: &str) -> Result<(String, String), String> {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    let valid_component = |component: &str| {
        !component.is_empty()
            && component.len() <= 255
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    if !valid_component(owner) || !valid_component(repository) || parts.next().is_some() {
        return Err("--repo must be exactly OWNER/REPO".to_owned());
    }
    Ok((owner.to_owned(), repository.to_owned()))
}

fn validate_host(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 253
        || value.contains('/')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        Err("--host must be a bare DNS hostname".to_owned())
    } else {
        Ok(())
    }
}

fn parse_format(value: &str) -> Result<Format, String> {
    match value {
        "text" => Ok(Format::Text),
        "json" => Ok(Format::Json),
        "markdown" => Ok(Format::Markdown),
        "sarif" => Ok(Format::Sarif),
        _ => Err("--format must be text, json, markdown, or sarif".to_owned()),
    }
}

fn parse_positive(value: &str, name: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("{name} must be a positive integer"))
}

enum ReviewCommandError {
    Usage(String),
    Runtime(String),
}

fn runtime(error: impl ToString) -> ReviewCommandError {
    ReviewCommandError::Runtime(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_require_one_complete_input_mode() {
        assert!(parse(&[]).is_err());
        assert!(parse(&["--base=main".to_owned()]).is_err());
        assert!(
            parse(&[
                "--base=main".to_owned(),
                "--head=HEAD".to_owned(),
                "--pull-request-number=1".to_owned(),
                "--repo=o/r".to_owned(),
            ])
            .is_ok()
        );
        assert!(
            parse(&[
                "--base=main".to_owned(),
                "--head=HEAD".to_owned(),
                "--pr=1".to_owned(),
                "--repo=o/r".to_owned(),
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--base=main".to_owned(),
                "--head=HEAD".to_owned(),
                "--format=sarif".to_owned(),
                "--output=review.sarif".to_owned(),
            ])
            .is_ok()
        );
        assert!(
            parse(&[
                "--pr=42".to_owned(),
                "--repo=owner/repo".to_owned(),
                "--host=github.example.com".to_owned(),
            ])
            .is_ok()
        );
    }

    #[test]
    fn options_reject_duplicates_invalid_hosts_and_formats() {
        for arguments in [
            vec!["--pr=1", "--repo=o/r", "--repo=x/y"],
            vec!["--pr=1", "--repo=o/r", "--host=https://github.com"],
            vec!["--base=a", "--head=b", "--format=html"],
            vec!["--base=a", "--head=b", "--unknown"],
        ] {
            assert!(parse(&arguments.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
    }

    #[test]
    fn options_accept_values_as_separate_arguments() -> Result<(), String> {
        let options = parse(&[
            "--base".to_owned(),
            "main".to_owned(),
            "--head".to_owned(),
            "feature".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ])?;
        assert_eq!(options.base.as_deref(), Some("main"));
        assert_eq!(options.head.as_deref(), Some("feature"));
        assert_eq!(options.format, Format::Json);
        Ok(())
    }
}
