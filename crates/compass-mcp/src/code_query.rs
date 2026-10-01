use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryResponse, DiscoveryDirection, DiscoveryLimits,
    DiscoveryQueryRequest, DiscoveryQueryResponse, DiscoveryScope, DiscoveryScopeKind,
    DiscoveryTraversal, ExploreRequest, ImpactRequest, NodeTrailRequest, SearchRequest,
};
use compass_query::{CodeQueryEngine, ExactSearchFilter, NaturalQueryRequest, QueryErrorKind};
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

pub(super) fn search_schema() -> Value {
    let mut result = schema(&["query"]);
    result["properties"]["semantic_search"] = json!({"type":"boolean","default":false,"description":"Optional offline local-LSA fallback; cannot be combined with exact."});
    result["properties"]["exact"] = json!({"type":"boolean","default":false,"description":"Only exact ID or normalized name matches; no lexical fallback."});
    result["properties"]["source_file"] = json!({"type":"string","minLength":1,"maxLength":4096,"description":"Exact stored source path; requires exact=true."});
    result["properties"]["start_line"] = json!({"type":"integer","minimum":1,"maximum":u32::MAX,"description":"Declaration start line; requires exact=true and source_file."});
    result["properties"]["kind"] = json!({"type":"string","description":"Stored node kind (for example function, class, struct); requires exact=true."});
    result
}

pub(super) fn impact_schema() -> Value {
    let mut result = schema(&["symbol"]);
    result["properties"]["relations"] = json!({"type":"array","minItems":1,"maxItems":32,"items":{"type":"string"},"description":"Stored relationship types to follow in reverse, such as calls, instantiates, references, extends, type_of, returns, or imports. Omit for the full dependency default."});
    result
}

pub(super) fn trail_schema() -> Value {
    let mut result = schema(&["source", "target"]);
    result["properties"]["calls_only"] = json!({"type":"boolean","default":false,"description":"Only directed calls edges; excludes structural shortcuts and applies to direction diagnostics too."});
    result
}

