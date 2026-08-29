//! Storage-neutral graph partitions, stable record keys, and canonical JSON encoding.

use serde_json::Value;

const KEY_SCHEMA_V1: &[u8] = &[1];
const NODE_KIND: &[u8] = &[1];
const EDGE_KIND: &[u8] = &[2];
const HYPEREDGE_KIND: &[u8] = &[3];

/// Version of the byte-stable canonical JSON encoding.
pub const CANONICAL_ENCODING_VERSION: u32 = 1;

/// Errors produced while encoding storage-neutral partition values.
#[derive(Debug, thiserror::Error)]
pub enum PartitionError {
    /// A value could not be represented by the canonical encoding.
    #[error("canonical encoding failed: {0}")]
    Canonical(String),
}

/// Deterministic typed records used to construct a partitioned graph.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PartitionedGraph {
    /// Canonical node key/value records.
    pub nodes: Vec<(Vec<u8>, Vec<u8>)>,
    /// Canonical edge key/value records.
    pub edges: Vec<(Vec<u8>, Vec<u8>)>,
    /// Canonical hyperedge key/value records.
    pub hyperedges: Vec<(Vec<u8>, Vec<u8>)>,
    /// Canonical graph-analysis key/value records.
    pub analysis: Vec<(Vec<u8>, Vec<u8>)>,
    /// Canonical graph-metadata key/value records.
    pub metadata: Vec<(Vec<u8>, Vec<u8>)>,
    /// Canonical program-fact key/value records.
    pub program_facts: Vec<(Vec<u8>, Vec<u8>)>,
    /// Canonical program-summary key/value records.
    pub program_summaries: Vec<(Vec<u8>, Vec<u8>)>,
}

/// Encode JSON into deterministic, whitespace-free UTF-8 bytes.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, PartitionError> {
    let mut output = Vec::new();
    write_canonical_value(value, &mut output)?;
    Ok(output)
}

fn write_canonical_value(value: &Value, output: &mut Vec<u8>) -> Result<(), PartitionError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Number(number) => output.extend_from_slice(number.to_string().as_bytes()),
        Value::String(text) => {
            serde_json::to_writer(output, text)
                .map_err(|error| PartitionError::Canonical(error.to_string()))?;
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_canonical_value(item, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)
                    .map_err(|error| PartitionError::Canonical(error.to_string()))?;
                output.push(b':');
                let item = values.get(key).ok_or_else(|| {
                    PartitionError::Canonical("object key disappeared during encoding".to_owned())
                })?;
                write_canonical_value(item, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

/// Construct a segment-safe node key.
#[must_use]
pub fn node_key(id: &str) -> Vec<u8> {
    encode_segments(&[KEY_SCHEMA_V1, NODE_KIND, id.as_bytes()])
}

/// Construct a direction-aware, segment-safe edge key.
#[must_use]
pub fn edge_key(
    source: &str,
    target: &str,
    relation: &str,
    directed: bool,
    discriminator: Option<&[u8]>,
) -> Vec<u8> {
    let (source, target) = if directed || source <= target {
        (source, target)
    } else {
        (target, source)
    };
    let mut segments = vec![
        KEY_SCHEMA_V1,
        EDGE_KIND,
        source.as_bytes(),
        target.as_bytes(),
        relation.as_bytes(),
    ];
    if let Some(value) = discriminator {
        segments.push(value);
    }
    encode_segments(&segments)
}

/// Construct a stable hyperedge key, optionally distinguishing an exact duplicate.
#[must_use]
pub fn hyperedge_key(identity: &[u8], occurrence: Option<u64>) -> Vec<u8> {
    let rank = occurrence.map(u64::to_be_bytes);
    let mut segments = vec![KEY_SCHEMA_V1, HYPEREDGE_KIND, identity];
    if let Some(value) = rank.as_ref() {
        segments.push(value);
    }
    encode_segments(&segments)
}

fn encode_segments(segments: &[&[u8]]) -> Vec<u8> {
    let capacity = segments.iter().fold(0_usize, |total, segment| {
        total.saturating_add(segment.len()).saturating_add(2)
    });
    let mut output = Vec::with_capacity(capacity);
    for segment in segments {
        for byte in *segment {
            if *byte == 0 {
                output.extend_from_slice(&[0, 0xff]);
            } else {
                output.push(*byte);
            }
        }
        output.extend_from_slice(&[0, 0]);
    }
    output
}
