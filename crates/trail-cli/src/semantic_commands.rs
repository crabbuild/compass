use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use trail_files::{Cache, write_text_atomic};
use trail_semantic::{
    MAX_SEMANTIC_FRAGMENT_BYTES, check_semantic_cache_mode, load_validated_semantic_fragment,
};

use crate::{Frontend, Outcome};

pub(super) fn command_cache_check(frontend: Frontend, args: &[String]) -> Outcome {
    let Some(files_from) = args.first().map(PathBuf::from) else {
        return Outcome::failure(cache_check_help(frontend));
    };
    let mut root = PathBuf::from(".");
    let mut mode = None;
    let mut prompt_file = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--root" if index + 1 < args.len() => {
                root = PathBuf::from(&args[index + 1]);
                index += 1;
            }
            value if value.starts_with("--root=") => root = PathBuf::from(&value[7..]),
            "--mode" if index + 1 < args.len() => {
                mode = Some(args[index + 1].clone());
                index += 1;
            }
            value if value.starts_with("--mode=") => mode = Some(value[7..].to_owned()),
            "--deep" => mode = Some("deep".to_owned()),
            "--prompt-file" if index + 1 < args.len() => {
                prompt_file = Some(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            value if value.starts_with("--prompt-file=") => {
                prompt_file = Some(PathBuf::from(&value[14..]));
            }
            _ => {}
        }
        index += 1;
    }
    let source = match fs::read_to_string(&files_from) {
        Ok(source) => source,
        Err(error) => {
            return Outcome::failure(format!(
                "error: could not read {}: {error}",
                files_from.display()
            ));
        }
    };
    let originals = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let root = match fs::canonicalize(&root) {
        Ok(root) => root,
        Err(error) => {
            return Outcome::failure(format!(
                "error: could not resolve {}: {error}",
                root.display()
            ));
        }
    };
    let mut stderr = String::new();
    let prompt = prompt_file
        .as_deref()
        .and_then(|path| match fs::read_to_string(path) {
            Ok(prompt) => Some(prompt),
            Err(error) => {
                stderr = format!(
                    "warning: could not read prompt file {}; using legacy cache namespace: {error}",
                    path.display()
                );
                None
            }
        });
    let mut cache = match Cache::new(&root, None) {
        Ok(cache) => cache,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut hyperedges = Vec::new();
    let mut uncached = Vec::new();
    for original in &originals {
        let path = Path::new(original);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let checked = match check_semantic_cache_mode(
            &mut cache,
            &[resolved],
            mode.as_deref(),
            prompt.as_deref(),
        ) {
            Ok(checked) => checked,
            Err(error) => return Outcome::failure(format!("error: {error}")),
        };
        nodes.extend(checked.nodes);
        edges.extend(checked.edges);
        hyperedges.extend(checked.hyperedges);
        if !checked.uncached.is_empty() {
            uncached.push(original.clone());
        }
    }
    let output_name = std::env::var("GRAPHIFY_OUT").unwrap_or_else(|_| "graphify-out".to_owned());
    let output = root.join(output_name);
    if let Err(error) = fs::create_dir_all(&output) {
        return Outcome::failure(format!(
            "error: could not create {}: {error}",
            output.display()
        ));
    }
    if (!nodes.is_empty() || !edges.is_empty() || !hyperedges.is_empty())
        && let Err(error) = write_compact_json(
            &output.join(".graphify_cached.json"),
            &json!({"nodes":nodes,"edges":edges,"hyperedges":hyperedges}),
        )
    {
        return Outcome::failure(format!("error: {error}"));
    }
    if let Err(error) =
        write_text_atomic(output.join(".graphify_uncached.txt"), &uncached.join("\n"))
    {
        return Outcome::failure(format!("error: {error}"));
    }
    Outcome {
        code: 0,
        stdout: format!(
            "Cache: {} hit, {} miss",
            originals.len().saturating_sub(uncached.len()),
            uncached.len()
        ),
        stderr,
    }
}

