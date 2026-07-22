use std::collections::HashMap;
use std::path::Path;

use compass_files::write_text_atomic;
use compass_graph::Communities;
use compass_model::GraphDocument;
use serde_json::Value;

use crate::OutputError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Scope {
    Graph,
    Node,
    Edge,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Graph => "graph",
            Self::Node => "node",
            Self::Edge => "edge",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum AttributeType {
    Boolean,
    Long,
    Double,
    String,
}

impl AttributeType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Long => "long",
            Self::Double => "double",
            Self::String => "string",
        }
    }
}

struct KeyRegistry {
    positions: HashMap<(String, AttributeType, Scope), usize>,
    keys: Vec<(String, AttributeType, Scope)>,
}

impl KeyRegistry {
    fn new() -> Self {
        Self {
            positions: HashMap::new(),
            keys: Vec::new(),
        }
    }

    fn key(&mut self, name: &str, value_type: AttributeType, scope: Scope) -> usize {
        let identity = (name.to_owned(), value_type, scope);
        if let Some(position) = self.positions.get(&identity) {
            return *position;
        }
        let position = self.keys.len();
        self.positions.insert(identity.clone(), position);
        self.keys.push(identity);
        position
    }
}

/// Produce NetworkX-compatible GraphML for Gephi, yEd, and GraphML consumers.
#[must_use]
pub fn graphml_document(document: &GraphDocument, communities: &Communities) -> String {
    let node_community = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<HashMap<_, _>>();
    let mut keys = KeyRegistry::new();
    let graph_attributes = document
        .graph
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "id" | "node_default" | "edge_default"))
        .map(|(name, value)| (name.clone(), graphml_value(value)))
        .collect::<Vec<_>>();
    let graph_data = register_attributes(&graph_attributes, Scope::Graph, &mut keys);

    let nodes = document
        .nodes
        .iter()
        .map(|node| {
            let mut attributes = node
                .attributes
                .iter()
                .filter(|(name, _)| !name.starts_with('_'))
                .map(|(name, value)| (name.clone(), graphml_value(value)))
                .collect::<Vec<_>>();
            attributes.push((
                "community".to_owned(),
                GraphmlValue::Long(
                    node_community
                        .get(node.id.as_str())
                        .copied()
                        .map_or(-1, |community| community as i64),
                ),
            ));
            let data = register_attributes(&attributes, Scope::Node, &mut keys);
            (node.id.as_str(), data)
        })
        .collect::<Vec<_>>();
    let edges = document
        .links
        .iter()
        .map(|edge| {
            let attributes = edge
                .attributes
                .iter()
                .filter(|(name, _)| !name.starts_with('_'))
                .map(|(name, value)| (name.clone(), graphml_value(value)))
                .collect::<Vec<_>>();
            let data = register_attributes(&attributes, Scope::Edge, &mut keys);
            (edge.source.as_str(), edge.target.as_str(), data)
        })
        .collect::<Vec<_>>();

    let mut output = String::from(
        "<?xml version='1.0' encoding='utf-8'?>\n<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"http://graphml.graphdrawing.org/xmlns http://graphml.graphdrawing.org/xmlns/1.0/graphml.xsd\">\n",
    );
    for (position, (name, value_type, scope)) in keys.keys.iter().enumerate().rev() {
        output.push_str(&format!(
            "  <key id=\"d{position}\" for=\"{}\" attr.name=\"{}\" attr.type=\"{}\" />\n",
            scope.as_str(),
            xml_attribute(name),
            value_type.as_str()
        ));
    }
    output.push_str(&format!(
        "  <graph edgedefault=\"{}\">\n",
        if document.directed {
            "directed"
        } else {
            "undirected"
        }
    ));
    for (key, value) in graph_data {
        output.push_str(&format!(
            "    <data key=\"d{key}\">{}</data>\n",
            xml_text(&value)
        ));
    }
    for (id, data) in nodes {
        if data.is_empty() {
            output.push_str(&format!("    <node id=\"{}\" />\n", xml_attribute(id)));
            continue;
        }
        output.push_str(&format!("    <node id=\"{}\">\n", xml_attribute(id)));
        for (key, value) in data {
            output.push_str(&format!(
                "      <data key=\"d{key}\">{}</data>\n",
                xml_text(&value)
            ));
        }
        output.push_str("    </node>\n");
    }
    for (source, target, data) in edges {
        if data.is_empty() {
            output.push_str(&format!(
                "    <edge source=\"{}\" target=\"{}\" />\n",
                xml_attribute(source),
                xml_attribute(target)
            ));
            continue;
        }
        output.push_str(&format!(
            "    <edge source=\"{}\" target=\"{}\">\n",
            xml_attribute(source),
            xml_attribute(target)
        ));
        for (key, value) in data {
            output.push_str(&format!(
                "      <data key=\"d{key}\">{}</data>\n",
                xml_text(&value)
            ));
        }
        output.push_str("    </edge>\n");
    }
    output.push_str("  </graph>\n</graphml>\n");
    output
}

