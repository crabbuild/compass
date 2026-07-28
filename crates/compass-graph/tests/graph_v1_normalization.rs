use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{Extraction, RawEdgeRecord, RawNodeRecord};
use compass_model::code_graph::{BuildMetadata, EdgeKind, ExtractionStatus, FileRecord, NodeKind};
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
    Ok(())
}
