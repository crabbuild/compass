use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use compass_files::{
    BuildScope, DetectOptions, Detection, PROJECT_CONFIG_RELATIVE_PATH, ProjectConfig,
    ScopeMatcher, detect,
};

use crate::ide_contract::{PROGRESS_SCHEMA, ProgressEvent, ProgressState, ProgressWriter};
use crate::{
    BuildOperation, Frontend, Outcome, command_build_with_precomputed_detection, write_outcome,
};

struct InitOptions {
    root: PathBuf,
    includes: Vec<String>,
    excludes: Vec<String>,
    yes: bool,
    force: bool,
    timing: bool,
}

pub fn run_init(
    arguments: &[OsString],
    input: &mut impl BufRead,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    input_is_terminal: bool,
) -> u8 {
    run_init_with_builder(
        arguments,
        input,
        stdout,
        stderr,
        input_is_terminal,
        |_, build_arguments, detection, started| {
            command_build_with_precomputed_detection(
                Frontend::Compass,
                build_arguments,
                BuildOperation::Init,
                detection,
                started,
                None,
            )
        },
    )
}

pub fn run_init_jsonl<W: Write + Send>(
    arguments: &[OsString],
    input: &mut impl BufRead,
    stdout: W,
    stderr: &mut impl Write,
    input_is_terminal: bool,
) -> u8 {
    let operation_id = format!("init-{}", std::process::id());
    let writer = Mutex::new(ProgressWriter::new(stdout));
    let progress_error = Mutex::new(None::<String>);
    if writer
        .lock()
        .map_err(|_| ())
        .and_then(|mut writer| {
            writer
                .write(&ProgressEvent {
                    schema: PROGRESS_SCHEMA,
                    operation_id: &operation_id,
                    operation: "init",
                    state: ProgressState::Started,
                    phase: "configuring",
                    current: None,
                    total: None,
                    message: "Preparing repository scope",
                    terminal: false,
                })
                .map_err(|_| ())
        })
        .is_err()
    {
        return 1;
    }

    let mut human_stdout = Vec::new();
    let mut human_stderr = Vec::new();
    let code = run_init_with_builder(
        arguments,
        input,
        &mut human_stdout,
        &mut human_stderr,
        input_is_terminal,
        |root, build_arguments, detection, started| {
            let report = |progress: compass_core::BuildFileProgress| {
                let path = progress
                    .path
                    .strip_prefix(root)
                    .unwrap_or(&progress.path)
                    .to_string_lossy()
                    .into_owned();
                let result = writer
                    .lock()
                    .map_err(|_| "progress writer lock was poisoned".to_owned())
                    .and_then(|mut writer| {
                        writer
                            .write(&ProgressEvent {
                                schema: PROGRESS_SCHEMA,
                                operation_id: &operation_id,
                                operation: "init",
                                state: ProgressState::Running,
                                phase: "indexing",
                                current: u64::try_from(progress.current).ok(),
                                total: u64::try_from(progress.total).ok(),
                                message: &path,
                                terminal: false,
                            })
                            .map_err(|error| error.to_string())
                    });
                if let Err(error) = result
                    && let Ok(mut slot) = progress_error.lock()
                    && slot.is_none()
                {
                    *slot = Some(error);
                }
            };
            command_build_with_precomputed_detection(
                Frontend::Compass,
                build_arguments,
                BuildOperation::Init,
                detection,
                started,
                Some(&report),
            )
        },
    );
    let _ = stderr.write_all(&human_stdout);
    let _ = stderr.write_all(&human_stderr);
    if let Ok(slot) = progress_error.lock()
        && slot.is_some()
    {
        return 1;
    }
    let terminal = ProgressEvent {
        schema: PROGRESS_SCHEMA,
        operation_id: &operation_id,
        operation: "init",
        state: if code == 0 {
            ProgressState::Succeeded
        } else {
            ProgressState::Failed
        },
        phase: if code == 0 { "complete" } else { "failed" },
        current: None,
        total: None,
        message: if code == 0 {
            "Compass index is ready"
        } else {
            "Compass initialization failed"
        },
        terminal: true,
    };
    if writer
        .lock()
        .map_err(|_| ())
        .and_then(|mut writer| writer.write(&terminal).map_err(|_| ()))
        .is_err()
    {
        1
    } else {
        code
    }
}

