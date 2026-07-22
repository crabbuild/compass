use std::collections::BTreeMap;

use compass_model::GraphDocument;
use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct CypherOperation {
    pub statement: String,
    pub params: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphOperations {
    pub nodes: Vec<CypherOperation>,
    pub edges: Vec<CypherOperation>,
}

#[must_use]
pub fn graph_operations(
    document: &GraphDocument,
    communities: Option<&BTreeMap<usize, Vec<String>>>,
) -> GraphOperations {
    let node_community = communities
        .into_iter()
        .flat_map(|communities| communities.iter())
        .flat_map(|(community, nodes)| nodes.iter().map(move |node| (node.as_str(), *community)))
        .collect::<BTreeMap<_, _>>();
    let nodes = document
        .nodes
        .iter()
        .map(|node| {
            let mut props = primitive_properties(&node.attributes);
            props.insert("id".to_owned(), Value::String(node.id.clone()));
            if let Some(community) = node_community.get(node.id.as_str()) {
                props.insert("community".to_owned(), Value::from(*community));
            }
            let file_type = node
                .attributes
                .get("file_type")
                .and_then(Value::as_str)
                .unwrap_or("Entity");
            let label = safe_label(&python_capitalize(file_type));
            CypherOperation {
                statement: format!("MERGE (n:{label} {{id: $id}}) SET n += $props"),
                params: BTreeMap::from([
                    ("id".to_owned(), Value::String(node.id.clone())),
                    ("props".to_owned(), Value::Object(props)),
                ]),
            }
        })
        .collect();
    let edges = document
        .links
        .iter()
        .map(|edge| {
            let relation = edge
                .attributes
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or("RELATED_TO");
            let relation = safe_relation(relation);
            CypherOperation {
                statement: format!(
                    "MATCH (a {{id: $src}}), (b {{id: $tgt}}) MERGE (a)-[r:{relation}]->(b) SET r += $props"
                ),
                params: BTreeMap::from([
                    ("src".to_owned(), Value::String(edge.source.clone())),
                    ("tgt".to_owned(), Value::String(edge.target.clone())),
                    (
                        "props".to_owned(),
                        Value::Object(primitive_properties(&edge.attributes)),
                    ),
                ]),
            }
        })
        .collect();
    GraphOperations { nodes, edges }
}

#[must_use]
pub fn falkordb_query(operation: &CypherOperation) -> String {
    let params = operation
        .params
        .iter()
        .map(|(key, value)| format!("{}={}", safe_parameter_name(key), cypher_value(value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!("CYPHER {params} {}", operation.statement)
}

fn primitive_properties(attributes: &Map<String, Value>) -> Map<String, Value> {
    attributes
        .iter()
        .filter(|(key, value)| {
            !key.starts_with('_')
                && matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_))
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn safe_relation(value: &str) -> String {
    let normalized = value.to_uppercase().replace([' ', '-'], "_");
    let sanitized = normalized
        .chars()
        .map(|character| {
            if character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "RELATED_TO".to_owned()
    } else {
        sanitized
    }
}

fn safe_label(value: &str) -> String {
    let sanitized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>();
    if sanitized.is_empty() {
        "Entity".to_owned()
    } else {
        sanitized
    }
}

fn python_capitalize(value: &str) -> String {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    first
        .to_uppercase()
        .chain(characters.flat_map(char::to_lowercase))
        .collect()
}

fn safe_parameter_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect()
}

fn cypher_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => format!("'{}'", cypher_string(value)),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(cypher_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "`{}`: {}",
                    key.replace('`', "``"),
                    cypher_value(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn cypher_string(value: &str) -> String {
    value
        .chars()
        .filter(|character| *character >= ' ' || *character == '\t')
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_filter_private_and_structured_properties() -> Result<(), serde_json::Error> {
        let document = serde_json::from_value(serde_json::json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [{
                "id": "n'1", "label": "Node", "file_type": "co-de", "count": 2,
                "enabled": true, "_private": "drop", "nested": {"drop": true}
            }],
            "links": []
        }))?;
        let operations = graph_operations(&document, None);
        assert_eq!(
            operations.nodes[0].statement,
            "MERGE (n:Code {id: $id}) SET n += $props"
        );
        let query = falkordb_query(&operations.nodes[0]);
        assert!(query.contains("id='n\\'1'"));
        assert!(!query.contains("_private"));
        assert!(!query.contains("nested"));
        Ok(())
    }
}
