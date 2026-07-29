use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, InventoryEvidence, extraction_from_v1, normalize_v1};
use compass_languages::{Extraction, RawEdgeRecord, RawNodeRecord};
use compass_model::code_graph::{
    BuildMetadata, CoverageRecord, CoverageStatus, DiagnosticSeverity, EdgeKind, ExtractionStatus,
    FileRecord, GraphDiagnostic, NodeKind,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

fn build_evidence(root: &Path) -> Result<BuildEvidence, Box<dyn std::error::Error>> {
    let bytes = vec![b'x'; 500];
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), &bytes)?;
    let mut evidence = BuildEvidence::new(
        root,
        BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        },
    );
    evidence.files.push(FileRecord {
        id: "raw".to_owned(),
        path: root.join("src/lib.rs").to_string_lossy().into_owned(),
        language: Some("rust".to_owned()),
        content_digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
        byte_size: bytes.len() as u64,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    Ok(evidence)
}

fn anchor(root: &Path, start: u64) -> Value {
    json!({
        "file": root.join("src/lib.rs"),
        "startByte": start,
        "endByte": start + 4,
        "startLine": start / 10 + 1,
        "startColumn": 0,
        "endLine": start / 10 + 1,
        "endColumn": 4
    })
}

fn raw_node(root: &Path, id: &str, name: &str, start: u64) -> RawNodeRecord {
    RawNodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), json!(name)),
            ("qualified_name".to_owned(), json!(format!("crate::{name}"))),
            ("symbol_kind".to_owned(), json!("function")),
            ("file_type".to_owned(), json!("code")),
            ("language".to_owned(), json!("rust")),
            ("extractor".to_owned(), json!("test.rust")),
            ("source_anchor".to_owned(), anchor(root, start)),
        ]),
    }
}

fn extraction(root: &Path) -> Extraction {
    Extraction {
        nodes: vec![
            raw_node(root, "raw:a", "caller", 10),
            raw_node(root, "raw:b", "callee", 30),
        ],
        edges: vec![RawEdgeRecord {
            source: "raw:a".to_owned(),
            target: "raw:b".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("indirect_call")),
                ("confidence".to_owned(), json!("INFERRED")),
                ("extractor".to_owned(), json!("test.rust")),
                ("source_anchor".to_owned(), anchor(root, 50)),
            ]),
        }],
        ..Extraction::default()
    }
}

#[test]
fn normalization_is_root_portable_order_independent_and_auditable()
-> Result<(), Box<dyn std::error::Error>> {
    let left_directory = tempfile::tempdir()?;
    let right_directory = tempfile::tempdir()?;
    let left_root = left_directory.path();
    let right_root = right_directory.path();
    let left = normalize_v1(extraction(left_root), build_evidence(left_root)?)?;
    let mut reordered = extraction(right_root);
    reordered.nodes.reverse();
    let right = normalize_v1(reordered, build_evidence(right_root)?)?;

    assert_eq!(left, right);
    assert_eq!(left.nodes[0].kind, NodeKind::Function);
    assert_eq!(left.links[0].kind, EdgeKind::Calls);
    assert_eq!(left.links[0].id, left.links[0].key);
    assert_eq!(
        left.links[0].evidence[0].rule.as_deref(),
        Some("indirect-call-resolution")
    );
    assert!(left.links[0].evidence[0].wiring_site.is_some());
    Ok(())
}

#[test]
fn symbol_identity_survives_leading_source_insertions() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut before = extraction(root);
    let mut after = extraction(root);
    for graph in [&mut before, &mut after] {
        graph.nodes[0].attributes.insert(
            "signature_hash".to_owned(),
            json!("sha256:caller-signature"),
        );
        graph.nodes[0]
            .attributes
            .insert("lexical_owner".to_owned(), json!("crate"));
    }
    after.nodes[0]
        .attributes
        .insert("source_anchor".to_owned(), anchor(root, 110));

    let before = normalize_v1(before, build_evidence(root)?)?;
    let after = normalize_v1(after, build_evidence(root)?)?;
    let before_id = &before
        .nodes
        .iter()
        .find(|node| node.name == "caller")
        .ok_or("missing caller")?
        .id;
    let after_id = &after
        .nodes
        .iter()
        .find(|node| node.name == "caller")
        .ok_or("missing caller")?
        .id;
    assert_eq!(before_id, after_id);
    Ok(())
}

