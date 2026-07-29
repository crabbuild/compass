use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::Engine;
use compass_model::code_graph::EdgeKind;

#[test]
fn constructor_calls_normalize_to_instantiation_edges() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("src/service.py");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    fs::write(
        &path,
        r#"
class Service:
    pass

def build():
    return Service()
"#,
    )?;

    let extraction = Engine::default().extract(&path)?;
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
