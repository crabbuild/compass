//! Validation and cleanup for untrusted semantic extraction fragments.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
pub const FILE_CHAR_CAP: usize = 20_000;

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

    use serde_json::json;

    use super::*;

    fn valid_fragment() -> Value {
        json!({
            "nodes": [{"id": "module_func", "label": "func", "file_type": "code"}],
            "edges": [{"source": "module_func", "target": "other_node"}],
            "hyperedges": []
        })
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
}
