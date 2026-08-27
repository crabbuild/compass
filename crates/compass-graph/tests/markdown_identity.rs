use std::collections::BTreeMap;
use std::error::Error;
use std::fs;

use compass_graph::{RawNodeRecord, build_from_extraction, normalize_document_v1};
use compass_languages::Engine;
use compass_model::code_graph::{NodeDetails, NodeKind, ResourceKind};
use serde_json::{Map, json};

#[test]
fn repeated_markdown_headings_use_stable_hierarchical_identities() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("docs/cookbook.md");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    let source = r#"# Cookbook
## Recipe 1
### Problem
First problem.
## Recipe 2
### Problem
Second problem.
"#;
    fs::write(&path, source)?;

    let identities = |path: &std::path::Path| -> Result<BTreeMap<String, String>, Box<dyn Error>> {
        let extraction = Engine::default().extract(path)?;
        let flexible = build_from_extraction(&extraction, true, Some(root));
        let graph = normalize_document_v1(&flexible, root, "sha256:test", None)?;
        let uris = graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Resource && node.name == "Problem")
            .map(|node| {
                let uri = match node.details.as_ref() {
                    Some(NodeDetails::Resource(resource)) => {
                        assert_eq!(resource.resource_kind, ResourceKind::Document);
                        resource.uri.clone()
                    }
                    _ => None,
                };
                (node.qualified_name.clone(), uri)
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            uris,
            BTreeMap::from([
                (
                    "Cookbook::Recipe 1::Problem".to_owned(),
                    Some("#problem".to_owned())
                ),
                (
                    "Cookbook::Recipe 2::Problem".to_owned(),
                    Some("#problem-1".to_owned())
                ),
            ])
        );
        Ok(graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Resource && node.name == "Problem")
            .map(|node| (node.qualified_name.clone(), node.id.clone()))
            .collect())
    };

    let before = identities(&path)?;
    assert_eq!(
        before.keys().map(String::as_str).collect::<Vec<_>>(),
        ["Cookbook::Recipe 1::Problem", "Cookbook::Recipe 2::Problem"]
    );

    fs::write(&path, format!("Introductory text.\n\n{source}"))?;
    let after = identities(&path)?;
    assert_eq!(after, before);
    Ok(())
}

#[test]
fn markdown_tables_keep_structural_nodes_and_stable_semantic_identities()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let path = root.join("docs/ownership.md");
    fs::create_dir_all(path.parent().ok_or("missing source parent")?)?;
    let source = r#"# Ownership
