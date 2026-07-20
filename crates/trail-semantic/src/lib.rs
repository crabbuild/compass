//! Validation and cleanup for untrusted semantic extraction fragments.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const MAX_SEMANTIC_FRAGMENT_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_SEMANTIC_FRAGMENT_NODES: usize = 10_000;
pub const MAX_SEMANTIC_FRAGMENT_EDGES: usize = 100_000;
pub const MAX_SEMANTIC_FRAGMENT_HYPEREDGES: usize = 10_000;
pub const MAX_SEMANTIC_HYPEREDGE_NODES: usize = 256;
pub const MAX_SEMANTIC_ID_LENGTH: usize = 256;
pub const LLM_JSON_MAX_CHARS: usize = 10 * 1024 * 1024;
pub const PROVIDER_RESPONSE_MAX_BYTES: u64 = 10 * 1024 * 1024;
pub const FILE_CHAR_CAP: usize = 20_000;

const EXTRACTION_PROMPT: &str = include_str!("../prompts/extraction.txt");
const DEEP_EXTRACTION_SUFFIX: &str = include_str!("../prompts/deep.txt");

const RATIONALE_MIN_CHARS: usize = 80;
const RATIONALE_MIN_WORDS: usize = 8;
const CONTEXT_EXCEEDED_MARKERS: &[&str] = &[
    "context size",
    "context length",
    "context_length",
    "context window",
    "n_keep",
    "exceeds the available",
    "n_ctx",
    "maximum context",
    "too many tokens",
    "prompt is too long",
    "context_length_exceeded",
];

#[derive(Clone, Copy, Debug)]
pub struct ValidationLimits {
    pub max_bytes: u64,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_hyperedges: usize,
    pub max_hyperedge_nodes: usize,
    pub max_id_chars: usize,
}

impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_SEMANTIC_FRAGMENT_BYTES,
            max_nodes: MAX_SEMANTIC_FRAGMENT_NODES,
            max_edges: MAX_SEMANTIC_FRAGMENT_EDGES,
            max_hyperedges: MAX_SEMANTIC_FRAGMENT_HYPEREDGES,
            max_hyperedge_nodes: MAX_SEMANTIC_HYPEREDGE_NODES,
            max_id_chars: MAX_SEMANTIC_ID_LENGTH,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    #[error("could not stat {path}: {source}")]
    Stat {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON: {0}")]
    InvalidJson(serde_json::Error),
    #[error("semantic fragment rejected: {0}")]
    InvalidFragment(String),
    #[error("invalid Claude CLI envelope: {0}")]
    InvalidEnvelope(String),
    #[error("invalid provider response: {0}")]
    InvalidProviderResponse(String),
    #[error("provider transport failed: {0}")]
    Transport(String),
}

/// Validate and normalize an arbitrary untrusted semantic JSON value.
///
/// Hyperedge member aliases are folded onto `nodes` in place, matching the
/// compatibility ingest boundary. All discovered violations are returned so a
/// caller can report a complete diagnostic rather than fail one field at a time.
pub fn validate_semantic_fragment(fragment: &mut Value) -> Vec<String> {
    validate_semantic_fragment_with_limits(fragment, ValidationLimits::default())
}

#[must_use]
pub fn validate_semantic_fragment_with_limits(
    fragment: &mut Value,
    limits: ValidationLimits,
) -> Vec<String> {
    let Some(object) = fragment.as_object_mut() else {
        return vec!["fragment must be a JSON object".to_owned()];
    };
    let mut errors = Vec::new();
    match python_json_payload_len(object) {
        Ok(payload_len) if payload_len > limits.max_bytes => errors.push(format!(
            "payload is {} bytes; max is {}",
            payload_len, limits.max_bytes
        )),
        Ok(_) => {}
        Err(error) => errors.push(format!("fragment is not JSON-serializable: {error}")),
    }

    validate_records(
        object.get("nodes"),
        "nodes",
        limits.max_nodes,
        &mut errors,
        |index, item, errors| {
            validate_id(
                errors,
                &format!("nodes[{index}].id"),
                item.get("id"),
                limits.max_id_chars,
            );
        },
    );
    validate_records(
        object.get("edges"),
        "edges",
        limits.max_edges,
        &mut errors,
        |index, item, errors| {
            validate_id(
                errors,
                &format!("edges[{index}].source"),
                item.get("source"),
                limits.max_id_chars,
            );
            validate_id(
                errors,
                &format!("edges[{index}].target"),
                item.get("target"),
                limits.max_id_chars,
            );
        },
    );

    let Some(hyperedges) = object.get_mut("hyperedges") else {
        return errors;
    };
    if hyperedges.is_null() {
        return errors;
    }
    let Some(items) = hyperedges.as_array_mut() else {
        errors.push("hyperedges must be a list".to_owned());
        return errors;
    };
    if items.len() > limits.max_hyperedges {
        errors.push(format!(
            "hyperedges has {} entries; max is {}",
            items.len(),
            limits.max_hyperedges
        ));
    }
    for (index, value) in items.iter_mut().enumerate() {
        let Some(item) = value.as_object_mut() else {
            errors.push(format!("hyperedges[{index}] must be an object"));
            continue;
        };
        normalize_hyperedge_members(item);
        validate_id(
            &mut errors,
            &format!("hyperedges[{index}].id"),
            item.get("id"),
            limits.max_id_chars,
        );
        let Some(members) = item.get("nodes").and_then(Value::as_array) else {
            errors.push(format!("hyperedges[{index}].nodes must be a list"));
            continue;
        };
        if members.len() > limits.max_hyperedge_nodes {
            errors.push(format!(
                "hyperedges[{index}].nodes has {} entries; max is {}",
                members.len(),
                limits.max_hyperedge_nodes
            ));
        }
        for (member_index, member) in members.iter().enumerate() {
            validate_id(
                &mut errors,
                &format!("hyperedges[{index}].nodes[{member_index}]"),
                Some(member),
                limits.max_id_chars,
            );
        }
    }
    errors
}

/// Return the UTF-8 byte length produced by Python's
/// `json.dumps(value, ensure_ascii=False)` defaults.
///
/// Serde's compact JSON has the same token encoding for values representable
/// by `serde_json::Value`; Python additionally emits one space after every
/// comma and colon. Counting those separators preserves the compatibility
/// security boundary without allocating a second padded payload.
fn python_json_payload_len(object: &Map<String, Value>) -> Result<u64, serde_json::Error> {
    let compact = serde_json::to_vec(object)?.len() as u64;
    let separators = object.len() + object.len().saturating_sub(1);
    let nested = object
        .values()
        .map(python_json_separator_spaces)
        .sum::<u64>();
    Ok(compact
        .saturating_add(separators as u64)
        .saturating_add(nested))
}

fn python_json_separator_spaces(value: &Value) -> u64 {
    match value {
        Value::Array(values) => {
            values.len().saturating_sub(1) as u64
                + values.iter().map(python_json_separator_spaces).sum::<u64>()
        }
        Value::Object(values) => {
            let separators = values.len() + values.len().saturating_sub(1);
            separators as u64
                + values
                    .values()
                    .map(python_json_separator_spaces)
                    .sum::<u64>()
        }
        _ => 0,
    }
}

