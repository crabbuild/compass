use compass_model::code_graph::{
    EdgeKind, EdgeRecord, NodeKind, NodeRecord, NodeRole, SymbolNodeDetails,
};
use compass_model::identity::edge_id;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};

fn anchor() -> SourceAnchor {
    SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 10,
        end_byte: 20,
        start_line: 2,
        start_column: 0,
        end_line: 2,
        end_column: 10,
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
        score: Some(1.0),
        candidates: Vec::new(),
    }
}

#[test]
fn projections_expose_only_registered_derived_properties() {
    let node = NodeRecord {
        id: "node".to_owned(),
        kind: NodeKind::Function,
        roles: vec![NodeRole::RouteHandler],
        name: "handle".to_owned(),
        qualified_name: "crate::handle".to_owned(),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(anchor()),
        details: Some(compass_model::code_graph::NodeDetails::Symbol(
            SymbolNodeDetails {
                signature: Some("fn handle()".to_owned()),
                modifiers: Vec::new(),
                overload_discriminator: None,
                declaring_type: None,
                signature_digest: Some("sha256:signature".to_owned()),
                implementation_digest: Some("sha256:implementation".to_owned()),
                source_digest: Some("sha256:source".to_owned()),
            },
        )),
        evidence: vec![evidence()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    };
    assert_eq!(node.string("label"), "handle");
    assert_eq!(node.string("source_file"), "src/lib.rs");
    assert_eq!(node.string("signature_hash"), "sha256:signature");
    assert!(node.property("arbitrary").is_none());
    assert!(node.properties().all(|(key, _)| key != "arbitrary"));

    let relationship_id = edge_id("node", EdgeKind::Calls, "target", Some(&anchor()), None);
    let edge = EdgeRecord {
        id: relationship_id.clone(),
        key: relationship_id,
        source: "node".to_owned(),
        target: "target".to_owned(),
        kind: EdgeKind::Calls,
        occurrence_rule: None,
        relationship_site: Some(anchor()),
        details: None,
        evidence: vec![evidence()],
        weight: Some(1.0),
        context: Some("call".to_owned()),
        deferred: false,
        diagnostics: Vec::new(),
    };
    assert_eq!(edge.string("relation"), "calls");
    assert_eq!(edge.string("confidence"), "EXTRACTED");
    assert!(edge.property("arbitrary").is_none());
}
