//! Validation and cleanup for untrusted semantic extraction fragments.

mod bedrock;
pub use bedrock::*;
mod community_labels;
pub use community_labels::*;
mod plain_text;
pub use plain_text::*;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc2822};
use trail_files::{FileError, FileSlice, read_slice_text};
use trail_media::extract_text;
use wait_timeout::ChildExt;

pub const MAX_SEMANTIC_FRAGMENT_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_SEMANTIC_FRAGMENT_NODES: usize = 10_000;
pub const MAX_SEMANTIC_FRAGMENT_EDGES: usize = 100_000;
pub const MAX_SEMANTIC_FRAGMENT_HYPEREDGES: usize = 10_000;
pub const MAX_SEMANTIC_HYPEREDGE_NODES: usize = 256;
pub const MAX_SEMANTIC_ID_LENGTH: usize = 256;
pub const LLM_JSON_MAX_CHARS: usize = 10 * 1024 * 1024;
pub const PROVIDER_RESPONSE_MAX_BYTES: u64 = 10 * 1024 * 1024;
pub const FILE_CHAR_CAP: usize = 20_000;
pub const MAX_INLINE_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
pub const IMAGE_TOKEN_ESTIMATE: usize = 1_600;
pub const MAX_IMAGES_PER_CHUNK: usize = 20;

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
    #[error("invalid provider configuration: {0}")]
    InvalidProviderConfiguration(String),
    #[error("provider transport failed: {0}")]
    Transport(String),
    #[error("semantic cache failed: {0}")]
    Cache(#[from] FileError),
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

/// Owned source text loaded from a root-confined semantic work unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedSemanticSource {
    pub path: PathBuf,
    pub relative_path: String,
    pub content: String,
}

/// Prompt material plus non-fatal compatibility skips encountered while loading.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticReadResult {
    pub prompt: String,
    pub sources: Vec<LoadedSemanticSource>,
    pub warnings: Vec<String>,
}

/// Result of one native semantic-provider call before corpus-level merging.
pub struct DirectExtractionResult {
    pub fragment: Value,
    pub warnings: Vec<String>,
    pub unverified_nodes: usize,
}

impl SemanticReadResult {
    /// Borrow loaded source bodies for evidence validation after extraction.
    #[must_use]
    pub fn evidence_sources(&self) -> Vec<EvidenceSource<'_>> {
        self.sources
            .iter()
            .map(|source| EvidenceSource {
                path: &source.path,
                content: &source.content,
            })
            .collect()
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticUnit {
    File(PathBuf),
    Slice(FileSlice),
}

impl SemanticUnit {
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Slice(slice) => &slice.path,
        }
    }
}

/// Load semantic work units without allowing symlink escapes from the corpus.
/// Malformed binary documents produce an empty, path-bearing source block just
/// like the Python extractors; unreadable ordinary text units are skipped.
#[must_use]
pub fn read_semantic_units(units: &[SemanticUnit], root: &Path) -> SemanticReadResult {
    let mut result = SemanticReadResult {
        prompt: String::new(),
        sources: Vec::new(),
        warnings: Vec::new(),
    };
    let Ok(resolved_root) = root.canonicalize() else {
        result
            .warnings
            .push(format!("could not resolve corpus root {}", root.display()));
        return result;
    };
    for unit in units {
        let path = unit.path();
        let Ok(resolved_path) = path.canonicalize() else {
            result.warnings.push(format!(
                "could not resolve semantic source {}",
                path.display()
            ));
            continue;
        };
        if !resolved_path.starts_with(&resolved_root) {
            result.warnings.push(format!(
                "skipping {} because its symlink target is outside the corpus root",
                path.display()
            ));
            continue;
        }
        let relative_path = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let loaded = match unit {
            SemanticUnit::Slice(slice) => {
                let safe_slice = FileSlice {
                    path: resolved_path.clone(),
                    ..slice.clone()
                };
                read_slice_text(&safe_slice).ok()
            }
            SemanticUnit::File(_) => match extract_text(&resolved_path) {
                Ok(content) => Some(content),
                Err(_) if is_compat_binary_document(&resolved_path) => Some(String::new()),
                Err(_) => None,
            },
        };
        let Some(content) = loaded else {
            result
                .warnings
                .push(format!("could not read semantic source {}", path.display()));
            continue;
        };
        let content = content.chars().take(FILE_CHAR_CAP).collect::<String>();
        result.sources.push(LoadedSemanticSource {
            path: resolved_path,
            relative_path,
            content,
        });
    }
    result.prompt = result
        .sources
        .iter()
        .map(|source| wrap_untrusted_source(&source.relative_path, &source.content))
        .collect::<Vec<_>>()
        .join("\n\n");
    result
}