#[test]
fn normalization_rejects_unknown_aliases_and_missing_wiring_sites() {
    let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let root = directory.path();
    let mut unknown = extraction(root);
    unknown.edges[0]
        .attributes
        .insert("relation".to_owned(), json!("approximately_calls"));
    let evidence = build_evidence(root).unwrap_or_else(|_| std::process::abort());
    assert!(normalize_v1(unknown, evidence).is_err());

    let mut missing_site = extraction(root);
    missing_site.edges[0].attributes.remove("source_anchor");
    let evidence = build_evidence(root).unwrap_or_else(|_| std::process::abort());
    assert!(normalize_v1(missing_site, evidence).is_err());
}

#[test]
fn normalization_maps_declared_raw_aliases_without_publishing_them()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut aliased = extraction(root);
    aliased.edges[0]
        .attributes
        .insert("relation".to_owned(), json!("imports_from"));
    aliased.edges[0]
        .attributes
        .insert("confidence".to_owned(), json!("EXTRACTED"));
    let document = normalize_v1(aliased, build_evidence(root)?)?;
    assert_eq!(document.links[0].kind, EdgeKind::Imports);
    let serialized = serde_json::to_string(&document)?;
    assert!(!serialized.contains("\"kind\":\"imports_from\""));
    assert!(!serialized.contains("\"relation\""));
    assert!(serialized.contains("raw-relation:imports_from"));
    Ok(())
}

#[test]
fn normalization_treats_blank_external_source_paths_as_unanchored()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut graph = extraction(root);
    graph.nodes[1].attributes.remove("source_anchor");
    graph.nodes[1]
        .attributes
        .insert("source_file".to_owned(), json!(""));
    graph.nodes[1]
        .attributes
        .insert("origin_file".to_owned(), json!("src/lib.rs"));
    graph.nodes[1].attributes.remove("symbol_kind");
    graph.edges.clear();

    let document = normalize_v1(graph, build_evidence(root)?)?;
    let external = document
        .nodes
        .iter()
        .find(|node| node.name == "callee")
        .ok_or("missing external node")?;
    assert_eq!(external.source, None);
    assert_eq!(external.kind, NodeKind::Variable);
    assert_eq!(
        external.evidence[0].rule.as_deref(),
        Some("external-symbol-placeholder")
    );
    assert!(external.evidence[0].wiring_site.is_some());
    Ok(())
}

#[test]
fn build_evidence_derives_digests_generation_and_byte_anchors()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), b"fn a() {}\nfn b() {}\n")?;
    let attributes = |name: &str, line: u64| {
        Map::from_iter([
            ("label".to_owned(), json!(name)),
            ("qualified_name".to_owned(), json!(format!("crate::{name}"))),
            ("symbol_kind".to_owned(), json!("function")),
            ("file_type".to_owned(), json!("code")),
            ("language".to_owned(), json!("rust")),
            ("source_file".to_owned(), json!("src/lib.rs")),
            ("line_start".to_owned(), json!(line)),
            ("line_end".to_owned(), json!(line)),
            ("_origin".to_owned(), json!("ast")),
        ])
    };
    let extraction = Extraction {
        nodes: vec![
            RawNodeRecord {
                id: "raw:a".to_owned(),
                attributes: attributes("a", 1),
            },
            RawNodeRecord {
                id: "raw:b".to_owned(),
                attributes: attributes("b", 2),
            },
        ],
        edges: vec![RawEdgeRecord {
            source: "raw:a".to_owned(),
            target: "raw:b".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("calls")),
                ("source_file".to_owned(), json!("src/lib.rs")),
                ("line_start".to_owned(), json!(1)),
                ("line_end".to_owned(), json!(1)),
                ("_origin".to_owned(), json!("ast")),
            ]),
        }],
        ..Extraction::default()
    };
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:config")?;
    let graph = normalize_v1(extraction, evidence)?;

    assert_eq!(graph.graph.files.len(), 1);
    assert!(graph.graph.files[0].content_digest.starts_with("sha256:"));
    assert!(graph.graph.build.generation_id.starts_with("sha256:"));
    let a = graph.nodes.iter().find(|node| node.name == "a");
    let b = graph.nodes.iter().find(|node| node.name == "b");
    assert_eq!(
        a.and_then(|node| node.source.as_ref())
            .map(|anchor| anchor.start_byte),
        Some(0)
    );
    assert_eq!(
        b.and_then(|node| node.source.as_ref())
            .map(|anchor| anchor.start_byte),
        Some(10)
    );
    assert_eq!(
        graph.links[0]
            .relationship_site
            .as_ref()
            .map(|anchor| anchor.end_byte),
        Some(10)
    );
    let projected = extraction_from_v1(&graph);
    let rebuilt_evidence = BuildEvidence::from_extraction(root, &projected, "sha256:config")?;
    let rebuilt = normalize_v1(projected, rebuilt_evidence)?;
    assert_eq!(rebuilt.nodes, graph.nodes);
    assert_eq!(rebuilt.links, graph.links);
    Ok(())
}