fn run_init_with_builder(
    arguments: &[OsString],
    input: &mut impl BufRead,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
    input_is_terminal: bool,
    build: impl FnOnce(&Path, &[String], Detection, Instant) -> Outcome,
) -> u8 {
    let args = arguments
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let mut options = match parse(&args) {
        Ok(options) => options,
        Err(error) => {
            let _ = writeln!(stderr, "{error}");
            return 2;
        }
    };
    if !options.yes && !input_is_terminal {
        let _ = writeln!(
            stderr,
            "error: compass init requires an interactive terminal; pass --yes for non-interactive setup"
        );
        return 2;
    }
    let root = match fs::canonicalize(&options.root) {
        Ok(root) => root,
        Err(error) => {
            let _ = writeln!(
                stderr,
                "error: could not resolve {}: {error}",
                options.root.display()
            );
            return 1;
        }
    };
    let config_path = root.join(PROJECT_CONFIG_RELATIVE_PATH);
    if config_path.exists() && !options.force {
        let _ = writeln!(
            stderr,
            "error: {} already exists; pass --force to replace it",
            config_path.display()
        );
        return 2;
    }

    if !options.yes
        && let Err(error) = collect_interactive(&mut options, input, stdout)
    {
        let _ = writeln!(stderr, "error: {error}");
        return 1;
    }

    let config = match ProjectConfig::new(BuildScope {
        include: options.includes,
        exclude: options.excludes,
    })
    .normalize(&root)
    {
        Ok(config) => config,
        Err(error) => {
            let _ = writeln!(stderr, "error: {error}");
            return 2;
        }
    };
    let mut operation_started = Instant::now();
    let detection = match detect(
        &root,
        &DetectOptions {
            scope: config.build.clone(),
            ..DetectOptions::default()
        },
    ) {
        Ok(detection) => detection,
        Err(error) => {
            let _ = writeln!(stderr, "error: {error}");
            return 1;
        }
    };
    let matcher = match ScopeMatcher::new(&root, &config.build) {
        Ok(matcher) => matcher,
        Err(error) => {
            let _ = writeln!(stderr, "error: {error}");
            return 2;
        }
    };
    let paths = detection
        .files
        .values()
        .flatten()
        .map(Path::new)
        .collect::<Vec<_>>();
    let unmatched = matcher.unmatched_includes(paths.iter().copied());
    if !unmatched.is_empty() {
        let _ = writeln!(
            stderr,
            "error: include rule(s) matched no eligible files: {}",
            unmatched.join(", ")
        );
        return 2;
    }
    if detection.total_files == 0 {
        let _ = writeln!(stderr, "error: configured scope contains no eligible files");
        return 2;
    }

    let count = |kind: &str| detection.files.get(kind).map_or(0, Vec::len);
    let _ = writeln!(stdout, "Project root: {}", root.display());
    let _ = writeln!(
        stdout,
        "Scope: {} include rule(s), {} exclude rule(s)",
        config.build.include.len(),
        config.build.exclude.len()
    );
    let _ = writeln!(
        stdout,
        "Matched: {} files ({} code, {} documents, {} papers, {} images, {} video)",
        detection.total_files,
        count("code"),
        count("document"),
        count("paper"),
        count("image"),
        count("video")
    );
    let _ = writeln!(stdout, "Config: {}", config_path.display());
    let output_name = std::env::var("COMPASS_OUT").unwrap_or_else(|_| "compass-out".to_owned());
    let _ = writeln!(stdout, "Output: {}", root.join(output_name).display());

    if !options.yes {
        let confirmation_started = Instant::now();
        match prompt(input, stdout, "Save configuration and build now? [y/N] ") {
            Ok(answer) if matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") => {
                if let Some(adjusted) =
                    operation_started.checked_add(confirmation_started.elapsed())
                {
                    operation_started = adjusted;
                }
            }
            Ok(_) => {
                let _ = writeln!(stdout, "Cancelled; no files were changed.");
                return 0;
            }
            Err(error) => {
                let _ = writeln!(stderr, "error: {error}");
                return 1;
            }
        }
    }

    if let Err(error) = config.write(&root) {
        let _ = writeln!(stderr, "error: {error}");
        return 1;
    }
    let mut build_arguments = vec![root.to_string_lossy().into_owned(), "--force".to_owned()];
    if options.timing {
        build_arguments.push("--timing".to_owned());
    }
    let outcome = build(&root, &build_arguments, detection, operation_started);
    if outcome.code != 0 {
        let _ = writeln!(
            stderr,
            "Compass configuration saved to {}.",
            config_path.display()
        );
        let _ = writeln!(stderr, "Initial build failed.");
        let _ = writeln!(stderr, "Fix the reported issue, then run `compass update`.");
    }
    write_outcome(&outcome, stdout, stderr)
}

fn collect_interactive(
    options: &mut InitOptions,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), String> {
    if options.includes.is_empty() {
        let mode = prompt(input, output, "Build scope [all/custom] (all): ")?;
        if mode.eq_ignore_ascii_case("custom") {
            let includes = prompt(
                input,
                output,
                "Include files/folders/globs (comma-separated): ",
            )?;
            options.includes = split_entries(&includes);
        }
    }
    if options.excludes.is_empty() {
        let excludes = prompt(input, output, "Exclude globs (comma-separated, optional): ")?;
        options.excludes = split_entries(&excludes);
    }
    Ok(())
}

fn prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    message: &str,
) -> Result<String, String> {
    write!(output, "{message}").map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())?;
    let mut line = String::new();
    if input
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("unexpected end of input".to_owned());
    }
    Ok(line.trim().to_owned())
}

fn split_entries(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse(args: &[String]) -> Result<InitOptions, String> {
    let mut options = InitOptions {
        root: PathBuf::from("."),
        includes: Vec::new(),
        excludes: Vec::new(),
        yes: false,
        force: false,
        timing: false,
    };
    let mut root_seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--yes" => options.yes = true,
            "--force" => options.force = true,
            "--timing" => options.timing = true,
            "--include" | "--exclude" => {
                let name = args[index].clone();
                index += 1;
                let Some(value) = args.get(index).filter(|value| !value.is_empty()) else {
                    return Err(format!("error: {name} requires a value"));
                };
                if name == "--include" {
                    options.includes.push(value.clone());
                } else {
                    options.excludes.push(value.clone());
                }
            }
            value if value.starts_with("--include=") => {
                let value = &value[10..];
                if value.is_empty() {
                    return Err("error: --include requires a value".to_owned());
                }
                options.includes.push(value.to_owned());
            }
            value if value.starts_with("--exclude=") => {
                let value = &value[10..];
                if value.is_empty() {
                    return Err("error: --exclude requires a value".to_owned());
                }
                options.excludes.push(value.to_owned());
            }
            value if value.starts_with('-') => {
                return Err(format!("error: unknown init option: {value}"));
            }
            value if !root_seen => {
                options.root = PathBuf::from(value);
                root_seen = true;
            }
            value => return Err(format!("error: init accepts one path, unexpected: {value}")),
        }
        index += 1;
    }
    Ok(options)
}