fn is_compat_binary_document(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["pdf", "docx", "xlsx"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

mod orchestration;
pub use orchestration::*;
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
        .filter(|value| value.is_finite() && *value > 0.0)
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

#[derive(Clone, PartialEq)]
pub struct ResolvedBackend {
    pub backend: &'static BackendSpec,
    pub base_url: Option<String>,
    pub model: String,
    api_key: Option<String>,
    pub temperature: Option<f64>,
    pub max_output_tokens: usize,
    pub timeout: Duration,
    pub max_retries: usize,
}

impl ResolvedBackend {
    #[must_use]
    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }
}

/// Resolve a built-in provider using the same environment precedence as the
/// Python implementation. The returned value intentionally has no `Debug` or
/// serialization implementation because it owns the provider credential.
pub fn resolve_builtin_backend(
    name: &str,
    environment: &HashMap<String, String>,
    explicit_model: Option<&str>,
) -> Result<ResolvedBackend, SemanticError> {
    let backend = builtin_backend(name).ok_or_else(|| {
        SemanticError::InvalidProviderConfiguration(format!("unknown built-in provider {name:?}"))
    })?;
    let base_url = resolved_base_url(name, backend, environment)?;
    if name == "ollama"
        && let Some(url) = base_url.as_deref()
    {
        let endpoint = ollama_base_url_check(url);
        if !endpoint.allowed {
            return Err(SemanticError::InvalidProviderConfiguration(
                endpoint
                    .warning
                    .unwrap_or_else(|| "unsafe Ollama endpoint".to_owned()),
            ));
        }
    }
    let model = explicit_model
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| resolved_default_model(name, backend, environment));
    let retries_override = environment.get("GRAPHIFY_MAX_RETRIES").map(String::as_str);
    let retry_default =
        if name == "ollama" && retries_override.is_none_or(|value| value.trim().is_empty()) {
            0
        } else {
            6
        };
    let timeout_seconds = resolve_positive_seconds(
        600.0,
        environment.get("GRAPHIFY_API_TIMEOUT").map(String::as_str),
    );
    Ok(ResolvedBackend {
        backend,
        base_url,
        model: model.clone(),
        api_key: backend_api_key(backend, environment).map(str::to_owned),
        temperature: resolve_temperature(
            backend.temperature,
            &model,
            environment
                .get("GRAPHIFY_LLM_TEMPERATURE")
                .map(String::as_str),
        ),
        max_output_tokens: resolve_positive_usize(
            backend.max_output_tokens,
            environment
                .get("GRAPHIFY_MAX_OUTPUT_TOKENS")
                .map(String::as_str),
        ),
        timeout: Duration::from_secs_f64(timeout_seconds),
        max_retries: resolve_max_retries(retry_default, retries_override),
    })
}

fn resolved_base_url(
    name: &str,
    backend: &BackendSpec,
    environment: &HashMap<String, String>,
) -> Result<Option<String>, SemanticError> {
    let variable = match name {
        "claude" => Some("ANTHROPIC_BASE_URL"),
        "kimi" => Some("KIMI_BASE_URL"),
        "ollama" => Some("OLLAMA_BASE_URL"),
        "gemini" => Some("GEMINI_BASE_URL"),
        "openai" => Some("OPENAI_BASE_URL"),
        "deepseek" => Some("DEEPSEEK_BASE_URL"),
        "azure" => {
            let endpoint = environment
                .get("AZURE_OPENAI_ENDPOINT")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    SemanticError::InvalidProviderConfiguration(
                        "Azure OpenAI backend requires AZURE_OPENAI_ENDPOINT".to_owned(),
                    )
                })?;
            return Ok(Some(endpoint.to_owned()));
        }
        _ => None,
    };
    Ok(variable
        .and_then(|key| environment.get(key).cloned())
        .or_else(|| backend.base_url.map(str::to_owned)))
}

