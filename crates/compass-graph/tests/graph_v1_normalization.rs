use std::path::Path;
use std::{collections::BTreeMap, fs};

use compass_graph::{
    BuildEvidence, InferenceLevel, InventoryEvidence, SourceDigest, apply_inference_level,
    build_from_extraction, build_owned_with_tiebreaker, build_owned_with_tiebreaker_at_inference,
    extraction_from_v1, normalize_document_v1,
    normalize_document_v1_with_evidence_best_effort_owned_at_inference,
    normalize_document_v1_with_inventory_best_effort_owned, normalize_v1, normalize_v1_best_effort,
    normalize_v1_best_effort_with_inference,
};
use compass_languages::{Extraction, RawEdgeRecord, RawNodeRecord};
use compass_model::code_graph::{
    BuildMetadata, CoverageRecord, CoverageStatus, DiagnosticSeverity, EdgeKind, ExtractionStatus,
    FileRecord, GraphDiagnostic, NodeKind,
};
use compass_model::identity::edge_id;
use compass_model::provenance::{
    EndpointRewriteEvidence, EndpointRewriteRule, EvidenceConfidence, EvidenceOrigin,
    NODE_PROVENANCE_ANCHOR_ATTRIBUTE, SEMANTIC_LAYER_EXTRACTOR, TRUSTED_EDGE_RECORD_ATTRIBUTE,
    TRUSTED_NODE_RECORD_ATTRIBUTE, append_endpoint_rewrite_evidence,
};
use compass_model::validate_code_graph;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const CLOSED_ENDPOINT_REWRITE_RULES: [&str; 12] = [
    "csharp-namespace-canonicalization",
    "language-family-stub-resolution",
    "php-qualified-type-resolution",
    "canonical-import-target",
    "unique-stub-endpoint-resolution",
    "source-scoped-node-disambiguation",
    "header-import-disambiguation",
    "graph-semantic-id-remap",
    "graph-document-twin-remap",
    "graph-ghost-endpoint-remap",
    "graph-normalized-id-remap",
    "incremental-ast-endpoint-remap",
];

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
    anchor_in(root, "src/lib.rs", start)
}

fn anchor_in(root: &Path, relative: &str, start: u64) -> Value {
    json!({
        "file": root.join(relative),
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

fn raw_file_node(root: &Path, id: &str, relative: &str) -> RawNodeRecord {
    RawNodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), json!(relative)),
            ("qualified_name".to_owned(), json!(relative)),
            ("symbol_kind".to_owned(), json!("file")),
            ("file_type".to_owned(), json!("code")),
            ("language".to_owned(), json!("php")),
            ("extractor".to_owned(), json!("test.php")),
            ("source_anchor".to_owned(), anchor_in(root, relative, 0)),
        ]),
    }
}

#[test]
fn node_navigation_extent_preserves_and_contains_exact_provenance()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let definition = json!({
        "file": root.join("src/lib.rs"),
        "startByte": 10,
        "endByte": 50,
        "startLine": 2,
        "startColumn": 0,
        "endLine": 5,
        "endColumn": 1
    });
    let identifier = json!({
        "file": root.join("src/lib.rs"),
        "startByte": 20,
        "endByte": 23,
        "startLine": 3,
        "startColumn": 4,
        "endLine": 3,
        "endColumn": 7
    });
    let mut method = raw_node(root, "method", "run", 10);
    method
        .attributes
        .insert("source_anchor".to_owned(), definition.clone());
    method.attributes.insert(
        NODE_PROVENANCE_ANCHOR_ATTRIBUTE.to_owned(),
        identifier.clone(),
    );
    let graph = normalize_v1(
        Extraction {
            nodes: vec![method],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;
    let mut published_definition = definition.clone();
    published_definition["file"] = json!("src/lib.rs");
    let mut published_identifier = identifier.clone();
    published_identifier["file"] = json!("src/lib.rs");
    assert_eq!(
        serde_json::to_value(graph.nodes[0].source.as_ref())?,
        published_definition
    );
    assert_eq!(
        serde_json::to_value(graph.nodes[0].evidence[0].anchors.first())?,
        published_identifier
    );

    let mut invalid = raw_node(root, "invalid", "invalid", 10);
    invalid
        .attributes
        .insert("source_anchor".to_owned(), definition);
    invalid.attributes.insert(
        NODE_PROVENANCE_ANCHOR_ATTRIBUTE.to_owned(),
        json!({
            "file": root.join("src/lib.rs"),
            "startByte": 49,
            "endByte": 55,
            "startLine": 5,
            "startColumn": 0,
            "endLine": 5,
            "endColumn": 6
        }),
    );
    let invalid_result = normalize_v1(
        Extraction {
            nodes: vec![invalid],
            ..Extraction::default()
        },
        build_evidence(root)?,
    );
    let error = match invalid_result {
        Ok(_) => return Err("out-of-range provenance unexpectedly normalized".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("not contained"));
    Ok(())
}

#[test]
fn repeated_document_blocks_use_occurrence_stable_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut first = RawNodeRecord {
        id: "raw:first".to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), json!("same block")),
            ("qualified_name".to_owned(), json!("same block")),
            ("symbol_kind".to_owned(), json!("markdown_block")),
            ("file_type".to_owned(), json!("document")),
            ("document_kind".to_owned(), json!("paragraph")),
            ("block_index".to_owned(), json!(0)),
            ("language".to_owned(), json!("markdown")),
            ("extractor".to_owned(), json!("compass.markdown")),
            ("source_file".to_owned(), json!("src/lib.rs")),
            ("source_anchor".to_owned(), anchor(root, 10)),
        ]),
    };
    let mut second = first.clone();
    first.id = "raw:first".to_owned();
    second.id = "raw:second".to_owned();
    second.attributes.insert("block_index".to_owned(), json!(1));
    second
        .attributes
        .insert("source_anchor".to_owned(), anchor(root, 30));
    let outcome = normalize_v1_best_effort(
        Extraction {
            nodes: vec![first, second],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;
    assert_eq!(outcome.document.nodes.len(), 2);
    assert_eq!(outcome.omissions.identity_collisions, 0);
    Ok(())
}

#[test]
fn markdown_heading_identity_uses_hierarchy_and_survives_source_movement()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let heading = |start| RawNodeRecord {
        id: format!("raw:heading:{start}"),
        attributes: Map::from_iter([
            ("label".to_owned(), json!("Problem")),
            (
                "qualified_name".to_owned(),
                json!("Cookbook::Recipe 1::Problem"),
            ),
            ("symbol_kind".to_owned(), json!("markdown_block")),
            ("file_type".to_owned(), json!("document")),
            ("document_kind".to_owned(), json!("heading")),
            ("heading_style".to_owned(), json!("atx")),
            ("anchor_slug".to_owned(), json!("problem")),
            ("language".to_owned(), json!("markdown")),
            ("extractor".to_owned(), json!("compass.markdown")),
            ("source_file".to_owned(), json!("src/lib.rs")),
            ("source_anchor".to_owned(), anchor(root, start)),
        ]),
    };

    let before = normalize_v1(
        Extraction {
            nodes: vec![heading(10)],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;
    let after = normalize_v1(
        Extraction {
            nodes: vec![heading(30)],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;

    assert_eq!(before.nodes[0].id, after.nodes[0].id);
    assert_ne!(before.nodes[0].source, after.nodes[0].source);
    let round_trip = normalize_v1(extraction_from_v1(&after), build_evidence(root)?)?;
    assert_eq!(round_trip.nodes[0].id, after.nodes[0].id);
    assert_eq!(
        round_trip.nodes[0]
            .details
            .as_ref()
            .and_then(|details| match details {
                compass_model::code_graph::NodeDetails::Resource(resource) => {
                    resource.uri.as_deref()
                }
                _ => None,
            }),
        Some("#problem")
    );
    Ok(())
}

#[test]
fn trusted_document_blocks_repair_legacy_global_identity() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut first = RawNodeRecord {
        id: "raw:first".to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), json!("same block")),
            ("qualified_name".to_owned(), json!("same block")),
            ("symbol_kind".to_owned(), json!("markdown_block")),
            ("file_type".to_owned(), json!("document")),
            ("document_kind".to_owned(), json!("paragraph")),
            ("language".to_owned(), json!("markdown")),
            ("extractor".to_owned(), json!("compass.markdown")),
            ("source_file".to_owned(), json!("src/lib.rs")),
            ("source_anchor".to_owned(), anchor(root, 10)),
        ]),
    };
    let mut second = first.clone();
    first.id = "raw:first".to_owned();
    second.id = "raw:second".to_owned();
    second
        .attributes
        .insert("source_anchor".to_owned(), anchor(root, 30));
    let baseline = normalize_v1(
        Extraction {
            nodes: vec![first, second],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;
    let mut trusted = extraction_from_v1(&baseline);
    for node in &mut trusted.nodes {
        let record = node
            .attributes
            .get_mut(TRUSTED_NODE_RECORD_ATTRIBUTE)
            .ok_or("missing trusted node record")?;
        let object = record
            .as_object_mut()
            .ok_or("trusted node is not an object")?;
        // Simulate the legacy producer identity that treated every document
        // block with the same qualified name as one global node.
        object.insert("id".to_owned(), json!("legacy:document:same block"));
    }
    let repaired = normalize_v1_best_effort(trusted, build_evidence(root)?)?;
    assert_eq!(repaired.document.nodes.len(), 2);
    assert_eq!(repaired.omissions.identity_collisions, 0);
    assert!(repaired.document.nodes.iter().all(|node| {
        node.source
            .as_ref()
            .is_some_and(|source| source.start_byte != source.end_byte)
    }));
    Ok(())
}

#[test]
fn raw_semantic_facts_receive_durable_layer_ownership() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut semantic = raw_node(root, "semantic", "semantic", 10);
    semantic
        .attributes
        .insert("_origin".to_owned(), json!("semantic"));
    semantic
        .attributes
        .insert("extractor".to_owned(), json!("third.party.semantic"));
    let ast = raw_node(root, "ast", "ast", 30);
    let graph = normalize_v1(
        Extraction {
            nodes: vec![semantic, ast],
            edges: vec![RawEdgeRecord {
                source: "semantic".to_owned(),
                target: "ast".to_owned(),
                attributes: Map::from_iter([
                    ("relation".to_owned(), json!("references")),
                    ("rule".to_owned(), json!("semantic-reference")),
                    ("_origin".to_owned(), json!("semantic")),
                    ("confidence".to_owned(), json!("INFERRED")),
                    ("extractor".to_owned(), json!("third.party.semantic")),
                    ("source_anchor".to_owned(), anchor(root, 50)),
                ]),
            }],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;

    let semantic = graph
        .nodes
        .iter()
        .find(|node| node.name == "semantic")
        .ok_or("missing semantic node")?;
    assert!(semantic.evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Heuristic
            && evidence.extractor == SEMANTIC_LAYER_EXTRACTOR
            && evidence.rule.as_deref() == Some("semantic-extraction")
    }));
    assert!(graph.links[0].evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Heuristic
            && evidence.extractor == SEMANTIC_LAYER_EXTRACTOR
            && evidence.rule.as_deref() == Some("semantic-reference")
    }));
    Ok(())
}

