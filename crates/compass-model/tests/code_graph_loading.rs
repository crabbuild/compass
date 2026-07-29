use std::fs;

use compass_model::GraphError;
use compass_model::code_graph::{
    BuildMetadata, ExtractionStatus, FileRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::identity::file_id;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};

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
    assert!(
        cache_entries[0]
            .file_name()
            .to_string_lossy()
            .contains(".graph.json.")
    );
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
