use std::path::Path;

use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, ExploreRequest,
    ImpactRequest, NodeTrailRequest, QueryDiagnosticCode, SearchRequest,
};
use compass_model::{
    code_graph::{BuildMetadata, EdgeKind, NodeKind, NodeRole},
    provenance::EvidenceConfidence,
};
use compass_query::open;
use compass_query::{NaturalQueryRequest, QueryErrorKind};
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

pub(super) fn output_schema(operation: CodeQueryOperation) -> Value {
    let operation = serde_json::to_value(operation).unwrap_or(Value::Null);
    let node_kind = enum_schema(NodeKind::ALL.into_iter().map(NodeKind::as_str));
    let node_role = enum_schema(NodeRole::ALL.into_iter().map(NodeRole::as_str));
    let edge_kind = enum_schema(EdgeKind::ALL.into_iter().map(EdgeKind::as_str));
    let node_details = json!({"oneOf": [
        tagged_detail("file", "#/$defs/fileNodeDetails"),
        tagged_detail("symbol", "#/$defs/symbolNodeDetails"),
        tagged_detail("import_export", "#/$defs/importExportNodeDetails"),
        tagged_detail("route", "#/$defs/routeNodeDetails"),
        tagged_detail("component", "#/$defs/componentNodeDetails"),
        tagged_detail("resource", "#/$defs/resourceNodeDetails"),
        tagged_detail("messaging", "#/$defs/messagingNodeDetails"),
        tagged_detail("job", "#/$defs/jobNodeDetails"),
        tagged_detail("schema", "#/$defs/schemaNodeDetails"),
        tagged_detail("query", "#/$defs/queryNodeDetails"),
        tagged_detail("config", "#/$defs/configNodeDetails"),
        tagged_detail("database", "#/$defs/databaseNodeDetails")
    ]});
    let edge_details = json!({"oneOf": [
        tagged_detail("call", "#/$defs/callEdgeDetails"),
        tagged_detail("route", "#/$defs/routeEdgeDetails"),
        tagged_detail("messaging", "#/$defs/messagingEdgeDetails"),
        tagged_detail("schedule", "#/$defs/scheduleEdgeDetails"),
        tagged_detail("mapping", "#/$defs/mappingEdgeDetails")
    ]});
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "schema": {"const": "compass.code_context.v1"},
            "repository": {"type": "string", "minLength": 1},
            "generation": {"type": "string", "minLength": 1},
            "freshness": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": {"enum": ["current", "stale", "unknown"]}
                },
                "required": ["status"]
            },
            "data": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "schema": {"const": "compass.query/1"},
                    "operation": {"const": operation},
                    "results": {"type": "array", "items": {"$ref": "#/$defs/searchHit"}},
                    "nodes": {"type": "array", "items": {"$ref": "#/$defs/queryNode"}},
                    "edges": {"type": "array", "items": {"$ref": "#/$defs/queryEdge"}},
                    "files": {"type": "array", "items": {"$ref": "#/$defs/queryFile"}},
                    "paths": {"type": "array", "items": {"$ref": "#/$defs/queryPath"}},
                    "diagnostics": {"type": "array", "items": {"$ref": "#/$defs/queryDiagnostic"}},
                    "limits": {"$ref": "#/$defs/limits"},
                    "truncated": {"type": "boolean"}
                },
                "required": [
                    "schema", "operation", "results", "nodes", "edges", "files",
                    "paths", "diagnostics", "limits", "truncated"
                ]
            },
            "evidence": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "records": {"type": "integer", "minimum": 0},
                    "anchored": {"type": "integer", "minimum": 0}
                },
                "required": ["records", "anchored"]
            },
            "confidence": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "exact": {"type": "integer", "minimum": 0},
                    "inferred": {"type": "integer", "minimum": 0},
                    "ambiguous": {"type": "integer", "minimum": 0}
                },
                "required": ["exact", "inferred", "ambiguous"]
            },
            "truncation": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "truncated": {"type": "boolean"},
                    "next": {"type": ["string", "null"]}
                },
                "required": ["truncated", "next"]
            },
            "warnings": {"type": "array", "items": {"$ref": "#/$defs/queryDiagnostic"}}
        },
        "required": [
            "schema", "repository", "generation", "freshness", "data", "evidence",
            "confidence", "truncation", "warnings"
        ],
        "$defs": {
            "sourceAnchor": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "file": {"type": "string"},
                    "startByte": {"type": "integer", "minimum": 0},
                    "endByte": {"type": "integer", "minimum": 0},
                    "startLine": {"type": "integer", "minimum": 0},
                    "startColumn": {"type": "integer", "minimum": 0},
                    "endLine": {"type": "integer", "minimum": 0},
                    "endColumn": {"type": "integer", "minimum": 0}
                },
                "required": ["file", "startByte", "endByte", "startLine", "startColumn", "endLine", "endColumn"]
            },
            "resolutionCandidate": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "nodeId": {"type": "string"},
                    "reason": {"type": "string"},
                    "confidence": {"enum": ["exact", "inferred", "ambiguous"]},
                    "score": {"type": ["number", "null"]},
                    "anchor": {"anyOf": [{"$ref": "#/$defs/sourceAnchor"}, {"type": "null"}]}
                },
                "required": ["nodeId", "reason", "confidence"]
            },
            "queryEvidence": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "layer": {"enum": ["structural_graph", "program_ir"]},
                    "origin": {"enum": ["ast", "config", "convention", "artifact", "heuristic"]},
                    "extractor": {"type": "string"},
                    "confidence": {"enum": ["exact", "inferred", "ambiguous"]},
                    "anchor": {"anyOf": [{"$ref": "#/$defs/sourceAnchor"}, {"type": "null"}]},
                    "rule": {"type": ["string", "null"]},
                    "wiringSite": {"anyOf": [{"$ref": "#/$defs/sourceAnchor"}, {"type": "null"}]},
                    "resolution": {"enum": ["exact", "ambiguous", "unresolved"]},
                    "candidates": {"type": "array", "items": {"$ref": "#/$defs/resolutionCandidate"}}
                },
                "required": ["layer", "origin", "extractor", "confidence", "anchor", "rule", "wiringSite", "resolution"]
            },
            "searchHit": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "nodeId": {"type": "string"},
                    "score": {"type": "number"},
                    "matchedFields": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["nodeId", "score", "matchedFields"]
            },
            "queryNode": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": {"type": "string"},
                    "kind": node_kind,
                    "roles": {"type": "array", "items": node_role},
                    "name": {"type": "string"},
                    "qualifiedName": {"type": "string"},
                    "language": {"type": ["string", "null"]},
                    "framework": {"type": ["string", "null"]},
                    "source": {"anyOf": [{"$ref": "#/$defs/sourceAnchor"}, {"type": "null"}]},
                    "details": {"anyOf": [node_details, {"type": "null"}]},
                    "evidence": {"type": "array", "items": {"$ref": "#/$defs/queryEvidence"}}
                },
                "required": ["id", "kind", "roles", "name", "qualifiedName", "language", "framework", "source", "details", "evidence"]
            },
            "queryEdge": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": {"type": "string"},
                    "source": {"type": "string"},
                    "target": {"type": "string"},
                    "kind": edge_kind,
                    "relationshipSite": {"anyOf": [{"$ref": "#/$defs/sourceAnchor"}, {"type": "null"}]},
                    "details": {"anyOf": [edge_details, {"type": "null"}]},
                    "evidence": {"type": "array", "items": {"$ref": "#/$defs/queryEvidence"}}
                },
                "required": ["id", "source", "target", "kind", "relationshipSite", "details", "evidence"]
            },
            "queryFile": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": {"type": "string"},
                    "contentDigest": {"type": "string"},
                    "source": {"type": ["string", "null"]},
                    "truncated": {"type": "boolean"}
                },
                "required": ["path", "contentDigest", "source", "truncated"]
            },
            "queryPath": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": {"type": "string"},
                    "nodeIds": {"type": "array", "items": {"type": "string"}},
                    "edgeIds": {"type": "array", "items": {"type": "string"}},
                    "weakestResolution": {"enum": ["exact", "ambiguous", "unresolved"]},
                    "weakestConfidence": {"enum": ["exact", "inferred", "ambiguous"]}
                },
                "required": ["id", "nodeIds", "edgeIds", "weakestResolution", "weakestConfidence"]
            },
            "queryDiagnostic": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "code": {"enum": ["no_match", "ambiguous_match", "direction_mismatch", "unresolved_handler", "incomplete_coverage", "stale_source_digest", "bounded_truncation", "program_orphan", "program_conflict", "program_unavailable"]},
                    "message": {"type": "string"},
                    "nodeId": {"type": ["string", "null"]},
                    "path": {"type": ["string", "null"]}
                },
                "required": ["code", "message", "nodeId", "path"]
            },
            "limits": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "maxDepth": {"type": "integer", "minimum": 1},
                    "maxNodes": {"type": "integer", "minimum": 1},
                    "maxEdges": {"type": "integer", "minimum": 1},
                    "maxPaths": {"type": "integer", "minimum": 1},
                    "maxCandidates": {"type": "integer", "minimum": 1},
                    "maxSourceBytes": {"type": "integer", "minimum": 1},
                    "maxResponseBytes": {"type": "integer", "minimum": 1}
                },
                "required": ["maxDepth", "maxNodes", "maxEdges", "maxPaths", "maxCandidates", "maxSourceBytes", "maxResponseBytes"]
            },
            "fileNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "contentDigest": {"type": "string"},
                    "byteSize": {"type": "integer", "minimum": 0},
                    "generated": {"type": "boolean"}
                },
                "required": ["contentDigest", "byteSize", "generated"]
            },
            "symbolNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "signature": {"type": "string"},
                    "modifiers": {"type": "array", "items": {"type": "string"}},
                    "overloadDiscriminator": {"type": "string"},
                    "declaringType": {"type": "string"},
                    "signatureDigest": {"type": "string"},
                    "implementationDigest": {"type": "string"},
                    "sourceDigest": {"type": "string"}
                }
            },
            "importExportNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "specifier": {"type": "string"},
                    "importedName": {"type": "string"},
                    "localName": {"type": "string"},
                    "typeOnly": {"type": "boolean"}
                },
                "required": ["specifier", "typeOnly"]
            },
            "routeStageDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "stage": {"enum": ["middleware", "handler"]},
                    "position": {"type": "integer", "minimum": 0},
                    "reference": {"type": "string"},
                    "resolution": {"enum": ["exact", "ambiguous", "unresolved"]},
                    "target": {"type": "string"},
                    "candidates": {"type": "array", "items": {"$ref": "#/$defs/resolutionCandidate"}}
                },
                "required": ["stage", "position", "reference", "resolution"]
            },
            "routeNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "operation": {"type": "string"},
                    "path": {"type": "string"},
                    "originalPath": {"type": "string"},
                    "declaringScope": {"type": "string"},
                    "resolution": {"enum": ["exact", "ambiguous", "unresolved"]},
                    "middlewareCount": {"type": "integer", "minimum": 0},
                    "stages": {"type": "array", "items": {"$ref": "#/$defs/routeStageDetails"}}
                },
                "required": ["operation", "path", "declaringScope", "resolution", "middlewareCount"]
            },
            "componentNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {"componentType": {"type": "string"}},
                "required": ["componentType"]
            },
            "resourceNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "resourceKind": {"enum": ["document", "paper", "image", "concept", "rationale"]},
                    "uri": {"type": "string"},
                    "mediaType": {"type": "string"}
                },
                "required": ["resourceKind"]
            },
            "messagingNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "transport": {"type": "string"},
                    "subject": {"type": "string"},
                    "declaringScope": {"type": "string"}
                },
                "required": ["transport", "subject", "declaringScope"]
            },
            "jobNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {"schedule": {"type": "string"}, "queue": {"type": "string"}}
            },
            "schemaNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "dialect": {"type": "string"},
                    "logicalDatabase": {"type": "string"},
                    "namespace": {"type": "string"}
                }
            },
            "queryNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "dialect": {"type": "string"},
                    "operation": {"type": "string"},
                    "textDigest": {"type": "string"}
                }
            },
            "configNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {"format": {"type": "string"}, "keyPath": {"type": "string"}},
                "required": ["format", "keyPath"]
            },
            "databaseNodeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "logicalDatabase": {"type": "string"},
                    "databaseSchema": {"type": "string"}
                },
                "required": ["logicalDatabase"]
            },
            "callEdgeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "dispatch": {"enum": ["static", "virtual", "dynamic"]},
                    "receiverType": {"type": "string"},
                    "argumentCount": {"type": "integer", "minimum": 0}
                },
                "required": ["dispatch"]
            },
            "routeEdgeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "stage": {"enum": ["middleware", "handler"]},
                    "position": {"type": "integer", "minimum": 0},
                    "operation": {"type": "string"}
                },
                "required": ["stage"]
            },
            "messagingEdgeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {"transport": {"type": "string"}, "subject": {"type": "string"}},
                "required": ["transport", "subject"]
            },
            "scheduleEdgeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {"expression": {"type": "string"}}
            },
            "mappingEdgeDetails": {
                "type": "object", "additionalProperties": false,
                "properties": {"mappingKind": {"type": "string"}},
                "required": ["mappingKind"]
            }
        }
    })
}

