use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One node in `NetworkX` node-link form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeRecord {
    pub id: String,
    #[serde(flatten)]
    pub attributes: Map<String, Value>,
}

impl NodeRecord {
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

/// One edge in `NetworkX` node-link form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeRecord {
    pub source: String,
    pub target: String,
    #[serde(flatten)]
    pub attributes: Map<String, Value>,
}

impl EdgeRecord {
    #[must_use]
    pub fn string(&self, key: &str) -> String {
        self.attributes
            .get(key)
            .and_then(value_as_python_string)
            .unwrap_or_default()
    }
}

/// Full node-link document, retaining unknown top-level fields.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphDocument {
    pub directed: bool,
    pub multigraph: bool,
    pub graph: Map<String, Value>,
    pub nodes: Vec<NodeRecord>,
    pub links: Vec<EdgeRecord>,
    pub extras: BTreeMap<String, Value>,
    pub used_legacy_edges_key: bool,
}

#[derive(Deserialize)]
struct RawGraphDocument {
    #[serde(default)]
    directed: bool,
    #[serde(default)]
    multigraph: bool,
    #[serde(default)]
    graph: Map<String, Value>,
    #[serde(default)]
    nodes: Vec<NodeRecord>,
    links: Option<Vec<EdgeRecord>>,
    edges: Option<Vec<EdgeRecord>>,
    #[serde(flatten)]
    extras: BTreeMap<String, Value>,
}

impl<'de> Deserialize<'de> for GraphDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawGraphDocument::deserialize(deserializer)?;
        let used_legacy_edges_key = raw.links.is_none() && raw.edges.is_some();
        let links = raw.links.or(raw.edges).unwrap_or_default();
        Ok(Self {
            directed: raw.directed,
            multigraph: raw.multigraph,
            graph: raw.graph,
            nodes: raw.nodes,
            links,
            extras: raw.extras,
            used_legacy_edges_key,
        })
    }
}

impl Serialize for GraphDocument {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(Some(5 + self.extras.len()))?;
        map.serialize_entry("directed", &self.directed)?;
        map.serialize_entry("multigraph", &self.multigraph)?;
        map.serialize_entry("graph", &self.graph)?;
        map.serialize_entry("nodes", &self.nodes)?;
        if self.used_legacy_edges_key {
            map.serialize_entry("edges", &self.links)?;
        } else {
            map.serialize_entry("links", &self.links)?;
        }
        for (key, value) in &self.extras {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
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