fn validate_records(
    value: Option<&Value>,
    field: &str,
    maximum: usize,
    errors: &mut Vec<String>,
    mut validate: impl FnMut(usize, &Map<String, Value>, &mut Vec<String>),
) {
    let value = value.unwrap_or(&Value::Null);
    let items = if value.is_null() {
        &[][..]
    } else if let Some(items) = value.as_array() {
        items
    } else {
        errors.push(format!("{field} must be a list"));
        return;
    };
    if items.len() > maximum {
        errors.push(format!(
            "{field} has {} entries; max is {maximum}",
            items.len()
        ));
    }
    for (index, value) in items.iter().enumerate() {
        let Some(item) = value.as_object() else {
            errors.push(format!("{field}[{index}] must be an object"));
            continue;
        };
        validate(index, item, errors);
    }
}

fn validate_id(errors: &mut Vec<String>, field: &str, value: Option<&Value>, maximum: usize) {
    let Some(value) = value.and_then(Value::as_str) else {
        errors.push(format!("{field} must be a string"));
        return;
    };
    if value.is_empty() {
        errors.push(format!("{field} must not be empty"));
        return;
    }
    let characters = value.chars().count();
    if characters > maximum {
        errors.push(format!("{field} is {characters} chars; max is {maximum}"));
    }
    if value.contains('/') || value.contains('\\') || value.contains("..") {
        errors.push(format!("{field} must not contain path separators or '..'"));
    }
    if !semantic_id_regex().is_match(value) {
        errors.push(format!("{field} contains unsupported characters"));
    }
}

fn semantic_id_regex() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^[\w.:-]+$").unwrap_or_else(|_| unreachable!()))
}

/// Parse a model response into a safe graph fragment.
///
/// This accepts clean JSON, JSON in Markdown fences (including a prose
/// preamble), and the first balanced object embedded in prose. Invalid,
/// oversized, array, and scalar responses degrade to an empty fragment.
#[must_use]
pub fn parse_llm_json(raw: &str) -> Value {
    if raw.chars().count() > LLM_JSON_MAX_CHARS {
        return empty_fragment();
    }
    let mut stripped = raw.trim();
    if let Some(fence_start) = stripped.find("```") {
        let mut after_fence = &stripped[fence_start + 3..];
        if let Some(newline) = after_fence.find('\n') {
            let language = after_fence[..newline].trim().to_ascii_lowercase();
            if matches!(language.as_str(), "json" | "javascript" | "js" | "") {
                after_fence = &after_fence[newline + 1..];
            }
        }
        stripped = after_fence
            .rfind("```")
            .map_or(after_fence, |fence_end| &after_fence[..fence_end])
            .trim();
    }
    if let Some(parsed) = parse_fragment(stripped) {
        return parsed;
    }
    if let Some(start) = stripped.find('{') {
        let mut depth = 0_u64;
        let mut in_string = false;
        let mut escape = false;
        for (offset, character) in stripped[start..].char_indices() {
            if escape {
                escape = false;
                continue;
            }
            if character == '\\' {
                escape = true;
                continue;
            }
            if character == '"' {
                in_string = !in_string;
                continue;
            }
            if in_string {
                continue;
            }
            match character {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let end = start + offset + character.len_utf8();
                        if let Some(parsed) = parse_fragment(&stripped[start..end]) {
                            return parsed;
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    empty_fragment()
}

fn parse_fragment(raw: &str) -> Option<Value> {
    let mut parsed = serde_json::from_str::<Value>(raw).ok()?;
    parsed.as_object()?;
    sanitize_llm_fragment(&mut parsed);
    Some(parsed)
}

fn empty_fragment() -> Value {
    serde_json::json!({"nodes": [], "edges": [], "hyperedges": []})
}

/// Force model-produced graph collections to contain objects only.
pub fn sanitize_llm_fragment(fragment: &mut Value) {
    let Some(object) = fragment.as_object_mut() else {
        return;
    };
    for key in ["nodes", "edges", "hyperedges"] {
        let Some(value) = object.get_mut(key) else {
            continue;
        };
        if let Some(items) = value.as_array_mut() {
            items.retain(Value::is_object);
        } else if !value.is_null() {
            *value = Value::Array(Vec::new());
        }
    }
}

/// Defang source-controlled chat-template and wrapper delimiter tokens.
#[must_use]
pub fn neutralize_injection_sentinels(content: &str) -> String {
    injection_sentinel_regex()
        .replace_all(content, |captures: &regex::Captures<'_>| {
            let matched = captures.get(0).map_or("", |item| item.as_str());
            let first_end = matched.chars().next().map_or(0, char::len_utf8);
            format!("{}\u{200b}{}", &matched[..first_end], &matched[first_end..])
        })
        .into_owned()
}

fn injection_sentinel_regex() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?im)</?untrusted_source\b[^>]*>|<\|(?:im_start|im_end|system|user|assistant|endoftext)\|>|<<SYS>>|<</SYS>>|\[/?INST\]|^\s*###?\s*(?:system|instruction)s?\s*:?\s*$",
        )
        .unwrap_or_else(|_| unreachable!())
    })
}

/// Wrap untrusted source text with a content digest and neutralized delimiters.
#[must_use]
pub fn wrap_untrusted_source(relative_path: &str, content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    let safe = neutralize_injection_sentinels(content);
    format!(
        "<untrusted_source path=\"{relative_path}\" sha256=\"{digest:x}\">\n{safe}\n</untrusted_source>"
    )
}

/// Exact text shown to a model for one dispatched source unit.
#[derive(Clone, Copy, Debug)]
pub struct EvidenceSource<'a> {
    pub path: &'a Path,
    pub content: &'a str,
}

