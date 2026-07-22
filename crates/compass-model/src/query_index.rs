use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{EdgeIndex, EdgeRecord, NodeIndex, NodeRecord};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SchemaFingerprint([u8; 32]);

impl SchemaFingerprint {
    #[must_use]
    pub const fn empty() -> Self {
        Self([0; 32])
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        output
    }
}

#[derive(Clone, Debug)]
pub struct QueryIndex {
    nodes_by_label: BTreeMap<String, Vec<NodeIndex>>,
    nodes_by_display_label: HashMap<String, Vec<NodeIndex>>,
    nodes_by_source_file: HashMap<String, Vec<NodeIndex>>,
    edges_by_type: BTreeMap<String, Vec<EdgeIndex>>,
    outgoing_by_type: Vec<BTreeMap<String, Vec<EdgeIndex>>>,
    incoming_by_type: Vec<BTreeMap<String, Vec<EdgeIndex>>>,
    schema_fingerprint: SchemaFingerprint,
}

impl QueryIndex {
    pub(crate) fn build(
        nodes: &[NodeRecord],
        edges: &[EdgeRecord],
        ids: &HashMap<String, NodeIndex>,
    ) -> Self {
        let mut nodes_by_label = BTreeMap::<String, Vec<NodeIndex>>::new();
        let mut nodes_by_display_label = HashMap::<String, Vec<NodeIndex>>::new();
        let mut nodes_by_source_file = HashMap::<String, Vec<NodeIndex>>::new();
        let mut edges_by_type = BTreeMap::<String, Vec<EdgeIndex>>::new();
        let mut outgoing_by_type = vec![BTreeMap::<String, Vec<EdgeIndex>>::new(); nodes.len()];
        let mut incoming_by_type = vec![BTreeMap::<String, Vec<EdgeIndex>>::new(); nodes.len()];

        for (index, node) in nodes.iter().enumerate() {
            let label = cypher_node_label(node);
            nodes_by_label.entry(label).or_default().push(index);
            nodes_by_display_label
                .entry(node.label().to_owned())
                .or_default()
                .push(index);
            if let Some(source_file) = node.attributes.get("source_file").and_then(Value::as_str) {
                nodes_by_source_file
                    .entry(source_file.to_owned())
                    .or_default()
                    .push(index);
            }
        }

        for (index, edge) in edges.iter().enumerate() {
            let relation = cypher_relationship_type(edge);
            edges_by_type
                .entry(relation.clone())
                .or_default()
                .push(index);
            if let Some(source) = ids.get(&edge.source) {
                outgoing_by_type[*source]
                    .entry(relation.clone())
                    .or_default()
                    .push(index);
            }
            if let Some(target) = ids.get(&edge.target) {
                incoming_by_type[*target]
                    .entry(relation)
                    .or_default()
                    .push(index);
            }
        }

        let schema_fingerprint = fingerprint_schema(nodes, edges);
        Self {
            nodes_by_label,
            nodes_by_display_label,
            nodes_by_source_file,
            edges_by_type,
            outgoing_by_type,
            incoming_by_type,
            schema_fingerprint,
        }
    }

    #[must_use]
    pub fn nodes_with_label(&self, label: &str) -> &[NodeIndex] {
        self.nodes_by_label.get(label).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn nodes_with_display_label(&self, label: &str) -> &[NodeIndex] {
        self.nodes_by_display_label
            .get(label)
            .map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn nodes_with_source_file(&self, source_file: &str) -> &[NodeIndex] {
        self.nodes_by_source_file
            .get(source_file)
            .map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn edges_with_type(&self, relation: &str) -> &[EdgeIndex] {
        self.edges_by_type.get(relation).map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn outgoing_with_type(&self, node: NodeIndex, relation: &str) -> &[EdgeIndex] {
        self.outgoing_by_type
            .get(node)
            .and_then(|relations| relations.get(relation))
            .map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn incoming_with_type(&self, node: NodeIndex, relation: &str) -> &[EdgeIndex] {
        self.incoming_by_type
            .get(node)
            .and_then(|relations| relations.get(relation))
            .map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub const fn schema_fingerprint(&self) -> SchemaFingerprint {
        self.schema_fingerprint
    }
}

#[must_use]
pub fn cypher_node_label(node: &NodeRecord) -> String {
    let raw = node
        .attributes
        .get("file_type")
        .and_then(Value::as_str)
        .unwrap_or("Entity");
    let mut value = raw.to_lowercase();
    if let Some(first) = value.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    cypher_identifier(&value, "Entity")
}

#[must_use]
pub fn cypher_relationship_type(edge: &EdgeRecord) -> String {
    let raw = edge
        .attributes
        .get("relation")
        .and_then(Value::as_str)
        .unwrap_or("RELATES_TO")
        .to_uppercase();
    cypher_identifier(&raw, "RELATES_TO")
}

fn cypher_identifier(value: &str, fallback: &str) -> String {
    let cleaned = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>();
    if cleaned
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
    {
        cleaned
    } else {
        fallback.to_owned()
    }
}

fn fingerprint_schema(nodes: &[NodeRecord], edges: &[EdgeRecord]) -> SchemaFingerprint {
    let mut entries = BTreeSet::new();
    for node in nodes {
        entries.insert(format!("N:L:{}", cypher_node_label(node)));
        entries.insert("N:P:id:string".to_owned());
        entries.insert("N:P:label:string".to_owned());
        for (key, value) in &node.attributes {
            entries.insert(format!("N:P:{key}:{}", value_kind(value)));
        }
    }
    for edge in edges {
        entries.insert(format!("R:T:{}", cypher_relationship_type(edge)));
        entries.insert("R:P:confidence:string".to_owned());
        for (key, value) in &edge.attributes {
            entries.insert(format!("R:P:{key}:{}", value_kind(value)));
        }
    }
    let mut digest = Sha256::new();
    for entry in entries {
        digest.update((entry.len() as u64).to_le_bytes());
        digest.update(entry.as_bytes());
    }
    SchemaFingerprint(digest.finalize().into())
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "float",
        Value::String(_) => "string",
        Value::Array(_) => "list",
        Value::Object(_) => "map",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Map;

    use super::*;

    #[test]
    fn graph_mapping_uses_documented_identifier_fallbacks() {
        let missing = NodeRecord {
            id: "missing".to_owned(),
            attributes: Map::new(),
        };
        assert_eq!(cypher_node_label(&missing), "Entity");

        let invalid = NodeRecord {
            id: "invalid".to_owned(),
            attributes: Map::from_iter([(
                "file_type".to_owned(),
                Value::String("123-!".to_owned()),
            )]),
        };
        assert_eq!(cypher_node_label(&invalid), "Entity");

        let edge = EdgeRecord {
            source: "missing".to_owned(),
            target: "invalid".to_owned(),
            attributes: Map::new(),
        };
        assert_eq!(cypher_relationship_type(&edge), "RELATES_TO");
    }
}