pub(super) fn command_merge_chunks(frontend: Frontend, args: &[String]) -> Outcome {
    if args.is_empty() {
        return Outcome::failure(merge_chunks_help(frontend));
    }
    let mut output = None;
    let mut chunk_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" if index + 1 < args.len() => {
                output = Some(PathBuf::from(&args[index + 1]));
                index += 1;
            }
            value if value.starts_with("--out=") => output = Some(PathBuf::from(&value[6..])),
            value => chunk_args.push(value.to_owned()),
        }
        index += 1;
    }
    let Some(output) = output else {
        return Outcome::failure("error: --out <path> required".to_owned());
    };
    let chunk_files = expand_chunk_arguments(&chunk_args);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut hyperedges = Vec::new();
    let mut seen = HashSet::new();
    let mut tokens = TokenTotals::default();
    let mut valid = 0_usize;
    let mut warnings = Vec::new();
    for path in &chunk_files {
        let fragment = match load_validated_semantic_fragment(path) {
            Ok(fragment) => fragment,
            Err(error) => {
                warnings.push(format!(
                    "[graphify merge-chunks] warning: skipping invalid chunk {}: {error}",
                    path.display()
                ));
                continue;
            }
        };
        valid = valid.saturating_add(1);
        for node in fragment
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = node.get("id").and_then(Value::as_str).unwrap_or_default();
            if seen.insert(id.to_owned()) {
                nodes.push(node.clone());
            }
        }
        edges.extend(array_values(&fragment, "edges"));
        hyperedges.extend(array_values(&fragment, "hyperedges"));
        tokens.add(fragment.get("input_tokens"), fragment.get("output_tokens"));
    }
    if valid == 0 {
        warnings.push(format!(
            "[graphify merge-chunks] error: no valid chunks to merge; refusing to write {}",
            output.display()
        ));
        return Outcome {
            code: 1,
            stdout: String::new(),
            stderr: warnings.join("\n"),
        };
    }
    let merged = json!({
        "nodes":nodes,
        "edges":edges,
        "hyperedges":hyperedges,
        "input_tokens":tokens.input_value(),
        "output_tokens":tokens.output_value(),
    });
    if let Err(error) = write_compact_json(&output, &merged) {
        return Outcome::failure(format!("error: {error}"));
    }
    let summary = if valid == chunk_files.len() {
        format!("{valid} chunks")
    } else {
        format!("{valid} of {} chunks", chunk_files.len())
    };
    Outcome {
        code: 0,
        stdout: format!(
            "Merged {summary}: {} nodes, {} edges, {} in / {} out tokens",
            merged["nodes"].as_array().map_or(0, Vec::len),
            merged["edges"].as_array().map_or(0, Vec::len),
            python_number_text(&merged["input_tokens"]),
            python_number_text(&merged["output_tokens"]),
        ),
        stderr: warnings.join("\n"),
    }
}

