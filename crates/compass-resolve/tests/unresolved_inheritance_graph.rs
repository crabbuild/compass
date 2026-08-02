use std::collections::HashMap;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::Engine;
use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::EvidenceOrigin;
use compass_resolve::resolve_with_root;

#[test]
fn unresolved_java_inheritance_is_typed_deferred_and_not_exact_topology()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("src/Child.java");
    let source = b"package a;\nclass Child extends Missing {}\n";
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join(relative), source)?;

    let extracted = Engine::default().extract_source(relative, source)?;
    let sources = HashMap::from([(
        relative.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let extraction = resolve_with_root(&[extracted], &sources, root);
    assert!(extraction.error.is_none(), "{:#?}", extraction.error);
    assert!(!extraction.nodes.is_empty(), "resolved Java graph is empty");
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test-java")?;
    let graph = normalize_v1(extraction, evidence)?;
    let inheritance = graph
        .links
        .iter()
        .find(|edge| edge.kind == EdgeKind::Extends)
        .ok_or_else(|| format!("missing deferred inheritance edge: {graph:#?}"))?;
    assert!(inheritance.deferred);
    assert!(inheritance.evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Heuristic
            && evidence.extractor == "compass.graph.external-placeholder"
            && evidence.wiring_site.is_some()
    }));
    assert!(
        graph
            .links
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Extends | EdgeKind::Implements))
            .all(|edge| edge.deferred)
    );
    let unresolved = graph
        .nodes
        .iter()
        .find(|node| node.name == "Missing")
        .ok_or("missing unresolved superclass")?;
    assert_eq!(unresolved.kind, NodeKind::Class);
    assert!(unresolved.evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Heuristic
            && evidence.extractor == "compass.graph.external-placeholder"
            && evidence.wiring_site.is_some()
    }));
    assert!(unresolved.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unresolved_external_symbol" && diagnostic.anchor.is_some()
    }));
    Ok(())
}