#[test]
fn raw_go_embeddings_remain_first_class_v1_relationships() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut owner = raw_node(root, "owner", "Owner", 10);
    owner
        .attributes
        .insert("symbol_kind".to_owned(), json!("struct"));
    owner.attributes.insert("language".to_owned(), json!("go"));
    let mut embedded = raw_node(root, "embedded", "Embedded", 30);
    embedded
        .attributes
        .insert("symbol_kind".to_owned(), json!("interface"));
    embedded
        .attributes
        .insert("language".to_owned(), json!("go"));
    let graph = normalize_v1(
        Extraction {
            nodes: vec![owner, embedded],
            edges: vec![RawEdgeRecord {
                source: "owner".to_owned(),
                target: "embedded".to_owned(),
                attributes: Map::from_iter([
                    ("relation".to_owned(), json!("embeds")),
                    ("confidence".to_owned(), json!("EXTRACTED")),
                    ("extractor".to_owned(), json!("compass.languages.go")),
                    ("source_anchor".to_owned(), anchor(root, 50)),
                ]),
            }],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;

    assert_eq!(graph.links.len(), 1);
    assert_eq!(graph.links[0].kind, EdgeKind::Embeds);
    assert!(validate_code_graph(&graph).is_ok());
    Ok(())
}

#[test]
fn raw_typescript_implements_type_alias_remains_a_published_relationship()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut implementation = raw_node(root, "implementation", "ParseInputLazyPath", 10);
    implementation
        .attributes
        .insert("symbol_kind".to_owned(), json!("class"));
    implementation
        .attributes
        .insert("language".to_owned(), json!("typescript"));
    implementation.attributes.insert(
        "extractor".to_owned(),
        json!("compass.languages.typescript"),
    );
    let mut contract = raw_node(root, "contract", "ParseInput", 30);
    contract
        .attributes
        .insert("symbol_kind".to_owned(), json!("type_alias"));
    contract
        .attributes
        .insert("language".to_owned(), json!("typescript"));
    contract.attributes.insert(
        "extractor".to_owned(),
        json!("compass.languages.typescript"),
    );
    let graph = normalize_v1(
        Extraction {
            nodes: vec![implementation, contract],
            edges: vec![RawEdgeRecord {
                source: "implementation".to_owned(),
                target: "contract".to_owned(),
                attributes: Map::from_iter([
                    ("relation".to_owned(), json!("implements")),
                    ("confidence".to_owned(), json!("EXTRACTED")),
                    (
                        "extractor".to_owned(),
                        json!("compass.languages.typescript"),
                    ),
                    ("source_anchor".to_owned(), anchor(root, 50)),
                ]),
            }],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;

    assert_eq!(graph.links.len(), 1);
    assert_eq!(graph.links[0].kind, EdgeKind::Implements);
    assert_eq!(graph.nodes[0].kind, NodeKind::Class);
    assert_eq!(graph.nodes[1].kind, NodeKind::TypeAlias);
    assert!(validate_code_graph(&graph).is_ok());
    Ok(())
}

fn raw_class_node(
    root: &Path,
    id: &str,
    relative: &str,
    qualified_name: &str,
    start: u64,
) -> RawNodeRecord {
    RawNodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), json!(qualified_name)),
            ("qualified_name".to_owned(), json!(qualified_name)),
            ("symbol_kind".to_owned(), json!("class")),
            ("file_type".to_owned(), json!("code")),
            ("language".to_owned(), json!("php")),
            ("extractor".to_owned(), json!("test.php")),
            ("source_anchor".to_owned(), anchor_in(root, relative, start)),
        ]),
    }
}

fn raw_external_node(id: &str, qualified_name: &str) -> RawNodeRecord {
    RawNodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), json!(qualified_name)),
            ("qualified_name".to_owned(), json!(qualified_name)),
            ("file_type".to_owned(), json!("code")),
            ("source_file".to_owned(), json!("")),
        ]),
    }
}

fn raw_php_edge(
    root: &Path,
    relative: &str,
    source: &str,
    target: &str,
    relation: &str,
    start: u64,
) -> RawEdgeRecord {
    RawEdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes: Map::from_iter([
            ("relation".to_owned(), json!(relation)),
            ("_origin".to_owned(), json!("ast")),
            ("confidence".to_owned(), json!("EXTRACTED")),
            ("extractor".to_owned(), json!("test.php")),
            ("source_anchor".to_owned(), anchor_in(root, relative, start)),
        ]),
    }
}