fn resolved_default_model(
    name: &str,
    backend: &BackendSpec,
    environment: &HashMap<String, String>,
) -> String {
    let bootstrap = match name {
        "claude" => environment.get("ANTHROPIC_MODEL"),
        "ollama" => environment.get("OLLAMA_MODEL"),
        "openai" => environment.get("OPENAI_MODEL"),
        "azure" => environment
            .get("AZURE_OPENAI_DEPLOYMENT")
            .or_else(|| environment.get("GRAPHIFY_AZURE_MODEL")),
        _ => None,
    }
    .cloned()
    .unwrap_or_else(|| backend.default_model.to_owned());
    backend
        .model_variable
        .and_then(|key| environment.get(key))
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or(bootstrap)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointCheck {
    pub allowed: bool,
    pub warning: Option<String>,
}

impl EndpointCheck {
    fn allowed(warning: Option<String>) -> Self {
        Self {
            allowed: true,
            warning,
        }
    }

    fn rejected(warning: String) -> Self {
        Self {
            allowed: false,
            warning: Some(warning),
        }
    }
}

/// Validate a custom provider endpoint before it can receive corpus content.
#[must_use]
pub fn provider_base_url_check(base_url: &str, name: &str) -> EndpointCheck {
    let Ok(parsed) = url::Url::parse(base_url) else {
        return EndpointCheck::rejected(format!(
            "provider {name:?} has an unparseable base_url; ignoring"
        ));
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return EndpointCheck::rejected(format!(
            "provider {name:?} base_url scheme {:?} is not http/https; ignoring",
            parsed.scheme()
        ));
    }
    let Some(host) = parsed.host_str() else {
        return EndpointCheck::rejected(format!(
            "provider {name:?} base_url has no host; ignoring"
        ));
    };
    let loopback = is_loopback_host(host);
    let warning = (parsed.scheme() == "http" && !loopback).then(|| {
        format!(
            "provider {name:?} sends your corpus to {host:?} over plaintext http; use https unless this is a trusted local endpoint"
        )
    });
    EndpointCheck::allowed(warning)
}

#[must_use]
pub fn graphify_endpoint_warning(base_url: &str, name: &str, allowed: bool) -> Option<String> {
    let parsed = url::Url::parse(base_url);
    if !allowed {
        return Some(match parsed {
            Ok(parsed) => format!(
                "[graphify] WARNING: provider '{name}' base_url scheme '{}' is not http/https; ignoring.",
                parsed.scheme()
            ),
            Err(_) => format!(
                "[graphify] WARNING: provider '{name}' has an unparseable base_url; ignoring."
            ),
        });
    }
    let Ok(parsed) = parsed else { return None };
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let loopback = host == "localhost" || host == "::1" || host.starts_with("127.");
    (parsed.scheme() == "http" && !loopback).then(|| {
        format!(
            "[graphify] WARNING: provider '{name}' sends your corpus to '{host}' over plaintext http. Use https unless this is a trusted local endpoint."
        )
    })
}

/// Validate Ollama routing, including aliases that resolve to link-local or
/// cloud-metadata addresses. General LAN hosts remain allowed with a warning.
#[must_use]
pub fn ollama_base_url_check(base_url: &str) -> EndpointCheck {
    let Ok(parsed) = url::Url::parse(base_url) else {
        return EndpointCheck::allowed(Some(format!(
            "OLLAMA_BASE_URL={base_url:?} is not a parseable URL"
        )));
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return EndpointCheck::allowed(Some(format!(
            "OLLAMA_BASE_URL has unexpected scheme {:?}; expected http or https",
            parsed.scheme()
        )));
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = parsed.port_or_known_default().unwrap_or(80);
    if ollama_host_is_link_local_or_metadata(&host, port) {
        return EndpointCheck::rejected(format!(
            "OLLAMA_BASE_URL points at a link-local/metadata address ({host:?}); refusing to send the corpus there"
        ));
    }
    if is_loopback_host(&host) {
        return EndpointCheck::allowed(None);
    }
    let encryption = if parsed.scheme() == "http" {
        " (UNENCRYPTED)"
    } else {
        ""
    };
    EndpointCheck::allowed(Some(format!(
        "OLLAMA_BASE_URL points to non-loopback host {host:?}{encryption}; your full corpus will be sent to that endpoint"
    )))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn ollama_host_is_link_local_or_metadata(host: &str, port: u16) -> bool {
    if matches!(
        host,
        "metadata.google.internal" | "metadata.google.com" | "0.0.0.0" | "::" | "[::]"
    ) || host.starts_with("169.254.")
    {
        return true;
    }
    (host, port)
        .to_socket_addrs()
        .ok()
        .into_iter()
        .flatten()
        .any(|address| match address.ip() {
            IpAddr::V4(ip) => ip.is_link_local(),
            IpAddr::V6(ip) => ip.is_unicast_link_local(),
        })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomProviderLoad {
    pub providers: Map<String, Value>,
    pub warnings: Vec<String>,
}

/// A validated OpenAI-compatible custom provider. This intentionally omits
/// `Debug` and serialization because it owns the selected API credential.
#[derive(Clone, PartialEq)]
pub struct ResolvedCustomBackend {
    pub name: String,
    pub base_url: String,
    pub model: String,
    api_key: String,
    pub temperature: Option<f64>,
    pub reasoning_effort: Option<String>,
    pub max_output_tokens: usize,
    pub vision: bool,
    pub extra_body: Option<Value>,
    pub timeout: Duration,
    pub max_retries: usize,
}

impl ResolvedCustomBackend {
    #[must_use]
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}

/// Load trusted global providers and, only with explicit opt-in, project-local
/// providers. Local definitions win because Python reads them first.
#[must_use]
pub fn load_custom_providers(
    global_path: &Path,
    local_path: &Path,
    allow_local: bool,
) -> CustomProviderLoad {
    let mut loaded = CustomProviderLoad::default();
    if local_path.is_file() && !allow_local {
        loaded.warnings.push(format!(
            "ignoring project-local {} because custom providers control where corpus content and API keys are sent",
            local_path.display()
        ));
    }
    let paths = if allow_local {
        [Some(local_path), Some(global_path)]
    } else {
        [None, Some(global_path)]
    };
    for path in paths.into_iter().flatten().filter(|path| path.is_file()) {
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(Value::Object(providers)) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        for (name, mut provider) in providers {
            if builtin_backend(&name).is_some() || loaded.providers.contains_key(&name) {
                continue;
            }
            let Some(config) = provider.as_object_mut() else {
                continue;
            };
            let base_url = config
                .get("base_url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let endpoint = provider_base_url_check(base_url, &name);
            if let Some(warning) = graphify_endpoint_warning(base_url, &name, endpoint.allowed) {
                loaded.warnings.push(warning);
            }
            if !endpoint.allowed {
                continue;
            }
            config.entry("pricing").or_insert_with(|| {
                serde_json::json!({
                    "input": 0.0,
                    "output": 0.0
                })
            });
            loaded.providers.insert(name, provider);
        }
    }
    loaded
}

fn custom_provider_environment_keys(config: &Map<String, Value>) -> Vec<&str> {
    let keys = config
        .get("env_keys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    if !keys.is_empty() {
        return keys;
    }
    config
        .get("env_key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .into_iter()
        .collect()
}

/// Resolve a loaded custom provider with the same explicit/model/key
/// precedence as Graphify's OpenAI-compatible provider path.
pub fn resolve_custom_backend(
    name: &str,
    config: &Value,
    environment: &HashMap<String, String>,
    explicit_model: Option<&str>,
    explicit_api_key: Option<&str>,
) -> Result<ResolvedCustomBackend, SemanticError> {
    let config = config.as_object().ok_or_else(|| {
        SemanticError::InvalidProviderConfiguration(format!(
            "custom provider {name:?} must be a JSON object"
        ))
    })?;
    let base_url = config
        .get("base_url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SemanticError::InvalidProviderConfiguration(format!(
                "custom provider {name:?} requires base_url"
            ))
        })?;
    let endpoint = provider_base_url_check(base_url, name);
    if !endpoint.allowed {
        return Err(SemanticError::InvalidProviderConfiguration(
            endpoint
                .warning
                .unwrap_or_else(|| format!("custom provider {name:?} has an unsafe base_url")),
        ));
    }
    let default_model = config
        .get("default_model")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SemanticError::InvalidProviderConfiguration(format!(
                "custom provider {name:?} requires default_model"
            ))
        })?;
    let model_environment = config
        .get("model_env_key")
        .and_then(Value::as_str)
        .and_then(|key| environment.get(key))
        .filter(|value| !value.is_empty())
        .map(String::as_str);
    let model = explicit_model
        .filter(|value| !value.is_empty())
        .or(model_environment)
        .unwrap_or(default_model)
        .to_owned();
    let api_key = explicit_api_key
        .filter(|value| !value.is_empty())
        .or_else(|| {
            custom_provider_environment_keys(config)
                .into_iter()
                .filter_map(|key| environment.get(key))
                .find(|value| !value.is_empty())
                .map(String::as_str)
        })
        .ok_or_else(|| {
            let keys = custom_provider_environment_keys(config).join(" or ");
            SemanticError::InvalidProviderConfiguration(if keys.is_empty() {
                format!("custom provider {name:?} has no API-key environment variable")
            } else {
                format!("no API key for custom provider {name:?}; set {keys}")
            })
        })?
        .to_owned();
    let default_temperature = match config.get("temperature") {
        Some(Value::Null) => None,
        Some(value) => value.as_f64(),
        None => Some(0.0),
    };
    let configured_max = config
        .get("max_completion_tokens")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .or_else(|| config.get("max_tokens").and_then(Value::as_u64))
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(8_192);
    let extra_body = config.get("extra_body").filter(|value| !value.is_null());
    if extra_body.is_some_and(|value| !value.is_object()) {
        return Err(SemanticError::InvalidProviderConfiguration(format!(
            "custom provider {name:?} extra_body must be an object"
        )));
    }
    Ok(ResolvedCustomBackend {
        name: name.to_owned(),
        base_url: base_url.to_owned(),
        model: model.clone(),
        api_key,
        temperature: resolve_temperature(
            default_temperature,
            &model,
            environment
                .get("GRAPHIFY_LLM_TEMPERATURE")
                .map(String::as_str),
        ),
        reasoning_effort: config
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .map(str::to_owned),
        max_output_tokens: resolve_positive_usize(
            configured_max,
            environment
                .get("GRAPHIFY_MAX_OUTPUT_TOKENS")
                .map(String::as_str),
        ),
        vision: config
            .get("vision")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        extra_body: extra_body.cloned(),
        timeout: Duration::from_secs_f64(resolve_positive_seconds(
            600.0,
            environment.get("GRAPHIFY_API_TIMEOUT").map(String::as_str),
        )),
        max_retries: resolve_max_retries(
            6,
            environment.get("GRAPHIFY_MAX_RETRIES").map(String::as_str),
        ),
    })
}

/// Detect a configured custom provider only after every built-in candidate.
#[must_use]
pub fn detect_backend_with_custom<'a>(
    providers: &'a Map<String, Value>,
    environment: &HashMap<String, String>,
) -> Option<&'a str> {
    if let Some(builtin) = detect_builtin_backend(environment) {
        return Some(builtin);
    }
    providers.iter().find_map(|(name, config)| {
        let config = config.as_object()?;
        custom_provider_environment_keys(config)
            .into_iter()
            .any(|key| environment.get(key).is_some_and(|value| !value.is_empty()))
            .then_some(name.as_str())
    })
}

