use serde::{Deserialize, Serialize};
use serde_json::Value;
use trail_model::{EdgeRecord, NodeRecord};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawCall {
    pub caller_nid: String,
    pub callee: String,
    pub is_member_call: bool,
    pub source_file: String,
    pub source_location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Extraction {
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hyperedges: Vec<Value>,
    #[serde(default)]
    pub raw_calls: Vec<RawCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub extensions: serde_json::Map<String, Value>,
}