fn add_inventory_file(
    root: &Path,
    evidence: &mut BuildEvidence,
    relative: &str,
    language: &str,
    byte: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = vec![byte; 500];
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &bytes)?;
    evidence.files.push(FileRecord {
        id: format!("raw:{relative}"),
        path: path.to_string_lossy().into_owned(),
        language: Some(language.to_owned()),
        content_digest: format!("sha256:{:x}", Sha256::digest(&bytes)),
        byte_size: bytes.len() as u64,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec![format!("test.{language}")],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    Ok(())
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

fn remapped_heuristic_occurrences(root: &Path) -> Result<Extraction, serde_json::Error> {
    serde_json::from_value(json!({
        "nodes":[
            {
                "id":"ast_caller",
                "label":"Caller",
                "qualified_name":"crate::Caller",
                "symbol_kind":"function",
                "file_type":"code",
                "source_file":root.join("src/lib.rs"),
                "source_anchor":anchor(root, 10),
                "_origin":"ast",
                "extractor":"test.rust"
            },
            {
                "id":"semantic_caller",
                "label":"Caller",
                "qualified_name":"crate::Caller",
                "symbol_kind":"function",
                "file_type":"code",
                "source_file":root.join("src/lib.rs"),
                "source_anchor":anchor(root, 10),
                "_origin":"semantic",
                "extractor":"test.rust"
            },
            {
                "id":"callee",
                "label":"callee()",
                "qualified_name":"crate::callee",
                "symbol_kind":"function",
                "file_type":"code",
                "source_file":root.join("src/lib.rs"),
                "source_anchor":anchor(root, 30),
                "_origin":"ast",
                "extractor":"test.rust"
            }
        ],
        "edges":[
            {
                "source":"semantic_caller",
                "target":"callee",
                "relation":"calls",
                "rule":"rust-call-expression",
                "extractor":"test.rust",
                "_origin":"heuristic",
                "confidence":"INFERRED",
                "source_anchor":anchor(root, 50)
            },
            {
                "source":"semantic_caller",
                "target":"callee",
                "relation":"calls",
                "rule":"scip-call-reference",
                "extractor":"test.rust",
                "_origin":"heuristic",
                "confidence":"INFERRED",
                "source_anchor":anchor(root, 50)
            }
        ]
    }))
}

fn trusted_producer_extraction(root: &Path) -> Extraction {
    let mut extraction = extraction(root);
    extraction.edges[0]
        .attributes
        .insert("relation".to_owned(), json!("calls"));
    extraction.edges[0]
        .attributes
        .insert("rule".to_owned(), json!("producer-rule"));
    extraction.edges[0]
        .attributes
        .insert("_origin".to_owned(), json!("ast"));
    extraction.edges[0]
        .attributes
        .insert("confidence".to_owned(), json!("EXTRACTED"));
    extraction
}

fn extraction_with_rewrite(
    root: &Path,
    rewrite: Value,
    trusted: bool,
) -> Result<Extraction, Box<dyn std::error::Error>> {
    let mut extraction = if trusted {
        extraction_from_v1(&normalize_v1(
            trusted_producer_extraction(root),
            build_evidence(root)?,
        )?)
    } else {
        trusted_producer_extraction(root)
    };
    extraction.edges[0]
        .attributes
        .insert("_endpoint_rewrite_rules".to_owned(), json!([rewrite]));
    Ok(extraction)
}

fn normalization_error(
    extraction: Extraction,
    evidence: BuildEvidence,
) -> Result<String, Box<dyn std::error::Error>> {
    match normalize_v1(extraction, evidence) {
        Ok(_) => Err("invalid endpoint rewrite entry was accepted".into()),
        Err(error) => Ok(error.to_string()),
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
fn framework_and_domain_scopes_are_root_portable() -> Result<(), Box<dyn std::error::Error>> {
    let left_directory = tempfile::tempdir()?;
    let right_directory = tempfile::tempdir()?;
    let graph_at = |root: &Path| -> Result<_, Box<dyn std::error::Error>> {
        let source = root.join("src/lib.rs");
        let source_string = source.to_string_lossy().into_owned();
        let dotted_scope = source
            .with_extension("")
            .to_string_lossy()
            .replace(['/', '\\'], ".")
            .trim_start_matches('.')
            .to_owned();
        let handler_id = format!("route-component:{source_string}");

        let mut route = raw_node(root, "raw:route", "GET /items", 10);
        route
            .attributes
            .insert("symbol_kind".to_owned(), json!("route"));
        route
            .attributes
            .insert("framework".to_owned(), json!("example"));
        route
            .attributes
            .insert("operation".to_owned(), json!("GET"));
        route.attributes.insert("path".to_owned(), json!("/items"));
        route
            .attributes
            .insert("declaring_scope".to_owned(), json!(dotted_scope));
        route.attributes.insert(
            "stages".to_owned(),
            json!([{
                "stage": "handler",
                "position": 0,
                "reference": "handler",
                "resolution": "exact",
                "target": handler_id,
                "candidates": []
            }]),
        );
        route.attributes.insert(
            "candidates".to_owned(),
            json!([{
                "nodeId": handler_id,
                "reason": "exact extractor-local ID",
                "confidence": "exact"
            }]),
        );

        let mut handler = raw_node(root, &handler_id, "handler", 30);
        handler
            .attributes
            .insert("symbol_kind".to_owned(), json!("component"));
        handler
            .attributes
            .insert("component_type".to_owned(), json!("route"));
        handler
            .attributes
            .insert("declaring_scope".to_owned(), json!(source_string));

        let mut message = raw_node(root, "raw:message", "orders.created", 50);
        message
            .attributes
            .insert("symbol_kind".to_owned(), json!("message"));
        message
            .attributes
            .insert("transport".to_owned(), json!("kafka"));
        message
            .attributes
            .insert("subject".to_owned(), json!("orders.created"));
        message
            .attributes
            .insert("declaring_scope".to_owned(), json!(source_string));

        let mut schema = raw_node(root, "raw:schema", "schema", 70);
        schema
            .attributes
            .insert("symbol_kind".to_owned(), json!("schema"));
        schema
            .attributes
            .insert("namespace".to_owned(), json!(source_string));

        let mut extraction = Extraction {
            nodes: vec![route, handler, message, schema],
            ..Extraction::default()
        };
        extraction.extensions.insert(
            "_compass_v1_graph_diagnostics".to_owned(),
            json!([{
                "severity": "warning",
                "code": "test_raw_related_id",
                "message": "raw diagnostic references are remapped",
                "relatedIds": [handler_id]
            }]),
        );
        normalize_v1(extraction, build_evidence(root)?).map_err(Into::into)
    };

    let left = graph_at(left_directory.path())?;
    let right = graph_at(right_directory.path())?;
    assert_eq!(left, right);
    assert!(left.nodes.iter().all(|node| {
        node.evidence.iter().all(|evidence| {
            evidence
                .candidates
                .iter()
                .all(|candidate| !candidate.node_id.contains("route-component:"))
        })
    }));
    assert!(left.graph.diagnostics.iter().all(|diagnostic| {
        diagnostic
            .related_ids
            .iter()
            .all(|related_id| !related_id.contains("route-component:"))
    }));
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
fn symbol_identity_survives_signature_digest_changes() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut before = extraction(root);
    let mut after = extraction(root);
    before.nodes[0]
        .attributes
        .insert("signature_hash".to_owned(), json!("sha256:old-signature"));
    after.nodes[0]
        .attributes
        .insert("signature_hash".to_owned(), json!("sha256:new-signature"));
    for graph in [&mut before, &mut after] {
        graph.nodes[0]
            .attributes
            .insert("lexical_owner".to_owned(), json!("crate"));
    }

    let before = normalize_v1(before, build_evidence(root)?)?;
    let after = normalize_v1(after, build_evidence(root)?)?;
    let before_id = before
        .nodes
        .iter()
        .find(|node| node.name == "caller")
        .ok_or("missing caller")?
        .id
        .clone();
    let after_id = after
        .nodes
        .iter()
        .find(|node| node.name == "caller")
        .ok_or("missing caller")?
        .id
        .clone();
    assert_eq!(before_id, after_id);
    Ok(())
}

#[test]
fn route_identity_uses_handler_reference_instead_of_extractor_local_target()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let route_extraction = |target: &str| {
        let mut route = raw_node(root, "raw:route", "GET /items", 10);
        route
            .attributes
            .insert("symbol_kind".to_owned(), json!("route"));
        route
            .attributes
            .insert("framework".to_owned(), json!("express"));
        route
            .attributes
            .insert("operation".to_owned(), json!("GET"));
        route.attributes.insert("path".to_owned(), json!("/items"));
        route
            .attributes
            .insert("declaring_scope".to_owned(), json!("router"));
        route.attributes.insert(
            "stages".to_owned(),
            json!([{
                "stage": "handler",
                "position": 0,
                "reference": "handlers.listItems",
                "resolution": "exact",
                "target": target,
                "candidates": []
            }]),
        );
        Extraction {
            nodes: vec![
                route,
                raw_node(root, "raw:a", "handler_a", 30),
                raw_node(root, "raw:b", "handler_b", 50),
            ],
            ..Extraction::default()
        }
    };

    let before = normalize_v1(route_extraction("raw:a"), build_evidence(root)?)?;
    let after = normalize_v1(route_extraction("raw:b"), build_evidence(root)?)?;
    let route_id = |document: &compass_model::code_graph::GraphDocument| {
        document
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Route)
            .map(|node| node.id.clone())
    };
    assert_eq!(route_id(&before), route_id(&after));
    Ok(())
}

#[test]
fn duplicate_route_registrations_coalesce_without_losing_relationship_sites()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let route = |id: &str, start: u64| {
        let mut route = raw_node(root, id, "GET /items", start);
        route
            .attributes
            .insert("symbol_kind".to_owned(), json!("route"));
        route
            .attributes
            .insert("framework".to_owned(), json!("express"));
        route
            .attributes
            .insert("operation".to_owned(), json!("GET"));
        route.attributes.insert("path".to_owned(), json!("/items"));
        route
            .attributes
            .insert("declaring_scope".to_owned(), json!("router"));
        route.attributes.insert(
            "stages".to_owned(),
            json!([{
                "stage": "handler",
                "position": 0,
                "reference": "handlers.listItems",
                "resolution": "exact",
                "target": "raw:handler",
                "candidates": []
            }]),
        );
        route
    };
    let route_edge = |source: &str, start: u64| RawEdgeRecord {
        source: source.to_owned(),
        target: "raw:handler".to_owned(),
        attributes: Map::from_iter([
            ("relation".to_owned(), json!("routes_to")),
            ("stage".to_owned(), json!("handler")),
            ("position".to_owned(), json!(0)),
            ("operation".to_owned(), json!("GET")),
            ("extractor".to_owned(), json!("test.routes")),
            ("source_anchor".to_owned(), anchor(root, start)),
        ]),
    };
    let mut route_with_middleware = route("raw:route-b", 100);
    route_with_middleware.attributes.insert(
        "stages".to_owned(),
        json!([
            {
                "stage": "middleware",
                "position": 0,
                "reference": "authenticate",
                "resolution": "unresolved",
                "candidates": []
            },
            {
                "stage": "handler",
                "position": 1,
                "reference": "handlers.listItems",
                "resolution": "exact",
                "target": "raw:handler",
                "candidates": []
            }
        ]),
    );
    let document = normalize_v1(
        Extraction {
            nodes: vec![
                route("raw:route-a", 10),
                route_with_middleware,
                raw_node(root, "raw:handler", "listItems", 200),
            ],
            edges: vec![
                route_edge("raw:route-a", 10),
                route_edge("raw:route-b", 100),
            ],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;

    let routes = document
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Route)
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 1, "nodes={:#?}", document.nodes);
    let route_edges = document
        .links
        .iter()
        .filter(|edge| edge.kind == EdgeKind::RoutesTo && edge.source == routes[0].id)
        .collect::<Vec<_>>();
    assert_eq!(route_edges.len(), 2, "edges={:#?}", document.links);
    assert_ne!(
        route_edges[0].relationship_site,
        route_edges[1].relationship_site
    );
    Ok(())
}

#[test]
fn rust_enum_variant_calls_normalize_to_instantiations() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let caller = raw_node(root, "raw:caller", "poll", 10);
    let mut variant = raw_node(root, "raw:variant", "Ready", 30);
    variant
        .attributes
        .insert("symbol_kind".to_owned(), json!("enum_member"));
    let document = normalize_v1(
        Extraction {
            nodes: vec![caller, variant],
            edges: vec![RawEdgeRecord {
                source: "raw:caller".to_owned(),
                target: "raw:variant".to_owned(),
                attributes: Map::from_iter([
                    ("relation".to_owned(), json!("calls")),
                    ("extractor".to_owned(), json!("test.rust")),
                    ("source_anchor".to_owned(), anchor(root, 50)),
                ]),
            }],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;
    assert!(
        document
            .links
            .iter()
            .any(|edge| edge.kind == EdgeKind::Instantiates),
        "edges={:#?}",
        document.links
    );
    Ok(())
}

#[test]
fn route_stage_becomes_ambiguous_when_multiple_edges_bind_after_remap()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut route = raw_node(root, "raw:route", "GET /items", 10);
    route
        .attributes
        .insert("symbol_kind".to_owned(), json!("route"));
    route
        .attributes
        .insert("framework".to_owned(), json!("express"));
    route
        .attributes
        .insert("operation".to_owned(), json!("GET"));
    route.attributes.insert("path".to_owned(), json!("/items"));
    route
        .attributes
        .insert("declaring_scope".to_owned(), json!("router"));
    route.attributes.insert(
        "stages".to_owned(),
        json!([{
            "stage": "handler",
            "position": 0,
            "reference": "handler",
            "resolution": "exact",
            "target": "raw:a",
            "candidates": []
        }]),
    );
    let route_edge = |target: &str, start| RawEdgeRecord {
        source: "raw:route".to_owned(),
        target: target.to_owned(),
        attributes: Map::from_iter([
            ("relation".to_owned(), json!("routes_to")),
            ("stage".to_owned(), json!("handler")),
            ("position".to_owned(), json!(0)),
            ("operation".to_owned(), json!("GET")),
            ("extractor".to_owned(), json!("test.routes")),
            ("source_anchor".to_owned(), anchor(root, start)),
        ]),
    };
    let extraction = Extraction {
        nodes: vec![
            route,
            raw_node(root, "raw:a", "handler_a", 30),
            raw_node(root, "raw:b", "handler_b", 50),
        ],
        edges: vec![route_edge("raw:a", 70), route_edge("raw:b", 90)],
        ..Extraction::default()
    };
    let document = normalize_v1(extraction, build_evidence(root)?)?;
    let route = document
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Route)
        .ok_or("missing route")?;
    let compass_model::code_graph::NodeDetails::Route(details) =
        route.details.as_ref().ok_or("missing route details")?
    else {
        return Err("wrong route details".into());
    };
    assert_eq!(
        details.stages[0].resolution,
        compass_model::provenance::ResolutionState::Ambiguous
    );
    assert!(details.stages[0].target.is_none());
    assert_eq!(details.stages[0].candidates.len(), 2);
    Ok(())
}

#[test]
fn route_candidates_coalescing_to_one_semantic_node_become_exact()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut route = raw_node(root, "raw:route", "GET /items", 10);
    route
        .attributes
        .insert("symbol_kind".to_owned(), json!("route"));
    route
        .attributes
        .insert("framework".to_owned(), json!("express"));
    route
        .attributes
        .insert("operation".to_owned(), json!("GET"));
    route.attributes.insert("path".to_owned(), json!("/items"));
    route
        .attributes
        .insert("declaring_scope".to_owned(), json!("router"));
    route.attributes.insert(
        "stages".to_owned(),
        json!([{
            "stage": "handler",
            "position": 0,
            "reference": "handler",
            "resolution": "ambiguous",
            "target": null,
            "candidates": [
                {
                    "nodeId": "raw:ast",
                    "reason": "z exact candidate",
                    "confidence": "exact"
                },
                {
                    "nodeId": "raw:semantic",
                    "reason": "a inferred candidate",
                    "confidence": "inferred"
                }
            ]
        }]),
    );
    let mut ast = raw_node(root, "raw:ast", "handler", 30);
    ast.attributes.insert("_origin".to_owned(), json!("ast"));
    let mut semantic = raw_node(root, "raw:semantic", "handler", 30);
    semantic
        .attributes
        .insert("_origin".to_owned(), json!("semantic"));
    let document = normalize_v1(
        Extraction {
            nodes: vec![route, ast, semantic],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;
    let route = document
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Route)
        .ok_or("missing route")?;
    let compass_model::code_graph::NodeDetails::Route(details) =
        route.details.as_ref().ok_or("missing route details")?
    else {
        return Err("wrong route details".into());
    };
    let stage = &details.stages[0];
    assert_eq!(
        stage.resolution,
        compass_model::provenance::ResolutionState::Exact
    );
    assert!(stage.target.is_some());
    assert_eq!(stage.candidates.len(), 1);
    assert_eq!(
        stage.candidates[0].confidence,
        compass_model::provenance::EvidenceConfidence::Exact
    );
    assert_eq!(
        stage.target.as_deref(),
        stage
            .candidates
            .first()
            .map(|candidate| candidate.node_id.as_str())
    );
    Ok(())
}