pub struct ImageRef {
    pub path: PathBuf,
    pub relative_path: String,
    pub media_type: String,
    pub raw: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct ImageRefBuild {
    pub images: Vec<ImageRef>,
    pub warnings: Vec<String>,
}

/// Resolve image paths under the corpus root and load only bounded inline
/// payloads. Oversized or unreadable images remain reference-only nodes.
pub fn build_image_refs(
    paths: &[PathBuf],
    root: &Path,
    read_bytes: bool,
) -> Result<ImageRefBuild, SemanticError> {
    let canonical_root = fs::canonicalize(root).map_err(|source| SemanticError::Read {
        path: root.to_path_buf(),
        source,
    })?;
    let mut built = ImageRefBuild::default();
    for path in paths {
        let candidate = if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        };
        let Ok(canonical_path) = fs::canonicalize(&candidate) else {
            built
                .warnings
                .push(format!("could not resolve image {}", path.display()));
            continue;
        };
        if !canonical_path.starts_with(&canonical_root) {
            built.warnings.push(format!(
                "skipping image {} because its symlink target is outside the corpus root",
                path.display()
            ));
            continue;
        }
        let relative_path = candidate
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let media_type = image_media_type(&candidate).to_owned();
        let raw = if read_bytes {
            match fs::metadata(&canonical_path) {
                Ok(metadata) if metadata.len() > MAX_INLINE_IMAGE_BYTES => {
                    built.warnings.push(format!(
                        "image {relative_path} is over the inline-image limit and will be reference-only"
                    ));
                    None
                }
                Ok(_) => match fs::read(&canonical_path) {
                    Ok(bytes) => Some(bytes),
                    Err(error) => {
                        built
                            .warnings
                            .push(format!("could not read image {relative_path}: {error}"));
                        None
                    }
                },
                Err(error) => {
                    built
                        .warnings
                        .push(format!("could not inspect image {relative_path}: {error}"));
                    None
                }
            }
        } else {
            None
        };
        built.images.push(ImageRef {
            path: canonical_path,
            relative_path,
            media_type,
            raw,
        });
    }
    Ok(built)
}

