use std::path::PathBuf;

/// Failures that can occur before a graph is safe to query.
#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("Graph path must be a .json file, got: {0:?}")]
    InvalidExtension(PathBuf),
    #[error("Graph file not found: {0}")]
    NotFound(PathBuf),
    #[error(
        "graph file {path} is {size} bytes, exceeds {cap}-byte cap\n(set GRAPHIFY_MAX_GRAPH_BYTES=<bytes> or GRAPHIFY_MAX_GRAPH_BYTES=<N>GB to raise the limit)"
    )]
    TooLarge { path: PathBuf, size: u64, cap: u64 },
    #[error("could not read graph file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("graph.json is corrupted ({0}). Re-run /graphify to rebuild.")]
    Corrupt(serde_json::Error),
    #[error("graph contains a node without a string id")]
    MissingNodeId,
    #[error("graph edge references an invalid endpoint")]
    InvalidEdgeEndpoint,
}
