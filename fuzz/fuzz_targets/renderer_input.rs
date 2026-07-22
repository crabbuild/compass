#![no_main]

use std::collections::BTreeMap;
use std::path::Path;

use libfuzzer_sys::fuzz_target;
use compass_graph::Communities;
use compass_model::GraphDocument;
use compass_output::{
    CanvasOptions, HtmlOptions, JsonExportOptions, SvgOptions, TreeOptions, canvas_document,
    cypher_document, export_json_value, graphml_document, html_document, svg_document,
    tree_html_document,
};

fuzz_target!(|data: &[u8]| {
    if data.len() > 1_048_576 {
        return;
    }
    let Ok(document) = serde_json::from_slice::<GraphDocument>(data) else {
        return;
    };
    if document.nodes.len() > 64 || document.links.len() > 256 {
        return;
    }
    let mut communities: Communities = BTreeMap::new();
    for (index, node) in document.nodes.iter().enumerate() {
        communities
            .entry(index % 4)
            .or_default()
            .push(node.id.clone());
    }
    let _ = export_json_value(&document, &communities, &JsonExportOptions::default());
    let _ = graphml_document(&document, &communities);
    let _ = cypher_document(&document);
    let _ = canvas_document(&document, &communities, &CanvasOptions::default());
    let _ = html_document(
        &document,
        &communities,
        Path::new("graph.html"),
        &HtmlOptions {
            node_limit: Some(64),
            ..HtmlOptions::default()
        },
    );
    let _ = svg_document(&document, &communities, &SvgOptions::default());
    let _ = tree_html_document(&document, &TreeOptions::default());
});