fn image_media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "image/png",
    }
}

#[must_use]
pub fn image_notes(images: &[ImageRef], with_paths: bool) -> String {
    if images.is_empty() {
        return String::new();
    }
    let header = if with_paths {
        "Use the Read tool to open and view each image file at the path below, then emit one node per image"
    } else {
        "The following image file(s) are attached as visual input. Emit one node per image"
    };
    let mut lines = vec![
        "=== IMAGES ===".to_owned(),
        format!(
            "{header} with \"file_type\":\"image\" and the listed source_file, a label describing what it depicts (diagram, screenshot, chart, photo, UI, logo), and edges to any code/doc nodes the image clearly references."
        ),
    ];
    for (index, image) in images.iter().enumerate() {
        let mut note = format!("[image {}] source_file: {}", index + 1, image.relative_path);
        if with_paths {
            note.push_str(&format!("  path: {}", image.path.display()));
        } else if image.raw.is_none() {
            note.push_str(" (not shown: unreadable or exceeds size limit)");
        }
        lines.push(note);
    }
    lines.join("\n")
}

#[must_use]
pub fn with_image_notes(user_message: &str, images: &[ImageRef], with_paths: bool) -> String {
    let notes = image_notes(images, with_paths);
    if notes.is_empty() {
        return user_message.to_owned();
    }
    if user_message.trim().is_empty() {
        notes
    } else {
        format!("{user_message}\n\n{notes}")
    }
}

#[must_use]
pub fn anthropic_content(user_message: &str, images: &[ImageRef]) -> Value {
    let text = with_image_notes(user_message, images, false);
    let mut blocks = images
        .iter()
        .filter_map(|image| {
            let raw = image.raw.as_deref()?;
            Some(serde_json::json!({
                "type":"image",
                "source": {
                    "type":"base64",
                    "media_type":image.media_type,
                    "data":base64::engine::general_purpose::STANDARD.encode(raw)
                }
            }))
        })
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        Value::String(text)
    } else {
        blocks.push(serde_json::json!({"type":"text","text":text}));
        Value::Array(blocks)
    }
}