pub fn write_graphml(
    document: &GraphDocument,
    communities: &Communities,
    output_path: impl AsRef<Path>,
) -> Result<(), OutputError> {
    write_text_atomic(output_path, &graphml_document(document, communities))?;
    Ok(())
}

fn register_attributes(
    attributes: &[(String, GraphmlValue)],
    scope: Scope,
    keys: &mut KeyRegistry,
) -> Vec<(usize, String)> {
    attributes
        .iter()
        .map(|(name, value)| (keys.key(name, value.value_type(), scope), value.as_text()))
        .collect()
}

enum GraphmlValue {
    Boolean(bool),
    Long(i64),
    Double(f64),
    String(String),
}

impl GraphmlValue {
    fn value_type(&self) -> AttributeType {
        match self {
            Self::Boolean(_) => AttributeType::Boolean,
            Self::Long(_) => AttributeType::Long,
            Self::Double(_) => AttributeType::Double,
            Self::String(_) => AttributeType::String,
        }
    }

    fn as_text(&self) -> String {
        match self {
            Self::Boolean(value) => {
                if *value {
                    "True".to_owned()
                } else {
                    "False".to_owned()
                }
            }
            Self::Long(value) => value.to_string(),
            Self::Double(value) if value.fract() == 0.0 && value.is_finite() => {
                format!("{value:.1}")
            }
            Self::Double(value) => value.to_string(),
            Self::String(value) => value.clone(),
        }
    }
}

fn graphml_value(value: &Value) -> GraphmlValue {
    match value {
        Value::Null => GraphmlValue::String(String::new()),
        Value::Bool(value) => GraphmlValue::Boolean(*value),
        Value::Number(value) if value.is_i64() => {
            GraphmlValue::Long(value.as_i64().unwrap_or_default())
        }
        Value::Number(value) if value.is_u64() => GraphmlValue::Long(
            value
                .as_u64()
                .and_then(|value| i64::try_from(value).ok())
                .unwrap_or(i64::MAX),
        ),
        Value::Number(value) => GraphmlValue::Double(value.as_f64().unwrap_or_default()),
        Value::String(value) => GraphmlValue::String(value.clone()),
        Value::Array(_) | Value::Object(_) => GraphmlValue::String(python_sorted_json(value)),
    }
}

fn python_sorted_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => python_json_string(value),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_sorted_json)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}: {}",
                        python_json_string(key),
                        python_sorted_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

fn python_json_string(value: &str) -> String {
    let encoded = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
    let mut output = String::with_capacity(encoded.len());
    for character in encoded.chars() {
        let code = character as u32;
        if code <= 0x7f {
            output.push(character);
        } else if code <= 0xffff {
            output.push_str(&format!("\\u{code:04x}"));
        } else {
            let scalar = code - 0x1_0000;
            output.push_str(&format!(
                "\\u{:04x}\\u{:04x}",
                0xd800 + (scalar >> 10),
                0xdc00 + (scalar & 0x3ff)
            ));
        }
    }
    output
}

fn xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_attribute(value: &str) -> String {
    xml_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