pub(super) fn invoke_with_engine(
    name: &str,
    arguments: &Map<String, Value>,
    engine: &CodeQueryEngine,
) -> Result<CodeQueryResponse, super::InvocationError> {
    if name != "get_node" && arguments.contains_key("calls_only") {
        return Err(super::InvocationError::InvalidParams(
            "calls_only requires get_node".to_owned(),
        ));
    }
    if name != "search_symbols"
        && ["exact", "source_file", "start_line", "kind"]
            .iter()
            .any(|key| arguments.contains_key(*key))
    {
        return Err(super::InvocationError::InvalidParams(
            "exact symbol filters require search_symbols".to_owned(),
        ));
    }
    if name != "get_impact" && arguments.contains_key("relations") {
        return Err(super::InvocationError::InvalidParams(
            "relations requires get_impact".to_owned(),
        ));
    }
    if arguments.contains_key("semantic_search")
        && !matches!(name, "query_graph" | "search_symbols")
    {
        return Err(super::InvocationError::InvalidParams(
            "semantic_search requires query_graph or search_symbols".to_owned(),
        ));
    }
    let limits = limits(arguments)?;
    match name {
        "query_graph" => engine.query_natural(NaturalQueryRequest {
            question: required_string(arguments, "question")?,
            include_heuristic: false,
            limits,
        }),
        "search_symbols" => {
            let request = SearchRequest {
                query: required_string(arguments, "query")?,
                limits,
            };
            let exact = boolean(arguments, "exact")?;
            if exact && boolean(arguments, "semantic_search")? {
                return Err(super::InvocationError::InvalidParams(
                    "semantic_search cannot be combined with exact".to_owned(),
                ));
            }
            if !exact
                && ["source_file", "start_line", "kind"]
                    .iter()
                    .any(|key| arguments.contains_key(*key))
            {
                return Err(super::InvocationError::InvalidParams(
                    "source_file, start_line and kind require exact=true".to_owned(),
                ));
            }
            if exact {
                let source_file = arguments
                    .get("source_file")
                    .map(|_| required_string(arguments, "source_file"))
                    .transpose()?;
                let start_line = arguments
                    .get("start_line")
                    .map(|_| u32_value(arguments, "start_line", 0))
                    .transpose()?;
                let kind = arguments
                    .get("kind")
                    .map(|value| {
                        serde_json::from_value(value.clone()).map_err(|_| {
                            "kind must be a stored node kind such as function, class, or struct"
                                .to_owned()
                        })
                    })
                    .transpose()?;
                engine.search_exact(
                    request,
                    ExactSearchFilter {
                        source_file,
                        start_line,
                        kind,
                    },
                )
            } else {
                engine.search(request)
            }
        }
        "get_callers" => engine.callers(CallRequest {
            symbol: required_string(arguments, "symbol")?,
            include_heuristic: boolean(arguments, "include_heuristic")?,
            limits,
        }),
        "get_callees" => engine.callees(CallRequest {
            symbol: required_string(arguments, "symbol")?,
            include_heuristic: boolean(arguments, "include_heuristic")?,
            limits,
        }),
        "get_impact" => {
            let request = ImpactRequest {
                symbol: required_string(arguments, "symbol")?,
                include_heuristic: boolean(arguments, "include_heuristic")?,
                limits,
            };
            if let Some(value) = arguments.get("relations") {
                let values = value
                    .as_array()
                    .filter(|values| !values.is_empty() && values.len() <= 32)
                    .ok_or_else(|| {
                        "relations must contain 1 to 32 stored relationship types".to_owned()
                    })?;
                let relations = values
                    .iter()
                    .map(|value| {
                        serde_json::from_value(value.clone()).map_err(|_| {
                            "relations contains an unknown stored relationship type".to_owned()
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                engine.impact_with_relations(request, &relations)
            } else {
                engine.impact(request)
            }
        }
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
            include_heuristic: boolean(arguments, "include_heuristic")?,
            limits,
        }),
        "get_node" => engine.node_trail(NodeTrailRequest {
            calls_only: boolean(arguments, "calls_only")?,
            source: required_string(arguments, "source")?,
            target: required_string(arguments, "target")?,
            include_heuristic: boolean(arguments, "include_heuristic")?,
            limits,
        }),
        _ => {
            return Err(super::InvocationError::InvalidParams(format!(
                "unknown code query tool {name}"
            )));
        }
    }
    .map_err(|error| match error.kind() {
        QueryErrorKind::InvalidParameter | QueryErrorKind::Type | QueryErrorKind::UnsafePath => {
            super::InvocationError::InvalidParams(error.to_string())
        }
        _ => super::InvocationError::Internal(error.to_string()),
    })
}

pub(super) fn has_discovery_arguments(arguments: &Map<String, Value>) -> bool {
    [
        "direction",
        "relation_contexts",
        "scope",
        "traversal",
        "include_heuristic",
        "max_seeds",
        "max_expanded_relationships",
        "timeout_ms",
        "max_depth",
        "max_candidates",
        "max_nodes",
        "max_edges",
        "max_response_bytes",
    ]
    .iter()
    .any(|name| arguments.contains_key(*name))
}

pub(super) fn validate_query_graph_arguments(
    arguments: &Map<String, Value>,
) -> Result<(), super::InvocationError> {
    const ALLOWED: &[&str] = &[
        "question",
        "semantic_search",
        "project_path",
        "mode",
        "depth",
        "token_budget",
        "context_filter",
        "direction",
        "relation_contexts",
        "scope",
        "traversal",
        "include_heuristic",
        "max_depth",
        "max_seeds",
        "max_candidates",
        "max_nodes",
        "max_edges",
        "max_expanded_relationships",
        "max_response_bytes",
        "timeout_ms",
    ];
    if let Some(unknown) = arguments
        .keys()
        .find(|name| !ALLOWED.contains(&name.as_str()))
    {
        return Err(super::InvocationError::InvalidParams(format!(
            "unknown query_graph argument {unknown:?}"
        )));
    }
    let legacy = ["mode", "depth", "token_budget", "context_filter"]
        .iter()
        .any(|name| arguments.contains_key(*name));
    if legacy && arguments.contains_key("semantic_search") {
        return Err(super::InvocationError::InvalidParams(
            "semantic_search requires bounded discovery; omit legacy traversal controls".to_owned(),
        ));
    }
    if legacy && has_discovery_arguments(arguments) {
        return Err(super::InvocationError::InvalidParams(
            "legacy traversal controls cannot be combined with discovery controls".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn invoke_discovery_with_engine(
    arguments: &Map<String, Value>,
    engine: &CodeQueryEngine,
) -> Result<DiscoveryQueryResponse, super::InvocationError> {
    let defaults = DiscoveryLimits::default();
    let request = DiscoveryQueryRequest {
        question: required_string(arguments, "question")?,
        direction: enum_value(arguments, "direction", "auto", |value| match value {
            "auto" => Some(DiscoveryDirection::Auto),
            "incoming" => Some(DiscoveryDirection::Incoming),
            "outgoing" => Some(DiscoveryDirection::Outgoing),
            "both" => Some(DiscoveryDirection::Both),
            _ => None,
        })?,
        relation_contexts: string_array(arguments, "relation_contexts")?,
        scope: discovery_scopes(arguments)?,
        traversal: enum_value(arguments, "traversal", "bfs", |value| match value {
            "bfs" => Some(DiscoveryTraversal::Bfs),
            "dfs" => Some(DiscoveryTraversal::Dfs),
            _ => None,
        })?,
        include_heuristic: boolean(arguments, "include_heuristic")?,
        limits: DiscoveryLimits {
            max_depth: u32_value(arguments, "max_depth", defaults.max_depth)?,
            max_seeds: u32_value(arguments, "max_seeds", defaults.max_seeds)?,
            max_candidates: u32_value(arguments, "max_candidates", defaults.max_candidates)?,
            max_nodes: u32_value(arguments, "max_nodes", defaults.max_nodes)?,
            max_edges: u32_value(arguments, "max_edges", defaults.max_edges)?,
            max_expanded_relationships: u64_value(
                arguments,
                "max_expanded_relationships",
                defaults.max_expanded_relationships,
            )?,
            max_response_bytes: u64_value(
                arguments,
                "max_response_bytes",
                defaults.max_response_bytes,
            )?,
            timeout_ms: u64_value(arguments, "timeout_ms", defaults.timeout_ms)?,
        },
    };
    engine.discover(request).map_err(query_invocation_error)
}

fn query_invocation_error(error: compass_query::QueryError) -> super::InvocationError {
    match error.kind() {
        QueryErrorKind::InvalidParameter | QueryErrorKind::Type | QueryErrorKind::UnsafePath => {
            super::InvocationError::InvalidParams(error.to_string())
        }
        _ => super::InvocationError::Internal(error.to_string()),
    }
}

fn discovery_scopes(arguments: &Map<String, Value>) -> Result<Vec<DiscoveryScope>, String> {
    let Some(values) = arguments.get("scope") else {
        return Ok(Vec::new());
    };
    values
        .as_array()
        .ok_or_else(|| "'scope' must be an array".to_owned())?
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or_else(|| "'scope' items must be objects".to_owned())?;
            if object.len() != 2 || !object.contains_key("kind") || !object.contains_key("value") {
                return Err("'scope' items must contain exactly 'kind' and 'value'".to_owned());
            }
            let kind = enum_value(object, "kind", "", |value| match value {
                "community" => Some(DiscoveryScopeKind::Community),
                "source" => Some(DiscoveryScopeKind::Source),
                "package" => Some(DiscoveryScopeKind::Package),
                "node" => Some(DiscoveryScopeKind::Node),
                _ => None,
            })?;
            Ok(DiscoveryScope {
                kind,
                value: required_string(object, "value")?,
            })
        })
        .collect()
}

fn string_array(arguments: &Map<String, Value>, name: &str) -> Result<Vec<String>, String> {
    let Some(values) = arguments.get(name) else {
        return Ok(Vec::new());
    };
    values
        .as_array()
        .ok_or_else(|| format!("'{name}' must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| format!("'{name}' items must be non-empty strings"))
        })
        .collect()
}

fn enum_value<T>(
    arguments: &Map<String, Value>,
    name: &str,
    default: &str,
    parse: impl FnOnce(&str) -> Option<T>,
) -> Result<T, String> {
    let value = arguments
        .get(name)
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("'{name}' must be a string"))
        })
        .transpose()?
        .unwrap_or(default);
    parse(value).ok_or_else(|| format!("unsupported '{name}' value {value:?}"))
}

pub(super) fn limits(arguments: &Map<String, Value>) -> Result<CodeQueryLimits, String> {
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

fn boolean(arguments: &Map<String, Value>, name: &str) -> Result<bool, String> {
    arguments.get(name).map_or(Ok(false), |value| {
        value
            .as_bool()
            .ok_or_else(|| format!("'{name}' must be a boolean"))
    })
}

fn u32_value(arguments: &Map<String, Value>, name: &str, default: u32) -> Result<u32, String> {
    let value = arguments
        .get(name)
        .map_or(Ok(u64::from(default)), |value| {
            value
                .as_u64()
                .ok_or_else(|| format!("'{name}' must be a positive 32-bit integer"))
        })?;
    u32::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("'{name}' must be a positive 32-bit integer"))
}

fn u64_value(arguments: &Map<String, Value>, name: &str, default: u64) -> Result<u64, String> {
    let value = arguments.get(name).map_or(Ok(default), |value| {
        value
            .as_u64()
            .ok_or_else(|| format!("'{name}' must be a positive integer"))
    })?;
    (value > 0)
        .then_some(value)
        .ok_or_else(|| format!("'{name}' must be a positive integer"))
}
