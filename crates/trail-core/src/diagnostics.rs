use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::CoreError;

pub fn diagnose_graph_file(
    path: &Path,
    directed: Option<bool>,
    max_examples: usize,
    extract_path: Option<&Path>,
) -> Result<Value, CoreError> {
    enforce_graph_size_cap(path)?;
    let bytes = fs::read(path).map_err(|source| {
        CoreError::DiagnosticFile(format!(
            "Cannot parse {}: {}. The file may be corrupted — re-run 'graphify extract'.",
            path.display(),
            python_io_error(path, &source)
        ))
    })?;
    let input: Value = serde_json::from_slice(&bytes).map_err(|source| {
        CoreError::DiagnosticFile(format!(
            "Cannot parse {}: {}. The file may be corrupted — re-run 'graphify extract'.",
            path.display(),
            python_json_error(&bytes, &source)
        ))
    })?;
    let object = input.as_object().ok_or(CoreError::InvalidDiagnostic)?;
    let nodes = object
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let edges = object
        .get("edges")
        .filter(|value| !value.is_null())
        .or_else(|| object.get("links"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let effective_directed = directed.unwrap_or_else(|| {
        object
            .get("directed")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    });
    let node_ids = nodes
        .iter()
        .filter_map(|node| node.get("id").filter(|id| !id.is_null()).map(text))
        .collect::<HashSet<_>>();
    let unverified = nodes
        .iter()
        .filter(|node| node.get("verification").and_then(Value::as_str) == Some("unverified"))
        .count();
    let mut exact = HashMap::<String, usize>::new();
    let mut directed_pairs = HashMap::<(String, String), usize>::new();
    let mut undirected_pairs = HashMap::<(String, String), usize>::new();
    let mut order = Vec::new();
    let mut groups = HashMap::<(String, String), Vec<Edge>>::new();
    let (mut non_object, mut missing, mut dangling, mut loops, mut valid) = (0, 0, 0, 0, 0);
    for raw in &edges {
        *exact.entry(signature(raw)).or_default() += 1;
        let Some(edge) = Edge::new(raw) else {
            non_object += 1;
            continue;
        };
        if edge.source.is_empty() || edge.target.is_empty() {
            missing += 1;
            continue;
        }
        if !node_ids.contains(&edge.source) || !node_ids.contains(&edge.target) {
            dangling += 1;
            continue;
        }
        if edge.source == edge.target {
            loops += 1;
        }
        valid += 1;
        let pair = (edge.source.clone(), edge.target.clone());
        if !directed_pairs.contains_key(&pair) {
            order.push(pair.clone());
        }
        *directed_pairs.entry(pair.clone()).or_default() += 1;
        let undirected = if edge.source <= edge.target {
            pair.clone()
        } else {
            (edge.target.clone(), edge.source.clone())
        };
        *undirected_pairs.entry(undirected).or_default() += 1;
        groups.entry(pair).or_default().push(edge);
    }
    let examples = order
        .iter()
        .filter(|pair| directed_pairs[*pair] > 1)
        .take(max_examples)
        .map(|pair| {
            let edges = &groups[pair];
            json!({"source":pair.0,"target":pair.1,"edge_count":directed_pairs[pair],
            "relations":set(edges,|e|&e.relation),"source_files":set(edges,|e|&e.source_file),
            "source_locations":set(edges,|e|&e.location),"contexts":set(edges,|e|&e.context)})
        })
        .collect::<Vec<_>>();
    let producer_suppression = extract_path.map_or_else(
        default_producer_suppression,
        scan_producer_suppression_sites,
    );
    Ok(json!({
        "node_count":node_ids.len(),"unverified_node_count":unverified,"raw_edge_count":edges.len(),
        "non_object_edges":non_object,"missing_endpoint_edges":missing,"dangling_endpoint_edges":dangling,
        "self_loop_edges":loops,"valid_candidate_edges":valid,"exact_duplicate_edges":extra(&exact),
        "directed_unique_endpoint_pairs":directed_pairs.len(),"directed_same_endpoint_collapsed_edges":extra(&directed_pairs),
        "undirected_unique_endpoint_pairs":undirected_pairs.len(),"undirected_same_endpoint_collapsed_edges":extra(&undirected_pairs),
        "same_endpoint_group_count":directed_pairs.values().filter(|count|**count>1).count(),
        "relation_variant_groups":variants(&groups,|e|&e.relation,false),
        "source_file_variant_groups":variants(&groups,|e|&e.source_file,true),
        "source_location_variant_groups":variants(&groups,|e|&e.location,true),
        "context_variant_groups":variants(&groups,|e|&e.context,true),
        "post_build_graph_type":if effective_directed{"DiGraph"}else{"Graph"},
        "post_build_node_count":node_ids.len(),"post_build_edge_count":if effective_directed{directed_pairs.len()}else{undirected_pairs.len()},
        "post_build_error":"","producer_suppression":producer_suppression,
        "examples":examples,"input_path":path.to_string_lossy(),"effective_directed":effective_directed
    }))
}

fn enforce_graph_size_cap(path: &Path) -> Result<(), CoreError> {
    let Ok(size) = path.metadata().map(|metadata| metadata.len()) else {
        return Ok(());
    };
    let cap = graph_size_cap();
    if u128::from(size) <= cap {
        return Ok(());
    }
    Err(CoreError::DiagnosticFile(format!(
        "graph file {} is {} bytes, exceeds {}-byte cap\n(set GRAPHIFY_MAX_GRAPH_BYTES=<bytes> or GRAPHIFY_MAX_GRAPH_BYTES=<N>GB to raise the limit)",
        path.display(),
        grouped(u128::from(size)),
        grouped(cap)
    )))
}

fn graph_size_cap() -> u128 {
    const DEFAULT: u128 = 512 * 1024 * 1024;
    let Ok(raw) = std::env::var("GRAPHIFY_MAX_GRAPH_BYTES") else {
        return DEFAULT;
    };
    let upper = raw.trim().to_uppercase();
    let (number, multiplier) = if let Some(number) = upper.strip_suffix("GB") {
        (number.trim(), 1024_u128 * 1024 * 1024)
    } else if let Some(number) = upper.strip_suffix("MB") {
        (number.trim(), 1024_u128 * 1024)
    } else {
        (upper.as_str(), 1)
    };
    number
        .replace('_', "")
        .parse::<u128>()
        .ok()
        .filter(|value| *value > 0)
        .and_then(|value| value.checked_mul(multiplier))
        .unwrap_or(DEFAULT)
}

fn grouped(value: u128) -> String {
    let digits = value.to_string();
    digits
        .chars()
        .enumerate()
        .flat_map(|(index, character)| {
            let separator = (index > 0 && (digits.len() - index).is_multiple_of(3)).then_some('_');
            separator.into_iter().chain(std::iter::once(character))
        })
        .collect()
}

fn python_io_error(path: &Path, source: &std::io::Error) -> String {
    let Some(errno) = source.raw_os_error() else {
        return source.to_string();
    };
    let suffix = format!(" (os error {errno})");
    let reason = source
        .to_string()
        .strip_suffix(&suffix)
        .map_or_else(|| source.to_string(), str::to_owned);
    format!("[Errno {errno}] {reason}: '{}'", path.display())
}

fn python_json_error(bytes: &[u8], source: &serde_json::Error) -> String {
    let raw = source.to_string();
    let description = raw
        .split_once(" at line ")
        .map_or(raw.as_str(), |(description, _)| description);
    let text = String::from_utf8_lossy(bytes);
    let (message, line, column) = if matches!(description, "expected ident" | "expected value") {
        let offset = text
            .char_indices()
            .find(|(_, character)| !character.is_whitespace())
            .map_or(text.len(), |(offset, _)| offset);
        let prefix = &text[..offset];
        (
            "Expecting value".to_owned(),
            prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
            prefix
                .rsplit('\n')
                .next()
                .map_or(1, |line| line.chars().count() + 1),
        )
    } else {
        let message = match description {
            "key must be a string" => "Expecting property name enclosed in double quotes",
            "trailing characters" => "Extra data",
            value if value.starts_with("expected `,` or") => "Expecting ',' delimiter",
            value => value,
        };
        (message.to_owned(), source.line(), source.column())
    };
    let character = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::chars)
        .map(Iterator::count)
        .sum::<usize>()
        + column.saturating_sub(1);
    format!("{message}: line {line} column {column} (char {character})")
}

#[must_use]
pub fn format_diagnostic_json(summary: &Value) -> Value {
    let mut body = summary.as_object().cloned().unwrap_or_default();
    let examples = body.remove("examples").unwrap_or_else(|| json!([]));
    let producer = body
        .remove("producer_suppression")
        .unwrap_or_else(|| json!({}));
    json!({"schema_version":1,"summary":body,"examples":examples,"producer_suppression":producer,"notes":["Diagnostics are read-only.","A normal graph.json is already post-build and cannot recover raw producer edges.","Producer suppression sites are heuristic source-code evidence."]})
}

#[must_use]
pub fn format_diagnostic_report(s: &Value) -> String {
    let get = |k: &str| s.get(k).map(text).unwrap_or_default();
    let mut lines = vec![
        "[graphify] MultiDiGraph edge-collapse diagnostic".to_owned(),
        format!("input: {}", get("input_path")),
        "input_stage: provided JSON (normal graph.json is post-build)".to_owned(),
        format!("effective_directed: {}", get("effective_directed")),
    ];
    for (label, key) in [
        ("nodes", "node_count"),
        ("unverified_code_nodes", "unverified_node_count"),
        ("raw_edges", "raw_edge_count"),
        ("valid_candidate_edges", "valid_candidate_edges"),
        ("missing_endpoint_edges", "missing_endpoint_edges"),
        ("dangling_endpoint_edges", "dangling_endpoint_edges"),
        ("self_loop_edges", "self_loop_edges"),
        ("exact_duplicate_edges", "exact_duplicate_edges"),
        (
            "directed_unique_endpoint_pairs",
            "directed_unique_endpoint_pairs",
        ),
        (
            "directed_same_endpoint_collapsed_edges",
            "directed_same_endpoint_collapsed_edges",
        ),
        (
            "undirected_unique_endpoint_pairs",
            "undirected_unique_endpoint_pairs",
        ),
        (
            "undirected_same_endpoint_collapsed_edges",
            "undirected_same_endpoint_collapsed_edges",
        ),
        ("same_endpoint_group_count", "same_endpoint_group_count"),
        ("relation_variant_groups", "relation_variant_groups"),
        ("source_file_variant_groups", "source_file_variant_groups"),
        (
            "source_location_variant_groups",
            "source_location_variant_groups",
        ),
        ("context_variant_groups", "context_variant_groups"),
        ("post_build_graph_type", "post_build_graph_type"),
        ("post_build_edges", "post_build_edge_count"),
    ] {
        lines.push(format!("{label}: {}", get(key)));
    }
    let suppression = s.get("producer_suppression").unwrap_or(&Value::Null);
    lines.push(format!(
        "producer_suppression_sites: {}",
        suppression
            .get("total_sites")
            .map(text)
            .unwrap_or_else(|| "0".to_owned())
    ));
    if let Some(error) = suppression
        .get("error")
        .and_then(Value::as_str)
        .filter(|error| !error.is_empty())
    {
        lines.push(format!("producer_suppression_error: {error}"));
    }
    if let Some(sites) = suppression
        .get("sites")
        .and_then(Value::as_array)
        .filter(|sites| !sites.is_empty())
    {
        lines.push("producer_suppression_examples:".to_owned());
        for site in sites.iter().take(8) {
            let arity = site
                .get("tuple_arity")
                .and_then(Value::as_u64)
                .filter(|arity| *arity > 0)
                .map_or_else(|| "unknown".to_owned(), |arity| arity.to_string());
            lines.push(format!(
                "  - L{} {} arity={arity}",
                site.get("line").map(text).unwrap_or_default(),
                site.get("name").map(text).unwrap_or_default(),
            ));
        }
    }
    if let Some(examples) = s
        .get("examples")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    {
        lines.push("examples:".to_owned());
        for e in examples {
            lines.push(format!(
                "  - {} -> {} edges={} relations={} locations={} contexts={}",
                text(&e["source"]),
                text(&e["target"]),
                text(&e["edge_count"]),
                list(&e["relations"]),
                list(&e["source_locations"]),
                list(&e["contexts"])
            ));
        }
    }
    lines.push(
        "note: normal graph.json is post-build; raw producer loss must be measured earlier."
            .to_owned(),
    );
    lines.join("\n")
}

fn default_producer_suppression() -> Value {
    let source = std::env::current_dir()
        .ok()
        .map(|directory| directory.join("graphify").join("extract.py"));
    if let Some(path) = source.filter(|path| path.is_file()) {
        return scan_producer_suppression_sites(&path);
    }

    // Binary releases contain no Python runtime or source tree. This versioned snapshot keeps
    // the producer-risk diagnostic useful while explicit --extract-path scans remain live.
    json!({
        "path": "graphify/extract.py",
        "total_sites": 10,
        "sites": [
            {"line":967,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids = {n[\"id\"] for n in nodes}"},
            {"line":1117,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids = {n[\"id\"] for n in nodes}"},
            {"line":1119,"name":"seen_doc_refs","tuple_arity":0,"sample":"seen_doc_refs: set[str] = set()"},
            {"line":1542,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = {n[\"id\"] for n in nodes}"},
            {"line":2011,"name":"seen_keys","tuple_arity":0,"sample":"seen_keys: set[tuple] = set()"},
            {"line":2934,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = set()"},
            {"line":3042,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = set()"},
            {"line":3121,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = set()"},
            {"line":3616,"name":"seen_ids","tuple_arity":0,"sample":"seen_ids: set[str] = set()"},
            {"line":3617,"name":"seen_edges","tuple_arity":4,"sample":"seen_edges: set[tuple[str, str, str, str | None]] = set()"}
        ],
        "error": ""
    })
}

fn scan_producer_suppression_sites(path: &Path) -> Value {
    let path_text = path.to_string_lossy();
    let Ok(source) = fs::read_to_string(path) else {
        return json!({"path":path_text,"total_sites":0,"sites":[],"error":"file not found"});
    };
    let sites = source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();
            let name_len = trimmed
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .map(char::len_utf8)
                .sum::<usize>();
            let name = &trimmed[..name_len];
            let declaration = trimmed[name_len..].trim_start();
            if !name.starts_with("seen_")
                || name.len() == "seen_".len()
                || !matches!(declaration.chars().next(), Some(':' | '='))
            {
                return None;
            }
            let tuple_arity = tuple_arity_from_annotation(line);
            Some(json!({
                "line":index + 1,
                "name":name,
                "tuple_arity":tuple_arity,
                "sample":line.trim().chars().take(120).collect::<String>()
            }))
        })
        .collect::<Vec<_>>();
    json!({"path":path_text,"total_sites":sites.len(),"sites":sites,"error":""})
}

fn tuple_arity_from_annotation(line: &str) -> usize {
    let Some(after) = line.split_once("set[tuple[").map(|(_, after)| after) else {
        return 0;
    };
    let Some(inside) = after.split_once("]]").map(|(inside, _)| inside.trim()) else {
        return 0;
    };
    if inside.is_empty() {
        0
    } else {
        inside.matches(',').count() + 1
    }
}

#[derive(Clone)]
struct Edge {
    source: String,
    target: String,
    relation: String,
    source_file: String,
    location: String,
    context: String,
}
impl Edge {
    fn new(v: &Value) -> Option<Self> {
        let o = v.as_object()?;
        Some(Self {
            source: text(
                o.get("source")
                    .or_else(|| o.get("from"))
                    .unwrap_or(&Value::Null),
            ),
            target: text(
                o.get("target")
                    .or_else(|| o.get("to"))
                    .unwrap_or(&Value::Null),
            ),
            relation: text(o.get("relation").unwrap_or(&Value::Null)),
            source_file: text(o.get("source_file").unwrap_or(&Value::Null)),
            location: text(o.get("source_location").unwrap_or(&Value::Null)),
            context: text(o.get("context").unwrap_or(&Value::Null)),
        })
    }
}
fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(v) => {
            if *v {
                "True".into()
            } else {
                "False".into()
            }
        }
        Value::String(v) => v.clone(),
        Value::Number(v) => v.to_string(),
        v => serde_json::to_string(v).unwrap_or_default(),
    }
}
fn signature(v: &Value) -> String {
    let Some(o) = v.as_object() else {
        return "<non-object>".into();
    };
    let mut b = BTreeMap::new();
    for (k, v) in o {
        if k != "from" && k != "to" {
            b.insert(k.clone(), v.clone());
        }
    }
    if !b.contains_key("source")
        && let Some(v) = o.get("from")
    {
        b.insert("source".to_owned(), v.clone());
    }
    if !b.contains_key("target")
        && let Some(v) = o.get("to")
    {
        b.insert("target".to_owned(), v.clone());
    }
    serde_json::to_string(&b).unwrap_or_default()
}
fn extra<K: Eq + std::hash::Hash>(m: &HashMap<K, usize>) -> usize {
    m.values().map(|v| v.saturating_sub(1)).sum()
}
fn set(edges: &[Edge], f: impl Fn(&Edge) -> &String) -> Vec<String> {
    edges
        .iter()
        .map(f)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn variants<'a>(
    groups: &'a HashMap<(String, String), Vec<Edge>>,
    f: impl Fn(&'a Edge) -> &'a String,
    relation_sensitive: bool,
) -> usize {
    groups
        .values()
        .map(|edges| {
            if relation_sensitive {
                let mut r = HashMap::<&str, HashSet<&str>>::new();
                for e in edges {
                    r.entry(&e.relation).or_default().insert(f(e));
                }
                r.values().filter(|v| v.len() > 1).count()
            } else {
                usize::from(edges.iter().map(&f).collect::<HashSet<_>>().len() > 1)
            }
        })
        .sum()
}
fn list(v: &Value) -> String {
    format!(
        "[{}]",
        v.as_array()
            .into_iter()
            .flatten()
            .map(|v| format!("'{}'", text(v)))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
