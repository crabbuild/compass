use std::collections::{HashMap, HashSet};

use compass_model::{EdgeRecord, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

/// Deterministic extraction facts produced from Graphify's simplified SCIP JSON shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScipExtraction {
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
}

#[derive(Clone)]
struct SymbolRecord {
    node_id: String,
    symbol_id: String,
    doc_path: String,
    raw: Map<String, Value>,
}

/// Convert the simplified, JSON-oriented SCIP shape accepted by Graphify into graph facts.
///
/// This intentionally does not parse the official SCIP protobuf. Invalid external input is
/// ignored defensively, and every relationship target is either resolved or represented by an
/// external stub so the returned graph never contains dangling edges.
#[must_use]
pub fn ingest_scip_json(doc: &Value, source_file: &str, language: &str) -> ScipExtraction {
    let Some(document_root) = doc.as_object() else {
        return ScipExtraction::default();
    };
    let Some(documents) = document_root.get("documents").and_then(Value::as_array) else {
        return ScipExtraction::default();
    };

    let mut per_doc_index = HashMap::<(String, String), String>::new();
    let mut global_index = HashMap::<String, Vec<String>>::new();
    let mut records = Vec::new();

    for document in documents {
        let Some(document) = document.as_object() else {
            continue;
        };
        let doc_path = string_or(document.get("relative_path"), source_file);
        let _doc_language = string_or(document.get("language"), language);
        let Some(symbols) = document.get("symbols").and_then(Value::as_array) else {
            continue;
        };
        for symbol in symbols {
            let Some(raw) = symbol.as_object() else {
                continue;
            };
            let symbol_id = string_or(raw.get("symbol"), "");
            if symbol_id.is_empty() {
                continue;
            }
            let node_id = make_scip_node_id(&symbol_id, &doc_path);
            per_doc_index
                .entry((symbol_id.clone(), doc_path.clone()))
                .or_insert_with(|| node_id.clone());
            let candidates = global_index.entry(symbol_id.clone()).or_default();
            if !candidates.contains(&node_id) {
                candidates.push(node_id.clone());
            }
            records.push(SymbolRecord {
                node_id,
                symbol_id,
                doc_path: doc_path.clone(),
                raw: raw.clone(),
            });
        }
    }

    let mut extraction = ScipExtraction::default();
    let mut seen_node_ids = HashSet::new();
    let mut seen_edges = HashSet::new();
    for record in &records {
        emit_symbol_node(record, &mut extraction.nodes, &mut seen_node_ids);
        emit_relationships(
            record,
            &per_doc_index,
            &global_index,
            &mut extraction,
            &mut seen_node_ids,
            &mut seen_edges,
        );
    }
    extraction
}

fn emit_symbol_node(
    record: &SymbolRecord,
    nodes: &mut Vec<NodeRecord>,
    seen_node_ids: &mut HashSet<String>,
) {
    if !seen_node_ids.insert(record.node_id.clone()) {
        return;
    }
    let kind = string_or(record.raw.get("kind"), "unknown");
    let display_name = string_or(record.raw.get("display_name"), "");
    let description = record
        .raw
        .get("documentation")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .unwrap_or_default();
    let line = first_occurrence_line(record.raw.get("occurrences"));
    let suffix = record
        .symbol_id
        .rsplit_once('#')
        .map_or(record.symbol_id.as_str(), |(_, suffix)| suffix);
    let label = if display_name.is_empty() {
        if suffix.is_empty() {
            record.symbol_id.clone()
        } else {
            suffix.to_owned()
        }
    } else {
        display_name
    };
    let mut attributes = Map::new();
    attributes.insert("label".to_owned(), Value::String(label));
    attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
    attributes.insert(
        "source_file".to_owned(),
        Value::String(record.doc_path.clone()),
    );
    attributes.insert(
        "source_location".to_owned(),
        Value::String(source_location(line)),
    );
    attributes.insert(
        "metadata".to_owned(),
        Value::Object(symbol_metadata(&record.symbol_id, &kind, description)),
    );
    nodes.push(NodeRecord {
        id: record.node_id.clone(),
        attributes,
    });
}

