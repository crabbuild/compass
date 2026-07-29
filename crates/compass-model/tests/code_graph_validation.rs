use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, FileRecord, GraphDocument, NodeKind, NodeRecord,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, Provenance, ResolutionCandidate, SourceAnchor,
};
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

fn document() -> GraphDocument {
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
        extraction_status: compass_model::code_graph::ExtractionStatus::Extracted,
        extractor_versions: vec!["test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    document.nodes = vec![
        node("route", NodeKind::Route),
        node("handler", NodeKind::Function),
    ];
    let relationship_id = edge_id(
        "route",
        EdgeKind::RoutesTo,
        "handler",
        Some(&anchor()),
        None,
    );
    document.links.push(EdgeRecord {
        id: relationship_id.clone(),
        key: relationship_id,
        source: "route".to_owned(),
        target: "handler".to_owned(),
        kind: EdgeKind::RoutesTo,
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
fn whole_document_validation_accepts_the_supported_route_shape() {
    assert!(validate_code_graph(&document()).is_ok());
}

#[test]
fn whole_document_validation_rejects_duplicate_missing_and_invalid_endpoints() {
    let mut document = document();
    document.nodes.push(node("handler", NodeKind::Function));
    document.links[0].target = "missing".to_owned();
    let errors = validate_code_graph(&document)
        .err()
        .map(|error| error.errors)
        .unwrap_or_default();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("duplicate node ID"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("does not match a node"))
    );
}

#[test]
fn whole_document_validation_rejects_an_invalid_endpoint_kind_pair() {
    let mut document = document();
    document.nodes[1].kind = NodeKind::Route;
    let errors = validate_code_graph(&document)
        .err()
        .map(|error| error.errors)
        .unwrap_or_default();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("invalid routes_to"))
    );
}

#[test]
fn top_level_calls_accept_file_and_module_sources_but_require_callable_targets() {
    for source_kind in [NodeKind::File, NodeKind::Module] {
        let mut graph = document();
        graph.nodes[0].kind = source_kind;
        graph.links[0].kind = EdgeKind::Calls;
        let id = edge_id("route", EdgeKind::Calls, "handler", Some(&anchor()), None);
        graph.links[0].id.clone_from(&id);
        graph.links[0].key = id;
        assert!(
            validate_code_graph(&graph).is_ok(),
            "rejected intentional {source_kind:?} -> function top-level call"
        );
    }

    let mut graph = document();
    graph.nodes[0].kind = NodeKind::File;
    graph.nodes[1].kind = NodeKind::Class;
    graph.links[0].kind = EdgeKind::Calls;
    let id = edge_id("route", EdgeKind::Calls, "handler", Some(&anchor()), None);
    graph.links[0].id.clone_from(&id);
    graph.links[0].key = id;
    assert!(
        validate_code_graph(&graph).is_err(),
        "top-level calls must still target a callable node"
    );
}

#[test]
fn non_recursive_self_loops_are_rejected_but_recursive_calls_are_valid() {
    let mut graph = document();
    graph.nodes = vec![node("handler", NodeKind::Function)];
    graph.links[0].source = "handler".to_owned();
    graph.links[0].target = "handler".to_owned();
    graph.links[0].kind = EdgeKind::Calls;
    let id = edge_id("handler", EdgeKind::Calls, "handler", Some(&anchor()), None);
    graph.links[0].id.clone_from(&id);
    graph.links[0].key = id;
    assert!(validate_code_graph(&graph).is_ok());

    graph.links[0].kind = EdgeKind::Imports;
    let id = edge_id(
        "handler",
        EdgeKind::Imports,
        "handler",
        Some(&anchor()),
        None,
    );
    graph.links[0].id.clone_from(&id);
    graph.links[0].key = id;
    let errors = validate_code_graph(&graph)
        .err()
        .map(|error| error.errors)
        .unwrap_or_default();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("unsupported self-loop"))
    );
}

#[test]
fn endpoint_matrix_rejects_invalid_pairs_across_relationship_families() {
    for (kind, source_kind, target_kind) in [
        (EdgeKind::Contains, NodeKind::Function, NodeKind::File),
        (EdgeKind::Calls, NodeKind::File, NodeKind::Class),
        (EdgeKind::Imports, NodeKind::Variable, NodeKind::Function),
        (EdgeKind::Extends, NodeKind::Class, NodeKind::Function),
        (EdgeKind::Implements, NodeKind::Class, NodeKind::Class),
        (EdgeKind::TypeOf, NodeKind::Variable, NodeKind::Function),
        (EdgeKind::Instantiates, NodeKind::File, NodeKind::Class),
        (EdgeKind::Reads, NodeKind::Route, NodeKind::Class),
        (EdgeKind::Handles, NodeKind::Variable, NodeKind::Event),
        (EdgeKind::Publishes, NodeKind::Variable, NodeKind::Message),
        (EdgeKind::Schedules, NodeKind::Variable, NodeKind::Job),
        (EdgeKind::Documents, NodeKind::Variable, NodeKind::Class),
        (
            EdgeKind::MapsTo,
            NodeKind::Function,
            NodeKind::DatabaseTable,
        ),
    ] {
        let mut graph = document();
        graph.nodes[0].kind = source_kind;
        graph.nodes[1].kind = target_kind;
        graph.links[0].kind = kind;
        let id = edge_id("route", kind, "handler", Some(&anchor()), None);
        graph.links[0].id.clone_from(&id);
        graph.links[0].key = id;
        assert!(
            validate_code_graph(&graph).is_err(),
            "{kind:?} accepted {source_kind:?} -> {target_kind:?}"
        );
    }
}

#[test]
fn heuristic_and_ambiguous_provenance_require_auditable_evidence() {
    let invalid = Provenance {
        origin: EvidenceOrigin::Heuristic,
        extractor: "test".to_owned(),
        confidence: EvidenceConfidence::Ambiguous,
        rule: Some("dynamic".to_owned()),
        anchors: Vec::new(),
        wiring_site: None,
        score: None,
        candidates: vec![ResolutionCandidate {
            node_id: "only".to_owned(),
            reason: "one candidate".to_owned(),
            confidence: EvidenceConfidence::Ambiguous,
            score: None,
            anchor: None,
        }],
    };
    assert!(invalid.validate().is_err());

    let too_many = Provenance {
        origin: EvidenceOrigin::Heuristic,
        extractor: "test".to_owned(),
        confidence: EvidenceConfidence::Ambiguous,
        rule: Some("dynamic".to_owned()),
        anchors: Vec::new(),
        wiring_site: Some(anchor()),
        score: None,
        candidates: (0..21)
            .map(|index| ResolutionCandidate {
                node_id: format!("{index:02}"),
                reason: "bounded candidate".to_owned(),
                confidence: EvidenceConfidence::Ambiguous,
                score: None,
                anchor: None,
            })
            .collect(),
    };
    assert!(too_many.validate().is_err());
}