/// Build the user-message source blocks for already decoded text units.
/// Sources outside the canonical corpus root are omitted.
#[must_use]
pub fn build_untrusted_prompt(sources: &[EvidenceSource<'_>], root: &Path) -> String {
    let Ok(resolved_root) = root.canonicalize() else {
        return String::new();
    };
    sources
        .iter()
        .filter_map(|source| {
            let resolved = source.path.canonicalize().ok()?;
            if !resolved.starts_with(&resolved_root) {
                return None;
            }
            let relative = source
                .path
                .strip_prefix(root)
                .unwrap_or(source.path)
                .to_string_lossy();
            let capped = source
                .content
                .chars()
                .take(FILE_CHAR_CAP)
                .collect::<String>();
            Some(wrap_untrusted_source(&relative, &capped))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Return checkable ASCII identifier tokens from a semantic node label.
#[must_use]
pub fn label_identifiers(label: &str) -> Vec<&str> {
    let base = label.split_once('(').map_or(label, |(prefix, _)| prefix);
    label_identifier_regex()
        .find_iter(base)
        .map(|item| item.as_str())
        .filter(|item| item.len() >= 3)
        .collect()
}

fn label_identifier_regex() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap_or_else(|_| unreachable!()))
}

/// Flag code-typed semantic nodes whose names do not occur in the exact source
/// text dispatched to the model. Returns the number newly flagged.
pub fn bind_node_evidence(
    fragment: &mut Value,
    sources: &[EvidenceSource<'_>],
    root: &Path,
) -> usize {
    let Some(nodes) = fragment.get_mut("nodes").and_then(Value::as_array_mut) else {
        return 0;
    };
    if !nodes.iter().any(|node| {
        node.get("file_type").and_then(Value::as_str) == Some("code")
            && node.get("source_file").is_some_and(json_truthy)
    }) {
        return 0;
    }
    let Ok(resolved_root) = root.canonicalize() else {
        return 0;
    };
    let mut source_by_path = HashMap::<PathBuf, String>::new();
    for source in sources {
        let Ok(path) = source.path.canonicalize() else {
            continue;
        };
        if !path.starts_with(&resolved_root) {
            continue;
        }
        source_by_path.entry(path).or_default().extend(
            source
                .content
                .chars()
                .take(FILE_CHAR_CAP)
                .flat_map(char::to_lowercase),
        );
    }
    if source_by_path.is_empty() {
        return 0;
    }
    let mut downgraded = 0;
    for node in nodes {
        let Some(item) = node.as_object_mut() else {
            continue;
        };
        if item.get("file_type").and_then(Value::as_str) != Some("code") {
            continue;
        }
        let Some(source_file) = item
            .get("source_file")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let source_path = Path::new(source_file);
        let candidate = if source_path.is_absolute() {
            source_path.to_path_buf()
        } else {
            root.join(source_path)
        };
        let Ok(candidate) = candidate.canonicalize() else {
            continue;
        };
        let Some(source_text) = source_by_path.get(&candidate) else {
            continue;
        };
        let identifiers = ["label", "id"]
            .into_iter()
            .flat_map(|key| {
                label_identifiers(item.get(key).and_then(Value::as_str).unwrap_or_default())
            })
            .collect::<Vec<_>>();
        if identifiers.is_empty()
            || identifiers
                .iter()
                .any(|identifier| source_text.contains(&identifier.to_ascii_lowercase()))
        {
            continue;
        }
        let confidence_is_solid = item.get("confidence").is_none_or(|confidence| {
            confidence.is_null()
                || confidence
                    .as_str()
                    .is_some_and(|value| value.is_empty() || value == "EXTRACTED")
        });
        let verification_is_empty = item
            .get("verification")
            .is_none_or(|value| !json_truthy(value));
        if confidence_is_solid && verification_is_empty {
            item.insert(
                "verification".to_owned(),
                Value::String("unverified".to_owned()),
            );
            downgraded += 1;
        }
    }
    downgraded
}

fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_none_or(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

/// Detect a successful provider response with no usable graph content.
#[must_use]
pub fn response_is_hollow(raw_content: Option<&str>, parsed: &Value) -> bool {
    if raw_content.is_none_or(|raw| raw.trim().is_empty()) {
        return true;
    }
    ["nodes", "edges", "hyperedges"]
        .into_iter()
        .all(|key| parsed.get(key).is_none_or(|value| !json_truthy(value)))
}

/// Classify provider errors that should trigger adaptive chunk bisection.
#[must_use]
pub fn looks_like_context_exceeded(message: &str) -> bool {
    let message = message.to_lowercase();
    CONTEXT_EXCEEDED_MARKERS
        .iter()
        .any(|marker| message.contains(marker))
}

/// Mark every graph item in a truncated semantic result as partial.
pub fn mark_partial(result: &mut Value) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    for bucket in ["nodes", "edges", "hyperedges"] {
        let Some(items) = object.get_mut(bucket).and_then(Value::as_array_mut) else {
            continue;
        };
        for item in items.iter_mut().filter_map(Value::as_object_mut) {
            item.insert("_partial".to_owned(), Value::Bool(true));
        }
    }
}

/// Return sorted source paths whose semantic extraction is incomplete.
#[must_use]
pub fn partial_source_files(result: &Value) -> Vec<String> {
    let mut paths = result
        .get("_partial_files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    for bucket in ["nodes", "edges", "hyperedges"] {
        let Some(items) = result.get(bucket).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            if item.get("_partial").is_some_and(json_truthy)
                && let Some(source) = item
                    .get("source_file")
                    .and_then(Value::as_str)
                    .filter(|source| !source.is_empty())
            {
                paths.insert(source.to_owned());
            }
        }
    }
    paths.into_iter().collect()
}

/// Union the internal partial-file lists from several chunk results.
#[must_use]
pub fn merged_partial_files(results: &[Value]) -> Vec<String> {
    results
        .iter()
        .filter_map(|result| result.get("_partial_files").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Remove internal partial-item markers after semantic cache persistence.
pub fn strip_partial_markers(result: &mut Value) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    for bucket in ["nodes", "edges", "hyperedges"] {
        let Some(items) = object.get_mut(bucket).and_then(Value::as_array_mut) else {
            continue;
        };
        for item in items.iter_mut().filter_map(Value::as_object_mut) {
            item.remove("_partial");
        }
    }
}

/// True for OpenAI reasoning-model families that reject explicit temperature.
#[must_use]
pub fn model_requires_default_temperature(model: &str) -> bool {
    let base = model
        .to_lowercase()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    base.starts_with("gpt-5")
        || ["o1", "o3", "o4"]
            .into_iter()
            .any(|family| base == family || base.starts_with(&format!("{family}-")))
}

/// Resolve the optional provider temperature from an environment-style value.
#[must_use]
pub fn resolve_temperature(
    default: Option<f64>,
    model: &str,
    raw_override: Option<&str>,
) -> Option<f64> {
    let raw = raw_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(raw) = raw {
        if matches!(
            raw.to_ascii_lowercase().as_str(),
            "none" | "omit" | "default"
        ) {
            return None;
        }
        if let Ok(value) = raw.parse::<f64>() {
            return Some(value);
        }
    }
    if model_requires_default_temperature(model) {
        None
    } else {
        default
    }
}

/// Resolve a strictly positive integer override, otherwise preserving default.
#[must_use]
pub fn resolve_positive_usize(default: usize, raw_override: Option<&str>) -> usize {
    raw_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

/// Resolve a strictly positive floating-point override.
#[must_use]
pub fn resolve_positive_seconds(default: f64, raw_override: Option<&str>) -> f64 {
    raw_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| *value > 0.0)
        .unwrap_or(default)
}

/// Resolve a non-negative provider retry count; zero explicitly disables it.
#[must_use]
pub fn resolve_max_retries(default: usize, raw_override: Option<&str>) -> usize {
    raw_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}

/// Derive Ollama's context request without over-allocating its KV cache.
#[must_use]
pub fn ollama_extra_body(
    user_message: &str,
    max_completion_tokens: usize,
    raw_num_ctx: Option<&str>,
    keep_alive: Option<&str>,
) -> Value {
    let estimated_input = user_message.chars().count() / 4 + 400;
    let automatic = (estimated_input + max_completion_tokens + 2_000).clamp(8_192, 131_072);
    let num_ctx = raw_num_ctx
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(automatic);
    serde_json::json!({
        "options": {"num_ctx": num_ctx},
        "keep_alive": keep_alive.unwrap_or("30m")
    })
}

/// Normalize old single-object and new streamed-array Claude CLI envelopes.
pub fn claude_cli_envelope(stdout: &str) -> Result<Value, SemanticError> {
    let envelope = serde_json::from_str::<Value>(stdout)
        .map_err(|error| SemanticError::InvalidEnvelope(error.to_string()))?;
    let Value::Array(events) = envelope else {
        return Ok(envelope);
    };
    if let Some(result) = events
        .iter()
        .rev()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("result"))
    {
        return Ok(result.clone());
    }
    if let Some(last) = events.last().filter(|event| event.is_object()) {
        return Ok(last.clone());
    }
    Err(SemanticError::InvalidEnvelope(
        "JSON array has no result object".to_owned(),
    ))
}

/// Immutable provider defaults used to construct native transport requests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackendSpec {
    pub name: &'static str,
    pub base_url: Option<&'static str>,
    pub default_model: &'static str,
    pub api_key_variables: &'static [&'static str],
    pub model_variable: Option<&'static str>,
    pub input_price_per_million: f64,
    pub output_price_per_million: f64,
    pub temperature: Option<f64>,
    pub max_output_tokens: usize,
    pub reasoning_effort: Option<&'static str>,
    pub vision: bool,
}