| Area | Owner | Status |
| :--- | :---: | ---: |
| Graph | `compass-model` | active |
| Empty | | values |
| Missing |
"#;
    fs::write(&path, source)?;

    let graph_for =
        |contents: &str| -> Result<compass_model::code_graph::GraphDocument, Box<dyn Error>> {
            fs::write(&path, contents)?;
            let extraction = Engine::default().extract(&path)?;
            let flexible = build_from_extraction(&extraction, true, Some(root));
            Ok(normalize_document_v1(&flexible, root, "sha256:test", None)?)
        };

    let graph = graph_for(source)?;
    assert_eq!(graph.graph.schema, "compass.graph/1");
    assert!(
        graph
            .nodes
            .iter()
            .all(|node| { !matches!(node.details.as_ref(), Some(NodeDetails::Document(_))) })
    );
    let table = graph
        .nodes
        .iter()
        .find(|node| {
            node.qualified_name
                .rsplit("::")
                .next()
                .is_some_and(|part| part.starts_with("pipe_table#"))
        })
        .ok_or("missing markdown table")?;
    assert!(table.name.contains("Area | Owner | Status"));
    let mut rows = graph
        .nodes
        .iter()
        .filter(|node| {
            node.qualified_name
                .rsplit("::")
                .next()
                .is_some_and(|part| part.starts_with("pipe_table_row#"))
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|node| node.source.as_ref().map(|source| source.start_byte));
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0].name,
        "Area=Graph · Owner=`compass-model` · Status=active"
    );
    assert_eq!(rows[1].name, "Area=Empty · Status=values");
    assert_eq!(rows[2].name, "Area=Missing");
    let cells = graph
        .nodes
        .iter()
        .filter(|node| node.qualified_name.contains("::pipe_table_cell#"))
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), 10);
    assert!(cells.iter().any(|node| node.name == "Owner: (empty)"));
    assert!(graph.links.iter().any(|edge| {
        edge.kind == compass_model::code_graph::EdgeKind::Contains
            && edge.source == table.id
            && rows.iter().any(|node| edge.target == node.id)
    }));

    let legacy = graph.to_legacy_document()?;
    let legacy_table = legacy
        .nodes
        .iter()
        .find(|node| node.id == table.id)
        .ok_or("legacy table projection lost table")?;
    assert_eq!(legacy_table.property("file_type"), Some(json!("document")));
    assert_eq!(legacy_table.document_role(), Some("pipe_table"));
    let legacy_row = legacy
        .nodes
        .iter()
        .find(|node| node.id == rows[0].id)
        .ok_or("legacy row projection lost document role")?;
    assert_eq!(legacy_row.document_role(), Some("pipe_table_row"));
    let round_tripped = compass_model::code_graph::GraphDocument::from_legacy_document(legacy)?;
    assert_eq!(round_tripped, graph);

    let before = rows
        .iter()
        .map(|node| (node.qualified_name.clone(), node.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let shifted = format!("Introductory prose.\n\n{source}").replace("active", "planned");
    let after_graph = graph_for(&shifted)?;
    let after = after_graph
        .nodes
        .iter()
        .filter(|node| {
            node.qualified_name
                .rsplit("::")
                .next()
                .is_some_and(|part| part.starts_with("pipe_table_row#"))
        })
        .map(|node| (node.qualified_name.clone(), node.id.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(after, before);
    Ok(())
}

#[test]
fn markdown_document_references_resolve_only_unique_exact_targets() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("docs"))?;
    fs::create_dir_all(root.join("src"))?;
    let markdown = root.join("docs/guide.md");
    let rust = root.join("src/lib.rs");
    fs::write(
        &markdown,
        "# Ownership\n\n| Name | Target |\n| --- | --- |\n| Widget | `Widget` |\n| Source | `../src/lib.rs` |\n| Missing | `NoSuchThing` |\n",
    )?;
    fs::write(&rust, "pub struct Widget;\n")?;

    let mut engine = Engine::default();
    let markdown_extraction = engine.extract(&markdown)?;
    let mut combined = markdown_extraction;
    combined.nodes.extend([
        RawNodeRecord {
            id: "raw-file".to_owned(),
            attributes: Map::from_iter([
                ("label".to_owned(), json!("lib.rs")),
                ("qualified_name".to_owned(), json!("src/lib.rs")),
                ("symbol_kind".to_owned(), json!("file")),
                ("file_type".to_owned(), json!("code")),
                ("language".to_owned(), json!("rust")),
                ("extractor".to_owned(), json!("test.rust")),
                ("source_file".to_owned(), json!("src/lib.rs")),
                (
                    "source_anchor".to_owned(),
                    json!({"file":"src/lib.rs","startByte":0,"endByte":19,"startLine":1,"startColumn":0,"endLine":2,"endColumn":0}),
                ),
            ]),
        },
        RawNodeRecord {
            id: "raw-widget".to_owned(),
            attributes: Map::from_iter([
                ("label".to_owned(), json!("Widget")),
                ("qualified_name".to_owned(), json!("crate::Widget")),
                ("symbol_kind".to_owned(), json!("struct")),
                ("file_type".to_owned(), json!("code")),
                ("language".to_owned(), json!("rust")),
                ("extractor".to_owned(), json!("test.rust")),
                ("source_file".to_owned(), json!("src/lib.rs")),
                (
                    "source_anchor".to_owned(),
                    json!({"file":"src/lib.rs","startByte":11,"endByte":17,"startLine":1,"startColumn":11,"endLine":1,"endColumn":17}),
                ),
            ]),
        },
    ]);
    let document = build_from_extraction(&combined, true, Some(root));
    let graph = normalize_document_v1(&document, root, "sha256:test", None)?;

    let widget_target = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Struct && node.name == "Widget")
        .ok_or("missing Widget target")?;
    let file_target = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::File && node.qualified_name == "src/lib.rs")
        .ok_or("missing file target")?;
    let widget_owner = graph
        .nodes
        .iter()
        .find(|node| node.name == "Target: `Widget`")
        .ok_or("missing Widget cell")?;
    let path_owner = graph
        .nodes
        .iter()
        .find(|node| node.name == "Target: `../src/lib.rs`")
        .ok_or("missing path cell")?;
    let missing_owner = graph
        .nodes
        .iter()
        .find(|node| node.name == "Target: `NoSuchThing`")
        .ok_or("missing unresolved cell")?;
    assert!(graph.links.iter().any(|edge| {
        edge.kind == compass_model::code_graph::EdgeKind::References
            && edge.source == widget_owner.id
            && edge.target == widget_target.id
            && edge.relationship_site.is_some()
    }));
    assert!(graph.links.iter().any(|edge| {
        edge.kind == compass_model::code_graph::EdgeKind::Documents
            && edge.source == path_owner.id
            && edge.target == file_target.id
            && edge.relationship_site.is_some()
    }));
    assert!(
        !graph
            .links
            .iter()
            .any(|edge| edge.source == missing_owner.id)
    );
    Ok(())
}