pub(super) fn command_merge_semantic(frontend: Frontend, args: &[String]) -> Outcome {
    if args.is_empty() {
        return Outcome::failure(merge_semantic_help(frontend));
    }
    let mut cached: Option<PathBuf> = None;
    let mut fresh: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut index = 0;
    while index < args.len() {
        let target = match args[index].as_str() {
            "--cached" => Some(&mut cached),
            "--new" => Some(&mut fresh),
            "--out" => Some(&mut output),
            _ => None,
        };
        if let Some(target) = target
            && let Some(value) = args.get(index + 1)
        {
            *target = Some(PathBuf::from(value));
            index += 1;
        }
        index += 1;
    }
    let Some(output) = output else {
        return Outcome::failure("error: --out <path> required".to_owned());
    };
    let cached = match read_optional_fragment(cached.as_deref()) {
        Ok(fragment) => fragment,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let fresh = match read_optional_fragment(fresh.as_deref()) {
        Ok(fragment) => fragment,
        Err(error) => return Outcome::failure(format!("error: {error}")),
    };
    let mut seen = HashSet::new();
    let nodes = array_values(&cached, "nodes")
        .chain(array_values(&fresh, "nodes"))
        .filter(|node| {
            let id = node.get("id").and_then(Value::as_str).unwrap_or_default();
            seen.insert(id.to_owned())
        })
        .collect::<Vec<_>>();
    let edges = array_values(&cached, "edges")
        .chain(array_values(&fresh, "edges"))
        .collect::<Vec<_>>();
    let hyperedges = array_values(&cached, "hyperedges")
        .chain(array_values(&fresh, "hyperedges"))
        .collect::<Vec<_>>();
    let merged = json!({"nodes":nodes,"edges":edges,"hyperedges":hyperedges});
    if let Err(error) = write_compact_json(&output, &merged) {
        return Outcome::failure(format!("error: {error}"));
    }
    Outcome::success(format!(
        "Merged: {} nodes, {} edges",
        merged["nodes"].as_array().map_or(0, Vec::len),
        merged["edges"].as_array().map_or(0, Vec::len)
    ))
}

fn expand_chunk_arguments(arguments: &[String]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for argument in arguments {
        let mut expanded = glob::glob(argument)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        expanded.sort();
        if expanded.is_empty() {
            files.push(PathBuf::from(argument));
        } else {
            files.extend(expanded);
        }
    }
    files
}

fn read_optional_fragment(path: Option<&Path>) -> Result<Value, String> {
    let Some(path) = path.filter(|path| path.exists()) else {
        return Ok(json!({"nodes":[],"edges":[],"hyperedges":[]}));
    };
    let metadata = path
        .metadata()
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if metadata.len() > MAX_SEMANTIC_FRAGMENT_BYTES {
        return Err(format!(
            "{} is {} bytes; maximum is {}",
            path.display(),
            metadata.len(),
            MAX_SEMANTIC_FRAGMENT_BYTES
        ));
    }
    let raw =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let fragment: Value = serde_json::from_slice(&raw)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))?;
    if !fragment.is_object() {
        return Err(format!("{} must contain a JSON object", path.display()));
    }
    Ok(fragment)
}

fn array_values<'a>(fragment: &'a Value, key: &str) -> impl Iterator<Item = Value> + 'a {
    fragment
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .cloned()
}

fn write_compact_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let encoded = serde_json::to_string(value).map_err(|error| error.to_string())?;
    write_text_atomic(path, &encoded).map_err(|error| error.to_string())
}

#[derive(Default)]
struct TokenTotals {
    input: TokenTotal,
    output: TokenTotal,
}

impl TokenTotals {
    fn add(&mut self, input: Option<&Value>, output: Option<&Value>) {
        self.input.add(input);
        self.output.add(output);
    }

    fn input_value(&self) -> Value {
        self.input.value()
    }

    fn output_value(&self) -> Value {
        self.output.value()
    }
}

#[derive(Default)]
struct TokenTotal {
    integer: i128,
    floating: Option<f64>,
}

impl TokenTotal {
    fn add(&mut self, value: Option<&Value>) {
        let Some(value) = value else { return };
        if let Some(boolean) = value.as_bool() {
            self.add_integer(i128::from(boolean));
        } else if let Some(number) = value.as_i64() {
            self.add_integer(i128::from(number));
        } else if let Some(number) = value.as_u64() {
            self.add_integer(i128::from(number));
        } else if let Some(number) = value.as_f64() {
            self.floating = Some(self.floating.unwrap_or(self.integer as f64) + number);
        }
    }

    fn add_integer(&mut self, value: i128) {
        if let Some(floating) = &mut self.floating {
            *floating += value as f64;
        } else {
            self.integer += value;
        }
    }

