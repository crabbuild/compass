use std::path::Path;

use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryResponse, ExploreRequest, ImpactRequest,
    NodeTrailRequest, SearchRequest,
};
use compass_query::open;
use serde_json::{Map, Value, json};

pub(super) fn schema(required: &[&str]) -> Value {
    let defaults = CodeQueryLimits::default();
    let mut properties = Map::from_iter([
        ("query".into(), json!({"type":"string"})),
        ("symbol".into(), json!({"type":"string"})),
        (
            "symbols".into(),
            json!({"type":"array","items":{"type":"string"},"maxItems":defaults.max_candidates}),
        ),
        ("source".into(), json!({"type":"string"})),
        ("target".into(), json!({"type":"string"})),
        ("root".into(), json!({"type":"string"})),
        (
            "include_heuristic".into(),
            json!({"type":"boolean","default":false}),
        ),
    ]);
    for (name, default) in [
        ("max_depth", u64::from(defaults.max_depth)),
        ("max_nodes", u64::from(defaults.max_nodes)),
        ("max_edges", u64::from(defaults.max_edges)),
        ("max_paths", u64::from(defaults.max_paths)),
        ("max_candidates", u64::from(defaults.max_candidates)),
        ("max_source_bytes", defaults.max_source_bytes),
        ("max_response_bytes", defaults.max_response_bytes),
    ] {
        properties.insert(
            name.to_owned(),
            json!({"type":"integer","minimum":1,"default":default}),
        );
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}

pub(super) fn invoke(
    name: &str,
    arguments: &Map<String, Value>,
    graph_path: &Path,
) -> Result<CodeQueryResponse, String> {
    let cache = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cache");
    let engine = open(graph_path, None, &cache).map_err(|error| error.to_string())?;
    let limits = limits(arguments)?;
    match name {
        "search_symbols" => engine.search(SearchRequest {
            query: required_string(arguments, "query")?,
            limits,
        }),
        "get_callers" => engine.callers(CallRequest {
            symbol: required_string(arguments, "symbol")?,
            limits,
        }),
        "get_callees" => engine.callees(CallRequest {
            symbol: required_string(arguments, "symbol")?,
            limits,
        }),
        "get_impact" => engine.impact(ImpactRequest {
            symbol: required_string(arguments, "symbol")?,
            include_heuristic: boolean(arguments, "include_heuristic"),
            limits,
        }),
        "explore_code" => engine.explore(ExploreRequest {
            symbols: arguments
                .get("symbols")
                .and_then(Value::as_array)
                .ok_or_else(|| "'symbols' must be an array".to_owned())?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "'symbols' items must be strings".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?,
            root: arguments
                .get("root")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            limits,
        }),
        "get_node" => engine.node_trail(NodeTrailRequest {
            source: required_string(arguments, "source")?,
            target: required_string(arguments, "target")?,
            include_heuristic: boolean(arguments, "include_heuristic"),
            limits,
        }),
        _ => return Err(format!("unknown code query tool {name}")),
    }
    .map_err(|error| error.to_string())
}

fn limits(arguments: &Map<String, Value>) -> Result<CodeQueryLimits, String> {
    let defaults = CodeQueryLimits::default();
    Ok(CodeQueryLimits {
        max_depth: u32_value(arguments, "max_depth", defaults.max_depth)?,
        max_nodes: u32_value(arguments, "max_nodes", defaults.max_nodes)?,
        max_edges: u32_value(arguments, "max_edges", defaults.max_edges)?,
        max_paths: u32_value(arguments, "max_paths", defaults.max_paths)?,
        max_candidates: u32_value(arguments, "max_candidates", defaults.max_candidates)?,
        max_source_bytes: u64_value(arguments, "max_source_bytes", defaults.max_source_bytes)?,
        max_response_bytes: u64_value(
            arguments,
            "max_response_bytes",
            defaults.max_response_bytes,
        )?,
    })
}

fn required_string(arguments: &Map<String, Value>, name: &str) -> Result<String, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("'{name}' must be a non-empty string"))
}

fn boolean(arguments: &Map<String, Value>, name: &str) -> bool {
    arguments
        .get(name)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn u32_value(arguments: &Map<String, Value>, name: &str, default: u32) -> Result<u32, String> {
    let value = arguments
        .get(name)
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(default));
    u32::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("'{name}' must be a positive 32-bit integer"))
}

fn u64_value(arguments: &Map<String, Value>, name: &str, default: u64) -> Result<u64, String> {
    let value = arguments
        .get(name)
        .and_then(Value::as_u64)
        .unwrap_or(default);
    (value > 0)
        .then_some(value)
        .ok_or_else(|| format!("'{name}' must be a positive integer"))
}
