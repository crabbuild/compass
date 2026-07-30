use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::Engine;
use compass_model::code_graph::{EdgeKind, NodeDetails, NodeKind, ResourceKind};

#[test]
fn repeated_python_rationales_at_distinct_sites_keep_distinct_v1_identities()
-> Result<(), Box<dyn Error>> {
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

    let extraction = Engine::default().extract(&path)?;
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
