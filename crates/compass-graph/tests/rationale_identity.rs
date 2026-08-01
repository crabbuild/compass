use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::Extraction;
use compass_model::code_graph::{EdgeKind, NodeDetails, NodeKind, ResourceKind};
use serde_json::json;

#[test]
fn repeated_rationales_at_distinct_sites_keep_distinct_v1_identities() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("service.py");
    fs::write(
        &path,
        r#"
def first():
    """Explain the shared compatibility behavior in enough detail to become a rationale resource."""

def second():
    """Explain the shared compatibility behavior in enough detail to become a rationale resource."""
"#,
    )?;

    let source_file = path.to_string_lossy();
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes": [
            {
                "id": "first",
                "label": "first()",
                "symbol_kind": "function",
                "file_type": "code",
                "source_file": source_file,
                "source_location": "L2",
                "start_byte": 5,
                "end_byte": 10,
                "line_start": 2,
                "line_end": 2,
                "column_start": 4,
                "column_end": 9,
                "confidence": "EXTRACTED",
                "_origin": "ast"
            },
            {
                "id": "second",
                "label": "second()",
                "symbol_kind": "function",
                "file_type": "code",
                "source_file": source_file,
                "source_location": "L5",
                "start_byte": 111,
                "end_byte": 117,
                "line_start": 5,
                "line_end": 5,
                "column_start": 4,
                "column_end": 10,
                "confidence": "EXTRACTED",
                "_origin": "ast"
            },
            {
                "id": "rationale-first",
                "label": "Explain the shared compatibility behavior in enough detail to become a rationale resource.",
                "file_type": "rationale",
                "source_file": source_file,
                "source_location": "L3",
                "start_byte": 18,
                "end_byte": 101,
                "line_start": 3,
                "line_end": 3,
                "column_start": 4,
                "column_end": 87,
                "confidence": "EXTRACTED",
                "_origin": "ast"
            },
            {
                "id": "rationale-second",
                "label": "Explain the shared compatibility behavior in enough detail to become a rationale resource.",
                "file_type": "rationale",
                "source_file": source_file,
                "source_location": "L6",
                "start_byte": 125,
                "end_byte": 208,
                "line_start": 6,
                "line_end": 6,
                "column_start": 4,
                "column_end": 87,
                "confidence": "EXTRACTED",
                "_origin": "ast"
            }
        ],
        "edges": [
            {
                "source": "rationale-first",
                "target": "first",
                "relation": "rationale_for",
                "source_file": source_file,
                "source_location": "L3",
                "start_byte": 18,
                "end_byte": 101,
                "line_start": 3,
                "line_end": 3,
                "column_start": 4,
                "column_end": 87,
                "confidence": "EXTRACTED",
                "_origin": "ast"
            },
            {
                "source": "rationale-second",
                "target": "second",
                "relation": "rationale_for",
                "source_file": source_file,
                "source_location": "L6",
                "start_byte": 125,
                "end_byte": 208,
                "line_start": 6,
                "line_end": 6,
                "column_start": 4,
                "column_end": 87,
                "confidence": "EXTRACTED",
                "_origin": "ast"
            }
        ]
    }))?;
    let flexible = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;

    let rationales = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == NodeKind::Resource
                && matches!(
                    node.details,
                    Some(NodeDetails::Resource(ref details))
                        if details.resource_kind == ResourceKind::Rationale
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(rationales.len(), 2, "nodes={:#?}", graph.nodes);
    assert_ne!(rationales[0].id, rationales[1].id);
    assert_ne!(rationales[0].source, rationales[1].source);

    let documents = graph
        .links
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Documents)
        .collect::<Vec<_>>();
    assert_eq!(documents.len(), 2, "edges={:#?}", graph.links);
    assert!(
        rationales
            .iter()
            .all(|rationale| { documents.iter().any(|edge| edge.source == rationale.id) })
    );
    Ok(())
}
