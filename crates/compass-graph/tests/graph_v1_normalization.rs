use std::fs;
use std::path::Path;

use compass_graph::{
    BuildEvidence, InventoryEvidence, build_from_extraction, extraction_from_v1,
    normalize_document_v1, normalize_v1,
};
use compass_languages::{Extraction, RawEdgeRecord, RawNodeRecord};
use compass_model::code_graph::{
    BuildMetadata, CoverageRecord, CoverageStatus, DiagnosticSeverity, EdgeKind, ExtractionStatus,
    FileRecord, GraphDiagnostic, NodeKind,
};
use compass_model::identity::edge_id;
use compass_model::provenance::{
    EndpointRewriteEvidence, EndpointRewriteRule, append_endpoint_rewrite_evidence,
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
fn sourceless_placeholder_identity_is_scoped_to_each_wiring_site()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let mut graph = extraction(root);
    graph.nodes[1].attributes.remove("source_anchor");
    graph.nodes[1]
        .attributes
        .insert("source_file".to_owned(), json!(""));
    graph.nodes[1].attributes.remove("symbol_kind");
    graph.nodes[1]
        .attributes
        .insert("label".to_owned(), json!("Shared"));
    graph.nodes[1]
        .attributes
        .insert("qualified_name".to_owned(), json!("Shared"));
    graph.edges = vec![
        RawEdgeRecord {
            source: "raw:a".to_owned(),
            target: "raw:b".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("references")),
                ("source_anchor".to_owned(), anchor(root, 50)),
            ]),
        },
        RawEdgeRecord {
            source: "raw:a".to_owned(),
            target: "raw:b".to_owned(),
            attributes: Map::from_iter([
                ("relation".to_owned(), json!("references")),
                ("source_anchor".to_owned(), anchor(root, 70)),
            ]),
        },
    ];

    let document = normalize_v1(graph, build_evidence(root)?)?;
    let external_ids = document
        .nodes
        .iter()
        .filter(|node| node.name == "Shared")
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(external_ids.len(), 2);
    let targets = document
        .links
        .iter()
        .map(|edge| edge.target.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(targets.len(), 2);
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

    let document = normalize_v1(graph, build_evidence(root)?)?;
    assert!(document.links.is_empty());
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