pub const BUILTIN_BACKENDS: &[BackendSpec] = &[
    BackendSpec {
        name: "claude",
        base_url: Some("https://api.anthropic.com"),
        default_model: "claude-sonnet-4-6",
        api_key_variables: &["ANTHROPIC_API_KEY"],
        model_variable: Some("ANTHROPIC_MODEL"),
        input_price_per_million: 3.0,
        output_price_per_million: 15.0,
        temperature: Some(0.0),
        max_output_tokens: 16_384,
        reasoning_effort: None,
        vision: true,
    },
    BackendSpec {
        name: "kimi",
        base_url: Some("https://api.moonshot.ai/v1"),
        default_model: "kimi-k2.6",
        api_key_variables: &["MOONSHOT_API_KEY"],
        model_variable: None,
        input_price_per_million: 0.74,
        output_price_per_million: 4.66,
        temperature: None,
        max_output_tokens: 16_384,
        reasoning_effort: None,
        vision: true,
    },
    BackendSpec {
        name: "ollama",
        base_url: Some("http://localhost:11434/v1"),
        default_model: "qwen2.5-coder:7b",
        api_key_variables: &["OLLAMA_API_KEY"],
        model_variable: Some("OLLAMA_MODEL"),
        input_price_per_million: 0.0,
        output_price_per_million: 0.0,
        temperature: Some(0.0),
        max_output_tokens: 16_384,
        reasoning_effort: None,
        vision: false,
    },
    BackendSpec {
        name: "gemini",
        base_url: Some("https://generativelanguage.googleapis.com/v1beta/openai/"),
        default_model: "gemini-3-flash-preview",
        api_key_variables: &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
        model_variable: Some("GRAPHIFY_GEMINI_MODEL"),
        input_price_per_million: 0.50,
        output_price_per_million: 3.0,
        temperature: Some(0.0),
        max_output_tokens: 16_384,
        reasoning_effort: Some("low"),
        vision: true,
    },
    BackendSpec {
        name: "openai",
        base_url: Some("https://api.openai.com/v1"),
        default_model: "gpt-4.1-mini",
        api_key_variables: &["OPENAI_API_KEY"],
        model_variable: Some("GRAPHIFY_OPENAI_MODEL"),
        input_price_per_million: 0.40,
        output_price_per_million: 1.60,
        temperature: Some(0.0),
        max_output_tokens: 16_384,
        reasoning_effort: None,
        vision: true,
    },
    BackendSpec {
        name: "deepseek",
        base_url: Some("https://api.deepseek.com"),
        default_model: "deepseek-v4-flash",
        api_key_variables: &["DEEPSEEK_API_KEY"],
        model_variable: Some("GRAPHIFY_DEEPSEEK_MODEL"),
        input_price_per_million: 0.14,
        output_price_per_million: 0.28,
        temperature: Some(0.0),
        max_output_tokens: 16_384,
        reasoning_effort: None,
        vision: false,
    },
    BackendSpec {
        name: "azure",
        base_url: None,
        default_model: "gpt-4o",
        api_key_variables: &["AZURE_OPENAI_API_KEY"],
        model_variable: Some("GRAPHIFY_AZURE_MODEL"),
        input_price_per_million: 2.50,
        output_price_per_million: 10.0,
        temperature: Some(0.0),
        max_output_tokens: 16_384,
        reasoning_effort: None,
        vision: false,
    },
    BackendSpec {
        name: "bedrock",
        base_url: None,
        default_model: "anthropic.claude-3-5-sonnet-20241022-v2:0",
        api_key_variables: &[],
        model_variable: Some("GRAPHIFY_BEDROCK_MODEL"),
        input_price_per_million: 3.0,
        output_price_per_million: 15.0,
        temperature: Some(0.0),
        max_output_tokens: 16_384,
        reasoning_effort: None,
        vision: true,
    },
    BackendSpec {
        name: "claude-cli",
        base_url: None,
        default_model: "claude-code-plan",
        api_key_variables: &[],
        model_variable: Some("GRAPHIFY_CLAUDE_CLI_MODEL"),
        input_price_per_million: 0.0,
        output_price_per_million: 0.0,
        temperature: Some(0.0),
        max_output_tokens: 16_384,
        reasoning_effort: None,
        vision: true,
    },
];

#[must_use]
pub fn builtin_backend(name: &str) -> Option<&'static BackendSpec> {
    BUILTIN_BACKENDS.iter().find(|backend| backend.name == name)
}

/// Return the first non-empty API key accepted by a provider.
#[must_use]
pub fn backend_api_key<'a>(
    backend: &BackendSpec,
    environment: &'a HashMap<String, String>,
) -> Option<&'a str> {
    backend
        .api_key_variables
        .iter()
        .filter_map(|key| environment.get(*key))
        .find(|value| !value.is_empty())
        .map(String::as_str)
}

/// Detect built-in providers using the compatibility priority order.
#[must_use]
pub fn detect_builtin_backend(environment: &HashMap<String, String>) -> Option<&'static str> {
    for name in ["gemini", "kimi", "claude", "openai", "deepseek"] {
        let backend = builtin_backend(name)?;
        if backend_api_key(backend, environment).is_some() {
            return Some(name);
        }
    }
    let azure = builtin_backend("azure")?;
    if backend_api_key(azure, environment).is_some()
        && environment
            .get("AZURE_OPENAI_ENDPOINT")
            .is_some_and(|value| !value.is_empty())
    {
        return Some("azure");
    }
    if ["AWS_PROFILE", "AWS_REGION", "AWS_DEFAULT_REGION"]
        .into_iter()
        .any(|key| environment.get(key).is_some_and(|value| !value.is_empty()))
    {
        return Some("bedrock");
    }
    environment
        .get("OLLAMA_BASE_URL")
        .filter(|value| !value.is_empty())
        .map(|_| "ollama")
}

#[must_use]
pub fn estimate_cost(backend: &BackendSpec, input_tokens: u64, output_tokens: u64) -> f64 {
    (input_tokens as f64 * backend.input_price_per_million
        + output_tokens as f64 * backend.output_price_per_million)
        / 1_000_000.0
}

/// Return the compatibility extraction prompt used for cache fingerprinting
/// and provider system messages.
#[must_use]
pub fn extraction_prompt(deep: bool) -> String {
    if deep {
        format!("{EXTRACTION_PROMPT}{DEEP_EXTRACTION_SUFFIX}")
    } else {
        EXTRACTION_PROMPT.to_owned()
    }
}

