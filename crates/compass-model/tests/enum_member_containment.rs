use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, ExtractionStatus, FileRecord, GraphDocument, NodeKind,
    NodeRecord,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
use compass_model::validate_code_graph;

fn anchor() -> SourceAnchor {
    SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 4,
    }
}

fn evidence() -> Provenance {
    Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "test".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: None,
        anchors: vec![anchor()],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    }
}

fn node(id: &str, kind: NodeKind) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        kind,
        roles: Vec::new(),
        name: id.to_owned(),
        qualified_name: id.to_owned(),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(anchor()),
        details: None,
        evidence: vec![evidence()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    }
}

fn containment_document(owner_kind: NodeKind) -> GraphDocument {
    let mut document = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "schema".to_owned(),
        source_tree_digest: "tree".to_owned(),
        configuration_digest: "config".to_owned(),
        generation_id: "generation".to_owned(),
        source_commit: None,
    });
    document.graph.files.push(FileRecord {
        id: file_id("src/lib.rs"),
        path: "src/lib.rs".to_owned(),
        language: Some("rust".to_owned()),
        content_digest: "sha256:test".to_owned(),
        byte_size: 100,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    document.nodes = vec![
        node("owner", owner_kind),
        node("member", NodeKind::EnumMember),
    ];
    let relationship_id = edge_id("owner", EdgeKind::Contains, "member", Some(&anchor()), None);
    document.links.push(EdgeRecord {
        id: relationship_id.clone(),
        key: relationship_id,
        source: "owner".to_owned(),
        target: "member".to_owned(),
        kind: EdgeKind::Contains,
        occurrence_rule: None,
        relationship_site: Some(anchor()),
        details: None,
        evidence: vec![evidence()],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    });
    document
}

#[test]
fn enum_can_contain_enum_member() {
    assert!(validate_code_graph(&containment_document(NodeKind::Enum)).is_ok());
}

#[test]
fn unrelated_owner_cannot_contain_enum_member() -> Result<(), &'static str> {
    let error = match validate_code_graph(&containment_document(NodeKind::Route)) {
        Err(error) => error,
        Ok(()) => return Err("route must not own an enum member"),
    };
    let errors = error.errors;

    assert!(
        errors
            .iter()
            .any(|error| error.contains("invalid contains endpoints route -> enum_member"))
    );
    Ok(())
}
