//! Developer stage tracing against an explicitly selected graph and cache.
//! This observes a known target; it never changes retrieval or its ordering.
use std::error::Error;
use std::io::{Error as IoError, ErrorKind};
use std::path::Path;

use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
};
use compass_query::{EngineSelection, open_with_engine};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = std::env::args().skip(1).take(5).collect::<Vec<_>>();
    if arguments.len() != 4 {
        return Err(IoError::new(
            ErrorKind::InvalidInput,
            "usage: broad_retrieval_trace GRAPH CACHE QUESTION EXPECTED_ID",
        )
        .into());
    }
    let engine = open_with_engine(
        Path::new(&arguments[0]),
        None,
        Path::new(&arguments[1]),
        EngineSelection::Json,
    )?;
    let request = DiscoveryQueryRequest {
        question: arguments[2].clone(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    };
    let trace = engine.trace_ranked(request, &arguments[3])?;
    serde_json::to_writer(std::io::stdout().lock(), &trace)?;
    Ok(())
}