#[test]
fn markdown_document_reference_ambiguity_and_limits_never_guess() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("docs"))?;
    fs::create_dir_all(root.join("src"))?;
    let markdown = root.join("docs/guide.md");
    let source_path = root.join("src/lib.rs");
    fs::write(&markdown, "# Guide\n\nSee `Duplicate`.\n")?;
    fs::write(&source_path, "pub struct Duplicate;\n")?;

    let mut extraction = Engine::default().extract(&markdown)?;
    extraction.nodes.push(RawNodeRecord {
        id: "raw-file".to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), json!("lib.rs")),
            ("qualified_name".to_owned(), json!("src/lib.rs")),
            ("symbol_kind".to_owned(), json!("file")),
            ("file_type".to_owned(), json!("code")),
            ("language".to_owned(), json!("rust")),
            ("extractor".to_owned(), json!("test.rust")),
            ("source_file".to_owned(), json!("src/lib.rs")),
            (
                "source_anchor".to_owned(),
                json!({"file":"src/lib.rs","startByte":0,"endByte":22,"startLine":1,"startColumn":0,"endLine":2,"endColumn":0}),
            ),
        ]),
    });
    for index in 0..21 {
        extraction.nodes.push(RawNodeRecord {
            id: format!("raw-duplicate-{index}"),
            attributes: Map::from_iter([
                ("label".to_owned(), json!("Duplicate")),
                (
                    "qualified_name".to_owned(),
                    json!(format!("crate::Duplicate{index}")),
                ),
                ("symbol_kind".to_owned(), json!("struct")),
                ("file_type".to_owned(), json!("code")),
                ("language".to_owned(), json!("rust")),
                ("extractor".to_owned(), json!("test.rust")),
                ("source_file".to_owned(), json!("src/lib.rs")),
                (
                    "source_anchor".to_owned(),
                    json!({"file":"src/lib.rs","startByte":11,"endByte":20,"startLine":1,"startColumn":11,"endLine":1,"endColumn":20}),
                ),
            ]),
        });
    }
    let document = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&document, root, "sha256:test", None)?;
    let owner = graph
        .nodes
        .iter()
        .find(|node| {
            node.language.as_deref() == Some("markdown") && node.name.contains("Duplicate")
        })
        .ok_or("missing Duplicate owner")?;
    assert!(!graph.links.iter().any(|edge| edge.source == owner.id
        && edge.kind == compass_model::code_graph::EdgeKind::References));
    assert!(graph.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "document_reference_candidates_limited"
            && diagnostic.related_ids.contains(&owner.id)
            && diagnostic.anchor.is_some()
    }));
    Ok(())
}

#[test]
fn markdown_explicit_link_evidence_survives_absolute_path_aliases() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("docs"))?;
    fs::create_dir_all(root.join("src"))?;
    let markdown = root.join("docs/guide.md");
    let source = root.join("src/lib.rs");
    fs::write(&markdown, "# Guide\n\n[implementation](../src/lib.rs)\n")?;
    fs::write(&source, "pub struct Widget;\n")?;

    // Engine::extract uses absolute source paths, while the compatibility
    // assembler canonicalizes endpoints relative to `root`. The nested
    // document evidence must follow that same remap and remain exact.
    let mut engine = Engine::default();
    let markdown_extraction = engine.extract(&markdown)?;
    let mut extraction = markdown_extraction;
    extraction.nodes.push(RawNodeRecord {
        id: compass_languages::make_id(&[&source.to_string_lossy()]),
        attributes: Map::from_iter([
            ("label".to_owned(), json!("lib.rs")),
            ("qualified_name".to_owned(), json!(source.to_string_lossy())),
            ("symbol_kind".to_owned(), json!("file")),
            ("file_type".to_owned(), json!("code")),
            ("language".to_owned(), json!("rust")),
            ("extractor".to_owned(), json!("test.rust")),
            ("source_file".to_owned(), json!(source.to_string_lossy())),
            (
                "source_anchor".to_owned(),
                json!({"file": source.to_string_lossy(), "startByte": 0, "endByte": 19, "startLine": 1, "startColumn": 0, "endLine": 2, "endColumn": 0}),
            ),
        ]),
    });
    let document = build_from_extraction(&extraction, true, Some(root));
    let graph = normalize_document_v1(&document, root, "sha256:test", None)?;
    let owner = graph
        .nodes
        .iter()
        .find(|node| node.name.contains("implementation"))
        .ok_or("missing explicit link owner")?;
    let edge = graph
        .links
        .iter()
        .find(|edge| {
            edge.source == owner.id && edge.kind == compass_model::code_graph::EdgeKind::Documents
        })
        .ok_or("missing exact document edge")?;
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .map(|node| node.kind),
        Some(NodeKind::File)
    );
    assert!(edge.relationship_site.is_some());
    Ok(())
}