#[test]
fn open_producer_shapes_normalize_into_the_closed_endpoint_matrix()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let test = raw_node(root, "raw:test", "tests_target", 10);
    let target = raw_node(root, "raw:target", "target", 30);
    let mut class = raw_node(root, "raw:class", "TargetType", 50);
    class
        .attributes
        .insert("symbol_kind".to_owned(), json!("class"));
    let mut annotation = raw_node(root, "raw:annotation", "logged", 70);
    annotation
        .attributes
        .insert("symbol_kind".to_owned(), json!("annotation_type"));
    let edge = |source: &str, relation: &str, target: &str, start| RawEdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes: Map::from_iter([
            ("relation".to_owned(), json!(relation)),
            ("extractor".to_owned(), json!("test.open-producer")),
            ("source_anchor".to_owned(), anchor(root, start)),
        ]),
    };
    let document = normalize_v1(
        Extraction {
            nodes: vec![test, target, class, annotation],
            edges: vec![
                edge("raw:test", "tests", "raw:target", 90),
                edge("raw:test", "type_of", "raw:class", 110),
                edge("raw:test", "decorates", "raw:annotation", 130),
            ],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;
    let test = document
        .nodes
        .iter()
        .find(|node| node.name == "tests_target")
        .ok_or("missing test node")?;
    assert!(
        document
            .nodes
            .iter()
            .any(|node| { node.name == "logged" && node.kind == NodeKind::Annotation })
    );
    assert!(
        test.roles
            .contains(&compass_model::code_graph::NodeRole::Test)
    );
    assert!(
        document
            .links
            .iter()
            .any(|edge| edge.kind == EdgeKind::Tests && edge.source == test.id)
    );
    assert!(
        document
            .links
            .iter()
            .any(|edge| edge.kind == EdgeKind::Returns && edge.source == test.id)
    );
    assert!(document.links.iter().any(|edge| {
        edge.kind == EdgeKind::Decorates
            && document.nodes.iter().any(|node| {
                node.id == edge.source
                    && node.kind == compass_model::code_graph::NodeKind::Annotation
            })
            && edge.target == test.id
    }));
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
fn best_effort_normalization_omits_an_unknown_relation() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut source = extraction(root);
    source.edges[0]
        .attributes
        .insert("relation".to_owned(), json!("bound_to"));

    let outcome = normalize_v1_best_effort(source, build_evidence(root)?)?;

    assert_eq!(outcome.document.nodes.len(), 2);
    assert!(outcome.document.links.is_empty());
    assert_eq!(outcome.omissions.edges, 1);
    assert_eq!(outcome.omissions.nodes, 0);
    assert!(outcome.document.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "publication_omitted_edge"
            && diagnostic.message.contains("unknown raw relation")
    }));
    assert!(validate_code_graph(&outcome.document).is_ok());
    Ok(())
}

#[test]
fn best_effort_normalization_quarantines_unwired_placeholders()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = Extraction {
        nodes: vec![
            raw_node(root, "raw:valid", "valid", 10),
            raw_external_node("raw:model", "ExternalModel"),
        ],
        edges: vec![RawEdgeRecord {
            source: "raw:valid".to_owned(),
            target: "raw:model".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("references")),
                ("extractor".to_owned(), json!("test.rust")),
            ]),
        }],
        ..Extraction::default()
    };

    let outcome = normalize_v1_best_effort(source, build_evidence(root)?)?;

    assert_eq!(outcome.document.nodes.len(), 1);
    assert!(outcome.document.links.is_empty());
    assert_eq!(outcome.omissions.nodes, 1);
    assert_eq!(outcome.omissions.edges, 1);
    assert!(validate_code_graph(&outcome.document).is_ok());
    Ok(())
}

#[test]
fn best_effort_stable_identity_collision_is_order_independent()
-> Result<(), Box<dyn std::error::Error>> {
    let left_directory = tempfile::tempdir()?;
    let right_directory = tempfile::tempdir()?;
    let left_root = left_directory.path();
    let right_root = right_directory.path();
    let source_at = |root: &Path| Extraction {
        nodes: vec![
            raw_node(root, "raw:first", "repeated", 10),
            raw_node(root, "raw:second", "repeated", 30),
        ],
        ..Extraction::default()
    };

    let left = normalize_v1_best_effort(source_at(left_root), build_evidence(left_root)?)?;
    let mut reversed = source_at(right_root);
    reversed.nodes.reverse();
    let right = normalize_v1_best_effort(reversed, build_evidence(right_root)?)?;

    assert_eq!(left.document, right.document);
    assert_eq!(left.omissions, right.omissions);
    assert_eq!(left.document.nodes.len(), 1);
    assert_eq!(left.omissions.nodes, 1);
    assert_eq!(left.omissions.identity_collisions, 1);
    assert!(validate_code_graph(&left.document).is_ok());
    Ok(())
}

#[test]
fn equivalent_external_labels_merge_without_quarantining_wiring()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut first = raw_external_node("raw:first", "chrono::DateTime::parse_from_rfc3339");
    first
        .attributes
        .insert("label".to_owned(), json!("parse_from_rfc3339"));
    let second = raw_external_node("raw:second", "chrono::DateTime::parse_from_rfc3339");
    let outcome = normalize_v1_best_effort(
        Extraction {
            nodes: vec![raw_node(root, "raw:caller", "caller", 10), first, second],
            edges: vec![
                RawEdgeRecord {
                    source: "raw:caller".to_owned(),
                    target: "raw:first".to_owned(),
                    attributes: Map::from_iter([
                        ("relation".to_owned(), json!("calls")),
                        ("source_anchor".to_owned(), anchor(root, 50)),
                    ]),
                },
                RawEdgeRecord {
                    source: "raw:caller".to_owned(),
                    target: "raw:second".to_owned(),
                    attributes: Map::from_iter([
                        ("relation".to_owned(), json!("calls")),
                        ("source_anchor".to_owned(), anchor(root, 70)),
                    ]),
                },
            ],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;
    assert_eq!(outcome.omissions.identity_collisions, 0);
    let external = outcome
        .document
        .nodes
        .iter()
        .find(|node| node.qualified_name == "chrono::DateTime::parse_from_rfc3339")
        .ok_or("missing merged external function")?;
    assert_eq!(external.kind, NodeKind::Function);
    assert_eq!(outcome.document.links.len(), 2);
    assert!(
        outcome
            .document
            .links
            .iter()
            .all(|edge| edge.target == external.id)
    );
    Ok(())
}

#[test]
fn owned_best_effort_publication_sorts_before_assigning_diagnostic_positions()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut source = extraction(root);
    source.edges.extend([
        RawEdgeRecord {
            source: "raw:a".to_owned(),
            target: "missing:z".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("calls")),
                ("confidence".to_owned(), json!("EXTRACTED")),
                ("extractor".to_owned(), json!("test.rust")),
                ("source_anchor".to_owned(), anchor(root, 70)),
            ]),
        },
        RawEdgeRecord {
            source: "raw:a".to_owned(),
            target: "missing:y".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("calls")),
                ("confidence".to_owned(), json!("EXTRACTED")),
                ("extractor".to_owned(), json!("test.rust")),
                ("source_anchor".to_owned(), anchor(root, 90)),
            ]),
        },
    ]);
    let document = build_from_extraction(&source, true, Some(root));
    let mut reversed = document.clone();
    reversed.links.reverse();

    let left = normalize_document_v1_with_inventory_best_effort_owned(
        document,
        root,
        "sha256:test",
        None,
        Vec::new(),
    )?;
    let right = normalize_document_v1_with_inventory_best_effort_owned(
        reversed,
        root,
        "sha256:test",
        None,
        Vec::new(),
    )?;

    assert_eq!(left.document, right.document);
    assert_eq!(left.omissions, right.omissions);
    assert_eq!(left.omissions.edges, 1);
    Ok(())
}

#[test]
fn canonical_raw_order_uses_content_addressed_omission_identities()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut source = extraction(root);
    source.edges = [70_u64, 90]
        .into_iter()
        .map(|start| RawEdgeRecord {
            source: "raw:a".to_owned(),
            target: "raw:b".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!(format!("unknown_{start}"))),
                ("source_anchor".to_owned(), anchor(root, start)),
            ]),
        })
        .collect();
    source.extensions.insert(
        "_compass_v1_canonical_raw_order".to_owned(),
        Value::Bool(true),
    );
    let mut reversed = source.clone();
    reversed.edges.reverse();

    let left = normalize_v1_best_effort(source, build_evidence(root)?)?;
    let right = normalize_v1_best_effort(reversed, build_evidence(root)?)?;

    assert_eq!(left.document, right.document);
    assert_eq!(left.omissions, right.omissions);
    assert_eq!(left.omissions.edges, 2);
    Ok(())
}

#[test]
fn best_effort_diagnostic_positions_sort_by_published_endpoint_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let build = |root: &Path, root_dependent_id: &str| -> Result<_, Box<dyn std::error::Error>> {
        let edge = |source: &str, target: &str, start| RawEdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("calls")),
                ("confidence".to_owned(), json!("EXTRACTED")),
                ("extractor".to_owned(), json!("test.rust")),
                ("source_anchor".to_owned(), anchor(root, start)),
            ]),
        };
        normalize_v1_best_effort(
            Extraction {
                nodes: vec![
                    raw_node(root, root_dependent_id, "alpha", 10),
                    raw_node(root, "middle", "middle", 30),
                    raw_node(root, "target", "target", 50),
                ],
                edges: vec![
                    edge(root_dependent_id, "target", 70),
                    edge("middle", "missing", 90),
                ],
                ..Extraction::default()
            },
            build_evidence(root)?,
        )
        .map_err(Into::into)
    };
    let left_root = directory.path().join("left");
    let right_root = directory.path().join("right");
    let left = build(&left_root, "alpha-root-dependent")?;
    let right = build(&right_root, "zulu-root-dependent")?;

    assert_eq!(left.document, right.document);
    assert_eq!(left.omissions, right.omissions);
    assert_eq!(left.omissions.edges, 1);
    Ok(())
}

#[test]
fn best_effort_typed_validation_omits_invalid_endpoint_edges()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut source = extraction(root);
    source.nodes[0]
        .attributes
        .insert("symbol_kind".to_owned(), json!("method"));
    source.nodes[1]
        .attributes
        .insert("symbol_kind".to_owned(), json!("module"));
    source.edges[0]
        .attributes
        .insert("relation".to_owned(), json!("calls"));
    source.edges[0]
        .attributes
        .insert("_origin".to_owned(), json!("ast"));
    source.edges[0]
        .attributes
        .insert("confidence".to_owned(), json!("EXTRACTED"));

    let outcome = normalize_v1_best_effort(source, build_evidence(root)?)?;

    assert_eq!(outcome.document.nodes.len(), 2);
    assert!(outcome.document.links.is_empty());
    assert_eq!(outcome.omissions.edges, 1);
    assert!(outcome.document.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "publication_omitted_edge"
            && diagnostic
                .message
                .contains("invalid calls endpoints method -> module")
    }));
    assert!(validate_code_graph(&outcome.document).is_ok());
    Ok(())
}