/// Build the OpenAI SDK-compatible call parameters used by Gemini, Kimi,
/// Ollama, OpenAI, DeepSeek, Azure, and custom compatible providers.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn openai_call_parameters(
    base_url: &str,
    model: &str,
    user_message: &str,
    temperature: Option<f64>,
    reasoning_effort: Option<&str>,
    max_completion_tokens: usize,
    backend: &str,
    deep_mode: bool,
    explicit_extra_body: Option<&Value>,
    disable_thinking: bool,
    ollama_num_ctx: Option<&str>,
    ollama_keep_alive: Option<&str>,
) -> Value {
    let mut parameters = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": extraction_prompt(deep_mode)},
            {"role": "user", "content": user_message}
        ],
        "max_completion_tokens": max_completion_tokens,
        "stream": false
    });
    let Some(object) = parameters.as_object_mut() else {
        return parameters;
    };
    if let Some(temperature) = temperature.and_then(serde_json::Number::from_f64) {
        object.insert("temperature".to_owned(), Value::Number(temperature));
    }
    if let Some(reasoning_effort) = reasoning_effort {
        object.insert(
            "reasoning_effort".to_owned(),
            Value::String(reasoning_effort.to_owned()),
        );
    }
    let extra_body = if let Some(explicit) = explicit_extra_body {
        Some(explicit.clone())
    } else if base_url.contains("moonshot") || disable_thinking {
        Some(serde_json::json!({"thinking":{"type":"disabled"}}))
    } else {
        None
    };
    if let Some(extra_body) = extra_body {
        object.insert("extra_body".to_owned(), extra_body);
    }
    if backend == "ollama" && explicit_extra_body.is_none() {
        object.insert(
            "extra_body".to_owned(),
            ollama_extra_body(
                user_message,
                max_completion_tokens,
                ollama_num_ctx,
                ollama_keep_alive,
            ),
        );
    }
    parameters
}

/// Convert an OpenAI-compatible HTTP response into Trail's semantic result.
pub fn normalize_openai_response(response: &Value, model: &str) -> Result<Value, SemanticError> {
    let choice = response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| {
            SemanticError::InvalidProviderResponse("missing response choice".to_owned())
        })?;
    let message = choice
        .get("message")
        .filter(|message| message.is_object())
        .ok_or_else(|| {
            SemanticError::InvalidProviderResponse("missing response message".to_owned())
        })?;
    let raw_content = message.get("content").and_then(Value::as_str);
    let content_for_parse = raw_content
        .filter(|content| !content.is_empty())
        .unwrap_or("{}");
    let mut result = parse_llm_json(content_for_parse);
    let Some(object) = result.as_object_mut() else {
        return Err(SemanticError::InvalidProviderResponse(
            "parsed fragment was not an object".to_owned(),
        ));
    };
    let usage = response.get("usage");
    object.insert(
        "input_tokens".to_owned(),
        Value::from(numeric_u64(
            usage.and_then(|value| value.get("prompt_tokens")),
        )),
    );
    object.insert(
        "output_tokens".to_owned(),
        Value::from(numeric_u64(
            usage.and_then(|value| value.get("completion_tokens")),
        )),
    );
    object.insert("model".to_owned(), Value::String(model.to_owned()));
    object.insert(
        "finish_reason".to_owned(),
        choice.get("finish_reason").cloned().unwrap_or(Value::Null),
    );
    if response_is_hollow(raw_content, &result)
        && result.get("finish_reason").and_then(Value::as_str) != Some("length")
    {
        result["finish_reason"] = Value::String("length".to_owned());
    }
    Ok(result)
}

/// Convert an Anthropic Messages response into the common semantic result.
pub fn normalize_anthropic_response(response: &Value, model: &str) -> Result<Value, SemanticError> {
    let raw_content = response
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str);
    let content_for_parse = raw_content
        .filter(|content| !content.is_empty())
        .unwrap_or("{}");
    let mut result = parse_llm_json(content_for_parse);
    let Some(object) = result.as_object_mut() else {
        return Err(SemanticError::InvalidProviderResponse(
            "parsed fragment was not an object".to_owned(),
        ));
    };
    let usage = response.get("usage");
    object.insert(
        "input_tokens".to_owned(),
        Value::from(numeric_u64(
            usage.and_then(|value| value.get("input_tokens")),
        )),
    );
    object.insert(
        "output_tokens".to_owned(),
        Value::from(numeric_u64(
            usage.and_then(|value| value.get("output_tokens")),
        )),
    );
    object.insert("model".to_owned(), Value::String(model.to_owned()));
    let finish = if response.get("stop_reason").and_then(Value::as_str) == Some("max_tokens") {
        "length"
    } else {
        "stop"
    };
    object.insert("finish_reason".to_owned(), Value::String(finish.to_owned()));
    if response_is_hollow(raw_content, &result)
        && result.get("finish_reason").and_then(Value::as_str) != Some("length")
    {
        result["finish_reason"] = Value::String("length".to_owned());
    }
    Ok(result)
}

fn numeric_u64(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_u64)
        .or_else(|| {
            value
                .and_then(Value::as_i64)
                .and_then(|number| number.try_into().ok())
        })
        .unwrap_or(0)
}

/// A JSON-over-HTTP provider request. This intentionally has no `Debug`
/// implementation because headers may contain API credentials.
pub struct JsonRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

/// Convert SDK-style OpenAI call parameters to the provider's wire request.
pub fn openai_http_request(
    base_url: &str,
    api_key: &str,
    mut parameters: Value,
) -> Result<JsonRequest, SemanticError> {
    let object = parameters.as_object_mut().ok_or_else(|| {
        SemanticError::InvalidProviderResponse("call parameters must be an object".to_owned())
    })?;
    if let Some(extra) = object.remove("extra_body") {
        let extra = extra.as_object().ok_or_else(|| {
            SemanticError::InvalidProviderResponse("extra_body must be an object".to_owned())
        })?;
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    Ok(JsonRequest {
        url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
        headers: vec![("Authorization".to_owned(), format!("Bearer {api_key}"))],
        body: parameters,
    })
}

/// Construct an Anthropic Messages API request for text extraction.
#[must_use]
pub fn anthropic_http_request(
    base_url: &str,
    api_key: &str,
    model: &str,
    user_message: &str,
    max_tokens: usize,
    deep_mode: bool,
) -> JsonRequest {
    JsonRequest {
        url: format!("{}/v1/messages", base_url.trim_end_matches('/')),
        headers: vec![
            ("x-api-key".to_owned(), api_key.to_owned()),
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
        ],
        body: serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": extraction_prompt(deep_mode),
            "messages": [{"role":"user","content":user_message}]
        }),
    }
}

/// Execute a bounded, non-redirecting provider request with transient retries.
pub fn execute_json_request(
    request: &JsonRequest,
    timeout: Duration,
    max_retries: usize,
) -> Result<Value, SemanticError> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    for attempt in 0..=max_retries {
        let mut builder = agent.post(&request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        match builder.send_json(&request.body) {
            Ok(mut response) => {
                let status = response.status().as_u16();
                if (200..300).contains(&status) {
                    return response
                        .body_mut()
                        .with_config()
                        .limit(PROVIDER_RESPONSE_MAX_BYTES)
                        .read_json::<Value>()
                        .map_err(|error| {
                            SemanticError::Transport(format!("invalid JSON response: {error}"))
                        });
                }
                if attempt < max_retries && transient_http_status(status) {
                    thread::sleep(retry_delay(attempt));
                    continue;
                }
                return Err(SemanticError::Transport(format!(
                    "provider returned HTTP {status}"
                )));
            }
            Err(error) if attempt < max_retries && transient_transport_error(&error) => {
                thread::sleep(retry_delay(attempt));
            }
            Err(error) => return Err(SemanticError::Transport(error.to_string())),
        }
    }
    Err(SemanticError::Transport(
        "provider retry loop exhausted".to_owned(),
    ))
}

