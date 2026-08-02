use compass_model::code_graph::{
    BuildMetadata, EdgeKind, EdgeRecord, FileRecord, GraphDocument, NodeKind, NodeRecord, NodeRole,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, OccurrenceRule, Provenance, ResolutionCandidate,
    SourceAnchor,
};
use compass_model::{validate_code_graph, validate_code_graph_records};

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
fn whole_document_validation_accepts_the_supported_route_shape() {
    assert!(validate_code_graph(&document()).is_ok());
}

#[test]
fn structured_validation_classifies_document_node_and_edge_failures() -> Result<(), Box<dyn Error>>
{
    let mut graph = document();
    graph.directed = false;
    let source = graph.nodes[0]
        .source
        .as_mut()
        .ok_or("fixture node must remain source-backed")?;
    source.end_byte = 101;
    graph.nodes[1].kind = NodeKind::Route;

    let report = validate_code_graph_records(&graph);

    assert_eq!(report.document_errors, vec!["directed must be true"]);
    assert_eq!(report.node_errors.len(), 1);
    assert_eq!(report.node_errors[0].id, "route");
    assert!(
        report.node_errors[0]
            .errors
            .iter()
            .any(|error| error.contains("source anchor exceeds"))
    );
    assert_eq!(report.edge_errors.len(), 1);
    assert_eq!(report.edge_errors[0].id, graph.links[0].id);
    assert!(
        report.edge_errors[0]
            .errors
            .iter()
            .any(|error| error.contains("invalid routes_to"))
    );
    Ok(())
}

#[test]
fn structured_validation_preserves_strict_error_order() -> Result<(), Box<dyn Error>> {
    let mut graph = document();
    graph.multigraph = false;
    graph.nodes[0].name.clear();
    graph.nodes[1].kind = NodeKind::Route;

    let report = validate_code_graph_records(&graph);
    let mut expected = report.document_errors.clone();
    expected.extend(
        report
            .node_errors
            .iter()
            .flat_map(|record| record.errors.iter().cloned()),
    );
    expected.extend(
        report
            .edge_errors
            .iter()
            .flat_map(|record| record.errors.iter().cloned()),
    );

    let errors = validate_code_graph(&graph)
        .err()
        .ok_or("invalid fixture unexpectedly passed strict validation")?
        .errors;
    assert_eq!(
        errors, expected,
        "strict validation must remain an ordered projection of the report"
    );
    Ok(())
}

#[test]
fn structured_validation_is_empty_for_a_valid_document() {
    let report = validate_code_graph_records(&document());
    assert!(report.is_valid());
    assert!(report.document_errors.is_empty());
    assert!(report.node_errors.is_empty());
    assert!(report.edge_errors.is_empty());
}

#[test]
fn relationship_identity_uses_the_typed_occurrence_rule_not_sorted_evidence() {
    let mut graph = document();
    graph.links[0].evidence[0].rule = Some("alphabetically-first-endpoint-rewrite".to_owned());
    graph.links[0].occurrence_rule = OccurrenceRule::new("producer-rule");
    let id = edge_id(
        "route",
        EdgeKind::RoutesTo,
        "handler",
        Some(&anchor()),
        Some("producer-rule"),
    );
    graph.links[0].id.clone_from(&id);
    graph.links[0].key = id;

    assert!(validate_code_graph(&graph).is_ok());
}