#[test]
fn best_effort_route_stage_becomes_unresolved_when_handler_edge_is_quarantined()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut route = raw_node(root, "raw:route", "GET /items", 10);
    route
        .attributes
        .insert("symbol_kind".to_owned(), json!("route"));
    route
        .attributes
        .insert("framework".to_owned(), json!("express"));
    route
        .attributes
        .insert("operation".to_owned(), json!("GET"));
    route.attributes.insert("path".to_owned(), json!("/items"));
    route
        .attributes
        .insert("declaring_scope".to_owned(), json!("router"));
    route.attributes.insert(
        "stages".to_owned(),
        json!([{
            "stage": "handler",
            "position": 0,
            "reference": "handlers.listItems",
            "resolution": "exact",
            "target": "raw:handler",
            "candidates": []
        }]),
    );
    let mut invalid_handler = raw_node(root, "raw:handler", "handlers", 30);
    invalid_handler
        .attributes
        .insert("symbol_kind".to_owned(), json!("module"));
    let outcome = normalize_v1_best_effort(
        Extraction {
            nodes: vec![route, invalid_handler],
            edges: vec![RawEdgeRecord {
                source: "raw:route".to_owned(),
                target: "raw:handler".to_owned(),
                attributes: Map::from_iter([
                    ("relation".to_owned(), json!("routes_to")),
                    ("stage".to_owned(), json!("handler")),
                    ("position".to_owned(), json!(0)),
                    ("operation".to_owned(), json!("GET")),
                    ("extractor".to_owned(), json!("test.routes")),
                    ("source_anchor".to_owned(), anchor(root, 10)),
                ]),
            }],
            ..Extraction::default()
        },
        build_evidence(root)?,
    )?;

    assert!(outcome.document.links.is_empty());
    let route = outcome
        .document
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Route)
        .ok_or("missing route")?;
    let compass_model::code_graph::NodeDetails::Route(details) =
        route.details.as_ref().ok_or("missing route details")?
    else {
        return Err("wrong route details".into());
    };
    assert_eq!(
        details.resolution,
        compass_model::provenance::ResolutionState::Unresolved
    );
    assert_eq!(
        details.stages[0].resolution,
        compass_model::provenance::ResolutionState::Unresolved
    );
    assert!(details.stages[0].target.is_none());
    assert!(validate_code_graph(&outcome.document).is_ok());
    Ok(())
}

#[test]
fn best_effort_diagnostic_examples_are_bounded_with_exact_counts()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut source = extraction(root);
    source.edges = (0..110)
        .map(|index| RawEdgeRecord {
            source: "raw:a".to_owned(),
            target: "raw:b".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!(format!("unknown_{index}"))),
                ("source_anchor".to_owned(), anchor(root, 50)),
            ]),
        })
        .collect();

    let outcome = normalize_v1_best_effort(source, build_evidence(root)?)?;

    assert_eq!(outcome.omissions.edges, 110);
    assert_eq!(outcome.omissions.examples_omitted, 10);
    assert_eq!(
        outcome
            .document
            .graph
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "publication_omitted_edge")
            .count(),
        100
    );
    assert!(outcome.document.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "publication_omission_summary"
            && diagnostic.message.contains("110 edges")
            && diagnostic.message.contains("10 examples")
    }));
    Ok(())
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
    assert!(
        document.nodes.iter().all(|node| node.name != "callee"),
        "a generic symbol without structural kind evidence must not be guessed as a variable"
    );
    assert!(document.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unresolved_node_kind"
            && diagnostic.message.contains("raw:b")
            && diagnostic.anchor.is_some()
    }));
    Ok(())
}

#[test]
fn normalization_infers_an_external_call_target_as_a_function()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let graph = Extraction {
        nodes: vec![
            raw_node(root, "raw:caller", "caller", 10),
            raw_external_node("raw:external", "external"),
        ],
        edges: vec![RawEdgeRecord {
            source: "raw:caller".to_owned(),
            target: "raw:external".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("calls")),
                ("confidence".to_owned(), json!("INFERRED")),
                ("extractor".to_owned(), json!("test.rust")),
                ("source_anchor".to_owned(), anchor(root, 50)),
            ]),
        }],
        ..Extraction::default()
    };

    let document = normalize_v1(graph, build_evidence(root)?)?;
    let external = document
        .nodes
        .iter()
        .find(|node| node.name == "external")
        .ok_or("missing inferred external function")?;
    assert_eq!(external.kind, NodeKind::Function);
    assert!(external.source.is_none());
    assert!(external.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unresolved_external_symbol" && diagnostic.anchor.is_some()
    }));
    assert!(document.links.iter().any(|edge| {
        edge.kind == EdgeKind::Calls && edge.target == external.id && edge.deferred
    }));
    Ok(())
}

#[test]
fn sourceless_placeholder_identity_unifies_same_file_occurrences_with_typed_deferred_inheritance()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let external_name = "Illuminate\\Database\\Eloquent\\Model";
    let mut inheritance = raw_php_edge(root, "src/lib.rs", "raw:user", "raw:model", "inherits", 70);
    append_endpoint_rewrite_evidence(
        &mut inheritance.attributes,
        EndpointRewriteEvidence {
            rule: EndpointRewriteRule::SourceScopedNodeDisambiguation,
            score: 1.0,
        },
    );
    let mut model = raw_external_node("raw:model", external_name);
    model.attributes.insert("_origin".to_owned(), json!("ast"));
    model
        .attributes
        .insert("confidence".to_owned(), json!("EXTRACTED"));
    model.attributes.insert(
        "extractor".to_owned(),
        json!("compass.resolve.php.universal"),
    );
    let graph = Extraction {
        nodes: vec![
            raw_file_node(root, "raw:file", "src/lib.rs"),
            raw_class_node(root, "raw:user", "src/lib.rs", "App\\User", 10),
            model,
        ],
        edges: vec![
            raw_php_edge(root, "src/lib.rs", "raw:file", "raw:model", "imports", 50),
            inheritance,
        ],
        ..Extraction::default()
    };

    let document = normalize_v1(graph, build_evidence(root)?)?;
    let external = document
        .nodes
        .iter()
        .filter(|node| node.qualified_name == external_name)
        .collect::<Vec<_>>();
    assert_eq!(external.len(), 1, "nodes={:#?}", document.nodes);
    let external = external[0];
    assert_eq!(external.kind, NodeKind::Class);
    assert_eq!(external.language.as_deref(), Some("php"));
    assert_eq!(external.source, None);
    assert!(external.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unresolved_external_symbol"
            && diagnostic
                .anchor
                .as_ref()
                .is_some_and(|anchor| anchor.file == "src/lib.rs" && anchor.start_byte == 50)
    }));
    assert!(external.evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Heuristic
            && evidence.confidence == EvidenceConfidence::Inferred
            && evidence.extractor == "compass.graph.external-placeholder"
            && evidence
                .wiring_site
                .as_ref()
                .is_some_and(|site| site.file == "src/lib.rs" && site.start_byte == 50)
    }));

    let import = document
        .links
        .iter()
        .find(|edge| edge.kind == EdgeKind::Imports)
        .ok_or("missing import edge")?;
    let inheritance = document
        .links
        .iter()
        .find(|edge| edge.kind == EdgeKind::Extends)
        .ok_or("missing typed inheritance edge")?;
    assert_eq!(import.target, external.id);
    assert_eq!(inheritance.target, external.id);
    assert!(import.deferred);
    assert!(inheritance.deferred);
    assert!(
        document
            .links
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Extends)
            .all(|edge| edge.deferred)
    );
    assert!(inheritance.evidence.iter().any(|evidence| {
        evidence.origin == EvidenceOrigin::Ast
            && evidence.confidence == EvidenceConfidence::Exact
            && evidence.extractor == "test.php"
            && !evidence.anchors.is_empty()
    }));
    assert!(inheritance.evidence.iter().any(|evidence| {
        evidence.rule.as_deref() == Some("source-scoped-node-disambiguation")
            && evidence.origin == EvidenceOrigin::Heuristic
            && evidence.confidence == EvidenceConfidence::Inferred
            && evidence.wiring_site.is_some()
    }));
    Ok(())
}

#[test]
fn sourceless_placeholder_identity_never_merges_same_name_across_source_files()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let external_name = "Illuminate\\Database\\Eloquent\\Model";
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/other.php"), vec![b'y'; 500])?;
    let graph = Extraction {
        nodes: vec![
            raw_class_node(root, "raw:user", "src/lib.rs", "App\\User", 10),
            raw_external_node("raw:model-one", external_name),
            raw_class_node(root, "raw:admin", "src/other.php", "Admin\\User", 10),
            raw_external_node("raw:model-two", external_name),
        ],
        edges: vec![
            raw_php_edge(
                root,
                "src/lib.rs",
                "raw:user",
                "raw:model-one",
                "inherits",
                70,
            ),
            raw_php_edge(
                root,
                "src/other.php",
                "raw:admin",
                "raw:model-two",
                "inherits",
                70,
            ),
        ],
        ..Extraction::default()
    };
    let mut evidence = build_evidence(root)?;
    evidence.files.push(FileRecord {
        id: "raw:other".to_owned(),
        path: root.join("src/other.php").to_string_lossy().into_owned(),
        language: Some("php".to_owned()),
        content_digest: format!("sha256:{:x}", Sha256::digest(vec![b'y'; 500])),
        byte_size: 500,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["test.php".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });

    let document = normalize_v1(graph, evidence)?;
    let external_ids = document
        .nodes
        .iter()
        .filter(|node| node.qualified_name == external_name)
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(external_ids.len(), 2);
    let targets = document
        .links
        .iter()
        .map(|edge| edge.target.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(targets.len(), 2);
    assert!(
        document
            .links
            .iter()
            .all(|edge| edge.kind == EdgeKind::Extends && edge.deferred)
    );
    Ok(())
}

#[test]
fn sourceless_implemented_placeholder_infers_a_deferred_interface()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let graph = Extraction {
        nodes: vec![
            raw_class_node(root, "raw:user", "src/lib.rs", "App\\User", 10),
            raw_external_node("raw:contract", "Vendor\\Contracts\\UserContract"),
        ],
        edges: vec![raw_php_edge(
            root,
            "src/lib.rs",
            "raw:user",
            "raw:contract",
            "implements",
            70,
        )],
        ..Extraction::default()
    };

    let document = normalize_v1(graph, build_evidence(root)?)?;
    let external = document
        .nodes
        .iter()
        .find(|node| node.qualified_name == "Vendor\\Contracts\\UserContract")
        .ok_or("missing external interface")?;
    assert_eq!(external.kind, NodeKind::Interface);
    let implementation = document
        .links
        .iter()
        .find(|edge| edge.kind == EdgeKind::Implements)
        .ok_or("missing implementation edge")?;
    assert_eq!(implementation.target, external.id);
    assert!(implementation.deferred);
    Ok(())
}

#[test]
fn semantic_external_marker_defers_project_placeholder_edges_for_any_known_producer()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let dependency = RawNodeRecord {
        id: "raw:dependency".to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), json!("Dependency.csproj")),
            ("qualified_name".to_owned(), json!("Dependency.csproj")),
            ("symbol_kind".to_owned(), json!("package")),
            ("file_type".to_owned(), json!("code")),
            ("language".to_owned(), json!("project-xml")),
            (
                "extractor".to_owned(),
                json!("compass.languages.project-xml"),
            ),
            ("source_file".to_owned(), json!("Dependency.csproj")),
        ]),
    };
    let graph = Extraction {
        nodes: vec![raw_file_node(root, "raw:file", "src/lib.rs"), dependency],
        edges: vec![raw_php_edge(
            root,
            "src/lib.rs",
            "raw:file",
            "raw:dependency",
            "imports",
            50,
        )],
        ..Extraction::default()
    };

    let document = normalize_v1(graph, build_evidence(root)?)?;
    let dependency = document
        .nodes
        .iter()
        .find(|node| node.name == "Dependency.csproj")
        .ok_or("missing project placeholder")?;
    assert!(dependency.source.is_none());
    assert!(dependency.evidence.iter().any(|evidence| {
        evidence.extractor == "compass.languages.project-xml"
            && evidence.origin == EvidenceOrigin::Heuristic
            && evidence.confidence == EvidenceConfidence::Inferred
            && evidence.rule.as_deref() == Some("external-symbol-placeholder")
            && evidence.wiring_site.is_some()
    }));
    let edge = document.links.first().ok_or("missing project import")?;
    assert!(edge.deferred);
    assert!(edge.evidence.iter().any(|evidence| {
        evidence.extractor == "compass.graph.external-placeholder"
            && evidence.rule.as_deref() == Some("external-symbol-placeholder")
            && evidence.wiring_site.is_some()
    }));
    Ok(())
}

