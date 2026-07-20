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
    let bytes = fs::read(path).map_err(|source| trail_files::FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let input: Value =
        serde_json::from_slice(&bytes).map_err(|source| trail_files::FileError::Json {
            path: path.to_path_buf(),
            source,
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
        || json!({"path":"","total_sites":0,"sites":[],"error":""}),
        |path| json!({"path":path.to_string_lossy(),"total_sites":0,"sites":[],"error":if path.exists(){""}else{"file not found"}}),
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
    lines.push("producer_suppression_sites: 0".to_owned());
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
