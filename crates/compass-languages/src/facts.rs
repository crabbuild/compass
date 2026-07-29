use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tree_sitter::Node;

use crate::frameworks::RawFrameworkFact;

/// One flexible node fact produced before the strict v1 publication boundary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawNodeRecord {
    pub id: String,
    #[serde(flatten)]
    pub attributes: Map<String, Value>,
}

impl RawNodeRecord {
    #[must_use]
    pub fn string(&self, key: &str) -> String {
        self.attributes
            .get(key)
            .and_then(value_as_python_string)
            .unwrap_or_default()
    }

    #[must_use]
    pub fn label(&self) -> &str {
        self.attributes
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(&self.id)
    }
}

/// One flexible relationship fact produced before v1 normalization.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawEdgeRecord {
    pub source: String,
    pub target: String,
    #[serde(flatten)]
    pub attributes: Map<String, Value>,
}

impl RawEdgeRecord {
    #[must_use]
    pub fn string(&self, key: &str) -> String {
        self.attributes
            .get(key)
            .and_then(value_as_python_string)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawCall {
    pub caller_nid: String,
    pub callee: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_member_call: Option<bool>,
    pub source_file: String,
    pub source_location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_type: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Language-specific deterministic facts used by later resolution passes.
    /// Keeping these fields lossless is required for forward-compatible caches.
    #[serde(flatten)]
    pub extensions: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Extraction {
    pub nodes: Vec<RawNodeRecord>,
    pub edges: Vec<RawEdgeRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hyperedges: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_calls: Option<Vec<RawCall>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub framework_facts: Vec<RawFrameworkFact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub extensions: serde_json::Map<String, Value>,
}

fn value_as_python_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.to_owned()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => Some(value.to_string()),
    }
}

impl Default for Extraction {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            hyperedges: Vec::new(),
            raw_calls: Some(Vec::new()),
            framework_facts: Vec::new(),
            error: None,
            extensions: serde_json::Map::new(),
        }
    }
}

impl Extraction {
    pub(crate) fn raw_calls_mut(&mut self) -> &mut Vec<RawCall> {
        self.raw_calls.get_or_insert_with(Vec::new)
    }
}

pub(crate) fn stamp_node_range(attributes: &mut Map<String, Value>, node: Node<'_>) {
    let start = node.start_position();
    let end = node.end_position();
    attributes.insert(
        "start_byte".to_owned(),
        Value::from(u64::try_from(node.start_byte()).unwrap_or(u64::MAX)),
    );
    attributes.insert(
        "end_byte".to_owned(),
        Value::from(u64::try_from(node.end_byte()).unwrap_or(u64::MAX)),
    );
    attributes.insert(
        "line_start".to_owned(),
        Value::from(u64::try_from(start.row.saturating_add(1)).unwrap_or(u64::MAX)),
    );
    attributes.insert(
        "line_end".to_owned(),
        Value::from(u64::try_from(end.row.saturating_add(1)).unwrap_or(u64::MAX)),
    );
    attributes.insert(
        "column_start".to_owned(),
        Value::from(u64::try_from(start.column).unwrap_or(u64::MAX)),
    );
    attributes.insert(
        "column_end".to_owned(),
        Value::from(u64::try_from(end.column).unwrap_or(u64::MAX)),
    );
}

pub(crate) fn node_range(node: Node<'_>) -> Map<String, Value> {
    let mut attributes = Map::new();
    stamp_node_range(&mut attributes, node);
    attributes
}

pub(crate) fn stamp_last_edge_range(extraction: &mut Extraction, node: Node<'_>) {
    if let Some(edge) = extraction.edges.last_mut() {
        stamp_node_range(&mut edge.attributes, node);
    }
}

pub(crate) fn source_range(source: &[u8], start: usize, end: usize) -> Map<String, Value> {
    let mut attributes = Map::new();
    stamp_source_range(&mut attributes, source, start, end);
    attributes
}

pub(crate) fn stamp_source_range(
    attributes: &mut Map<String, Value>,
    source: &[u8],
    start: usize,
    end: usize,
) {
    let start = start.min(source.len());
    let end = end.clamp(start, source.len());
    let (start_line, start_column) = source_point(source, start);
    let (end_line, end_column) = source_point(source, end);
    attributes.insert("start_byte".to_owned(), Value::from(start as u64));
    attributes.insert("end_byte".to_owned(), Value::from(end as u64));
    attributes.insert("line_start".to_owned(), Value::from(start_line as u64));
    attributes.insert("line_end".to_owned(), Value::from(end_line as u64));
    attributes.insert("column_start".to_owned(), Value::from(start_column as u64));
    attributes.insert("column_end".to_owned(), Value::from(end_column as u64));
}

fn source_point(source: &[u8], offset: usize) -> (usize, usize) {
    let prefix = &source[..offset.min(source.len())];
    let line = prefix.iter().filter(|byte| **byte == b'\n').count() + 1;
    let column = prefix
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(prefix.len(), |newline| prefix.len() - newline - 1);
    (line, column)
}