fn emit_relationships(
    record: &SymbolRecord,
    per_doc_index: &HashMap<(String, String), String>,
    global_index: &HashMap<String, Vec<String>>,
    extraction: &mut ScipExtraction,
    seen_node_ids: &mut HashSet<String>,
    seen_edges: &mut HashSet<(String, String, String, String)>,
) {
    let Some(relationships) = record.raw.get("relationships").and_then(Value::as_array) else {
        return;
    };
    let location = source_location(first_occurrence_line(record.raw.get("occurrences")));
    for relationship in relationships {
        let Some(relationship) = relationship.as_object() else {
            continue;
        };
        let target_symbol = string_or(relationship.get("symbol"), "");
        if target_symbol.is_empty() {
            continue;
        }
        let target_node_id = resolve_relationship_target(
            &target_symbol,
            &record.doc_path,
            per_doc_index,
            global_index,
        )
        .unwrap_or_else(|| make_scip_node_id(&target_symbol, &record.doc_path));

        if !per_doc_index.values().any(|node| node == &target_node_id)
            && seen_node_ids.insert(target_node_id.clone())
        {
            let suffix = target_symbol
                .rsplit_once('#')
                .map_or(target_symbol.as_str(), |(_, suffix)| suffix);
            let label = if suffix.is_empty() {
                target_symbol.clone()
            } else {
                suffix.to_owned()
            };
            let mut attributes = Map::new();
            attributes.insert("label".to_owned(), Value::String(label));
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            attributes.insert(
                "source_file".to_owned(),
                Value::String(record.doc_path.clone()),
            );
            attributes.insert("source_location".to_owned(), Value::String(String::new()));
            attributes.insert(
                "metadata".to_owned(),
                Value::Object(symbol_metadata(&target_symbol, "external", "")),
            );
            extraction.nodes.push(NodeRecord {
                id: target_node_id.clone(),
                attributes,
            });
        }

        let relation = scip_relation_for(relationship).to_owned();
        let key = (
            record.node_id.clone(),
            target_node_id.clone(),
            relation.clone(),
            location.clone(),
        );
        if !seen_edges.insert(key) {
            continue;
        }
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation));
        attributes.insert(
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        );
        attributes.insert("confidence_score".to_owned(), Value::from(1.0));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(record.doc_path.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(location.clone()),
        );
        attributes.insert("weight".to_owned(), Value::from(1.0));
        attributes.insert("context".to_owned(), Value::String("scip".to_owned()));
        let mut metadata = Map::new();
        metadata.insert(
            "scip_relationship".to_owned(),
            Value::Object(relationship.clone()),
        );
        attributes.insert("metadata".to_owned(), sanitize_metadata_object(&metadata));
        extraction.edges.push(EdgeRecord {
            source: record.node_id.clone(),
            target: target_node_id,
            attributes,
        });
    }
}

fn resolve_relationship_target(
    target_symbol: &str,
    source_doc_path: &str,
    per_doc_index: &HashMap<(String, String), String>,
    global_index: &HashMap<String, Vec<String>>,
) -> Option<String> {
    if let Some(node) = per_doc_index.get(&(target_symbol.to_owned(), source_doc_path.to_owned())) {
        return Some(node.clone());
    }
    let candidates = global_index.get(target_symbol)?;
    (candidates.len() == 1).then(|| candidates[0].clone())
}

fn scip_relation_for(relationship: &Map<String, Value>) -> &'static str {
    if relationship.get("is_implementation") == Some(&Value::Bool(true)) {
        "scip_impl"
    } else if relationship.get("is_type_definition") == Some(&Value::Bool(true)) {
        "scip_typed"
    } else if relationship.get("is_definition") == Some(&Value::Bool(true)) {
        "scip_def"
    } else {
        "scip_ref"
    }
}