#[test]
fn canonical_external_exact_binding_is_published_as_inferred_placeholder()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut external = raw_external_node("raw:blueprint", "flask.Blueprint");
    external.attributes.extend(Map::from_iter([
        ("symbol_kind".to_owned(), json!("class")),
        ("language".to_owned(), json!("python")),
        (
            "extractor".to_owned(),
            json!("compass.resolve.python.universal"),
        ),
        ("_origin".to_owned(), json!("ast")),
        ("confidence".to_owned(), json!("EXTRACTED")),
        ("_canonical_external_symbol".to_owned(), json!(true)),
    ]));
    let graph = Extraction {
        nodes: vec![raw_file_node(root, "raw:file", "src/lib.rs"), external],
        edges: vec![raw_php_edge(
            root,
            "src/lib.rs",
            "raw:file",
            "raw:blueprint",
            "imports",
            50,
        )],
        ..Extraction::default()
    };

    let document = normalize_v1(graph, build_evidence(root)?)?;
    let external = document
        .nodes
        .iter()
        .find(|node| node.qualified_name == "flask.Blueprint")
        .ok_or("missing canonical external placeholder")?;
    assert!(external.evidence.iter().any(|evidence| {
        evidence.extractor == "compass.graph.external-placeholder"
            && evidence.origin == EvidenceOrigin::Heuristic
            && evidence.confidence == EvidenceConfidence::Inferred
            && evidence.rule.as_deref() == Some("external-symbol-placeholder")
            && evidence.wiring_site.is_some()
    }));
    assert!(document.links.first().is_some_and(|edge| edge.deferred));
    Ok(())
}

#[test]
fn authoritative_incident_scope_separates_precoalesced_java_and_typescript_placeholders()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut java_file = raw_file_node(root, "raw:java-file", "src/jpa.java");
    java_file
        .attributes
        .insert("language".to_owned(), json!("java"));
    let mut typescript_owner = raw_class_node(root, "raw:ts-owner", "src/typeorm.ts", "Order", 10);
    typescript_owner
        .attributes
        .insert("language".to_owned(), json!("typescript"));
    let mut global_entity = raw_external_node("raw:entity-global", "Entity");
    global_entity
        .attributes
        .insert("language".to_owned(), json!("typescript"));
    global_entity
        .attributes
        .insert("lexical_owner".to_owned(), json!("stale-global-owner"));
    global_entity
        .attributes
        .insert("origin_file".to_owned(), json!("src/typeorm.ts"));
    global_entity
        .attributes
        .insert("symbol_kind".to_owned(), json!("type_alias"));
    let mut duplicate_typescript_entity = raw_external_node("raw:entity-ts", "Entity");
    duplicate_typescript_entity
        .attributes
        .insert("language".to_owned(), json!("java"));
    duplicate_typescript_entity
        .attributes
        .insert("symbol_kind".to_owned(), json!("type_alias"));
    duplicate_typescript_entity.attributes.insert(
        "declaring_scope".to_owned(),
        json!("stale-typescript-owner"),
    );
    let graph = Extraction {
        nodes: vec![
            java_file,
            typescript_owner,
            global_entity,
            duplicate_typescript_entity,
        ],
        edges: vec![
            raw_php_edge(
                root,
                "src/jpa.java",
                "raw:java-file",
                "raw:entity-global",
                "imports",
                20,
            ),
            raw_php_edge(
                root,
                "src/typeorm.ts",
                "raw:ts-owner",
                "raw:entity-global",
                "references",
                30,
            ),
            raw_php_edge(
                root,
                "src/typeorm.ts",
                "raw:ts-owner",
                "raw:entity-ts",
                "references",
                50,
            ),
        ],
        ..Extraction::default()
    };
    let mut evidence = build_evidence(root)?;
    add_inventory_file(root, &mut evidence, "src/jpa.java", "java", b'j')?;
    add_inventory_file(root, &mut evidence, "src/typeorm.ts", "typescript", b't')?;

    let document = normalize_v1(graph, evidence)?;
    let entities = document
        .nodes
        .iter()
        .filter(|node| node.qualified_name == "Entity" && node.source.is_none())
        .collect::<Vec<_>>();
    assert_eq!(entities.len(), 2, "nodes={:#?}", document.nodes);
    let java = entities
        .iter()
        .find(|node| node.language.as_deref() == Some("java"))
        .ok_or("missing Java placeholder")?;
    let typescript = entities
        .iter()
        .find(|node| node.language.as_deref() == Some("typescript"))
        .ok_or("missing TypeScript placeholder")?;
    assert_ne!(java.id, typescript.id);
    assert!(java.evidence.iter().all(|evidence| {
        evidence
            .wiring_site
            .as_ref()
            .is_some_and(|site| site.file == "src/jpa.java")
    }));
    assert!(typescript.evidence.iter().all(|evidence| {
        evidence
            .wiring_site
            .as_ref()
            .is_some_and(|site| site.file == "src/typeorm.ts")
    }));
    for edge in &document.links {
        let file = edge
            .relationship_site
            .as_ref()
            .map(|site| site.file.as_str());
        match file {
            Some("src/jpa.java") => assert_eq!(edge.target, java.id),
            Some("src/typeorm.ts") => assert_eq!(edge.target, typescript.id),
            _ => {}
        }
        assert!(edge.deferred);
    }
    Ok(())
}

#[test]
fn sourceless_placeholder_without_exact_wiring_fails_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let graph = Extraction {
        nodes: vec![raw_external_node(
            "raw:model",
            "Illuminate\\Database\\Eloquent\\Model",
        )],
        ..Extraction::default()
    };
    let error = match normalize_v1(graph, build_evidence(root)?) {
        Ok(_) => return Err("unwired placeholder published direct AST evidence".into()),
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("requires an exact wiring site"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[test]
fn normalization_drops_non_recursive_self_loops_and_invalid_inheritance_targets()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut graph = extraction(root);
    graph.nodes[0]
        .attributes
        .insert("symbol_kind".to_owned(), json!("class"));
    graph.nodes[1]
        .attributes
        .insert("symbol_kind".to_owned(), json!("variable"));
    let mut dependency = graph.nodes[0].clone();
    dependency.id = "raw:dependency".to_owned();
    dependency
        .attributes
        .insert("label".to_owned(), json!("dependency"));
    dependency
        .attributes
        .insert("qualified_name".to_owned(), json!("dependency"));
    dependency
        .attributes
        .insert("symbol_kind".to_owned(), json!("package"));
    graph.nodes.push(dependency);
    graph.edges = vec![
        RawEdgeRecord {
            source: "raw:dependency".to_owned(),
            target: "raw:dependency".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("imports")),
                ("source_anchor".to_owned(), anchor(root, 50)),
            ]),
        },
        RawEdgeRecord {
            source: "raw:a".to_owned(),
            target: "raw:b".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("extends")),
                ("source_anchor".to_owned(), anchor(root, 70)),
            ]),
        },
    ];

    let clean = normalize_v1(graph.clone(), build_evidence(root)?)?;
    let mut repeated_evidence = build_evidence(root)?;
    repeated_evidence
        .diagnostics
        .clone_from(&clean.graph.diagnostics);
    let document = normalize_v1(graph, repeated_evidence)?;
    assert!(document.links.is_empty());
    assert_eq!(document.graph.diagnostics, clean.graph.diagnostics);
    assert!(
        document
            .graph
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "dropped_non_recursive_self_loop")
    );
    assert!(
        document
            .graph
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "dropped_invalid_inheritance_target")
    );
    Ok(())
}

#[test]
fn normalization_preserves_rust_blanket_implementation_edges()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut parameter = raw_node(root, "raw:parameter", "T", 10);
    parameter
        .attributes
        .insert("symbol_kind".to_owned(), json!("parameter"));
    parameter.attributes.insert(
        "qualified_name".to_owned(),
        json!("<impl<T> Render for T>::<T>"),
    );
    let mut render = raw_node(root, "raw:render", "Render", 30);
    render
        .attributes
        .insert("symbol_kind".to_owned(), json!("trait"));
    let graph = Extraction {
        nodes: vec![parameter, render],
        edges: vec![RawEdgeRecord {
            source: "raw:parameter".to_owned(),
            target: "raw:render".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("implements")),
                ("confidence".to_owned(), json!("EXTRACTED")),
                ("extractor".to_owned(), json!("test.rust")),
                ("source_anchor".to_owned(), anchor(root, 50)),
            ]),
        }],
        ..Extraction::default()
    };

    let document = normalize_v1(graph, build_evidence(root)?)?;
    assert_eq!(document.links.len(), 1);
    assert_eq!(document.links[0].kind, EdgeKind::Implements);
    assert_eq!(document.nodes[0].language.as_deref(), Some("rust"));
    validate_code_graph(&document)?;
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
fn build_evidence_reuses_precomputed_source_digests_without_changing_file_records()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    let bytes = b"fn source() {}\n";
    fs::write(root.join("src/lib.rs"), bytes)?;
    let extraction = Extraction {
        nodes: vec![raw_node(root, "raw:source", "source", 0)],
        ..Extraction::default()
    };
    let document = build_from_extraction(&extraction, true, Some(root));
    let baseline = BuildEvidence::from_document(root, &document, "sha256:config")?;
    let digests = BTreeMap::from([(
        "src/lib.rs".to_owned(),
        SourceDigest {
            content_digest: format!("sha256:{:x}", Sha256::digest(bytes)),
            byte_size: bytes.len() as u64,
        },
    )]);
    let reused = BuildEvidence::from_document_with_source_digests(
        root,
        &document,
        "sha256:config",
        &digests,
    )?;

    assert_eq!(reused.files, baseline.files);
    assert_eq!(reused.build, baseline.build);

    let extraction_baseline = BuildEvidence::from_extraction(root, &extraction, "sha256:config")?;
    let extraction_reused = BuildEvidence::from_extraction_with_source_digests(
        root,
        &extraction,
        "sha256:config",
        &digests,
    )?;
    assert_eq!(extraction_reused.files, extraction_baseline.files);
    assert_eq!(extraction_reused.build, extraction_baseline.build);
    Ok(())
}