fn tagged_detail(kind: &str, data_reference: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "type": {"const": kind},
            "data": {"$ref": data_reference}
        },
        "required": ["type", "data"]
    })
}

fn enum_schema<'a>(values: impl Iterator<Item = &'a str>) -> Value {
    json!({"enum": values.collect::<Vec<_>>()})
}

pub(super) fn envelope(
    response: &CodeQueryResponse,
    build: &BuildMetadata,
) -> Result<Value, serde_json::Error> {
    let evidence = response
        .nodes
        .iter()
        .flat_map(|node| node.evidence.iter())
        .chain(response.edges.iter().flat_map(|edge| edge.evidence.iter()));
    let mut records = 0_u64;
    let mut anchored = 0_u64;
    let mut exact = 0_u64;
    let mut inferred = 0_u64;
    let mut ambiguous = 0_u64;
    for item in evidence {
        records = records.saturating_add(1);
        if item.anchor.is_some() || item.wiring_site.is_some() {
            anchored = anchored.saturating_add(1);
        }
        match item.confidence {
            EvidenceConfidence::Exact => exact = exact.saturating_add(1),
            EvidenceConfidence::Inferred => inferred = inferred.saturating_add(1),
            EvidenceConfidence::Ambiguous => ambiguous = ambiguous.saturating_add(1),
        }
    }
    let freshness = if response
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::StaleSourceDigest)
    {
        "stale"
    } else if response.files.is_empty() {
        "unknown"
    } else {
        "current"
    };
    Ok(json!({
        "schema": "compass.code_context.v1",
        "repository": build.source_tree_digest,
        "generation": build.generation_id,
        "freshness": {"status": freshness},
        "data": response,
        "evidence": {"records": records, "anchored": anchored},
        "confidence": {
            "exact": exact,
            "inferred": inferred,
            "ambiguous": ambiguous
        },
        "truncation": {"truncated": response.truncated, "next": Value::Null},
        "warnings": response.diagnostics
    }))
}

