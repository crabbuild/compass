use std::collections::HashSet;
use std::fmt::Write as _;

use serde_json::Value;

const VALID_FILE_TYPES: [&str; 6] = ["code", "concept", "document", "image", "paper", "rationale"];
const VALID_CONFIDENCES: [&str; 3] = ["AMBIGUOUS", "EXTRACTED", "INFERRED"];

// CPython's set iteration order with the compatibility harness' PYTHONHASHSEED=0.
const REQUIRED_NODE_FIELDS: [&str; 4] = ["file_type", "id", "source_file", "label"];
const REQUIRED_EDGE_FIELDS: [&str; 5] =
    ["source_file", "target", "confidence", "source", "relation"];

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum HashableJson {
    None,
    String(String),
    Integer(i128),
    Float(u64),
}

/// Validate raw extraction JSON using Graphify's external schema and diagnostics.
#[must_use]
pub fn validate_extraction(data: &Value) -> Vec<String> {
    let Some(data) = data.as_object() else {
        return vec!["Extraction must be a JSON object".to_owned()];
    };
    let mut errors = Vec::new();
    let mut node_ids = HashSet::new();

    match data.get("nodes") {
        None => errors.push("Missing required key 'nodes'".to_owned()),
        Some(Value::Array(nodes)) => {
            for (index, node) in nodes.iter().enumerate() {
                let Some(node) = node.as_object() else {
                    errors.push(format!("Node {index} must be an object"));
                    continue;
                };
                for field in REQUIRED_NODE_FIELDS {
                    if !node.contains_key(field) {
                        let id = node.get("id").map_or_else(|| "'?'".to_owned(), python_repr);
                        errors.push(format!(
                            "Node {index} (id={id}) missing required field '{field}'"
                        ));
                    }
                }
                if let Some(id) = node.get("id") {
                    if let Some(key) = hashable_json(id) {
                        node_ids.insert(key);
                    } else {
                        errors.push(format!(
                            "Node {index} has non-hashable id {} - id must be a string",
                            python_repr(id)
                        ));
                    }
                }
                if let Some(file_type) = node.get("file_type") {
                    let valid = file_type
                        .as_str()
                        .is_some_and(|value| VALID_FILE_TYPES.contains(&value));
                    if !valid {
                        errors.push(format!(
                            "Node {index} (id={}) has invalid file_type '{}' - must be one of {}",
                            node.get("id").map_or_else(|| "'?'".to_owned(), python_repr),
                            python_string(file_type),
                            python_string_list(&VALID_FILE_TYPES)
                        ));
                    }
                }
            }
        }
        Some(_) => errors.push("'nodes' must be a list".to_owned()),
    }

    let edge_list = if data.contains_key("edges") {
        data.get("edges")
    } else {
        data.get("links")
    };
    match edge_list {
        None | Some(Value::Null) => errors.push("Missing required key 'edges'".to_owned()),
        Some(Value::Array(edges)) => {
            for (index, edge) in edges.iter().enumerate() {
                let Some(edge) = edge.as_object() else {
                    errors.push(format!("Edge {index} must be an object"));
                    continue;
                };
                for field in REQUIRED_EDGE_FIELDS {
                    if !edge.contains_key(field) {
                        errors.push(format!("Edge {index} missing required field '{field}'"));
                    }
                }
                if let Some(confidence) = edge.get("confidence") {
                    let valid = confidence
                        .as_str()
                        .is_some_and(|value| VALID_CONFIDENCES.contains(&value));
                    if !valid {
                        errors.push(format!(
                            "Edge {index} has invalid confidence '{}' - must be one of {}",
                            python_string(confidence),
                            python_string_list(&VALID_CONFIDENCES)
                        ));
                    }
                }
                for endpoint in ["source", "target"] {
                    let Some(value) = edge.get(endpoint) else {
                        continue;
                    };
                    // Python short-circuits before hashing malformed endpoints
                    // when no valid node id has been collected.
                    if node_ids.is_empty() {
                        continue;
                    }
                    let Some(key) = hashable_json(value) else {
                        errors.push(format!(
                            "Edge {index} {endpoint} {} is non-hashable - must be a string",
                            python_repr(value)
                        ));
                        continue;
                    };
                    if !node_ids.contains(&key) {
                        errors.push(format!(
                            "Edge {index} {endpoint} '{}' does not match any node id",
                            python_string(value)
                        ));
                    }
                }
            }
        }
        Some(_) => errors.push("'edges' must be a list".to_owned()),
    }
    errors
}

