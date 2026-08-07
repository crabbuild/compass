use std::fs;

use compass_model::GraphError;
use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, ExtractionStatus, FileRecord, GraphDocument, NodeKind,
    NodeRecord,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, OccurrenceRule, Provenance, SourceAnchor,
};

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

fn document() -> GraphDocument {
    GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "schema".to_owned(),
        source_tree_digest: "tree".to_owned(),
        configuration_digest: "config".to_owned(),
        generation_id: "generation".to_owned(),
        source_commit: None,
    })
}

fn endpoint_rewrite_evidence() -> Provenance {
    Provenance {
        origin: EvidenceOrigin::Heuristic,
        extractor: "test.incremental".to_owned(),
        confidence: EvidenceConfidence::Inferred,
        rule: Some("incremental-ast-endpoint-remap".to_owned()),
        anchors: Vec::new(),
        wiring_site: Some(SourceAnchor {
            file: "src/lib.rs".to_owned(),
            start_byte: 0,
            end_byte: 4,
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 4,
        }),
        score: Some(0.75),
        candidates: Vec::new(),
    }
}

fn document_with_evidence(evidence: Provenance) -> GraphDocument {
    let mut graph = document();
    graph.graph.files.push(FileRecord {
        id: file_id("src/lib.rs"),
        path: "src/lib.rs".to_owned(),
        language: Some("rust".to_owned()),
        content_digest: "sha256:test".to_owned(),
        byte_size: 4,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    graph.nodes.push(NodeRecord {
        id: "node".to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: "node".to_owned(),
        qualified_name: "crate::node".to_owned(),
        language: Some("rust".to_owned()),
        framework: None,
        source: None,
        details: None,
        evidence: vec![evidence],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    });
    graph
}

fn document_with_occurrence(rule: &str) -> Result<GraphDocument, &'static str> {
    let mut graph = document_with_evidence(Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "test.ast".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: Some("producer-evidence".to_owned()),
        anchors: vec![
            endpoint_rewrite_evidence()
                .wiring_site
                .ok_or("missing anchor")?,
        ],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    });
    let mut target = graph.nodes[0].clone();
    target.id = "target".to_owned();
    target.name = "target".to_owned();
    target.qualified_name = "crate::target".to_owned();
    graph.nodes.push(target);
    let occurrence = OccurrenceRule::new(rule.to_owned()).ok_or("invalid occurrence rule")?;
    let relationship_site = endpoint_rewrite_evidence()
        .wiring_site
        .ok_or("missing relationship site")?;
    let id = edge_id(
        "node",
        EdgeKind::Calls,
        "target",
        Some(&relationship_site),
        Some(rule),
    );
    graph.links.push(EdgeRecord {
        id: id.clone(),
        key: id,
        source: "node".to_owned(),
        target: "target".to_owned(),
        kind: EdgeKind::Calls,
        occurrence_rule: Some(occurrence),
        relationship_site: Some(relationship_site),
        details: None,
        evidence: graph.nodes[0].evidence.clone(),
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    });
    graph.multigraph = true;
    Ok(graph)
}

#[test]
fn strict_loading_rejects_pre_contract_and_unknown_graphs() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    fs::write(&graph_path, r#"{"nodes":[],"links":[]}"#)?;
    assert!(matches!(
        GraphDocument::load(&graph_path),
        Err(GraphError::UnsupportedGraphSchema { found: None })
    ));

    fs::write(
        &graph_path,
        r#"{"directed":true,"multigraph":true,"graph":{"schema":"compass.graph/2"},"nodes":[],"links":[]}"#,
    )?;
    assert!(matches!(
        GraphDocument::load(&graph_path),
        Err(GraphError::UnsupportedGraphSchema { found: Some(schema) }) if schema == "compass.graph/2"
    ));
    Ok(())
}

#[test]
fn strict_loading_uses_a_content_addressed_validated_cache()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    fs::write(&graph_path, serde_json::to_vec(&document())?)?;

    assert_eq!(GraphDocument::load(&graph_path)?, document());
    let cache_entries =
        fs::read_dir(directory.path().join("cache"))?.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(cache_entries.len(), 1);
    let cache_name = cache_entries[0].file_name();
    let cache_name = cache_name.to_string_lossy();
    assert!(cache_name.starts_with("graph.json."));
    assert!(cache_name.ends_with(".content-v1.cache"));
    assert!(!cache_name.contains("compass"));
    assert_eq!(GraphDocument::load(&graph_path)?, document());
    Ok(())
}

#[test]
fn strict_loading_rejects_spoofed_or_incomplete_typed_endpoint_rewrites()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    let valid = endpoint_rewrite_evidence();
    let mut ast_exact = valid.clone();
    ast_exact.origin = EvidenceOrigin::Ast;
    ast_exact.confidence = EvidenceConfidence::Exact;
    ast_exact.anchors = vec![valid.wiring_site.clone().ok_or("missing wiring site")?];
    ast_exact.wiring_site = None;
    let mut missing_site = valid.clone();
    missing_site.wiring_site = None;
    let mut missing_score = valid.clone();
    missing_score.score = None;
    let mut out_of_range_score = valid.clone();
    out_of_range_score.score = Some(1.01);
    let mut direct_anchor = valid;
    direct_anchor.anchors.push(
        direct_anchor
            .wiring_site
            .clone()
            .ok_or("missing wiring site")?,
    );

    for (case, evidence) in [
        ("AST/exact spoof", ast_exact),
        ("missing wiring site", missing_site),
        ("missing score", missing_score),
        ("out-of-range score", out_of_range_score),
        ("direct anchor", direct_anchor),
    ] {
        let graph = document_with_evidence(evidence);
        fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
        assert!(
            GraphDocument::load(&graph_path).is_err(),
            "{case} endpoint rewrite was accepted"
        );
    }
    Ok(())
}

#[test]
fn strict_loading_reserves_every_closed_rewrite_name_from_occurrence_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    for rule in CLOSED_ENDPOINT_REWRITE_RULES {
        fs::write(
            &graph_path,
            serde_json::to_vec(&document_with_occurrence(rule)?)?,
        )?;
        assert!(
            GraphDocument::load(&graph_path).is_err(),
            "closed occurrence rule {rule} was accepted"
        );
    }

    let arbitrary = document_with_occurrence("future-endpoint-remap")?;
    fs::write(&graph_path, serde_json::to_vec(&arbitrary)?)?;
    assert_eq!(GraphDocument::load(&graph_path)?, arbitrary);
    Ok(())
}