#[test]
fn early_inference_admission_preserves_low_publication_and_omission_diagnostics()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut placeholder = RawNodeRecord {
        id: "deferred:helper".to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), json!("helper")),
            ("qualified_name".to_owned(), json!("external::helper")),
            ("file_type".to_owned(), json!("code")),
            (
                "extractor".to_owned(),
                json!("compass.graph.external-placeholder"),
            ),
            ("_origin".to_owned(), json!("heuristic")),
            ("confidence".to_owned(), json!("INFERRED")),
        ]),
    };
    placeholder
        .attributes
        .insert("rule".to_owned(), json!("deferred-receiver"));
    let extraction = Extraction {
        nodes: vec![raw_node(root, "source", "source", 0), placeholder],
        edges: vec![RawEdgeRecord {
            source: "source".to_owned(),
            target: "deferred:helper".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("calls")),
                ("extractor".to_owned(), json!("test.rust")),
                ("_origin".to_owned(), json!("heuristic")),
                ("confidence".to_owned(), json!("INFERRED")),
                ("rule".to_owned(), json!("deferred-receiver")),
                ("source_anchor".to_owned(), anchor(root, 10)),
            ]),
        }],
        ..Extraction::default()
    };
    let mut baseline = normalize_v1_best_effort(extraction.clone(), build_evidence(root)?)?;
    apply_inference_level(&mut baseline.document, InferenceLevel::Low);
    let mut admitted = normalize_v1_best_effort_with_inference(
        extraction.clone(),
        build_evidence(root)?,
        InferenceLevel::Low,
    )?;
    apply_inference_level(&mut admitted.document, InferenceLevel::Low);

    assert_eq!(admitted.omissions, baseline.omissions);
    assert_eq!(admitted.document, baseline.document);

    let baseline_document = build_from_extraction(&extraction, true, Some(root));
    let mut built_baseline = normalize_document_v1_with_evidence_best_effort_owned_at_inference(
        baseline_document,
        build_evidence(root)?,
        InferenceLevel::Low,
    )?;
    apply_inference_level(&mut built_baseline.document, InferenceLevel::Low);
    let early_document = build_owned_with_tiebreaker_at_inference(
        extraction,
        true,
        false,
        Some(root),
        None,
        InferenceLevel::Low,
    )?;
    let mut built_early = normalize_document_v1_with_evidence_best_effort_owned_at_inference(
        early_document,
        build_evidence(root)?,
        InferenceLevel::Low,
    )?;
    apply_inference_level(&mut built_early.document, InferenceLevel::Low);

    assert_eq!(built_early.omissions, built_baseline.omissions);
    assert_eq!(built_early.document, built_baseline.document);
    Ok(())
}

#[test]
fn early_build_inference_admission_preserves_coalesced_duplicate_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut exact = RawEdgeRecord {
        source: "source".to_owned(),
        target: "target".to_owned(),
        attributes: Map::from_iter([
            ("relation".to_owned(), json!("calls")),
            ("extractor".to_owned(), json!("test.rust")),
            ("_origin".to_owned(), json!("parser")),
            ("confidence".to_owned(), json!("EXTRACTED")),
            ("rule".to_owned(), json!("direct-call")),
            ("source_anchor".to_owned(), anchor(root, 10)),
        ]),
    };
    let mut inferred = exact.clone();
    inferred
        .attributes
        .insert("_origin".to_owned(), json!("heuristic"));
    inferred
        .attributes
        .insert("confidence".to_owned(), json!("INFERRED"));
    exact
        .attributes
        .insert("confidence_score".to_owned(), json!(1.0));
    inferred
        .attributes
        .insert("confidence_score".to_owned(), json!(0.5));
    let extraction = Extraction {
        nodes: vec![
            raw_node(root, "source", "source", 0),
            raw_node(root, "target", "target", 20),
        ],
        edges: vec![exact, inferred],
        ..Extraction::default()
    };

    let baseline_document =
        build_owned_with_tiebreaker(extraction.clone(), true, false, Some(root), None)?;
    let early_document = build_owned_with_tiebreaker_at_inference(
        extraction,
        true,
        false,
        Some(root),
        None,
        InferenceLevel::Low,
    )?;
    let mut baseline = normalize_document_v1_with_evidence_best_effort_owned_at_inference(
        baseline_document,
        build_evidence(root)?,
        InferenceLevel::Low,
    )?;
    apply_inference_level(&mut baseline.document, InferenceLevel::Low);
    let mut early = normalize_document_v1_with_evidence_best_effort_owned_at_inference(
        early_document,
        build_evidence(root)?,
        InferenceLevel::Low,
    )?;
    apply_inference_level(&mut early.document, InferenceLevel::Low);

    assert_eq!(early.omissions, baseline.omissions);
    assert_eq!(early.document, baseline.document);
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
fn v1_publication_preserves_distinct_remapped_producer_occurrences()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), vec![b'x'; 500])?;
    let extraction = remapped_heuristic_occurrences(root)?;

    let flexible = build_from_extraction(&extraction, true, Some(root));
    let typed = normalize_document_v1(&flexible, root, "sha256:test", None)?;

    assert_eq!(flexible.links.len(), 2, "links={:?}", flexible.links);
    assert_eq!(typed.links.len(), 2, "links={:?}", typed.links);
    assert_ne!(typed.links[0].id, typed.links[1].id);
    Ok(())
}

#[test]
fn trusted_incremental_round_trip_preserves_remapped_producer_occurrences()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/lib.rs"), vec![b'x'; 500])?;
    let extraction = remapped_heuristic_occurrences(root)?;
    let flexible = build_from_extraction(&extraction, true, Some(root));
    let typed = normalize_document_v1(&flexible, root, "sha256:test", None)?;

    let projected = extraction_from_v1(&typed);
    let rebuilt = normalize_v1(projected, build_evidence(root)?)?;

    assert_eq!(rebuilt.links, typed.links);
    assert_eq!(rebuilt.links.len(), 2);
    Ok(())
}

#[test]
fn trusted_incremental_merges_new_constituent_resolver_and_graph_rewrite_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let baseline = normalize_v1(trusted_producer_extraction(root), build_evidence(root)?)?;
    let baseline_id = baseline.links[0].id.clone();
    let baseline_occurrence = baseline.links[0].occurrence_rule.clone();
    let mut projected = extraction_from_v1(&baseline);
    let edge = projected
        .edges
        .first_mut()
        .ok_or("missing projected edge")?;
    append_endpoint_rewrite_evidence(
        &mut edge.attributes,
        EndpointRewriteEvidence {
            rule: EndpointRewriteRule::SourceScopedNodeDisambiguation,
            score: 1.0,
        },
    );
    edge.attributes.insert(
        "_coalesced_edge_evidence".to_owned(),
        json!([{
            "_origin":"artifact",
            "confidence":"EXTRACTED",
            "extractor":"test.incremental.artifact",
            "rule":"artifact-call-reference",
            "source_anchor":anchor(root, 50)
        }]),
    );
    let projected_source = edge.source.clone();
    let mut semantic_source = projected
        .nodes
        .iter()
        .find(|node| node.id == projected_source)
        .cloned()
        .ok_or("missing projected source")?;
    semantic_source.id = "semantic-caller".to_owned();
    semantic_source
        .attributes
        .insert("_origin".to_owned(), json!("semantic"));
    semantic_source.attributes.remove("_compass_v1_node_record");
    projected.nodes.push(semantic_source);
    projected.edges[0].source = "semantic-caller".to_owned();

    let flexible = build_from_extraction(&projected, true, Some(root));
    let rebuilt = normalize_document_v1(&flexible, root, "sha256:test", None)?;
    let rebuilt_edge = rebuilt.links.first().ok_or("missing rebuilt edge")?;

    assert_eq!(rebuilt_edge.id, baseline_id);
    assert_eq!(rebuilt_edge.occurrence_rule, baseline_occurrence);
    for expected in [
        "producer-rule",
        "artifact-call-reference",
        "source-scoped-node-disambiguation",
        "graph-ghost-endpoint-remap",
    ] {
        assert!(
            rebuilt_edge
                .evidence
                .iter()
                .any(|evidence| evidence.rule.as_deref() == Some(expected)),
            "missing {expected}: {:?}",
            rebuilt_edge.evidence
        );
    }
    assert!(rebuilt_edge.evidence.iter().any(|evidence| {
        evidence.rule.as_deref() == Some("source-scoped-node-disambiguation")
            && evidence.wiring_site.as_ref().is_some_and(|site| {
                site.file == "src/lib.rs" && site.start_byte == 50 && site.end_byte == 54
            })
    }));

    let graph_path = root.join("graph.json");
    fs::write(&graph_path, serde_json::to_vec_pretty(&rebuilt)?)?;
    let loaded = compass_model::code_graph::GraphDocument::load(&graph_path)?;
    assert_eq!(loaded.links, rebuilt.links);
    let second_projection = extraction_from_v1(&loaded);
    let second = normalize_v1(second_projection, build_evidence(root)?)?;
    assert_eq!(second.links, rebuilt.links);
    Ok(())
}

#[test]
fn trusted_incremental_rejects_conflicting_raw_occurrence_rule()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let baseline = normalize_v1(trusted_producer_extraction(root), build_evidence(root)?)?;
    let mut projected = extraction_from_v1(&baseline);
    projected.edges[0]
        .attributes
        .insert("_occurrence_rule".to_owned(), json!("conflicting-producer"));

    let error = match normalize_v1(projected, build_evidence(root)?) {
        Ok(_) => return Err("conflicting raw occurrence identity was accepted".into()),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("conflicting raw occurrence rule"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[test]
fn trusted_occurrence_identity_reserves_every_closed_rewrite_name()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let baseline = normalize_v1(trusted_producer_extraction(root), build_evidence(root)?)?;

    for rule in CLOSED_ENDPOINT_REWRITE_RULES {
        let mut spoofed = baseline.clone();
        let edge = spoofed.links.first_mut().ok_or("missing trusted edge")?;
        edge.occurrence_rule = compass_model::provenance::OccurrenceRule::new(rule);
        edge.id = edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            Some(rule),
        );
        edge.key.clone_from(&edge.id);
        let error = normalization_error(extraction_from_v1(&spoofed), build_evidence(root)?)?;
        assert!(
            error.contains("occurrence rule"),
            "closed occurrence rule {rule} produced unexpected error: {error}"
        );
    }

    let mut arbitrary = baseline;
    let edge = arbitrary.links.first_mut().ok_or("missing trusted edge")?;
    edge.occurrence_rule = compass_model::provenance::OccurrenceRule::new("future-endpoint-remap");
    edge.id = edge_id(
        &edge.source,
        edge.kind,
        &edge.target,
        edge.relationship_site.as_ref(),
        Some("future-endpoint-remap"),
    );
    edge.key.clone_from(&edge.id);
    let rebuilt = normalize_v1(extraction_from_v1(&arbitrary), build_evidence(root)?)?;
    assert_eq!(rebuilt.links, arbitrary.links);
    Ok(())
}