/// Return an aggregated validation error when an extraction is invalid.
pub fn assert_valid_extraction(data: &Value) -> Result<(), ExtractionValidationError> {
    let errors = validate_extraction(data);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ExtractionValidationError { errors })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtractionValidationError {
    pub errors: Vec<String>,
}

impl std::fmt::Display for ExtractionValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            formatter,
            "Extraction JSON has {} error(s):",
            self.errors.len()
        )?;
        for (index, error) in self.errors.iter().enumerate() {
            write!(formatter, "  • {error}")?;
            if index + 1 != self.errors.len() {
                formatter.write_char('\n')?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for ExtractionValidationError {}

fn hashable_json(value: &Value) -> Option<HashableJson> {
    match value {
        Value::Null => Some(HashableJson::None),
        Value::Bool(value) => Some(HashableJson::Integer(i128::from(*value))),
        Value::String(value) => Some(HashableJson::String(value.clone())),
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Some(HashableJson::Integer(i128::from(integer)))
            } else if let Some(integer) = value.as_u64() {
                Some(HashableJson::Integer(i128::from(integer)))
            } else {
                let number = value.as_f64()?;
                if number == 0.0 {
                    Some(HashableJson::Integer(0))
                } else if number.fract() == 0.0
                    && number >= i128::MIN as f64
                    && number <= i128::MAX as f64
                {
                    Some(HashableJson::Integer(number as i128))
                } else {
                    Some(HashableJson::Float(number.to_bits()))
                }
            }
        }
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn python_string_list(values: &[&str]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| python_repr(&Value::String((*value).to_owned())))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn python_string(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(value) => if *value { "True" } else { "False" }.to_owned(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => python_repr(value),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(value) => if *value { "True" } else { "False" }.to_owned(),
        Value::String(value) => {
            let escaped = value
                .chars()
                .flat_map(|character| match character {
                    '\\' => "\\\\".chars().collect::<Vec<_>>(),
                    '\'' => "\\'".chars().collect::<Vec<_>>(),
                    '\n' => "\\n".chars().collect::<Vec<_>>(),
                    '\r' => "\\r".chars().collect::<Vec<_>>(),
                    '\t' => "\\t".chars().collect::<Vec<_>>(),
                    character if character.is_control() => {
                        format!("\\x{:02x}", character as u32).chars().collect()
                    }
                    character => vec![character],
                })
                .collect::<String>();
            format!("'{escaped}'")
        }
        Value::Number(value) => value.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}: {}",
                    python_repr(&Value::String(key.clone())),
                    python_repr(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn accepts_links_and_python_numeric_id_equality() {
        let extraction = json!({
            "nodes":[{"id":true,"label":"x","file_type":"code","source_file":"x"}],
            "links":[{"source":1,"target":1.0,"relation":"x","confidence":"EXTRACTED","source_file":"x"}]
        });
        assert!(validate_extraction(&extraction).is_empty());
    }

    #[test]
    fn aggregate_error_matches_python_shape() -> Result<(), Box<dyn std::error::Error>> {
        let Err(error) = assert_valid_extraction(&json!({"nodes":"bad","edges":[]})) else {
            return Err("invalid extraction unexpectedly passed".into());
        };
        assert_eq!(
            error.to_string(),
            "Extraction JSON has 1 error(s):\n  • 'nodes' must be a list"
        );
        Ok(())
    }
}
