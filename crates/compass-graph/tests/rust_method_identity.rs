use std::collections::BTreeSet;
use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::Engine;
use compass_model::code_graph::{EdgeKind, NodeKind};

#[test]
fn rust_methods_in_distinct_impls_publish_distinct_stable_nodes() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("src/changes.rs");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    fs::write(
        &path,
        r#"
trait ChangeSink {
    fn change(&mut self);
}
struct ExactDiffWriter;
struct ChangeCounts;
impl ChangeSink for ExactDiffWriter {
    fn change(&mut self) {}
}
impl ChangeSink for ChangeCounts {
    fn change(&mut self) {}
}
"#,
    )?;

    let extraction = Engine::default().extract(&path)?;
    let flexible = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;
    let methods = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Method && node.name == ".change()")
        .collect::<Vec<_>>();

    assert_eq!(methods.len(), 2, "nodes={:?}", graph.nodes);
    assert_eq!(
        methods
            .iter()
            .map(|node| node.qualified_name.as_str())
            .collect::<BTreeSet<_>>(),
        [
            "ChangeCounts as ChangeSink::change(_)@200",
            "ExactDiffWriter as ChangeSink::change(_)@135",
        ]
        .into_iter()
        .collect()
    );
    assert_ne!(methods[0].id, methods[1].id);
    for method in &methods {
        let source = method.source.as_ref().ok_or("method source is missing")?;
        let ownership = graph
            .links
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Contains && edge.target == method.id)
            .collect::<Vec<_>>();
        assert_eq!(ownership.len(), 1, "method={method:#?}");
        assert_eq!(ownership[0].relationship_site.as_ref(), Some(source));
    }
    let stable_ids = methods
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let repeated = Engine::default().extract(&path)?;
    let repeated = build_from_extraction(&repeated, true, Some(root));
    let repeated = normalize_document_v1(&repeated, root, "sha256:test", None)?;
    assert_eq!(
        repeated
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Method && node.name == ".change()")
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>(),
        stable_ids
    );
    Ok(())
}

#[test]
fn generic_methods_in_distinct_classes_publish_distinct_stable_nodes() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("assets/bundle.js");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    fs::write(
        &path,
        r#"
class First {
    constructor(e, t) { this.value = e + t; }
}
class Second {
    constructor(e, t) { this.value = e * t; }
}
"#,
    )?;

    let extraction = Engine::default().extract(&path)?;
    let flexible = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;
    let methods = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Method && node.name == ".constructor()")
        .collect::<Vec<_>>();

    assert_eq!(methods.len(), 2, "nodes={:?}", graph.nodes);
    assert_eq!(
        methods
            .iter()
            .map(|node| node.qualified_name.as_str())
            .collect::<BTreeSet<_>>(),
        ["First::constructor", "Second::constructor"]
            .into_iter()
            .collect()
    );
    assert_ne!(methods[0].id, methods[1].id);
    Ok(())
}

#[test]
fn php_methods_in_distinct_classes_publish_distinct_stable_nodes() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("routes/controllers.php");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    fs::write(
        &path,
        r#"<?php
class FirstController {
    public function index() {}
}
class SecondController {
    public function index() {}
}
"#,
    )?;

    let extraction = Engine::default().extract(&path)?;
    let flexible = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;
    let methods = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Method && node.name == ".index()")
        .collect::<Vec<_>>();

    assert_eq!(methods.len(), 2, "nodes={:?}", graph.nodes);
    assert_eq!(
        methods
            .iter()
            .map(|node| node.qualified_name.as_str())
            .collect::<BTreeSet<_>>(),
        ["FirstController::index", "SecondController::index"]
            .into_iter()
            .collect()
    );
    assert_ne!(methods[0].id, methods[1].id);
    Ok(())
}
