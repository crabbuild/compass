use std::collections::BTreeMap;
use std::fs;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use compass_cypher::{
    CompassValue, CompileLimits, CompileRequest, ParameterTypes, Parameters, compile,
    plan_cache_key,
};
use compass_files::write_text_atomic;
use compass_output::{render_cql_json, render_cql_jsonl, render_cql_table};
use compass_query::{PlanCache, QueryLimits, QueryRequest, execute};
use serde_json::Value;

use super::{Frontend, GraphSelection, Outcome, load_selection, parse_graph_selection};

static CQL_PLAN_CACHE: OnceLock<PlanCache> = OnceLock::new();

const MAX_QUERY_BYTES: usize = 1024 * 1024;
const MAX_PARAMETER_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn command_query(frontend: Frontend, args: &[String]) -> Outcome {
    if !args.iter().any(|argument| argument == "--cql") {
        return super::command_natural_query(frontend, args);
    }
    if frontend == Frontend::Graphify {
        return Outcome::failure(
            "error: --cql is a Compass-only query mode; graphify compatibility is unchanged"
                .to_owned(),
        );
    }
    let (graph_selection, args) = match parse_graph_selection(args) {
        Ok(parsed) => parsed,
        Err(error) => return Outcome::failure_with_code(format!("error: {error}"), 2),
    };
    match parse_request(&args, graph_selection).and_then(run_request) {
        Ok(output) => Outcome::success(output),
        Err(error) => Outcome::failure_with_code(error.message, error.code),
    }
}

#[derive(Clone, Copy)]
enum OutputFormat {
    Table,
    Json,
    Jsonl,
}

enum SourceSelection {
    Inline(String),
    File(PathBuf),
    Stdin,
    Repl,
}

struct CqlCliRequest {
    source: SourceSelection,
    graph_selection: GraphSelection,
    parameters: Parameters,
    format: OutputFormat,
    output: Option<PathBuf>,
    timeout: Duration,
    max_rows: usize,
    max_path_depth: usize,
    max_expanded_relationships: u64,
    max_memory_bytes: usize,
}

struct CliError {
    code: u8,
    message: String,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: format!("error: {}", message.into()),
        }
    }

    fn graph(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            message: format!("error: {}", message.into()),
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: 4,
            message: format!("error: {}", message.into()),
        }
    }
}

