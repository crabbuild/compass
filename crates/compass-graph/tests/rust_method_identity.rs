use std::collections::BTreeSet;
use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::Engine;
use compass_model::code_graph::NodeKind;

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
            "ChangeCounts as ChangeSink::change",
            "ExactDiffWriter as ChangeSink::change",
        ]
        .into_iter()
        .collect()
    );
    assert_ne!(methods[0].id, methods[1].id);
    Ok(())
}
