use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::GraphError;

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

impl GraphDocument {
    /// Load a node-link document under the compatible extension and size guards.
    pub fn load(path: &Path) -> Result<Self, GraphError> {
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            return Err(GraphError::InvalidExtension(path.to_path_buf()));
        }
        if let Some((size, cap)) = Self::size_cap_exceeded(path) {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size,
                cap,
            });
        }
        let before = graph_signature(path);
        if let Some(signature) = before
            && let Some(document) = load_query_cache(path, signature)
            && graph_signature(path) == Some(signature)
        {
            let _ = write_affected_cache(path, signature, &document);
            return Ok(document);
        }
        let document = Self::load_for_recluster_compatibility(path)?;
        if let Some(signature) = before
            && graph_signature(path) == Some(signature)
        {
            let _ = write_query_cache(path, signature, &document);
            let _ = write_affected_cache(path, signature, &document);
        }
        Ok(document)
    }

    /// Load the compact, lossless projection required by `graph affected`.
    ///
    /// The projection retains every node endpoint and edge relation while
    /// omitting attributes that cannot influence seed resolution, traversal,
    /// or rendering. Other graph commands continue to load the full document.
    pub fn load_for_affected(path: &Path) -> Result<Self, GraphError> {
        if path.extension().and_then(|part| part.to_str()) != Some("json") {
            return Err(GraphError::InvalidExtension(path.to_path_buf()));
        }
        if let Some((size, cap)) = Self::size_cap_exceeded(path) {
            return Err(GraphError::TooLarge {
                path: crate::graph::absolute_path(path),
                size,
                cap,
            });
        }
        let signature = graph_signature(path);
        if let Some(signature) = signature
            && let Some(document) = load_affected_cache(path, signature)
            && graph_signature(path) == Some(signature)
        {
            return Ok(document);
        }
        let document = Self::load(path)?;
        if let Some(signature) = signature
            && let Some(compact) = load_affected_cache(path, signature)
            && graph_signature(path) == Some(signature)
        {
            return Ok(compact);
        }
        let compact = document.compact_for_affected();
        if let Some(signature) = signature
            && graph_signature(path) == Some(signature)
        {
            let _ = write_compact_cache(path, signature, &compact);
        }
        Ok(compact)
    }

    /// Load a node-link document like Python's re-clustering command.
    ///
    /// That command accepts arbitrary filenames and warns on oversized files
    /// while still refreshing the core graph artifacts.
    pub fn load_for_recluster_compatibility(path: &Path) -> Result<Self, GraphError> {
        if !path.exists() {
            return Err(GraphError::NotFound(crate::graph::absolute_path(path)));
        }
        let bytes = fs::read(path).map_err(|source| GraphError::Read {
            path: crate::graph::absolute_path(path),
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(GraphError::Corrupt)
    }

    #[must_use]
    pub fn size_cap_exceeded(path: &Path) -> Option<(u64, u64)> {
        let size = path.metadata().ok()?.len();
        let cap = crate::graph::graph_size_cap();
        (size > cap).then_some((size, cap))
    }

    fn compact_for_affected(&self) -> Self {
        let nodes = self
            .nodes
            .iter()
            .map(|node| {
                let attributes = ["label", "source_file", "source_location"]
                    .into_iter()
                    .filter_map(|key| {
                        node.attributes
                            .get(key)
                            .cloned()
                            .map(|value| (key.to_owned(), value))
                    })
                    .collect();
                NodeRecord {
                    id: node.id.clone(),
                    attributes,
                }
            })
            .collect();
        let links = self
            .links
            .iter()
            .map(|edge| {
                let attributes = edge
                    .attributes
                    .get("relation")
                    .cloned()
                    .map(|value| [("relation".to_owned(), value)].into_iter().collect())
                    .unwrap_or_default();
                EdgeRecord {
                    source: edge.source.clone(),
                    target: edge.target.clone(),
                    attributes,
                }
            })
            .collect();
        Self {
            directed: self.directed,
            multigraph: self.multigraph,
            graph: Map::new(),
            nodes,
            links,
            extras: BTreeMap::new(),
            used_legacy_edges_key: self.used_legacy_edges_key,
        }
    }
}

const QUERY_CACHE_MAGIC: &[u8; 8] = b"TRAILG01";
const AFFECTED_CACHE_MAGIC: &[u8; 8] = b"TRAILA01";
const QUERY_CACHE_HEADER_LEN: usize = 28;
static QUERY_CACHE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GraphSignature {
    len: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

fn graph_signature(path: &Path) -> Option<GraphSignature> {
    let metadata = path.metadata().ok()?;
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(GraphSignature {
        len: metadata.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

fn query_cache_path(graph_path: &Path) -> PathBuf {
    let file_name = graph_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("graph.json");
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cache")
        .join(format!(".{file_name}.compass-cache-v1"))
}

fn affected_cache_path(graph_path: &Path) -> PathBuf {
    let file_name = graph_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("graph.json");
    graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cache")
        .join(format!(".{file_name}.compass-affected-v1"))
}

fn encode_cache_header(magic: &[u8; 8], signature: GraphSignature) -> [u8; QUERY_CACHE_HEADER_LEN] {
    let mut header = [0_u8; QUERY_CACHE_HEADER_LEN];
    header[..8].copy_from_slice(magic);
    header[8..16].copy_from_slice(&signature.len.to_le_bytes());
    header[16..24].copy_from_slice(&signature.modified_secs.to_le_bytes());
    header[24..28].copy_from_slice(&signature.modified_nanos.to_le_bytes());
    header
}

fn load_query_cache(path: &Path, signature: GraphSignature) -> Option<GraphDocument> {
    load_cache(&query_cache_path(path), QUERY_CACHE_MAGIC, signature)
}

fn load_affected_cache(path: &Path, signature: GraphSignature) -> Option<GraphDocument> {
    load_cache(&affected_cache_path(path), AFFECTED_CACHE_MAGIC, signature)
}

fn load_cache(
    cache_path: &Path,
    magic: &[u8; 8],
    signature: GraphSignature,
) -> Option<GraphDocument> {
    let mut reader = BufReader::new(File::open(cache_path).ok()?);
    let mut header = [0_u8; QUERY_CACHE_HEADER_LEN];
    reader.read_exact(&mut header).ok()?;
    if !cache_header_matches(cache_path, magic, signature, &header) {
        return None;
    }
    rmp_serde::from_read(reader).ok()
}

fn cache_header_matches(
    cache_path: &Path,
    magic: &[u8; 8],
    signature: GraphSignature,
    header: &[u8; QUERY_CACHE_HEADER_LEN],
) -> bool {
    let Some(cache_size) = cache_path.metadata().ok().map(|metadata| metadata.len()) else {
        return false;
    };
    let maximum = signature.len.saturating_mul(2).saturating_add(1024 * 1024);
    cache_size <= maximum && header == &encode_cache_header(magic, signature)
}

fn cache_is_valid(cache_path: &Path, magic: &[u8; 8], signature: GraphSignature) -> bool {
    let Ok(mut file) = File::open(cache_path) else {
        return false;
    };
    let mut header = [0_u8; QUERY_CACHE_HEADER_LEN];
    file.read_exact(&mut header).is_ok()
        && cache_header_matches(cache_path, magic, signature, &header)
}

fn write_query_cache(
    graph_path: &Path,
    signature: GraphSignature,
    document: &GraphDocument,
) -> std::io::Result<()> {
    write_cache(
        &query_cache_path(graph_path),
        QUERY_CACHE_MAGIC,
        signature,
        document,
    )
}

fn write_affected_cache(
    graph_path: &Path,
    signature: GraphSignature,
    document: &GraphDocument,
) -> std::io::Result<()> {
    if cache_is_valid(
        &affected_cache_path(graph_path),
        AFFECTED_CACHE_MAGIC,
        signature,
    ) {
        return Ok(());
    }
    write_compact_cache(graph_path, signature, &document.compact_for_affected())
}

fn write_compact_cache(
    graph_path: &Path,
    signature: GraphSignature,
    document: &GraphDocument,
) -> std::io::Result<()> {
    write_cache(
        &affected_cache_path(graph_path),
        AFFECTED_CACHE_MAGIC,
        signature,
        document,
    )
}

fn write_cache(
    cache_path: &Path,
    magic: &[u8; 8],
    signature: GraphSignature,
    document: &GraphDocument,
) -> std::io::Result<()> {
    let sequence = QUERY_CACHE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = cache_path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        let mut writer = BufWriter::new(file);
        writer.write_all(&encode_cache_header(magic, signature))?;
        rmp_serde::encode::write_named(&mut writer, document).map_err(std::io::Error::other)?;
        writer.flush()?;
        drop(writer);
        #[cfg(windows)]
        if cache_path.exists() {
            fs::remove_file(&cache_path)?;
        }
        fs::rename(&temporary, cache_path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[derive(Deserialize)]
struct RawGraphDocument {
    #[serde(default)]
    directed: bool,
    // NetworkX's node_link_graph() treats an omitted `multigraph` member as
    // true. Graphify's compact graph writer relies on that legacy default, so
    // treating omission as false would collapse parallel edges and change
    // degree-sensitive traversal semantics.
    #[serde(default = "networkx_default_multigraph")]
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

const fn networkx_default_multigraph() -> bool {
    true
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{GraphDocument, affected_cache_path, query_cache_path};

    #[test]
    fn omitted_multigraph_uses_networkx_legacy_default() {
        let document: GraphDocument = serde_json::from_str(r#"{"nodes":[{"id":"a"}],"links":[]}"#)
            .unwrap_or_else(|_| std::process::abort());
        assert!(document.multigraph);
    }

    #[test]
    fn explicit_multigraph_false_remains_false() {
        let document: GraphDocument =
            serde_json::from_str(r#"{"multigraph":false,"nodes":[{"id":"a"}],"links":[]}"#)
                .unwrap_or_else(|_| std::process::abort());
        assert!(!document.multigraph);
    }

    #[test]
    fn query_cache_is_hidden_and_invalidates_when_the_graph_changes() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = directory.path().join("graph.json");
        fs::create_dir(directory.path().join("cache")).unwrap_or_else(|_| std::process::abort());
        fs::write(&path, r#"{"nodes":[{"id":"a"}],"links":[]}"#)
            .unwrap_or_else(|_| std::process::abort());

        let first = GraphDocument::load(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(first.nodes[0].id, "a");
        assert!(query_cache_path(&path).is_file());

        fs::write(
            &path,
            r#"{"nodes":[{"id":"changed-and-longer"}],"links":[]}"#,
        )
        .unwrap_or_else(|_| std::process::abort());
        let changed = GraphDocument::load(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(changed.nodes[0].id, "changed-and-longer");

        fs::write(query_cache_path(&path), b"corrupt").unwrap_or_else(|_| std::process::abort());
        let recovered = GraphDocument::load(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(recovered.nodes[0].id, "changed-and-longer");
    }

    #[test]
    fn affected_cache_retains_contract_fields_and_omits_irrelevant_payload() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let path = directory.path().join("graph.json");
        fs::create_dir(directory.path().join("cache")).unwrap_or_else(|_| std::process::abort());
        fs::write(
            &path,
            r#"{"directed":true,"multigraph":true,"nodes":[{"id":"a","label":"A","source_file":"a.rs","source_location":"L2","large_payload":"discard"},{"id":"b","label":"B"}],"links":[{"source":"a","target":"b","relation":"custom","confidence":"EXTRACTED"}]}"#,
        )
        .unwrap_or_else(|_| std::process::abort());

        let full = GraphDocument::load(&path).unwrap_or_else(|_| std::process::abort());
        assert!(affected_cache_path(&path).is_file());
        let compact =
            GraphDocument::load_for_affected(&path).unwrap_or_else(|_| std::process::abort());
        assert_eq!(compact.directed, full.directed);
        assert_eq!(compact.multigraph, full.multigraph);
        assert_eq!(compact.nodes[0].string("label"), "A");
        assert_eq!(compact.nodes[0].string("source_file"), "a.rs");
        assert_eq!(compact.nodes[0].string("source_location"), "L2");
        assert!(!compact.nodes[0].attributes.contains_key("large_payload"));
        assert_eq!(compact.links[0].string("relation"), "custom");
        assert!(!compact.links[0].attributes.contains_key("confidence"));
    }
}
