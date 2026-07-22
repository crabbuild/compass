use prolly::KeyBuilder;

pub(crate) const KEY_SCHEMA_V1: &[u8] = &[1];
pub(crate) const NODE_KIND: &[u8] = &[1];
pub(crate) const EDGE_KIND: &[u8] = &[2];
pub(crate) const HYPEREDGE_KIND: &[u8] = &[3];

/// Construct a segment-safe node key.
#[must_use]
pub fn node_key(id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push_segment(KEY_SCHEMA_V1)
        .push_segment(NODE_KIND)
        .push_str(id)
        .finish()
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
    let builder = KeyBuilder::new()
        .push_segment(KEY_SCHEMA_V1)
        .push_segment(EDGE_KIND)
        .push_str(source)
        .push_str(target)
        .push_str(relation);
    match discriminator {
        Some(value) => builder.push_segment(value).finish(),
        None => builder.finish(),
    }
}

/// Construct a stable hyperedge key, optionally distinguishing an exact duplicate.
#[must_use]
pub fn hyperedge_key(identity: &[u8], occurrence: Option<u64>) -> Vec<u8> {
    let builder = KeyBuilder::new()
        .push_segment(KEY_SCHEMA_V1)
        .push_segment(HYPEREDGE_KIND)
        .push_segment(identity);
    match occurrence {
        Some(rank) => builder.push_segment(rank.to_be_bytes()).finish(),
        None => builder.finish(),
    }
}

pub(crate) fn root_name(parts: &[&[u8]]) -> Vec<u8> {
    parts
        .iter()
        .fold(KeyBuilder::new(), |builder, part| {
            builder.push_segment(part)
        })
        .finish()
}