pub(super) struct InvocationOutput {
    pub response: CodeQueryResponse,
    pub build: BuildMetadata,
}

pub(super) fn invoke(
    name: &str,
    arguments: &Map<String, Value>,
    graph_path: &Path,
) -> Result<InvocationOutput, super::InvocationError> {
    let cache = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cache");
    let engine = open(graph_path, None, &cache)
        .map_err(|error| super::InvocationError::Internal(error.to_string()))?;
    let build = engine
        .build_metadata()
        .map_err(|error| super::InvocationError::Internal(error.to_string()))?;
    let limits = limits(arguments)?;
    let response = match name {
        "query_graph" => engine.query_natural(NaturalQueryRequest {
            question: required_string(arguments, "question")?,
            include_heuristic: false,
            limits,
        }),
        "search_symbols" => engine.search(SearchRequest {
            query: required_string(arguments, "query")?,
            limits,
        }),
        "get_callers" => engine.callers(CallRequest {
            symbol: required_string(arguments, "symbol")?,
            include_heuristic: boolean(arguments, "include_heuristic"),
            limits,
        }),
        "get_callees" => engine.callees(CallRequest {
            symbol: required_string(arguments, "symbol")?,
            include_heuristic: boolean(arguments, "include_heuristic"),
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
            include_heuristic: boolean(arguments, "include_heuristic"),
            limits,
        }),
        "get_node" => engine.node_trail(NodeTrailRequest {
            source: required_string(arguments, "source")?,
            target: required_string(arguments, "target")?,
            include_heuristic: boolean(arguments, "include_heuristic"),
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
    })?;
    Ok(InvocationOutput { response, build })
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