fn first_occurrence_line(occurrences: Option<&Value>) -> Option<u64> {
    occurrences?
        .as_array()?
        .first()?
        .as_object()?
        .get("range")?
        .as_array()?
        .first()?
        .as_u64()
        .filter(|line| *line > 0)
}

fn source_location(line: Option<u64>) -> String {
    line.map_or_else(String::new, |line| format!("L{line}"))
}

fn string_or(value: Option<&Value>, default: &str) -> String {
    value.and_then(Value::as_str).unwrap_or(default).to_owned()
}

fn make_scip_node_id(symbol: &str, source_file: &str) -> String {
    let digest = Sha1::digest(format!("{source_file}:{symbol}").as_bytes());
    let hash = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let suffix = symbol.rsplit_once('#').map_or(symbol, |(_, suffix)| suffix);
    let suffix = suffix
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let suffix = suffix.trim_matches('_');
    if suffix.is_empty() {
        format!("scip_{hash}")
    } else {
        format!("scip_{suffix}_{hash}")
    }
}

fn symbol_metadata(symbol: &str, kind: &str, description: &str) -> Map<String, Value> {
    let mut metadata = Map::new();
    metadata.insert("scip_symbol".to_owned(), Value::String(symbol.to_owned()));
    metadata.insert("scip_kind".to_owned(), Value::String(kind.to_owned()));
    if !description.is_empty() {
        metadata.insert(
            "scip_description".to_owned(),
            Value::String(description.to_owned()),
        );
    }
    sanitize_metadata_map(&metadata)
}

fn sanitize_metadata_object(metadata: &Map<String, Value>) -> Value {
    Value::Object(sanitize_metadata_map(metadata))
}

fn sanitize_metadata_map(metadata: &Map<String, Value>) -> Map<String, Value> {
    metadata
        .iter()
        .filter_map(|(key, value)| {
            let key = sanitize_metadata_string(key);
            (!key.is_empty()).then(|| (key, sanitize_metadata_value(value)))
        })
        .collect()
}

fn sanitize_metadata_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(sanitize_metadata_string(text)),
        Value::Object(map) => Value::Object(sanitize_metadata_map(map)),
        Value::Array(items) => {
            Value::Array(items.iter().take(50).map(sanitize_metadata_value).collect())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
    }
}

fn sanitize_metadata_string(value: &str) -> String {
    let clean = value
        .chars()
        .filter(|character| !matches!(*character as u32, 0..=31 | 127))
        .collect::<String>();
    let escaped = clean
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;");
    escaped.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rejects_non_objects_and_non_list_documents() {
        assert_eq!(
            ingest_scip_json(&Value::Null, "", "python"),
            ScipExtraction::default()
        );
        assert_eq!(
            ingest_scip_json(&json!({"documents":"bad"}), "", "python"),
            ScipExtraction::default()
        );
    }

    #[test]
    fn relationship_flags_require_literal_true() {
        let document = json!({"documents":[{"relative_path":"a.py","symbols":[{
            "symbol":"pkg#A","relationships":[
                {"symbol":"pkg#B","is_implementation":"true"},
                {"symbol":"pkg#C","is_type_definition":true}
            ]
        }]}]});
        let result = ingest_scip_json(&document, "", "python");
        assert_eq!(result.edges[0].attributes["relation"], "scip_ref");
        assert_eq!(result.edges[1].attributes["relation"], "scip_typed");
    }

    #[test]
    fn metadata_is_recursive_bounded_and_html_safe() {
        let document = json!({"documents":[{"relative_path":"a.py","symbols":[{
            "symbol":"pkg#A","documentation":["<script>\u{0}bad</script>"],
            "relationships":[{"symbol":"pkg#B","payload":["<b>", true]}]
        }]}]});
        let result = ingest_scip_json(&document, "", "python");
        assert_eq!(
            result.nodes[0].attributes["metadata"]["scip_description"],
            "&lt;script&gt;bad&lt;/script&gt;"
        );
        assert_eq!(
            result.edges[0].attributes["metadata"]["scip_relationship"]["payload"][0],
            "&lt;b&gt;"
        );
    }
}