    fn value(&self) -> Value {
        if let Some(floating) = self.floating {
            return serde_json::Number::from_f64(floating).map_or(Value::from(0), Value::Number);
        }
        if let Ok(integer) = u64::try_from(self.integer) {
            Value::from(integer)
        } else if let Ok(integer) = i64::try_from(self.integer) {
            Value::from(integer)
        } else {
            serde_json::Number::from_f64(self.integer as f64).map_or(Value::from(0), Value::Number)
        }
    }
}

fn python_number_text(value: &Value) -> String {
    if let Some(number) = value.as_u64() {
        return format_with_commas(number);
    }
    let text = value.to_string();
    let (sign, unsigned) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |unsigned| ("-", unsigned));
    let (integer, suffix) = unsigned
        .find(['.', 'e', 'E'])
        .map_or((unsigned, ""), |index| unsigned.split_at(index));
    let Ok(integer) = integer.parse::<u64>() else {
        return text;
    };
    format!("{sign}{}{suffix}", format_with_commas(integer))
}

fn format_with_commas(number: u64) -> String {
    let digits = number.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

pub(super) fn cache_check_help(frontend: Frontend) -> String {
    format!(
        "Usage: {} cache-check <files_from> [--root <dir>] [--mode <m> | --deep] [--prompt-file <path>]",
        prefix(frontend)
    )
}

pub(super) fn merge_chunks_help(frontend: Frontend) -> String {
    format!(
        "Usage: {} merge-chunks <chunk_files...> --out <path>",
        prefix(frontend)
    )
}

pub(super) fn merge_semantic_help(frontend: Frontend) -> String {
    format!(
        "Usage: {} merge-semantic --cached <path> --new <path> --out <path>",
        prefix(frontend)
    )
}

fn prefix(frontend: Frontend) -> &'static str {
    match frontend {
        Frontend::Trail => "trail graph",
        Frontend::Graphify => "graphify",
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn merge_chunks_skips_invalid_siblings() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let valid = directory.path().join("valid.json");
        let invalid = directory.path().join("invalid.json");
        let output = directory.path().join("merged.json");
        fs::write(
            &valid,
            serde_json::to_vec(&json!({
                "nodes":[{"id":"pkg.good"}],
                "edges":[],
                "hyperedges":[],
            }))?,
        )?;
        fs::write(
            &invalid,
            serde_json::to_vec(&json!({
                "nodes":[{"id":"../../escape"}],
                "edges":[],
                "hyperedges":[],
            }))?,
        )?;
        let outcome = command_merge_chunks(
            Frontend::Graphify,
            &[
                valid.to_string_lossy().into_owned(),
                invalid.to_string_lossy().into_owned(),
                "--out".to_owned(),
                output.to_string_lossy().into_owned(),
            ],
        );
        assert_eq!(outcome.code, 0, "{}", outcome.stderr);
        assert!(outcome.stdout.contains("Merged 1 of 2 chunks"));
        assert!(outcome.stderr.contains("skipping invalid chunk"));
        let merged: Value = serde_json::from_slice(&fs::read(output)?)?;
        assert_eq!(merged["nodes"], json!([{"id":"pkg.good"}]));
        Ok(())
    }

    #[test]
    fn merge_chunks_fails_closed_without_overwriting() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let invalid = directory.path().join("invalid.json");
        let output = directory.path().join("merged.json");
        fs::write(&invalid, br#"{"nodes":"not-an-array","edges":[]}"#)?;
        fs::write(&output, br#"{"previous":true}"#)?;
        let outcome = command_merge_chunks(
            Frontend::Graphify,
            &[
                invalid.to_string_lossy().into_owned(),
                "--out".to_owned(),
                output.to_string_lossy().into_owned(),
            ],
        );
        assert_eq!(outcome.code, 1);
        assert!(outcome.stderr.contains("no valid chunks to merge"));
        assert_eq!(fs::read(output)?, br#"{"previous":true}"#);
        Ok(())
    }
}