#[test]
fn typed_incremental_projection_preserves_all_trusted_fields()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut graph = normalize_v1(extraction(root), build_evidence(root)?)?;
    let mut second_evidence = graph.nodes[0].evidence[0].clone();
    second_evidence.extractor = "test.second".to_owned();
    graph.nodes[0].evidence.push(second_evidence);
    let node_anchor = graph.nodes[0].source.clone();
    let node_id = graph.nodes[0].id.clone();
    let edge_anchor = graph.links[0].relationship_site.clone();
    let edge_id = graph.links[0].id.clone();
    graph.nodes[0].coverage.push(CoverageRecord {
        capability: "node:function".to_owned(),
        producer: "test.second".to_owned(),
        status: CoverageStatus::Partial,
        file_id: graph.graph.files.first().map(|file| file.id.clone()),
        reason: Some("fixture partial".to_owned()),
        anchor: node_anchor.clone(),
    });
    graph.nodes[0].diagnostics.push(GraphDiagnostic {
        severity: DiagnosticSeverity::Warning,
        code: "fixture_node_warning".to_owned(),
        message: "node warning".to_owned(),
        anchor: node_anchor,
        related_ids: vec![node_id],
    });
    graph.links[0].diagnostics.push(GraphDiagnostic {
        severity: DiagnosticSeverity::Warning,
        code: "fixture_edge_warning".to_owned(),
        message: "edge warning".to_owned(),
        anchor: edge_anchor,
        related_ids: vec![edge_id],
    });
    graph
        .graph
        .coverage
        .push(graph.nodes[0].coverage[0].clone());
    graph
        .graph
        .diagnostics
        .push(graph.nodes[0].diagnostics[0].clone());

    let projected = extraction_from_v1(&graph);
    let rebuilt = normalize_v1(projected, build_evidence(root)?)?;
    assert_eq!(rebuilt.nodes, graph.nodes);
    assert_eq!(rebuilt.links, graph.links);
    assert_eq!(rebuilt.graph.coverage, graph.graph.coverage);
    assert_eq!(rebuilt.graph.diagnostics, graph.graph.diagnostics);
    Ok(())
}

#[test]
fn inventory_includes_detected_files_that_produced_no_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    for name in ["unsupported.xyz", "partial.rs", "generated.rs", "broken.rs"] {
        fs::write(root.join(name), "opaque")?;
    }
    let extraction = Extraction::default();
    let mut evidence = BuildEvidence::from_extraction(root, &extraction, "config")?;
    evidence.include_inventory([
        InventoryEvidence {
            path: root.join("unsupported.xyz"),
            status: ExtractionStatus::Unsupported,
            reason: Some("no extractor".to_owned()),
        },
        InventoryEvidence {
            path: root.join("partial.rs"),
            status: ExtractionStatus::Partial,
            reason: Some("partial semantic extraction".to_owned()),
        },
        InventoryEvidence {
            path: root.join("generated.rs"),
            status: ExtractionStatus::Generated,
            reason: None,
        },
        InventoryEvidence {
            path: root.join("broken.rs"),
            status: ExtractionStatus::ParseFailure,
            reason: Some("parser failed".to_owned()),
        },
    ])?;
    let document = normalize_v1(extraction, evidence)?;
    assert_eq!(document.graph.files.len(), 4);
    for (status, coverage_status) in [
        (ExtractionStatus::Unsupported, CoverageStatus::Unsupported),
        (ExtractionStatus::Partial, CoverageStatus::Partial),
        (ExtractionStatus::Generated, CoverageStatus::Indeterminate),
        (ExtractionStatus::ParseFailure, CoverageStatus::Failed),
    ] {
        let file = document
            .graph
            .files
            .iter()
            .find(|file| file.extraction_status == status)
            .ok_or("missing inventory status")?;
        assert!(document.graph.coverage.iter().any(|coverage| {
            coverage.file_id.as_deref() == Some(file.id.as_str())
                && coverage.status == coverage_status
        }));
    }
    Ok(())
}
