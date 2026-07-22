use compass_model::{EdgeRecord, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hyperedges: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_calls: Option<Vec<RawCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub extensions: serde_json::Map<String, Value>,
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
