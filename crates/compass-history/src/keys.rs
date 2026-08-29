use prolly::KeyBuilder;

pub use compass_partition::{edge_key, hyperedge_key, node_key};

pub(crate) const KEY_SCHEMA_V1: &[u8] = &[1];
pub(crate) const NODE_KIND: &[u8] = &[1];
pub(crate) const EDGE_KIND: &[u8] = &[2];
pub(crate) const HYPEREDGE_KIND: &[u8] = &[3];

pub(crate) fn root_name(parts: &[&[u8]]) -> Vec<u8> {
    parts
        .iter()
        .fold(KeyBuilder::new(), |builder, part| {
            builder.push_segment(part)
        })
        .finish()
}
