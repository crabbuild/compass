use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, extraction_from_v1, normalize_document_v1};
use compass_languages::Engine;
use compass_model::code_graph::{EdgeKind, NodeKind};

type TestResult = Result<(), Box<dyn Error>>;

fn extract_and_publish(
    path: &std::path::Path,
    root: &std::path::Path,
    source: &str,
) -> Result<
    (
        compass_languages::Extraction,
        compass_model::code_graph::GraphDocument,
    ),
    Box<dyn Error>,
> {
    fs::write(path, source)?;
    let extraction = Engine::default().extract(path)?;
    let flexible = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;
    Ok((extraction, graph))
}

fn assert_no_publication_diagnostics(graph: &compass_model::code_graph::GraphDocument) {
    assert!(
        graph
            .graph
            .diagnostics
            .iter()
            .all(|diagnostic| { !diagnostic.code.starts_with("publication_") })
    );
}

#[test]
fn html_tables_with_equal_text_keep_every_occurrence() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("index.html");
    for source in [
        "<table><tr><th>Name</th><th>Status</th></tr></table>",
        "<table><tr><td>same</td></tr><tr><td>same</td></tr></table>",
        "<table><thead><tr><th>Name</th><th>Status</th></tr></thead><tbody></tbody></table>",
    ] {
        let (extraction, graph) = extract_and_publish(&path, root, source)?;
        assert_no_publication_diagnostics(&graph);
        assert_eq!(graph.nodes.len(), extraction.nodes.len());
        assert_eq!(graph.links.len(), extraction.edges.len());
        let mut ids = graph
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), extraction.nodes.len());
    }

    let (extraction, graph) = extract_and_publish(
        &path,
        root,
        "<table><tr><th>Name</th><th>Status</th></tr></table>",
    )?;
    let repeated = extraction_from_v1(&graph);
    let repeated = build_from_extraction(&repeated, true, Some(root));
    let repeated = normalize_document_v1(&repeated, root, "sha256:test", None)?;
    assert_eq!(
        repeated
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<Vec<_>>(),
        graph
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(extraction.nodes.len(), graph.nodes.len());
    Ok(())
}

#[test]
fn html_permalink_references_start_at_the_link_occurrence_and_skip_self_links() -> TestResult {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("index.html");
    let source = "<html><body><h2 id=\"title\">Title<a href=\"#title\">#</a><a href=\"#title\">#</a></h2><a id=\"self\" href=\"#self\">self</a></body></html>";
    let (_, graph) = extract_and_publish(&path, root, source)?;
    assert_no_publication_diagnostics(&graph);

    let heading = graph
        .nodes
        .iter()
        .find(|node| {
            node.source.as_ref().is_some_and(|anchor| {
                source
                    .find("<h2")
                    .is_some_and(|start| anchor.start_byte == start as u64)
            })
        })
        .ok_or("missing source-anchored heading")?;
    let link_starts = source
        .match_indices("<a href=")
        .map(|(start, _)| start as u64)
        .collect::<Vec<_>>();
    let links = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind == NodeKind::Resource
                && node
                    .source
                    .as_ref()
                    .is_some_and(|anchor| link_starts.contains(&anchor.start_byte))
        })
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(links.len(), 2);
    let references = graph
        .links
        .iter()
        .filter(|edge| edge.kind == EdgeKind::References)
        .collect::<Vec<_>>();
    assert_eq!(references.len(), 2);
    assert!(
        references
            .iter()
            .all(|edge| { edge.target == heading.id && links.contains(&edge.source.as_str()) })
    );
    assert_ne!(references[0].source, references[1].source);
    assert!(graph.links.iter().all(|edge| edge.source != edge.target));
    Ok(())
}