fn parse_request(
    args: &[String],
    graph_selection: GraphSelection,
) -> Result<CqlCliRequest, CliError> {
    let mut inline = Vec::new();
    let mut file = None;
    let mut stdin = false;
    let mut repl = false;
    let mut parameters = Parameters::new();
    let mut params_file = None;
    let mut format = OutputFormat::Table;
    let mut output = None;
    let mut timeout = Duration::from_secs(5);
    let mut max_rows = 10_000;
    let mut max_path_depth = 32;
    let mut max_expanded_relationships = 5_000_000;
    let mut max_memory_bytes = 256 * 1024 * 1024;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--cql" => index += 1,
            "--file" => {
                file = Some(PathBuf::from(required_value(args, index, "--file")?));
                index += 2;
            }
            "--stdin" => {
                stdin = true;
                index += 1;
            }
            "--repl" => {
                repl = true;
                index += 1;
            }
            "--param" => {
                parse_parameter(required_value(args, index, "--param")?, &mut parameters)?;
                index += 2;
            }
            "--params-file" => {
                params_file = Some(PathBuf::from(required_value(args, index, "--params-file")?));
                index += 2;
            }
            "--format" => {
                format = parse_format(required_value(args, index, "--format")?)?;
                index += 2;
            }
            "--output" => {
                output = Some(PathBuf::from(required_value(args, index, "--output")?));
                index += 2;
            }
            "--timeout-ms" => {
                timeout = Duration::from_millis(parse_number(
                    required_value(args, index, "--timeout-ms")?,
                    "--timeout-ms",
                )?);
                index += 2;
            }
            "--max-rows" => {
                max_rows = parse_number(required_value(args, index, "--max-rows")?, "--max-rows")?;
                index += 2;
            }
            "--max-path-depth" => {
                max_path_depth = parse_number(
                    required_value(args, index, "--max-path-depth")?,
                    "--max-path-depth",
                )?;
                index += 2;
            }
            "--max-expanded-relationships" => {
                max_expanded_relationships = parse_number(
                    required_value(args, index, "--max-expanded-relationships")?,
                    "--max-expanded-relationships",
                )?;
                index += 2;
            }
            "--max-memory-bytes" => {
                max_memory_bytes = parse_number(
                    required_value(args, index, "--max-memory-bytes")?,
                    "--max-memory-bytes",
                )?;
                index += 2;
            }
            value if value.starts_with("--file=") => {
                file = Some(PathBuf::from(&value[7..]));
                index += 1;
            }
            value if value.starts_with("--param=") => {
                parse_parameter(&value[8..], &mut parameters)?;
                index += 1;
            }
            value if value.starts_with("--params-file=") => {
                params_file = Some(PathBuf::from(&value[14..]));
                index += 1;
            }
            value if value.starts_with("--format=") => {
                format = parse_format(&value[9..])?;
                index += 1;
            }
            value if value.starts_with("--output=") => {
                output = Some(PathBuf::from(&value[9..]));
                index += 1;
            }
            value if value.starts_with("--timeout-ms=") => {
                timeout = Duration::from_millis(parse_number(&value[13..], "--timeout-ms")?);
                index += 1;
            }
            value if value.starts_with("--max-rows=") => {
                max_rows = parse_number(&value[11..], "--max-rows")?;
                index += 1;
            }
            value if value.starts_with("--max-path-depth=") => {
                max_path_depth = parse_number(&value[17..], "--max-path-depth")?;
                index += 1;
            }
            value if value.starts_with("--max-expanded-relationships=") => {
                max_expanded_relationships =
                    parse_number(&value[29..], "--max-expanded-relationships")?;
                index += 1;
            }
            value if value.starts_with("--max-memory-bytes=") => {
                max_memory_bytes = parse_number(&value[19..], "--max-memory-bytes")?;
                index += 1;
            }
            value if value.starts_with('-') => {
                return Err(CliError::usage(format!(
                    "unknown CompassQL option '{value}'"
                )));
            }
            value => {
                inline.push(value.to_owned());
                index += 1;
            }
        }
    }
    if let Some(path) = params_file {
        for (key, value) in read_parameters(&path)? {
            if parameters.insert(key.clone(), value).is_some() {
                return Err(CliError::usage(format!("duplicate parameter '${key}'")));
            }
        }
    }
    let source_count = usize::from(!inline.is_empty())
        + usize::from(file.is_some())
        + usize::from(stdin)
        + usize::from(repl);
    if source_count != 1 {
        return Err(CliError::usage(
            "select exactly one CompassQL source: positional query, --file, --stdin, or --repl",
        ));
    }
    if inline.len() > 1 {
        return Err(CliError::usage(
            "the positional CompassQL source must be passed as one quoted argument",
        ));
    }
    if max_path_depth > 32 {
        return Err(CliError::usage("--max-path-depth cannot exceed 32"));
    }
    if timeout.is_zero()
        || max_rows == 0
        || max_memory_bytes == 0
        || max_expanded_relationships == 0
    {
        return Err(CliError::usage("query limits must be greater than zero"));
    }
    let source = if !inline.is_empty() {
        SourceSelection::Inline(inline.into_iter().next().unwrap_or_default())
    } else if let Some(path) = file {
        SourceSelection::File(path)
    } else if stdin {
        SourceSelection::Stdin
    } else {
        SourceSelection::Repl
    };
    Ok(CqlCliRequest {
        source,
        graph_selection,
        parameters,
        format,
        output,
        timeout,
        max_rows,
        max_path_depth,
        max_expanded_relationships,
        max_memory_bytes,
    })
}

fn run_request(request: CqlCliRequest) -> Result<String, CliError> {
    if matches!(request.source, SourceSelection::Repl) {
        return run_repl(request);
    }
    let (source_name, source) = read_source(&request.source)?;
    run_source(&request, &source_name, &source)
}

fn run_source(
    request: &CqlCliRequest,
    source_name: &str,
    source: &str,
) -> Result<String, CliError> {
    let loaded = load_selection(Frontend::Compass, &request.graph_selection, true)
        .map_err(|outcome| CliError::graph(outcome.stderr))?;
    run_source_with_graph(request, source_name, source, &loaded.graph)
}