fn transient_http_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 425 | 429) || status >= 500
}

fn transient_transport_error(error: &ureq::Error) -> bool {
    matches!(
        error,
        ureq::Error::Io(_)
            | ureq::Error::Timeout(_)
            | ureq::Error::HostNotFound
            | ureq::Error::ConnectionFailed
    )
}

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(250_u64.saturating_mul(1_u64 << attempt.min(5)))
}

/// Load a fragment with a metadata size gate before allocating its payload.
pub fn load_validated_semantic_fragment(path: &Path) -> Result<Value, SemanticError> {
    let metadata = fs::metadata(path).map_err(|source| SemanticError::Stat {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_SEMANTIC_FRAGMENT_BYTES {
        return Err(SemanticError::InvalidFragment(format!(
            "payload is {} bytes; max is {}",
            metadata.len(),
            MAX_SEMANTIC_FRAGMENT_BYTES
        )));
    }
    let bytes = fs::read(path).map_err(|source| SemanticError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut fragment = serde_json::from_slice(&bytes).map_err(SemanticError::InvalidJson)?;
    let errors = validate_semantic_fragment(&mut fragment);
    if errors.is_empty() {
        Ok(fragment)
    } else {
        Err(SemanticError::InvalidFragment(errors.join("; ")))
    }
}

/// Remove invalid semantic pseudo-nodes, attach rationale prose to its target,
/// and repair hyperedges after node removal.
pub fn sanitize_semantic_fragment(fragment: &mut Value) {
    let Some(object) = fragment.as_object_mut() else {
        return;
    };
    let mut nodes = object
        .remove("nodes")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let edges = object
        .remove("edges")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut hyperedges = object
        .remove("hyperedges")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();

    let node_indexes = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| Some((node.get("id")?.as_str()?.to_owned(), index)))
        .collect::<HashMap<_, _>>();
    let rationale_sources = edges
        .iter()
        .filter(|edge| edge.get("relation").and_then(Value::as_str) == Some("rationale_for"))
        .filter_map(|edge| edge.get("source")?.as_str().map(str::to_owned))
        .collect::<HashSet<_>>();
    let mut remove = HashSet::new();
    let mut rationales = Vec::new();
    for node in &nodes {
        let Some(id) = node
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let file_type = node
            .get("file_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let label = node
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matches!(file_type, "rationale" | "concept")
            || (rationale_sources.contains(id) && sentence_like(label))
        {
            remove.insert(id.to_owned());
            if sentence_like(label) {
                rationales.push((id.to_owned(), label.trim().to_owned()));
            }
        }
    }
    for (source, text) in rationales {
        let targets = edges.iter().filter_map(|edge| {
            (edge.get("relation").and_then(Value::as_str) == Some("rationale_for")
                && edge.get("source").and_then(Value::as_str) == Some(source.as_str()))
            .then(|| edge.get("target")?.as_str())
            .flatten()
        });
        for target in targets {
            if remove.contains(target) {
                continue;
            }
            if let Some(node) = node_indexes
                .get(target)
                .and_then(|index| nodes.get_mut(*index))
                .and_then(Value::as_object_mut)
            {
                append_rationale(node, &text);
            }
        }
    }
    nodes.retain(|node| {
        node.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && !remove.contains(id))
    });
    let edges = edges
        .into_iter()
        .filter(|edge| {
            edge.get("source")
                .and_then(Value::as_str)
                .is_none_or(|id| !remove.contains(id))
                && edge
                    .get("target")
                    .and_then(Value::as_str)
                    .is_none_or(|id| !remove.contains(id))
        })
        .collect::<Vec<_>>();
    let surviving = nodes
        .iter()
        .filter_map(|node| node.get("id")?.as_str())
        .collect::<HashSet<_>>();
    hyperedges.retain_mut(|hyperedge| {
        let Some(item) = hyperedge.as_object_mut() else {
            return false;
        };
        normalize_hyperedge_members(item);
        let Some(members) = item.get_mut("nodes").and_then(Value::as_array_mut) else {
            return false;
        };
        members.retain(|member| member.as_str().is_some_and(|id| surviving.contains(id)));
        members.len() >= 2
    });

    object.insert("nodes".to_owned(), Value::Array(nodes));
    object.insert("edges".to_owned(), Value::Array(edges));
    object.insert("hyperedges".to_owned(), Value::Array(hyperedges));
}

fn sentence_like(label: &str) -> bool {
    let label = label.trim();
    if label.is_empty()
        || (label.chars().count() < RATIONALE_MIN_CHARS
            && label.split_whitespace().count() < RATIONALE_MIN_WORDS)
    {
        return false;
    }
    label.contains(['.', '!', '?', ':'])
}

fn append_rationale(node: &mut Map<String, Value>, text: &str) {
    let existing = node
        .get("rationale")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let rationale = if existing.is_empty() {
        text.to_owned()
    } else {
        format!("{existing}\n\n{text}")
    };
    node.insert("rationale".to_owned(), Value::String(rationale));
}

fn normalize_hyperedge_members(hyperedge: &mut Map<String, Value>) {
    if !hyperedge.get("nodes").is_some_and(Value::is_array) {
        for alias in ["members", "node_ids"] {
            if let Some(values) = hyperedge.get(alias).and_then(Value::as_array) {
                let mut deduped = Vec::new();
                for value in values {
                    if !deduped.contains(value) {
                        deduped.push(value.clone());
                    }
                }
                hyperedge.insert("nodes".to_owned(), Value::Array(deduped));
                break;
            }
        }
    }
    hyperedge.remove("members");
    hyperedge.remove("node_ids");
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::thread::JoinHandle;

    use serde_json::json;

    use super::*;

    type CapturedRequests = JoinHandle<Result<Vec<String>, std::io::Error>>;

    fn valid_fragment() -> Value {
        json!({
            "nodes": [{"id": "module_func", "label": "func", "file_type": "code"}],
            "edges": [{"source": "module_func", "target": "other_node"}],
            "hyperedges": []
        })
    }

    fn read_http_request(socket: &mut TcpStream) -> Result<String, std::io::Error> {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        let (header_end, content_length, chunked) = loop {
            let count = socket.read(&mut buffer)?;
            if count == 0 {
                return Ok(String::from_utf8_lossy(&request).into_owned());
            }
            request.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                });
                let chunked = headers.lines().any(|line| {
                    line.split_once(':').is_some_and(|(name, value)| {
                        name.eq_ignore_ascii_case("transfer-encoding")
                            && value.trim().eq_ignore_ascii_case("chunked")
                    })
                });
                break (header_end, content_length, chunked);
            }
        };
        let expected = content_length.map(|length| header_end + 4 + length);
        while expected.is_some_and(|length| request.len() < length)
            || (chunked && !request.ends_with(b"0\r\n\r\n"))
        {
            let count = socket.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
        }
        Ok(String::from_utf8_lossy(&request).into_owned())
    }

    fn spawn_http_server(
        responses: Vec<String>,
    ) -> Result<(SocketAddr, CapturedRequests), std::io::Error> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let server = thread::spawn(move || {
            let mut requests = Vec::with_capacity(responses.len());
            for response in responses {
                let (mut socket, _) = listener.accept()?;
                requests.push(read_http_request(&mut socket)?);
                socket.write_all(response.as_bytes())?;
            }
            Ok(requests)
        });
        Ok((address, server))
    }

    #[test]
    fn validation_rejects_hostile_ids_and_normalizes_aliases() {
        let mut fragment = valid_fragment();
        fragment["nodes"][0]["id"] = Value::String("../etc/passwd".to_owned());
        fragment["hyperedges"] = json!([{
            "id": "组:一", "node_ids": ["module_func", "other_node", "other_node"]
        }]);
        let errors = validate_semantic_fragment(&mut fragment);
        assert!(errors.iter().any(|error| error.contains("nodes[0].id")));
        assert_eq!(
            fragment["hyperedges"][0]["nodes"],
            json!(["module_func", "other_node"])
        );
        assert!(fragment["hyperedges"][0].get("node_ids").is_none());
    }

    #[test]
    fn validation_enforces_configurable_caps() {
        let mut fragment = valid_fragment();
        let limits = ValidationLimits {
            max_bytes: 64,
            max_nodes: 0,
            max_edges: 0,
            ..ValidationLimits::default()
        };
        let errors = validate_semantic_fragment_with_limits(&mut fragment, limits);
        assert!(errors.iter().any(|error| error.contains("payload")));
        assert!(errors.iter().any(|error| error.contains("nodes has 1")));
        assert!(errors.iter().any(|error| error.contains("edges has 1")));
    }

    #[test]
    fn validation_counts_python_default_separator_spaces() -> Result<(), serde_json::Error> {
        let mut fragment = json!({"nodes": [], "edges": []});
        let compact = serde_json::to_vec(&fragment)?.len() as u64;
        let errors = validate_semantic_fragment_with_limits(
            &mut fragment,
            ValidationLimits {
                max_bytes: compact,
                ..ValidationLimits::default()
            },
        );
        assert_eq!(
            errors,
            vec![format!(
                "payload is {} bytes; max is {compact}",
                compact + 3
            )]
        );
        Ok(())
    }

    #[test]
    fn cleanup_attaches_only_explicit_rationale_and_repairs_hyperedges() {
        let mut fragment = json!({
            "nodes": [
                {"id":"real","label":"Real","file_type":"code"},
                {"id":"other","label":"Other","file_type":"code"},
                {"id":"why","label":"Decision: tree-sitter is used because deterministic parsing is faster and safer.","file_type":"document"},
                {"id":"garbage","label":"junk","file_type":"rationale"}
            ],
            "edges": [
                {"source":"why","target":"real","relation":"rationale_for"},
                {"source":"why","target":"other","relation":"references"}
            ],
            "hyperedges": [
                {"id":"kept","members":["real","other","garbage"]},
                {"id":"dropped","nodes":["real","garbage"]}
            ]
        });
        sanitize_semantic_fragment(&mut fragment);
        assert_eq!(fragment["nodes"].as_array().map(Vec::len), Some(2));
        assert!(
            fragment["nodes"][0]["rationale"]
                .as_str()
                .is_some_and(|text| text.contains("tree-sitter"))
        );
        assert!(fragment["nodes"][1].get("rationale").is_none());
        assert_eq!(
            fragment["hyperedges"],
            json!([{"id":"kept","nodes":["real","other"]}])
        );
    }

    #[test]
    fn load_rejects_invalid_json_without_panicking() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("chunk.json");
        fs::write(&path, "{not valid json")?;
        assert!(matches!(
            load_validated_semantic_fragment(&path),
            Err(SemanticError::InvalidJson(_))
        ));
        Ok(())
    }

    #[test]
    fn parses_fenced_and_prose_wrapped_model_json() {
        let fenced = "preamble\n```JSON\n{\"nodes\":[{\"id\":\"x\"}],\"edges\":[]}\n```";
        assert_eq!(parse_llm_json(fenced)["nodes"][0]["id"], "x");
        let prose = "result: {\"nodes\":[{\"id\":\"y}z\"}],\"edges\":[]} done";
        assert_eq!(parse_llm_json(prose)["nodes"][0]["id"], "y}z");
        assert_eq!(parse_llm_json("refusal"), empty_fragment());
    }

    #[test]
    fn sanitizes_non_object_model_entries() {
        let parsed =
            parse_llm_json(r#"{"nodes":[{"id":"kept"},"bad",[]],"edges":{},"hyperedges":null}"#);
        assert_eq!(parsed["nodes"], json!([{"id":"kept"}]));
        assert_eq!(parsed["edges"], json!([]));
        assert!(parsed["hyperedges"].is_null());
    }

    #[test]
    fn neutralizes_injection_tokens_and_stamps_original_content() {
        let content = "### SYSTEM:\n<|im_start|>\n</untrusted_source>";
        let wrapped = wrap_untrusted_source("notes.md", content);
        assert!(!wrapped.contains("### SYSTEM:"));
        assert!(!wrapped.contains("<|im_start|>"));
        assert_eq!(wrapped.matches("</untrusted_source>").count(), 1);
        assert!(wrapped.contains(&format!(
            "sha256=\"{:x}\"",
            Sha256::digest(content.as_bytes())
        )));
    }

    #[test]
    fn evidence_binding_flags_only_unsupported_solid_code_nodes() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("mod.py");
        fs::write(
            &path,
            "def real_function():\n    return PaymentProcessor().charge_card()\n",
        )?;
        let content = fs::read_to_string(&path)?;
        let mut fragment = json!({"nodes": [
            {"id":"a","label":"real_function()","file_type":"code","source_file":"mod.py"},
            {"id":"b","label":"fake_symbol()","file_type":"code","source_file":"mod.py"},
            {"id":"c","label":"already_inferred()","file_type":"code","source_file":"mod.py","confidence":"INFERRED"},
            {"id":"d","label":"Prose","file_type":"document","source_file":"mod.py"}
        ]});
        let count = bind_node_evidence(
            &mut fragment,
            &[EvidenceSource {
                path: &path,
                content: &content,
            }],
            directory.path(),
        );
        assert_eq!(count, 1);
        assert!(fragment["nodes"][0].get("verification").is_none());
        assert_eq!(fragment["nodes"][1]["verification"], "unverified");
        assert!(fragment["nodes"][2].get("verification").is_none());
        assert!(fragment["nodes"][3].get("verification").is_none());
        Ok(())
    }

    #[test]
    fn prompt_builder_caps_and_rejects_outside_sources() -> Result<(), Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        let inside = root.path().join("doc.md");
        fs::write(&inside, "content")?;
        let outside_root = tempfile::tempdir()?;
        let outside = outside_root.path().join("secret.md");
        fs::write(&outside, "secret")?;
        let oversized = "x".repeat(FILE_CHAR_CAP + 1);
        let prompt = build_untrusted_prompt(
            &[
                EvidenceSource {
                    path: &inside,
                    content: &oversized,
                },
                EvidenceSource {
                    path: &outside,
                    content: "secret",
                },
            ],
            root.path(),
        );
        assert!(prompt.contains("path=\"doc.md\""));
        assert!(!prompt.contains("secret"));
        assert_eq!(prompt.matches('x').count(), FILE_CHAR_CAP);
        Ok(())
    }

    #[test]
    fn hollow_context_and_partial_helpers_preserve_retry_state() {
        assert!(response_is_hollow(None, &json!({})));
        assert!(response_is_hollow(Some(" {} "), &json!({})));
        assert!(!response_is_hollow(
            Some("json"),
            &json!({"nodes": [{"id":"x"}]})
        ));
        assert!(looks_like_context_exceeded(
            "maximum context length exceeded"
        ));
        assert!(!looks_like_context_exceeded("authentication failed"));

        let mut result = json!({
            "nodes": [{"id":"x","source_file":"x.md"}],
            "edges": [{"source":"x","target":"y","source_file":"y.md"}],
            "hyperedges": [],
            "_partial_files": ["big.md"]
        });
        mark_partial(&mut result);
        assert_eq!(partial_source_files(&result), ["big.md", "x.md", "y.md"]);
        strip_partial_markers(&mut result);
        assert!(result["nodes"][0].get("_partial").is_none());
    }

    #[test]
    fn provider_option_resolution_handles_reasoning_models_and_ollama() {
        assert!(model_requires_default_temperature("openai/o3-mini"));
        assert!(!model_requires_default_temperature("gpt-4.1-mini"));
        assert_eq!(resolve_temperature(Some(0.0), "o3-mini", None), None);
        assert_eq!(
            resolve_temperature(Some(0.0), "o3-mini", Some("0.7")),
            Some(0.7)
        );
        assert_eq!(resolve_positive_usize(16_384, Some("0")), 16_384);
        assert_eq!(resolve_max_retries(6, Some("0")), 0);
        assert_eq!(
            ollama_extra_body("small", 8_192, None, None),
            json!({"options":{"num_ctx":10593},"keep_alive":"30m"})
        );
    }

    #[test]
    fn claude_cli_envelope_uses_last_result_event() -> Result<(), SemanticError> {
        let envelope = claude_cli_envelope(
            r#"[{"type":"system"},{"type":"result","result":"first"},{"type":"result","result":"last"}]"#,
        )?;
        assert_eq!(envelope["result"], "last");
        Ok(())
    }

    #[test]
    fn builtin_detection_prefers_gemini_and_requires_azure_endpoint() {
        let environment = HashMap::from([
            ("OPENAI_API_KEY".to_owned(), "openai".to_owned()),
            ("GEMINI_API_KEY".to_owned(), "gemini".to_owned()),
        ]);
        assert_eq!(detect_builtin_backend(&environment), Some("gemini"));
        let azure_only = HashMap::from([("AZURE_OPENAI_API_KEY".to_owned(), "azure".to_owned())]);
        assert_eq!(detect_builtin_backend(&azure_only), None);
        let Some(openai) = builtin_backend("openai") else {
            return;
        };
        assert_eq!(estimate_cost(openai, 1_000, 2_000), 0.0036);
    }

    #[test]
    fn provider_parameters_and_responses_normalize_hollow_results() -> Result<(), SemanticError> {
        let call = openai_call_parameters(
            "http://localhost:11434/v1",
            "qwen",
            "content",
            Some(0.0),
            None,
            8_192,
            "ollama",
            false,
            None,
            false,
            None,
            None,
        );
        assert_eq!(call["stream"], false);
        assert_eq!(call["extra_body"]["keep_alive"], "30m");
        let response = json!({
            "choices":[{"message":{"content":""},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":0}
        });
        let normalized = normalize_openai_response(&response, "qwen")?;
        assert_eq!(normalized["finish_reason"], "length");
        assert_eq!(normalized["input_tokens"], 10);
        Ok(())
    }

    #[test]
    fn native_json_transport_sends_headers_and_parses_bounded_response()
    -> Result<(), Box<dyn Error>> {
        let (address, server) = spawn_http_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}".to_owned(),
        ])?;
        let request = JsonRequest {
            url: format!("http://{address}/v1/messages"),
            headers: vec![("Authorization".to_owned(), "Bearer test-key".to_owned())],
            body: json!({"hello":"world"}),
        };
        assert_eq!(
            execute_json_request(&request, Duration::from_secs(5), 0)?,
            json!({"ok":true})
        );
        let captured = server
            .join()
            .map_err(|_| std::io::Error::other("test server panicked"))??
            .pop()
            .ok_or_else(|| std::io::Error::other("server captured no request"))?;
        assert!(
            captured
                .to_ascii_lowercase()
                .contains("authorization: bearer test-key")
        );
        let (_, body) = captured
            .split_once("\r\n\r\n")
            .ok_or_else(|| std::io::Error::other("request headers were incomplete"))?;
        assert_eq!(
            serde_json::from_str::<Value>(body)?,
            json!({"hello":"world"})
        );
        Ok(())
    }

    #[test]
    fn native_json_transport_retries_transient_status() -> Result<(), Box<dyn Error>> {
        let (address, server) = spawn_http_server(vec![
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_owned(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"retried\":true}"
                .to_owned(),
        ])?;
        let request = JsonRequest {
            url: format!("http://{address}/v1/chat/completions"),
            headers: Vec::new(),
            body: json!({}),
        };
        assert_eq!(
            execute_json_request(&request, Duration::from_secs(5), 1)?,
            json!({"retried":true})
        );
        let requests = server
            .join()
            .map_err(|_| std::io::Error::other("test server panicked"))??;
        assert_eq!(requests.len(), 2);
        Ok(())
    }

    #[test]
    fn native_json_transport_refuses_redirects_and_redacts_secrets() -> Result<(), Box<dyn Error>> {
        let (address, server) = spawn_http_server(vec![
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/exfiltrate\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_owned(),
        ])?;
        let request = JsonRequest {
            url: format!("http://{address}/v1/messages"),
            headers: vec![(
                "Authorization".to_owned(),
                "Bearer super-secret-key".to_owned(),
            )],
            body: json!({"private":"corpus-secret"}),
        };
        let Err(error) = execute_json_request(&request, Duration::from_secs(5), 0) else {
            return Err(std::io::Error::other("redirect was unexpectedly followed").into());
        };
        let error = error.to_string();
        assert!(error.contains("HTTP 302"));
        assert!(!error.contains("super-secret-key"));
        assert!(!error.contains("corpus-secret"));
        let requests = server
            .join()
            .map_err(|_| std::io::Error::other("test server panicked"))??;
        assert_eq!(requests.len(), 1);
        Ok(())
    }

    #[test]
    fn native_json_transport_rejects_malformed_provider_json() -> Result<(), Box<dyn Error>> {
        let (address, server) = spawn_http_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnot-json"
                .to_owned(),
        ])?;
        let request = JsonRequest {
            url: format!("http://{address}/v1/messages"),
            headers: Vec::new(),
            body: json!({}),
        };
        let Err(error) = execute_json_request(&request, Duration::from_secs(5), 0) else {
            return Err(std::io::Error::other("malformed JSON was unexpectedly accepted").into());
        };
        let error = error.to_string();
        assert!(error.contains("invalid JSON response"));
        server
            .join()
            .map_err(|_| std::io::Error::other("test server panicked"))??;
        Ok(())
    }
}