#[test]
fn trusted_redirect_requires_current_endpoint_rewrite_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut source = trusted_producer_extraction(root);
    source
        .nodes
        .push(raw_node(root, "raw:alternate", "alternate", 70));
    let direct = normalize_v1(source, build_evidence(root)?)?;
    let mut first_projection = extraction_from_v1(&direct);
    append_endpoint_rewrite_evidence(
        &mut first_projection.edges[0].attributes,
        EndpointRewriteEvidence {
            rule: EndpointRewriteRule::IncrementalAstEndpointRemap,
            score: 1.0,
        },
    );
    let baseline = normalize_v1(first_projection, build_evidence(root)?)?;
    assert!(
        baseline.links[0]
            .evidence
            .iter()
            .any(|evidence| { evidence.rule.as_deref() == Some("incremental-ast-endpoint-remap") })
    );

    let mut projected = extraction_from_v1(&baseline);
    let alternate = projected
        .nodes
        .iter()
        .find(|node| node.label() == "alternate")
        .map(|node| node.id.clone())
        .ok_or("missing alternate projected node")?;
    projected.edges[0].target = alternate;

    let error = normalization_error(projected, build_evidence(root)?)?;
    assert!(
        error.contains("current endpoint rewrite"),
        "unexpected error: {error}"
    );
    Ok(())
}

#[test]
fn trusted_embedded_endpoint_rewrites_reject_spoofed_or_incomplete_semantics()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let baseline = normalize_v1(trusted_producer_extraction(root), build_evidence(root)?)?;
    let mut valid = json!({
        "origin":"heuristic",
        "extractor":"test.incremental",
        "confidence":"inferred",
        "rule":"incremental-ast-endpoint-remap",
        "wiringSite":{
            "file":"src/lib.rs",
            "startByte":50,
            "endByte":54,
            "startLine":6,
            "startColumn":0,
            "endLine":6,
            "endColumn":4
        },
        "score":0.75
    });
    let mut ast_exact = serde_json::to_value(&baseline.links[0].evidence[0])?;
    ast_exact["rule"] = json!("incremental-ast-endpoint-remap");
    let mut missing_site = valid.clone();
    missing_site
        .as_object_mut()
        .ok_or("evidence is not an object")?
        .remove("wiringSite");
    let mut missing_score = valid.clone();
    missing_score
        .as_object_mut()
        .ok_or("evidence is not an object")?
        .remove("score");
    let mut out_of_range_score = valid.clone();
    out_of_range_score["score"] = json!(1.01);
    valid["anchors"] = json!([{
        "file":"src/lib.rs",
        "startByte":50,
        "endByte":54,
        "startLine":6,
        "startColumn":0,
        "endLine":6,
        "endColumn":4
    }]);

    for (case, evidence) in [
        ("AST/exact spoof", ast_exact),
        ("missing wiring site", missing_site),
        ("missing score", missing_score),
        ("out-of-range score", out_of_range_score),
        ("direct anchor", valid),
    ] {
        let mut projected = extraction_from_v1(&baseline);
        projected.edges[0].attributes[TRUSTED_EDGE_RECORD_ATTRIBUTE]["evidence"] =
            json!([evidence]);
        let error = normalization_error(projected, build_evidence(root)?)?;
        assert!(
            error.contains("endpoint rewrite"),
            "{case} produced unexpected error: {error}"
        );
    }
    Ok(())
}

#[test]
fn closed_rewrite_names_are_reserved_but_unknown_rewrite_like_producers_remain_open()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut ordinary = trusted_producer_extraction(root);
    ordinary.edges[0]
        .attributes
        .insert("rule".to_owned(), json!("incremental-ast-endpoint-remap"));
    let error = normalization_error(ordinary, build_evidence(root)?)?;
    assert!(
        error.contains("endpoint rewrite"),
        "unexpected error: {error}"
    );

    let baseline = normalize_v1(trusted_producer_extraction(root), build_evidence(root)?)?;
    let mut coalesced = extraction_from_v1(&baseline);
    coalesced.edges[0].attributes.insert(
        "_coalesced_edge_evidence".to_owned(),
        json!([{
            "rule":"incremental-ast-endpoint-remap",
            "_origin":"heuristic",
            "confidence":"INFERRED",
            "extractor":"test.spoof",
            "source_anchor":anchor(root, 50),
            "score":0.5
        }]),
    );
    let error = normalization_error(coalesced, build_evidence(root)?)?;
    assert!(
        error.contains("endpoint rewrite"),
        "unexpected error: {error}"
    );

    let mut future = trusted_producer_extraction(root);
    future.edges[0]
        .attributes
        .insert("rule".to_owned(), json!("future-endpoint-remap"));
    let normalized = normalize_v1(future, build_evidence(root)?)?;
    assert_eq!(
        normalized.links[0]
            .occurrence_rule
            .as_ref()
            .map(|rule| rule.as_str()),
        Some("future-endpoint-remap")
    );
    let rebuilt = normalize_v1(extraction_from_v1(&normalized), build_evidence(root)?)?;
    assert_eq!(rebuilt.links, normalized.links);
    Ok(())
}

#[test]
fn ordinary_and_trusted_edges_reject_untyped_or_spoofed_endpoint_rewrites()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let valid = || {
        json!({
            "rule":"incremental-ast-endpoint-remap",
            "score":1.0,
            "extractor":"test.rewrite",
            "source_anchor":anchor(root, 50)
        })
    };
    let mut unknown_rule = valid();
    unknown_rule["rule"] = json!("not-a-closed-rewrite-rule");
    let mut missing_rule = valid();
    missing_rule
        .as_object_mut()
        .ok_or("rewrite is not an object")?
        .remove("rule");
    let mut missing_score = valid();
    missing_score
        .as_object_mut()
        .ok_or("rewrite is not an object")?
        .remove("score");
    let mut malformed_score = valid();
    malformed_score["score"] = json!("certain");
    let mut out_of_range_score = valid();
    out_of_range_score["score"] = json!(1.01);
    let mut unknown_field = valid();
    unknown_field["untrusted"] = json!(true);
    let mut exact_spoof = valid();
    exact_spoof["_origin"] = json!("ast");
    exact_spoof["confidence"] = json!("EXTRACTED");

    for trusted in [false, true] {
        for (case, rewrite) in [
            ("non-object", json!("rewrite")),
            ("missing rule", missing_rule.clone()),
            ("unknown rule", unknown_rule.clone()),
            ("missing score", missing_score.clone()),
            ("malformed score", malformed_score.clone()),
            ("out-of-range score", out_of_range_score.clone()),
            ("unknown field", unknown_field.clone()),
            ("AST/exact spoof", exact_spoof.clone()),
        ] {
            let error = normalization_error(
                extraction_with_rewrite(root, rewrite, trusted)?,
                build_evidence(root)?,
            )?;
            assert!(
                error.contains("endpoint rewrite"),
                "{case} trusted={trusted} produced unexpected error: {error}"
            );
        }
    }
    Ok(())
}

#[test]
fn ordinary_and_trusted_typed_rewrites_are_forced_to_heuristic_inferred_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    for trusted in [false, true] {
        let normalized = normalize_v1(
            extraction_with_rewrite(
                root,
                json!({
                    "rule":"incremental-ast-endpoint-remap",
                    "score":0.75,
                    "extractor":"test.rewrite",
                    "source_anchor":anchor(root, 50)
                }),
                trusted,
            )?,
            build_evidence(root)?,
        )?;
        assert!(normalized.links[0].evidence.iter().any(|evidence| {
            evidence.rule.as_deref() == Some("incremental-ast-endpoint-remap")
                && evidence.origin == compass_model::provenance::EvidenceOrigin::Heuristic
                && evidence.confidence == compass_model::provenance::EvidenceConfidence::Inferred
                && evidence.score == Some(0.75)
                && evidence.wiring_site.as_ref().is_some_and(|site| {
                    site.file == "src/lib.rs" && site.start_byte == 50 && site.end_byte == 54
                })
        }));
    }
    Ok(())
}

#[test]
fn trusted_null_relationship_site_derives_stable_exact_rewrite_wiring()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut baseline = normalize_v1(trusted_producer_extraction(root), build_evidence(root)?)?;
    let edge = baseline.links.first_mut().ok_or("missing baseline edge")?;
    let producer_anchor = edge.evidence[0]
        .anchors
        .first()
        .cloned()
        .ok_or("missing producer anchor")?;
    edge.relationship_site = None;
    edge.id = edge_id(
        &edge.source,
        edge.kind,
        &edge.target,
        None,
        edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
    );
    edge.key.clone_from(&edge.id);
    let stable_id = edge.id.clone();

    let mut projected = extraction_from_v1(&baseline);
    append_endpoint_rewrite_evidence(
        &mut projected.edges[0].attributes,
        EndpointRewriteEvidence {
            rule: EndpointRewriteRule::IncrementalAstEndpointRemap,
            score: 1.0,
        },
    );
    let rebuilt = normalize_v1(projected, build_evidence(root)?)?;
    let rebuilt_edge = rebuilt.links.first().ok_or("missing rebuilt edge")?;
    assert_eq!(rebuilt_edge.id, stable_id);
    assert!(rebuilt_edge.relationship_site.is_none());
    assert!(rebuilt_edge.evidence.iter().any(|evidence| {
        evidence.rule.as_deref() == Some("incremental-ast-endpoint-remap")
            && evidence.wiring_site.as_ref() == Some(&producer_anchor)
    }));

    let graph_path = root.join("null-relationship-site.json");
    fs::write(&graph_path, serde_json::to_vec_pretty(&rebuilt)?)?;
    let loaded = compass_model::code_graph::GraphDocument::load(&graph_path)?;
    let second = normalize_v1(extraction_from_v1(&loaded), build_evidence(root)?)?;
    assert_eq!(second.links, rebuilt.links);
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
            language: None,
            producer: "compass.files.detect".to_owned(),
            status: ExtractionStatus::Unsupported,
            reason: Some("no extractor".to_owned()),
        },
        InventoryEvidence {
            path: root.join("partial.rs"),
            language: Some("rust".to_owned()),
            producer: "compass.languages.rust".to_owned(),
            status: ExtractionStatus::Partial,
            reason: Some("partial semantic extraction".to_owned()),
        },
        InventoryEvidence {
            path: root.join("generated.rs"),
            language: Some("rust".to_owned()),
            producer: "compass.languages.rust".to_owned(),
            status: ExtractionStatus::Generated,
            reason: None,
        },
        InventoryEvidence {
            path: root.join("broken.rs"),
            language: Some("rust".to_owned()),
            producer: "compass.languages.rust".to_owned(),
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

#[test]
fn unresolved_external_reference_diagnostics_are_bounded_and_deterministic()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let extraction = Extraction {
        nodes: (0..105)
            .map(|index| RawNodeRecord {
                id: format!("external-{index:03}"),
                attributes: Map::from_iter([
                    ("label".to_owned(), json!(format!("External{index:03}"))),
                    ("symbol_kind".to_owned(), json!("package")),
                    (
                        "source_file".to_owned(),
                        json!(format!("external/Project{index:03}.csproj")),
                    ),
                ]),
            })
            .collect(),
        ..Extraction::default()
    };
    let first = BuildEvidence::from_extraction(directory.path(), &extraction, "config")?;
    let second = BuildEvidence::from_extraction(directory.path(), &extraction, "config")?;

    assert_eq!(first.diagnostics, second.diagnostics);
    assert_eq!(
        first
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "unresolved_external_reference")
            .count(),
        100
    );
    assert!(first.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unresolved_external_reference_truncated"
            && diagnostic.message == "omitted 5 additional unresolved external references"
    }));
    Ok(())
}