fn run_source_with_graph(
    request: &CqlCliRequest,
    source_name: &str,
    source: &str,
    graph: &compass_model::Graph,
) -> Result<String, CliError> {
    let parameter_types = request
        .parameters
        .iter()
        .map(|(name, value)| (name.clone(), value.compass_type()))
        .collect::<ParameterTypes>();
    let schema = graph.schema_fingerprint();
    let compile_request = CompileRequest {
        source_name,
        source,
        parameter_types: &parameter_types,
        schema: &schema,
        limits: CompileLimits {
            max_path_depth: request.max_path_depth,
            ..CompileLimits::default()
        },
    };
    let cache = CQL_PLAN_CACHE.get_or_init(PlanCache::default);
    let key = plan_cache_key(compile_request);
    let (compiled, cache_hit) = if let Some(compiled) = cache.get(&key) {
        (compiled, true)
    } else {
        let compiled = Arc::new(compile(compile_request).map_err(|diagnostics| {
            CliError::usage(format_diagnostics(source_name, source, &diagnostics))
        })?);
        cache.insert(key, Arc::clone(&compiled));
        (compiled, false)
    };
    let cancellation = super::process_cancellation()
        .map_err(|error| CliError::runtime(format!("could not install Ctrl+C handler: {error}")))?;
    let mut result = execute(QueryRequest {
        compiled: &compiled,
        graph,
        parameters: &request.parameters,
        limits: QueryLimits {
            deadline: Instant::now() + request.timeout,
            max_rows: request.max_rows,
            max_path_depth: request.max_path_depth,
            max_expanded_relationships: request.max_expanded_relationships,
            max_memory_bytes: request.max_memory_bytes,
        },
        cancellation,
    })
    .map_err(|error| CliError::runtime(error.to_string()))?;
    if let Some(profile) = &mut result.profile {
        profile.plan_cache_hit = Some(cache_hit);
    }
    let rendered = match request.format {
        OutputFormat::Table => render_cql_table(&result),
        OutputFormat::Json => {
            render_cql_json(&result).map_err(|error| CliError::runtime(error.to_string()))?
        }
        OutputFormat::Jsonl => {
            render_cql_jsonl(&result).map_err(|error| CliError::runtime(error.to_string()))?
        }
    };
    if let Some(path) = &request.output {
        write_text_atomic(path, &rendered).map_err(|error| CliError::runtime(error.to_string()))?;
        Ok(format!(
            "Wrote {} row(s) to {}",
            result.rows.len(),
            path.display()
        ))
    } else {
        Ok(rendered)
    }
}

fn run_repl(request: CqlCliRequest) -> Result<String, CliError> {
    if !std::io::stdin().is_terminal() {
        return Err(CliError::usage("--repl requires an interactive terminal"));
    }
    let loaded = load_selection(Frontend::Compass, &request.graph_selection, true)
        .map_err(|outcome| CliError::graph(outcome.stderr))?;
    let mut transcript = Vec::new();
    let mut buffer = String::new();
    loop {
        let mut line = String::new();
        let read = std::io::stdin()
            .read_line(&mut line)
            .map_err(|error| CliError::runtime(format!("could not read REPL input: {error}")))?;
        if read == 0 {
            break;
        }
        let trimmed = line.trim();
        match trimmed {
            ":quit" | ":q" => break,
            ":clear" => {
                buffer.clear();
                continue;
            }
            ":help" => {
                transcript.push("Commands: :help :quit :clear :params".to_owned());
                continue;
            }
            ":params" => {
                transcript.push(format!("{} parameter(s)", request.parameters.len()));
                continue;
            }
            _ => {}
        }
        buffer.push_str(&line);
        if trimmed.ends_with(';') {
            match run_source_with_graph(&request, "<repl>", &buffer, &loaded.graph) {
                Ok(output) => transcript.push(output),
                Err(error) => transcript.push(error.message),
            }
            buffer.clear();
        }
    }
    Ok(transcript.join("\n"))
}

fn read_source(source: &SourceSelection) -> Result<(String, String), CliError> {
    match source {
        SourceSelection::Inline(source) => {
            enforce_source_limit(source.as_bytes())?;
            Ok(("<command>".to_owned(), source.clone()))
        }
        SourceSelection::File(path) => {
            let metadata = fs::metadata(path).map_err(|error| {
                CliError::usage(format!("could not read {}: {error}", path.display()))
            })?;
            if !metadata.is_file() || metadata.len() > MAX_QUERY_BYTES as u64 {
                return Err(CliError::usage(format!(
                    "query file must be regular and no larger than {MAX_QUERY_BYTES} bytes"
                )));
            }
            let source = fs::read_to_string(path).map_err(|error| {
                CliError::usage(format!("could not read {}: {error}", path.display()))
            })?;
            Ok((path.display().to_string(), source))
        }
        SourceSelection::Stdin => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .take((MAX_QUERY_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|error| CliError::usage(format!("could not read stdin: {error}")))?;
            enforce_source_limit(&bytes)?;
            let source = String::from_utf8(bytes)
                .map_err(|_| CliError::usage("CompassQL stdin must be UTF-8"))?;
            Ok(("<stdin>".to_owned(), source))
        }
        SourceSelection::Repl => Err(CliError::usage("internal REPL source error")),
    }
}

