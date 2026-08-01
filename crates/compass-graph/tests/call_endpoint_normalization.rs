use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::Extraction;
use compass_model::code_graph::EdgeKind;
use serde_json::json;

#[test]
fn constructor_calls_normalize_to_instantiation_edges() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("src/service.py");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    fs::write(
        &path,
        "\nclass Service:\n    pass\n\ndef build():\n    return Service()\n",
    )?;
    let source_file = path.to_string_lossy();
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes": [
            {
                "id": "build",
                "label": "build()",
                "symbol_kind": "function",
                "file_type": "code",
                "source_file": source_file,
                "source_location": "L4",
                "start_byte": 31,
                "end_byte": 36,
                "line_start": 5,
                "line_end": 5,
                "column_start": 4,
                "column_end": 9,
                "language": "python",
                "confidence": "EXTRACTED",
                "_origin": "ast"
            },
            {
                "id": "service",
                "label": "Service",
                "symbol_kind": "class",
                "file_type": "code",
                "source_file": source_file,
                "source_location": "L1",
                "start_byte": 7,
                "end_byte": 14,
                "line_start": 2,
                "line_end": 2,
                "column_start": 6,
                "column_end": 13,
                "language": "python",
                "confidence": "EXTRACTED",
                "_origin": "ast"
            }
        ],
        "edges": [{
            "source": "build",
            "target": "service",
            "relation": "calls",
            "source_file": source_file,
            "source_location": "L5",
            "start_byte": 51,
            "end_byte": 58,
            "line_start": 6,
            "line_end": 6,
            "column_start": 11,
            "column_end": 18,
            "confidence": "EXTRACTED",
            "_origin": "ast"
        }]
    }))?;
    assert!(
        extraction.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && extraction
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.source)
                    .is_some_and(|node| node.label() == "build()")
                && extraction
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.target)
                    .is_some_and(|node| node.label() == "Service")
        }),
        "raw edges={:?}",
        extraction.edges
    );

    let flexible = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;
    assert!(
        graph.links.iter().any(|edge| {
            edge.kind == EdgeKind::Instantiates
                && graph
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.source)
                    .is_some_and(|node| node.name == "build()")
                && graph
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.target)
                    .is_some_and(|node| node.name == "Service")
        }),
        "links={:?}",
        graph.links
    );
    Ok(())
}