#[test]
fn typed_occurrence_identity_reserves_every_closed_rewrite_name() {
    for rule in CLOSED_ENDPOINT_REWRITE_RULES {
        let mut graph = document();
        graph.links[0].occurrence_rule = OccurrenceRule::new(rule);
        let id = edge_id(
            "route",
            EdgeKind::RoutesTo,
            "handler",
            Some(&anchor()),
            Some(rule),
        );
        graph.links[0].id.clone_from(&id);
        graph.links[0].key = id;
        assert!(
            validate_code_graph(&graph).is_err(),
            "closed occurrence rule {rule} was accepted"
        );
    }

    let mut graph = document();
    graph.links[0].occurrence_rule = OccurrenceRule::new("future-endpoint-remap");
    let id = edge_id(
        "route",
        EdgeKind::RoutesTo,
        "handler",
        Some(&anchor()),
        Some("future-endpoint-remap"),
    );
    graph.links[0].id.clone_from(&id);
    graph.links[0].key = id;
    assert!(validate_code_graph(&graph).is_ok());
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
fn invalid_endpoint_diagnostics_name_both_symbols_and_the_relationship_site() {
    let mut graph = document();
    graph.nodes[0].kind = NodeKind::Method;
    graph.nodes[1].kind = NodeKind::Interface;
    graph.links[0].kind = EdgeKind::Calls;
    let id = edge_id("route", EdgeKind::Calls, "handler", Some(&anchor()), None);
    graph.links[0].id.clone_from(&id);
    graph.links[0].key = id;

    let errors = validate_code_graph(&graph)
        .err()
        .map(|error| error.errors)
        .unwrap_or_default();
    assert!(errors.iter().any(|error| {
        error.contains("invalid calls endpoints method -> interface")
            && error.contains("source=route")
            && error.contains("target=handler")
            && error.contains("site=src/lib.rs:1:0")
    }));
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
fn top_level_instantiations_accept_file_and_module_sources() {
    for source_kind in [NodeKind::File, NodeKind::Module] {
        let mut graph = document();
        graph.nodes[0].kind = source_kind;
        graph.nodes[1].kind = NodeKind::Class;
        graph.links[0].kind = EdgeKind::Instantiates;
        let id = edge_id(
            "route",
            EdgeKind::Instantiates,
            "handler",
            Some(&anchor()),
            None,
        );
        graph.links[0].id.clone_from(&id);
        graph.links[0].key = id;
        assert!(
            validate_code_graph(&graph).is_ok(),
            "rejected intentional {source_kind:?} -> class top-level instantiation"
        );
    }
}

#[test]
fn declaration_initializers_accept_calls_and_instantiations() {
    for (kind, source_kind, target_kind) in [
        (EdgeKind::Calls, NodeKind::Variable, NodeKind::Function),
        (EdgeKind::Calls, NodeKind::Field, NodeKind::Method),
        (EdgeKind::Calls, NodeKind::Constant, NodeKind::Function),
        (EdgeKind::Instantiates, NodeKind::Variable, NodeKind::Class),
        (EdgeKind::Instantiates, NodeKind::Field, NodeKind::Class),
        (EdgeKind::Instantiates, NodeKind::Constant, NodeKind::Class),
        (EdgeKind::Calls, NodeKind::EnumMember, NodeKind::Method),
        (
            EdgeKind::Instantiates,
            NodeKind::EnumMember,
            NodeKind::Class,
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
            validate_code_graph(&graph).is_ok(),
            "rejected initializer relationship {kind:?} {source_kind:?} -> {target_kind:?}"
        );
    }
}

#[test]
fn language_specific_declarations_use_supported_endpoint_shapes() {
    for (kind, source_kind, target_kind) in [
        (EdgeKind::Imports, NodeKind::File, NodeKind::EnumMember),
        (EdgeKind::Imports, NodeKind::File, NodeKind::Annotation),
        (EdgeKind::Imports, NodeKind::File, NodeKind::Field),
        (EdgeKind::References, NodeKind::Method, NodeKind::Annotation),
        (
            EdgeKind::References,
            NodeKind::Annotation,
            NodeKind::Annotation,
        ),
        (EdgeKind::References, NodeKind::Method, NodeKind::EnumMember),
        (
            EdgeKind::References,
            NodeKind::EnumMember,
            NodeKind::Annotation,
        ),
        (EdgeKind::References, NodeKind::Struct, NodeKind::Parameter),
        (EdgeKind::Imports, NodeKind::File, NodeKind::Annotation),
        (EdgeKind::Imports, NodeKind::File, NodeKind::Field),
        (EdgeKind::Contains, NodeKind::EnumMember, NodeKind::Method),
        (EdgeKind::Contains, NodeKind::Field, NodeKind::Method),
        (
            EdgeKind::Instantiates,
            NodeKind::Method,
            NodeKind::EnumMember,
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
            validate_code_graph(&graph).is_ok(),
            "rejected Rust {kind:?} {source_kind:?} -> {target_kind:?}"
        );
    }
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
        (EdgeKind::Embeds, NodeKind::Struct, NodeKind::Function),
        (EdgeKind::Contains, NodeKind::Database, NodeKind::Function),
        (EdgeKind::Contains, NodeKind::Function, NodeKind::Queue),
        (EdgeKind::Calls, NodeKind::File, NodeKind::Class),
        (EdgeKind::Imports, NodeKind::Variable, NodeKind::Function),
        (EdgeKind::Imports, NodeKind::ConfigKey, NodeKind::Method),
        (EdgeKind::Imports, NodeKind::File, NodeKind::Parameter),
        (EdgeKind::Exports, NodeKind::File, NodeKind::Parameter),
        (EdgeKind::Returns, NodeKind::Function, NodeKind::File),
        (EdgeKind::Aliases, NodeKind::Import, NodeKind::Route),
        (EdgeKind::Registers, NodeKind::Function, NodeKind::Parameter),
        (EdgeKind::References, NodeKind::Parameter, NodeKind::Route),
        (
            EdgeKind::DependsOn,
            NodeKind::Parameter,
            NodeKind::Parameter,
        ),
        (EdgeKind::Extends, NodeKind::Class, NodeKind::Function),
        (EdgeKind::Implements, NodeKind::Class, NodeKind::Class),
        (EdgeKind::TypeOf, NodeKind::Variable, NodeKind::Function),
        (EdgeKind::Reads, NodeKind::Route, NodeKind::Class),
        (EdgeKind::Handles, NodeKind::Variable, NodeKind::Event),
        (EdgeKind::Publishes, NodeKind::Variable, NodeKind::Message),
        (
            EdgeKind::Publishes,
            NodeKind::DatabaseView,
            NodeKind::Message,
        ),
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
fn endpoint_matrix_closes_relationships_that_require_both_endpoint_shapes() {
    for (kind, source_kind, source_roles, target_kind) in [
        (
            EdgeKind::Contains,
            NodeKind::Class,
            Vec::new(),
            NodeKind::DatabaseTable,
        ),
        (
            EdgeKind::TypeOf,
            NodeKind::Function,
            Vec::new(),
            NodeKind::Class,
        ),
        (
            EdgeKind::Tests,
            NodeKind::Function,
            Vec::new(),
            NodeKind::Function,
        ),
        (
            EdgeKind::Tests,
            NodeKind::Function,
            vec![NodeRole::Test],
            NodeKind::Parameter,
        ),
        (
            EdgeKind::Documents,
            NodeKind::Resource,
            Vec::new(),
            NodeKind::Parameter,
        ),
        (
            EdgeKind::Decorates,
            NodeKind::Function,
            Vec::new(),
            NodeKind::Annotation,
        ),
    ] {
        let mut graph = document();
        graph.nodes[0].kind = source_kind;
        graph.nodes[0].roles = source_roles;
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
fn tests_edge_accepts_an_explicit_test_role_and_testable_target() {
    for target_kind in [NodeKind::Function, NodeKind::EnumMember] {
        let mut graph = document();
        graph.nodes[0].kind = NodeKind::Function;
        graph.nodes[0].roles = vec![NodeRole::Test];
        graph.nodes[1].kind = target_kind;
        graph.links[0].kind = EdgeKind::Tests;
        let id = edge_id("route", EdgeKind::Tests, "handler", Some(&anchor()), None);
        graph.links[0].id.clone_from(&id);
        graph.links[0].key = id;
        assert!(
            validate_code_graph(&graph).is_ok(),
            "rejected test target {target_kind:?}"
        );
    }
}

#[test]
fn endpoint_matrix_accepts_nested_dynamic_and_database_producer_shapes() {
    for (kind, source_kind, target_kind) in [
        (EdgeKind::Embeds, NodeKind::Struct, NodeKind::Interface),
        (EdgeKind::Contains, NodeKind::Function, NodeKind::Method),
        (EdgeKind::Contains, NodeKind::Method, NodeKind::Class),
        (EdgeKind::Contains, NodeKind::TypeAlias, NodeKind::Method),
        (EdgeKind::Calls, NodeKind::Class, NodeKind::Method),
        (EdgeKind::Calls, NodeKind::Variable, NodeKind::Function),
        (EdgeKind::Calls, NodeKind::Struct, NodeKind::Method),
        (EdgeKind::Calls, NodeKind::TypeAlias, NodeKind::Function),
        (EdgeKind::Calls, NodeKind::Enum, NodeKind::Function),
        (EdgeKind::Calls, NodeKind::Function, NodeKind::Variable),
        (EdgeKind::Calls, NodeKind::Function, NodeKind::Import),
        (EdgeKind::Calls, NodeKind::Function, NodeKind::TypeAlias),
        (EdgeKind::References, NodeKind::Method, NodeKind::Annotation),
        (EdgeKind::References, NodeKind::Function, NodeKind::Macro),
        (EdgeKind::References, NodeKind::Module, NodeKind::Macro),
        (EdgeKind::References, NodeKind::Macro, NodeKind::Macro),
        (EdgeKind::Imports, NodeKind::File, NodeKind::Variable),
        (
            EdgeKind::Triggers,
            NodeKind::DatabaseTrigger,
            NodeKind::DatabaseTable,
        ),
        (
            EdgeKind::Contains,
            NodeKind::Database,
            NodeKind::DatabaseIndex,
        ),
        (
            EdgeKind::Contains,
            NodeKind::Database,
            NodeKind::DatabaseTrigger,
        ),
        (EdgeKind::Contains, NodeKind::File, NodeKind::Database),
    ] {
        let mut graph = document();
        graph.nodes[0].kind = source_kind;
        graph.nodes[1].kind = target_kind;
        graph.links[0].kind = kind;
        let id = edge_id("route", kind, "handler", Some(&anchor()), None);
        graph.links[0].id.clone_from(&id);
        graph.links[0].key = id;
        assert!(
            validate_code_graph(&graph).is_ok(),
            "{kind:?} rejected producer shape {source_kind:?} -> {target_kind:?}"
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

#[test]
fn typed_endpoint_rewrites_require_heuristic_inferred_bounded_indirect_evidence() {
    let valid = Provenance {
        origin: EvidenceOrigin::Heuristic,
        extractor: "test.incremental".to_owned(),
        confidence: EvidenceConfidence::Inferred,
        rule: Some("incremental-ast-endpoint-remap".to_owned()),
        anchors: Vec::new(),
        wiring_site: Some(anchor()),
        score: Some(0.75),
        candidates: Vec::new(),
    };
    assert!(valid.validate().is_ok());

    let mut non_finite = valid;
    non_finite.score = Some(f64::NAN);
    assert!(non_finite.validate().is_err());
}

#[test]
fn unknown_rewrite_like_names_remain_valid_open_ended_producer_rules() {
    let mut graph = document();
    graph.links[0].evidence[0].rule = Some("future-endpoint-remap".to_owned());
    assert!(validate_code_graph(&graph).is_ok());
}
use std::error::Error;