fn enforce_source_limit(bytes: &[u8]) -> Result<(), CliError> {
    if bytes.len() > MAX_QUERY_BYTES {
        Err(CliError::usage(format!(
            "CompassQL source exceeds {MAX_QUERY_BYTES} bytes"
        )))
    } else {
        Ok(())
    }
}

fn read_parameters(path: &Path) -> Result<Parameters, CliError> {
    let metadata = fs::metadata(path)
        .map_err(|error| CliError::usage(format!("could not read {}: {error}", path.display())))?;
    if !metadata.is_file() || metadata.len() > MAX_PARAMETER_BYTES {
        return Err(CliError::usage(format!(
            "parameter file must be regular and no larger than {MAX_PARAMETER_BYTES} bytes"
        )));
    }
    let bytes = fs::read(path)
        .map_err(|error| CliError::usage(format!("could not read {}: {error}", path.display())))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| CliError::usage(format!("invalid parameter JSON: {error}")))?;
    let Value::Object(values) = value else {
        return Err(CliError::usage(
            "parameter file must contain one JSON object",
        ));
    };
    values
        .into_iter()
        .map(|(key, value)| Ok((key, parameter_value(value)?)))
        .collect()
}

fn parse_parameter(raw: &str, parameters: &mut Parameters) -> Result<(), CliError> {
    let Some((name, raw_value)) = raw.split_once('=') else {
        return Err(CliError::usage("--param requires NAME=VALUE"));
    };
    if name.is_empty()
        || !name.chars().enumerate().all(|(index, value)| {
            value == '_' || value.is_alphanumeric() && (index > 0 || !value.is_numeric())
        })
    {
        return Err(CliError::usage("invalid parameter name"));
    }
    let json =
        serde_json::from_str(raw_value).unwrap_or_else(|_| Value::String(raw_value.to_owned()));
    let value = parameter_value(json)?;
    if parameters.insert(name.to_owned(), value).is_some() {
        return Err(CliError::usage(format!("duplicate parameter '${name}'")));
    }
    Ok(())
}

fn parameter_value(value: Value) -> Result<CompassValue, CliError> {
    match value {
        Value::Null => Ok(CompassValue::Null),
        Value::Bool(value) => Ok(CompassValue::Boolean(value)),
        Value::Number(value) => value
            .as_i64()
            .map(CompassValue::Integer)
            .or_else(|| {
                value
                    .as_u64()
                    .and_then(|value| i64::try_from(value).ok())
                    .map(CompassValue::Integer)
            })
            .or_else(|| {
                (!value.is_u64())
                    .then(|| value.as_f64())
                    .flatten()
                    .filter(|value| value.is_finite())
                    .map(CompassValue::Float)
            })
            .ok_or_else(|| CliError::usage("parameter number is outside the CompassQL range")),
        Value::String(value) => Ok(CompassValue::String(value.into())),
        Value::Array(values) => values
            .into_iter()
            .map(parameter_value)
            .collect::<Result<Vec<_>, _>>()
            .map(|values| CompassValue::List(values.into())),
        Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| Ok((key, parameter_value(value)?)))
            .collect::<Result<BTreeMap<_, _>, CliError>>()
            .map(|values| CompassValue::Map(values.into())),
    }
}

fn required_value<'a>(args: &'a [String], index: usize, name: &str) -> Result<&'a str, CliError> {
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{name} requires a value")))
}

fn parse_number<T>(raw: &str, name: &str) -> Result<T, CliError>
where
    T: std::str::FromStr,
{
    raw.parse::<T>()
        .map_err(|_| CliError::usage(format!("{name} requires a non-negative integer")))
}

fn parse_format(raw: &str) -> Result<OutputFormat, CliError> {
    match raw {
        "table" | "text" => Ok(OutputFormat::Table),
        "json" => Ok(OutputFormat::Json),
        "jsonl" => Ok(OutputFormat::Jsonl),
        _ => Err(CliError::usage("--format must be table, json, or jsonl")),
    }
}

fn format_diagnostics(
    source_name: &str,
    source: &str,
    diagnostics: &compass_cypher::Diagnostics,
) -> String {
    diagnostics
        .items()
        .iter()
        .map(|diagnostic| {
            let prefix = &source[..diagnostic.span().start.min(source.len())];
            let line = prefix.bytes().filter(|value| *value == b'\n').count() + 1;
            let column = prefix
                .rsplit_once('\n')
                .map_or(prefix.len() + 1, |(_, tail)| tail.len() + 1);
            let help = diagnostic
                .help()
                .map_or(String::new(), |help| format!("\nhelp: {help}"));
            format!(
                "{}: {}\n  --> {source_name}:{line}:{column}{help}",
                diagnostic.code(),
                diagnostic.message()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
