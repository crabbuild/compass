#![no_main]

use std::collections::HashMap;

use libfuzzer_sys::fuzz_target;
use trail_model::{Graph, GraphDocument};
use trail_query::{TraversalMode, query_graph_text, render_explanation};

fuzz_target!(|data: &[u8]| {
    if data.len() > 1_048_576 {
        return;
    }
    let Ok(document) = serde_json::from_slice::<GraphDocument>(data) else {
        return;
    };
    if document.nodes.len() > 2_000 || document.links.len() > 8_000 {
        return;
    }
    let Ok(graph) = Graph::from_document(document) else {
        return;
    };
    let query = String::from_utf8_lossy(&data[..data.len().min(256)]);
    let labels = HashMap::new();
    let _ = query_graph_text(
        &graph,
        &query,
        TraversalMode::Bfs,
        2,
        2_000,
        &[],
        &labels,
    );
    let _ = render_explanation(&graph, &query, &labels);
});
