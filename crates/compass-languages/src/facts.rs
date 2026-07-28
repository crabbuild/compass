use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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
