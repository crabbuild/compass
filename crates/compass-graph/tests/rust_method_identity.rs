use std::collections::BTreeSet;
use std::error::Error;
use std::fs;

use compass_graph::{build_from_extraction, normalize_document_v1};
use compass_languages::{Engine, Extraction};
use compass_model::code_graph::{EdgeKind, NodeKind};
use serde_json::json;

#[test]
fn rust_methods_in_distinct_impls_publish_distinct_stable_nodes() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("src/changes.rs");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    let source = r#"
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
"#;
    fs::write(&path, source)?;

    let source_file = path.to_string_lossy();
    let exact_type_start = source.find("ExactDiffWriter").ok_or("exact type")?;
    let counts_type_start = source.find("ChangeCounts").ok_or("counts type")?;
    let method_starts = source
        .match_indices("fn change")
        .map(|(start, _)| start + 3)
        .collect::<Vec<_>>();
    let exact_method_start = *method_starts.get(1).ok_or("exact method")?;
    let counts_method_start = *method_starts.get(2).ok_or("counts method")?;
    let line = |byte: usize| {
        source.as_bytes()[..byte]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
            + 1
    };
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes": [
            {
                "id": "exact",
                "label": "ExactDiffWriter",
                "qualified_name": "crate::ExactDiffWriter",
                "symbol_kind": "struct",
                "file_type": "code",
                "source_file": source_file,
                "source_location": format!("L{}", line(exact_type_start)),
                "start_byte": exact_type_start,
                "end_byte": exact_type_start + "ExactDiffWriter".len(),
                "line_start": line(exact_type_start),
                "line_end": line(exact_type_start),
                "column_start": 7,
                "column_end": 22,
                "language": "rust",
                "confidence": "EXTRACTED",
                "_origin": "ast"
            },
            {
                "id": "counts",
                "label": "ChangeCounts",
                "qualified_name": "crate::ChangeCounts",
                "symbol_kind": "struct",
                "file_type": "code",
                "source_file": source_file,
                "source_location": format!("L{}", line(counts_type_start)),
                "start_byte": counts_type_start,
                "end_byte": counts_type_start + "ChangeCounts".len(),
                "line_start": line(counts_type_start),
                "line_end": line(counts_type_start),
                "column_start": 7,
                "column_end": 19,
                "language": "rust",
                "confidence": "EXTRACTED",
                "_origin": "ast"
            },
            {
                "id": "exact-change",
                "label": ".change()",
                "qualified_name": "<crate::ExactDiffWriter as crate::ChangeSink>::change",
                "symbol_kind": "method",
                "file_type": "code",
                "source_file": source_file,
                "source_location": format!("L{}", line(exact_method_start)),
                "start_byte": exact_method_start,
                "end_byte": exact_method_start + "change".len(),
                "line_start": line(exact_method_start),
                "line_end": line(exact_method_start),
                "column_start": 7,
                "column_end": 13,
                "language": "rust",
                "confidence": "EXTRACTED",
                "_origin": "ast"
            },
            {
                "id": "counts-change",
                "label": ".change()",
                "qualified_name": "<crate::ChangeCounts as crate::ChangeSink>::change",
                "symbol_kind": "method",
                "file_type": "code",
                "source_file": source_file,
                "source_location": format!("L{}", line(counts_method_start)),
                "start_byte": counts_method_start,
                "end_byte": counts_method_start + "change".len(),
                "line_start": line(counts_method_start),
                "line_end": line(counts_method_start),
                "column_start": 7,
                "column_end": 13,
                "language": "rust",
                "confidence": "EXTRACTED",
                "_origin": "ast"
            }
        ],
        "edges": [
            {
                "source": "exact",
                "target": "exact-change",
                "relation": "contains",
                "source_file": source_file,
                "source_location": format!("L{}", line(exact_method_start)),
                "start_byte": exact_method_start,
                "end_byte": exact_method_start + "change".len(),
                "line_start": line(exact_method_start),
                "line_end": line(exact_method_start),
                "column_start": 7,
                "column_end": 13,
                "confidence": "EXTRACTED",
                "_origin": "ast"
            },
            {
                "source": "counts",
                "target": "counts-change",
                "relation": "contains",
                "source_file": source_file,
                "source_location": format!("L{}", line(counts_method_start)),
                "start_byte": counts_method_start,
                "end_byte": counts_method_start + "change".len(),
                "line_start": line(counts_method_start),
                "line_end": line(counts_method_start),
                "column_start": 7,
                "column_end": 13,
                "confidence": "EXTRACTED",
                "_origin": "ast"
            }
        ]
    }))?;
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
            "<crate::ChangeCounts as crate::ChangeSink>::change",
            "<crate::ExactDiffWriter as crate::ChangeSink>::change",
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
    let repeated = build_from_extraction(&extraction, true, Some(root));
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
fn generic_constructors_in_distinct_classes_publish_distinct_stable_nodes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("assets/bundle.js");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    let source = r#"
class First {
    constructor(e, t) { this.value = e + t; }
}
class Second {
    constructor(e, t) { this.value = e * t; }
}
"#;
    fs::write(&path, source)?;

    let source_file = path.to_string_lossy();
    let constructor_starts = source
        .match_indices("constructor")
        .map(|(start, _)| start)
        .collect::<Vec<_>>();
    let first_start = *constructor_starts.first().ok_or("first constructor")?;
    let second_start = *constructor_starts.get(1).ok_or("second constructor")?;
    let line = |byte: usize| {
        source.as_bytes()[..byte]
            .iter()
            .filter(|value| **value == b'\n')
            .count()
            + 1
    };
    let constructor = |id: &str, declaring_type: &str, start: usize| {
        json!({
            "id": id,
            "label": ".constructor()",
            "qualified_name": format!("bundle.{declaring_type}.constructor"),
            "declaring_type": declaring_type,
            "symbol_kind": "constructor",
            "file_type": "code",
            "source_file": source_file,
            "source_location": format!("L{}", line(start)),
            "start_byte": start,
            "end_byte": start + "constructor".len(),
            "line_start": line(start),
            "line_end": line(start),
            "column_start": 4,
            "column_end": 15,
            "language": "javascript",
            "extractor": "compass.languages.javascript",
            "confidence": "EXTRACTED",
            "_origin": "ast"
        })
    };
    let extraction: Extraction = serde_json::from_value(json!({
        "nodes": [
            constructor("first-constructor", "First", first_start),
            constructor("second-constructor", "Second", second_start)
        ],
        "edges": []
    }))?;
    let flexible = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;
    let methods = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Constructor && node.name == ".constructor()")
        .collect::<Vec<_>>();

    assert_eq!(methods.len(), 2, "nodes={:?}", graph.nodes);
    assert_eq!(
        methods
            .iter()
            .map(|node| node.qualified_name.as_str())
            .collect::<BTreeSet<_>>(),
        ["bundle.First.constructor", "bundle.Second.constructor"]
            .into_iter()
            .collect()
    );
    assert_ne!(methods[0].id, methods[1].id);
    assert!(methods.iter().all(|method| {
        method
            .source
            .as_ref()
            .is_some_and(|source| source.start_byte < source.end_byte)
    }));
    let stable_ids = methods
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let repeated = build_from_extraction(&extraction, true, Some(root));
    let repeated = normalize_document_v1(&repeated, root, "sha256:test", None)?;
    assert_eq!(
        repeated
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Constructor)
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>(),
        stable_ids
    );
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