#[must_use]
pub fn openai_content(user_message: &str, images: &[ImageRef]) -> Value {
    let text = with_image_notes(user_message, images, false);
    let image_parts = images.iter().filter_map(|image| {
        let raw = image.raw.as_deref()?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
        Some(serde_json::json!({
            "type":"image_url",
            "image_url": {
                "url":format!("data:{};base64,{encoded}", image.media_type),
                "detail":"auto"
            }
        }))
    });
    let parts = std::iter::once(serde_json::json!({"type":"text","text":text}))
        .chain(image_parts)
        .collect::<Vec<_>>();
    if parts.len() == 1 {
        parts.into_iter().next().map_or(Value::Null, |part| {
            part.get("text").cloned().unwrap_or(Value::Null)
        })
    } else {
        Value::Array(parts)
    }
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

#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn openai_call_parameters_with_images(
    base_url: &str,
    model: &str,
    user_message: &str,
    images: &[ImageRef],
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
    let mut parameters = openai_call_parameters(
        base_url,
        model,
        user_message,
        temperature,
        reasoning_effort,
        max_completion_tokens,
        backend,
        deep_mode,
        explicit_extra_body,
        disable_thinking,
        ollama_num_ctx,
        ollama_keep_alive,
    );
    parameters["messages"][1]["content"] = openai_content(user_message, images);
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

/// Normalize the Claude Code CLI result envelope to the shared extraction
/// response contract.
pub fn normalize_claude_cli_response(envelope: &Value) -> Result<Value, SemanticError> {
    let object = envelope.as_object().ok_or_else(|| {
        SemanticError::InvalidProviderResponse(
            "Claude CLI result envelope must be an object".to_owned(),
        )
    })?;
    let raw_content = object.get("result").and_then(Value::as_str).unwrap_or("");
    let mut result = parse_llm_json(if raw_content.is_empty() {
        "{}"
    } else {
        raw_content
    });
    let Some(parsed) = result.as_object_mut() else {
        return Err(SemanticError::InvalidProviderResponse(
            "parsed Claude CLI fragment was not an object".to_owned(),
        ));
    };
    let usage = object.get("usage");
    let input_tokens = numeric_u64(usage.and_then(|value| value.get("input_tokens")))
        .saturating_add(numeric_u64(
            usage.and_then(|value| value.get("cache_read_input_tokens")),
        ))
        .saturating_add(numeric_u64(
            usage.and_then(|value| value.get("cache_creation_input_tokens")),
        ));
    parsed.insert("input_tokens".to_owned(), Value::from(input_tokens));
    parsed.insert(
        "output_tokens".to_owned(),
        Value::from(numeric_u64(
            usage.and_then(|value| value.get("output_tokens")),
        )),
    );
    let model = object
        .get("modelUsage")
        .and_then(Value::as_object)
        .and_then(|models| models.keys().next())
        .map_or("claude-code-plan", String::as_str);
    parsed.insert("model".to_owned(), Value::String(model.to_owned()));
    let finish = if object.get("stop_reason").and_then(Value::as_str) == Some("max_tokens") {
        "length"
    } else {
        "stop"
    };
    parsed.insert("finish_reason".to_owned(), Value::String(finish.to_owned()));
    if response_is_hollow(Some(raw_content), &result)
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

/// Construct the Azure OpenAI chat-completions wire request produced by the
/// Python SDK for a resource endpoint, deployment name, and API version.
pub fn azure_openai_http_request(
    endpoint: &str,
    api_key: &str,
    deployment: &str,
    api_version: &str,
    body: Value,
) -> Result<JsonRequest, SemanticError> {
    let mut url = url::Url::parse(endpoint).map_err(|error| {
        SemanticError::InvalidProviderConfiguration(format!(
            "invalid Azure OpenAI endpoint: {error}"
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(SemanticError::InvalidProviderConfiguration(
            "Azure OpenAI endpoint must be an absolute HTTP(S) URL".to_owned(),
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut segments = url.path_segments_mut().map_err(|()| {
            SemanticError::InvalidProviderConfiguration(
                "Azure OpenAI endpoint cannot be used as a hierarchical URL".to_owned(),
            )
        })?;
        segments.pop_if_empty();
        segments.extend(["openai", "deployments", deployment, "chat", "completions"]);
    }
    url.query_pairs_mut()
        .append_pair("api-version", api_version);
    Ok(JsonRequest {
        url: url.into(),
        headers: vec![("api-key".to_owned(), api_key.to_owned())],
        body,
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
    anthropic_http_request_with_content(
        base_url,
        api_key,
        model,
        Value::String(user_message.to_owned()),
        max_tokens,
        deep_mode,
    )
}

/// Construct an Anthropic Messages API request with optional image blocks.
#[must_use]
pub fn anthropic_http_request_with_images(
    base_url: &str,
    api_key: &str,
    model: &str,
    user_message: &str,
    images: &[ImageRef],
    max_tokens: usize,
    deep_mode: bool,
) -> JsonRequest {
    anthropic_http_request_with_content(
        base_url,
        api_key,
        model,
        anthropic_content(user_message, images),
        max_tokens,
        deep_mode,
    )
}

fn anthropic_http_request_with_content(
    base_url: &str,
    api_key: &str,
    model: &str,
    user_content: Value,
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
            "messages": [{"role":"user","content":user_content}]
        }),
    }
}

fn claude_cli_message(user_message: &str, images: &[ImageRef], deep_mode: bool) -> String {
    let user_message = with_image_notes(user_message, images, true);
    format!(
        "{}\n\n---\nNow extract the knowledge graph from the following source file(s) and output ONLY the JSON object described above. No prose, no preamble, no markdown fences.\n\n{}",
        extraction_prompt(deep_mode),
        user_message
    )
}

fn read_process_stream<R: Read>(mut stream: R, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::new();
    let mut overflowed = false;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok((retained, overflowed));
        }
        let remaining = limit.saturating_sub(retained.len());
        let retain = remaining.min(count);
        retained.extend_from_slice(&buffer[..retain]);
        overflowed |= retain < count;
    }
}

fn receive_process_stream(
    receiver: &Receiver<std::io::Result<(Vec<u8>, bool)>>,
    name: &str,
) -> Result<(Vec<u8>, bool), SemanticError> {
    match receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(SemanticError::Transport(format!(
            "Claude CLI {name}: {error}"
        ))),
        Err(RecvTimeoutError::Timeout) => Err(SemanticError::Transport(format!(
            "Claude CLI {name} remained open after the process exited"
        ))),
        Err(RecvTimeoutError::Disconnected) => Err(SemanticError::Transport(format!(
            "Claude CLI {name} reader stopped unexpectedly"
        ))),
    }
}

fn execute_bounded_process(
    program: &Path,
    arguments: &[String],
    stdin: &str,
    timeout: Duration,
) -> Result<String, SemanticError> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|error| {
        SemanticError::Transport(format!(
            "could not start Claude Code CLI at {}: {error}",
            program.display()
        ))
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        SemanticError::Transport("could not capture Claude CLI stdout".to_owned())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        SemanticError::Transport("could not capture Claude CLI stderr".to_owned())
    })?;
    let (stdout_sender, stdout_receiver) = mpsc::channel();
    let (stderr_sender, stderr_receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = stdout_sender.send(read_process_stream(
            stdout,
            PROVIDER_RESPONSE_MAX_BYTES as usize,
        ));
    });
    thread::spawn(move || {
        let _ = stderr_sender.send(read_process_stream(stderr, 64 * 1024));
    });

    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| SemanticError::Transport("could not open Claude CLI stdin".to_owned()))
        .and_then(|mut input| {
            input
                .write_all(stdin.as_bytes())
                .map_err(|error| SemanticError::Transport(format!("Claude CLI stdin: {error}")))
        });
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SemanticError::Transport(format!(
                "Claude CLI timed out after {:.3} seconds",
                timeout.as_secs_f64()
            )));
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SemanticError::Transport(format!(
                "could not wait for Claude CLI: {error}"
            )));
        }
    };
    let (stdout, stdout_overflowed) = receive_process_stream(&stdout_receiver, "stdout")?;
    let (stderr, _) = receive_process_stream(&stderr_receiver, "stderr")?;
    if stdout_overflowed {
        return Err(SemanticError::Transport(format!(
            "Claude CLI response exceeded {PROVIDER_RESPONSE_MAX_BYTES} bytes"
        )));
    }
    if !status.success() {
        let message = String::from_utf8_lossy(&stderr)
            .chars()
            .take(500)
            .collect::<String>();
        return Err(SemanticError::Transport(format!(
            "Claude CLI exited {}: {}",
            status
                .code()
                .map_or_else(|| "without a status".to_owned(), |code| code.to_string()),
            message.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// Execute the authenticated local Claude Code CLI without invoking a shell.
pub fn execute_claude_cli_backend(
    backend: &ResolvedBackend,
    user_message: &str,
    images: &[ImageRef],
    deep_mode: bool,
    environment: &HashMap<String, String>,
) -> Result<Value, SemanticError> {
    if backend.backend.name != "claude-cli" {
        return Err(SemanticError::InvalidProviderConfiguration(format!(
            "backend {:?} is not the Claude CLI backend",
            backend.backend.name
        )));
    }
    let program = if cfg!(windows) {
        Path::new("claude.cmd")
    } else {
        Path::new("claude")
    };
    let arguments = claude_cli_arguments(images, environment);
    let stdout = execute_bounded_process(
        program,
        &arguments,
        &claude_cli_message(user_message, images, deep_mode),
        backend.timeout,
    )?;
    let envelope = claude_cli_envelope(&stdout)?;
    normalize_claude_cli_response(&envelope)
}

fn claude_cli_arguments(images: &[ImageRef], environment: &HashMap<String, String>) -> Vec<String> {
    let mut arguments = vec![
        "-p".to_owned(),
        "--output-format".to_owned(),
        "json".to_owned(),
        "--no-session-persistence".to_owned(),
    ];
    let mut directories = HashSet::new();
    for image in images {
        if let Some(directory) = image.path.parent() {
            let directory = directory.to_string_lossy().into_owned();
            if directories.insert(directory.clone()) {
                arguments.push("--add-dir".to_owned());
                arguments.push(directory);
            }
        }
    }
    if let Some(model) = environment
        .get("GRAPHIFY_CLAUDE_CLI_MODEL")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        arguments.push("--model".to_owned());
        arguments.push(model.to_owned());
    }
    arguments
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
                    let delay = retry_after_delay(response.headers(), OffsetDateTime::now_utc())
                        .unwrap_or_else(|| retry_delay(attempt));
                    thread::sleep(delay);
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

/// Execute a fully resolved built-in HTTP provider and normalize its response.
/// Non-HTTP providers (Bedrock and Claude CLI) intentionally remain separate
/// transports so their credential and subprocess boundaries cannot be confused
/// with bearer-token APIs.
pub fn execute_resolved_http_backend(
    backend: &ResolvedBackend,
    user_message: &str,
    images: &[ImageRef],
    deep_mode: bool,
    environment: &HashMap<String, String>,
) -> Result<Value, SemanticError> {
    let name = backend.backend.name;
    if matches!(name, "bedrock" | "claude-cli") {
        return Err(SemanticError::InvalidProviderConfiguration(format!(
            "backend {name:?} does not use the built-in JSON HTTP transport"
        )));
    }
    let base_url = backend.base_url.as_deref().ok_or_else(|| {
        SemanticError::InvalidProviderConfiguration(format!(
            "backend {name:?} has no resolved base URL"
        ))
    })?;
    let api_key = match backend.api_key() {
        Some(value) => value,
        None if name == "ollama" => "ollama",
        None => {
            return Err(SemanticError::InvalidProviderConfiguration(format!(
                "backend {name:?} has no API key"
            )));
        }
    };
    if name == "claude" {
        let request = anthropic_http_request_with_images(
            base_url,
            api_key,
            &backend.model,
            user_message,
            images,
            backend.max_output_tokens,
            deep_mode,
        );
        let response = execute_json_request(&request, backend.timeout, backend.max_retries)?;
        return normalize_anthropic_response(&response, &backend.model);
    }

    if name == "azure" {
        let parameters = openai_call_parameters(
            base_url,
            &backend.model,
            user_message,
            backend.temperature,
            None,
            backend.max_output_tokens,
            name,
            deep_mode,
            None,
            false,
            None,
            None,
        );
        let api_version = environment
            .get("AZURE_OPENAI_API_VERSION")
            .map_or("2024-12-01-preview", String::as_str)
            .trim();
        let request =
            azure_openai_http_request(base_url, api_key, &backend.model, api_version, parameters)?;
        let response = execute_json_request(&request, backend.timeout, backend.max_retries)?;
        return normalize_openai_response(&response, &backend.model);
    }

    let disable_thinking = environment
        .get("GRAPHIFY_DISABLE_THINKING")
        .is_some_and(|value| env_truthy(value));
    let parameters = openai_call_parameters_with_images(
        base_url,
        &backend.model,
        user_message,
        images,
        backend.temperature,
        backend.backend.reasoning_effort,
        backend.max_output_tokens,
        name,
        deep_mode,
        None,
        disable_thinking,
        environment
            .get("GRAPHIFY_OLLAMA_NUM_CTX")
            .map(String::as_str),
        environment
            .get("GRAPHIFY_OLLAMA_KEEP_ALIVE")
            .map(String::as_str),
    );
    let request = openai_http_request(base_url, api_key, parameters)?;
    let response = execute_json_request(&request, backend.timeout, backend.max_retries)?;
    normalize_openai_response(&response, &backend.model)
}

/// Dispatch a resolved provider through its native transport boundary.
pub fn execute_resolved_backend(
    backend: &ResolvedBackend,
    user_message: &str,
    images: &[ImageRef],
    deep_mode: bool,
    environment: &HashMap<String, String>,
) -> Result<Value, SemanticError> {
    match backend.backend.name {
        "bedrock" => execute_bedrock_backend(backend, user_message, images, deep_mode, environment),
        "claude-cli" => {
            execute_claude_cli_backend(backend, user_message, images, deep_mode, environment)
        }
        _ => execute_resolved_http_backend(backend, user_message, images, deep_mode, environment),
    }
}

/// Execute a validated custom OpenAI-compatible provider.
pub fn execute_resolved_custom_backend(
    backend: &ResolvedCustomBackend,
    user_message: &str,
    images: &[ImageRef],
    deep_mode: bool,
    environment: &HashMap<String, String>,
) -> Result<Value, SemanticError> {
    let disable_thinking = environment
        .get("GRAPHIFY_DISABLE_THINKING")
        .is_some_and(|value| env_truthy(value));
    let mut parameters = openai_call_parameters_with_images(
        &backend.base_url,
        &backend.model,
        user_message,
        images,
        backend.temperature,
        backend.reasoning_effort.as_deref(),
        backend.max_output_tokens,
        &backend.name,
        deep_mode,
        backend.extra_body.as_ref(),
        disable_thinking,
        None,
        None,
    );
    if !backend.vision {
        parameters["messages"][1]["content"] =
            Value::String(with_image_notes(user_message, images, false));
    }
    let request = openai_http_request(&backend.base_url, backend.api_key(), parameters)?;
    let response = execute_json_request(&request, backend.timeout, backend.max_retries)?;
    normalize_openai_response(&response, &backend.model)
}

fn env_truthy(value: &str) -> bool {
    ["1", "true", "yes", "on"]
        .iter()
        .any(|candidate| value.trim().eq_ignore_ascii_case(candidate))
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

fn retry_after_delay(headers: &ureq::http::HeaderMap, now: OffsetDateTime) -> Option<Duration> {
    let milliseconds = headers
        .get("retry-after-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<f64>().ok())
        .map(|value| value / 1_000.0);
    let seconds = milliseconds.or_else(|| {
        let value = headers.get("retry-after")?.to_str().ok()?.trim();
        value.parse::<f64>().ok().or_else(|| {
            let retry_at = OffsetDateTime::parse(value, &Rfc2822).ok()?;
            Some((retry_at - now).as_seconds_f64())
        })
    })?;
    (seconds.is_finite() && seconds > 0.0 && seconds <= 60.0)
        .then(|| Duration::from_secs_f64(seconds))
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
mod tests;
