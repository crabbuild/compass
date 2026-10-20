use compass_model::code_graph::{
    EdgeKind, EdgeRecord, NodeDetails, NodeKind, NodeRecord, ResourceKind, ResourceNodeDetails,
};
use compass_model::identity::edge_id;
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, LEADING_DOCUMENTATION_RULE, OccurrenceRule, Provenance,
    SourceAnchor, is_proven_leading_documentation,
};

fn records() -> (NodeRecord, NodeRecord, EdgeRecord) {
    let excerpt = SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 8,
        start_line: 1,
        start_column: 0,
        end_line: 2,
        end_column: 0,
    };
    let definition = SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 8,
        end_byte: 25,
        start_line: 2,
        start_column: 0,
        end_line: 2,
        end_column: 17,
    };
    let proof = Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "compass.resolve.rust.universal".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: Some(LEADING_DOCUMENTATION_RULE.to_owned()),
        anchors: vec![excerpt.clone(), definition.clone()],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    };
    let document = NodeRecord {
        id: "documentation".to_owned(),
        kind: NodeKind::Resource,
        roles: Vec::new(),
        name: "Documentation for work".to_owned(),
        qualified_name: "sample.work::doc".to_owned(),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(excerpt.clone()),
        details: Some(NodeDetails::Resource(ResourceNodeDetails {
            resource_kind: ResourceKind::Rationale,
            uri: None,
            media_type: None,
            content: Some("/// doc\n".to_owned()),
        })),
        evidence: vec![proof.clone()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    };
    let mut owner = document.clone();
    owner.id = "owner".to_owned();
    owner.kind = NodeKind::Function;
    owner.name = "work".to_owned();
    owner.qualified_name = "sample.work".to_owned();
    owner.source = Some(definition);
    owner.details = None;
    let id = edge_id(
        &document.id,
        EdgeKind::Documents,
        &owner.id,
        Some(&excerpt),
        Some(LEADING_DOCUMENTATION_RULE),
    );
    let edge = EdgeRecord {
        id: id.clone(),
        key: id,
        source: document.id.clone(),
        target: owner.id.clone(),
        kind: EdgeKind::Documents,
        occurrence_rule: OccurrenceRule::new(LEADING_DOCUMENTATION_RULE),
        relationship_site: Some(excerpt),
        details: None,
        evidence: vec![proof],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    };
    (document, owner, edge)
}

#[test]
fn two_exact_ast_anchors_establish_leading_ownership() {
    let (document, owner, mut edge) = records();
    assert!(is_proven_leading_documentation(&document, &owner, &edge));
    edge.evidence[0].anchors.reverse();
    assert!(is_proven_leading_documentation(&document, &owner, &edge));
}

#[test]
fn generic_heuristic_incomplete_and_wrong_owner_proofs_are_rejected() {
    let (document, owner, edge) = records();
    let mut variants = Vec::new();
    let mut other = edge.clone();
    other.occurrence_rule = None;
    variants.push(other);
    let mut other = edge.clone();
    other.evidence[0].rule = None;
    variants.push(other);
    let mut other = edge.clone();
    other.evidence[0].origin = EvidenceOrigin::Heuristic;
    variants.push(other);
    let mut other = edge.clone();
    other.evidence[0].confidence = EvidenceConfidence::Inferred;
    variants.push(other);
    let mut other = edge.clone();
    other.evidence[0].anchors.pop();
    variants.push(other);
    let mut other = edge.clone();
    other.evidence[0].anchors[1].start_byte += 1;
    variants.push(other);
    let mut other = edge.clone();
    other.relationship_site = None;
    variants.push(other);
    let mut other = edge.clone();
    other.source = "other".to_owned();
    variants.push(other);
    let mut other = edge.clone();
    other.target = "same_name_other_owner".to_owned();
    variants.push(other);
    for variant in variants {
        assert!(!is_proven_leading_documentation(
            &document, &owner, &variant
        ));
    }
    let mut wrong_owner = owner.clone();
    wrong_owner.source = document.source.clone();
    assert!(!is_proven_leading_documentation(
        &document,
        &wrong_owner,
        &edge
    ));
    let mut wrong_content = document.clone();
    if let Some(NodeDetails::Resource(details)) = &mut wrong_content.details {
        details.content = Some("short".to_owned());
    }
    assert!(!is_proven_leading_documentation(
        &wrong_content,
        &owner,
        &edge
    ));
}

#[test]
fn strict_graph_validation_rejects_a_claimed_rule_without_owner_proof()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::{BuildMetadata, ExtractionStatus, FileRecord, GraphDocument};
    let (document, owner, edge) = records();
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "schema".to_owned(),
        source_tree_digest: "tree".to_owned(),
        configuration_digest: "config".to_owned(),
        generation_id: "generation".to_owned(),
        source_commit: None,
    });
    graph.graph.files.push(FileRecord {
        id: compass_model::identity::file_id("src/lib.rs"),
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
    graph.nodes = vec![document, owner];
    graph.links = vec![edge];
    compass_model::validate_code_graph(&graph)?;
    graph.links[0].evidence[0].anchors.pop();
    let report = compass_model::validate_code_graph_records(&graph);
    assert!(!report.is_valid());
    assert!(
        report
            .edge_errors
            .iter()
            .flat_map(|record| &record.errors)
            .any(|error| error.contains("invalid leading documentation ownership proof"))
    );
    Ok(())
}

#[test]
fn contradictory_or_missing_node_evidence_cannot_be_promoted_by_an_exact_edge() {
    let (document, owner, edge) = records();
    let mut documents = Vec::new();
    let mut other = document.clone();
    other.evidence.clear();
    documents.push(other);
    let mut other = document.clone();
    other.evidence[0].confidence = EvidenceConfidence::Inferred;
    documents.push(other);
    let mut other = document.clone();
    other.evidence[0].origin = EvidenceOrigin::Config;
    documents.push(other);
    let mut other = document.clone();
    other.evidence[0].anchors.remove(0);
    documents.push(other);
    for other in documents {
        assert!(!is_proven_leading_documentation(&other, &owner, &edge));
    }
    let mut owners = Vec::new();
    let mut other = owner.clone();
    other.evidence.clear();
    owners.push(other);
    let mut other = owner.clone();
    other.evidence[0].origin = EvidenceOrigin::Config;
    owners.push(other);
    let mut other = owner.clone();
    other.evidence[0].confidence = EvidenceConfidence::Inferred;
    owners.push(other);
    let mut other = owner.clone();
    other.evidence[0].anchors.pop();
    owners.push(other);
    for other in owners {
        assert!(!is_proven_leading_documentation(&document, &other, &edge));
    }
    let mut heuristic = edge.evidence[0].clone();
    heuristic.origin = EvidenceOrigin::Heuristic;
    heuristic.confidence = EvidenceConfidence::Inferred;
    heuristic.rule = Some("semantic-text-supplement".to_owned());
    heuristic.wiring_site = document.source.clone();
    heuristic.anchors.clear();
    assert!(heuristic.validate().is_ok());
    let mut mixed_document = document.clone();
    mixed_document.evidence.push(heuristic.clone());
    assert!(!is_proven_leading_documentation(
        &mixed_document,
        &owner,
        &edge
    ));
    let mut mixed_edge = edge;
    mixed_edge.evidence.push(heuristic);
    assert!(!is_proven_leading_documentation(
        &document,
        &owner,
        &mixed_edge
    ));
}
