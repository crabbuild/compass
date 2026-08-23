use std::fs;

use compass_core::{
    TaskContext, TaskContextIntent, TaskContextLimits, TaskContextRequest, TaskContextSectionKind,
    TaskContextTarget, build_task_context,
};
use compass_model::code_graph::{
    BuildMetadata, EdgeDetails, EdgeKind, EdgeRecord, ExtractionStatus, FileRecord, GraphDocument,
    NodeDetails, NodeKind, NodeRecord, RenderEdgeDetails, RenderKind, RouteNodeDetails, RouteStage,
    RouteStageDetails, SymbolNodeDetails,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, Provenance, ResolutionState, SourceAnchor,
};
use compass_query::open_with_document;
use compass_reflect::MemoryDoc;
use sha2::{Digest, Sha256};

fn anchor(path: &str) -> SourceAnchor {
    SourceAnchor {
        file: path.to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 4,
    }
}

fn provenance(path: &str) -> Provenance {
    Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "task-context-fixture".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: None,
        anchors: vec![anchor(path)],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    }
}

fn node(id: &str, name: &str, qualified_name: &str, path: &str) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        kind: NodeKind::Function,
        roles: Vec::new(),
        name: name.to_owned(),
        qualified_name: qualified_name.to_owned(),
        language: Some("rust".to_owned()),
        framework: None,
        source: Some(anchor(path)),
        details: None,
        evidence: vec![provenance(path)],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    }
}

fn edge(source: &str, kind: EdgeKind, target: &str, path: &str) -> EdgeRecord {
    let site = anchor(path);
    let id = edge_id(source, kind, target, Some(&site), None);
    EdgeRecord {
        id: id.clone(),
        key: id,
        source: source.to_owned(),
        target: target.to_owned(),
        kind,
        occurrence_rule: None,
        relationship_site: Some(site),
        details: None,
        evidence: vec![provenance(path)],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    }
}

#[test]
fn qualification_composes_verified_priority_sections_and_memory_deterministically()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source = b"code";
    for path in ["src/lib.rs", "tests/parser_test.rs"] {
        let absolute = directory.path().join(path);
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(absolute, source)?;
    }
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "sha256:schema".to_owned(),
        source_tree_digest: "sha256:tree".to_owned(),
        configuration_digest: "sha256:config".to_owned(),
        generation_id: "sha256:generation".to_owned(),
        source_commit: None,
    });
    for path in ["src/lib.rs", "tests/parser_test.rs"] {
        graph.graph.files.push(FileRecord {
            id: file_id(path),
            path: path.to_owned(),
            language: Some("rust".to_owned()),
            content_digest: format!("sha256:{:x}", Sha256::digest(source)),
            byte_size: 4,
            generated: false,
            extraction_status: ExtractionStatus::Extracted,
            extractor_versions: vec!["test".to_owned()],
            coverage: Vec::new(),
            diagnostics: Vec::new(),
        });
    }
    let mut target = node("target", "parse", "Parser::parse", "src/lib.rs");
    target.details = Some(NodeDetails::Symbol(SymbolNodeDetails {
        signature: Some("fn parse(input: &str) -> Parser".to_owned()),
        modifiers: Vec::new(),
        overload_discriminator: None,
        declaring_type: Some("Parser".to_owned()),
        signature_digest: None,
        implementation_digest: Some("sha256:implementation".to_owned()),
        source_digest: None,
    }));
    graph.nodes = vec![
        target,
        node("caller", "run", "Runner::run", "src/lib.rs"),
        node("callee", "tokenize", "Parser::tokenize", "src/lib.rs"),
        node(
            "test-target",
            "test_parse",
            "tests::test_parse",
            "tests/parser_test.rs",
        ),
    ];
    graph.links = vec![
        edge("caller", EdgeKind::Calls, "target", "src/lib.rs"),
        edge("target", EdgeKind::Calls, "callee", "src/lib.rs"),
        edge(
            "test-target",
            EdgeKind::Calls,
            "target",
            "tests/parser_test.rs",
        ),
    ];
    let graph_path = directory.path().join("graph.json");
    let engine = open_with_document(graph, &graph_path, None, &directory.path().join("cache"))?;
    let request = TaskContextRequest {
        intent: TaskContextIntent::Test,
        target: "Parser::parse".to_owned(),
        repository_root: directory.path().to_string_lossy().into_owned(),
        limits: TaskContextLimits::default(),
    };
    let memory = vec![MemoryDoc {
        query_type: "test".to_owned(),
        date: "2026-08-12".to_owned(),
        question: "How is parsing verified?".to_owned(),
        outcome: "Use the focused parser test.".to_owned(),
        correction: String::new(),
        contributor: "fixture".to_owned(),
        source_nodes: vec!["target".to_owned()],
        path: "parser.md".to_owned(),
    }];
    let first = build_task_context(&engine, &request, &memory)?;
    let second = build_task_context(&engine, &request, &memory)?;
    assert_eq!(first.result_digest, second.result_digest);
    assert_eq!(serde_json::to_vec(&first)?, serde_json::to_vec(&second)?);
    assert_eq!(
        first.target,
        TaskContextTarget::Exact {
            node_id: "target".to_owned()
        }
    );
    assert!(first.sections.iter().any(|section| {
        section.kind == TaskContextSectionKind::DeclarationSource
            && section
                .evidence
                .files
                .iter()
                .any(|file| file.source.as_deref() == Some("code"))
    }));
    assert!(first.sections.iter().any(|section| {
        section.kind == TaskContextSectionKind::ImplementationType
            && section
                .evidence
                .nodes
                .iter()
                .any(|node| node.id == "target" && node.details.is_some())
            && section
                .evidence
                .files
                .iter()
                .any(|file| file.source.as_deref() == Some("code"))
    }));
    assert!(first.sections.iter().any(|section| {
        section.kind == TaskContextSectionKind::ExactCallers
            && section
                .evidence
                .nodes
                .iter()
                .any(|node| node.id == "caller")
            && section.evidence.edges.iter().all(|edge| {
                edge.evidence
                    .iter()
                    .all(|evidence| evidence.confidence == EvidenceConfidence::Exact)
            })
    }));
    assert!(first.sections.iter().any(|section| {
        section.kind == TaskContextSectionKind::RelatedTests
            && section
                .evidence
                .nodes
                .iter()
                .any(|node| node.id == "test-target")
    }));
    assert_eq!(first.project_knowledge.len(), 1);
    let encoded = serde_json::to_vec(&first)?;
    assert_eq!(TaskContext::from_json(&encoded)?, first);
    assert_eq!(first.work.response_bytes, encoded.len() as u64);

    for (intent, has_transitive_impact) in [
        (TaskContextIntent::Explain, false),
        (TaskContextIntent::Modify, true),
        (TaskContextIntent::Debug, true),
        (TaskContextIntent::Test, true),
    ] {
        let mut qualified = request.clone();
        qualified.intent = intent;
        let context = build_task_context(&engine, &qualified, &memory)?;
        assert_eq!(
            context
                .sections
                .iter()
                .any(|section| section.kind == TaskContextSectionKind::TransitiveImpact),
            has_transitive_impact,
            "unexpected impact policy for {intent:?}"
        );
    }

    fs::write(directory.path().join("src/lib.rs"), b"changed")?;
    let stale = build_task_context(&engine, &request, &[])?;
    assert!(stale.omissions.iter().any(|omission| {
        omission.category == "verified_source" && omission.reason.contains("digest verification")
    }));
    assert!(
        stale
            .omissions
            .iter()
            .any(|omission| { omission.category == "history_project_knowledge" })
    );
    Ok(())
}

#[test]
fn framework_context_reports_render_direction_and_rejects_v1_schema()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source = b"component";
    fs::create_dir_all(directory.path().join("src"))?;
    fs::write(directory.path().join("src/ui.tsx"), source)?;
    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "sha256:schema".to_owned(),
        source_tree_digest: "sha256:tree".to_owned(),
        configuration_digest: "sha256:config".to_owned(),
        generation_id: "sha256:generation".to_owned(),
        source_commit: None,
    });
    graph.graph.files.push(FileRecord {
        id: file_id("src/ui.tsx"),
        path: "src/ui.tsx".to_owned(),
        language: Some("tsx".to_owned()),
        content_digest: format!("sha256:{:x}", Sha256::digest(source)),
        byte_size: source.len() as u64,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });
    let mut component = node("component", "Card", "Card", "src/ui.tsx");
    component.kind = NodeKind::Component;
    component.framework = Some("react".to_owned());
    component.roles = vec![compass_model::code_graph::NodeRole::UiComponent];
    let mut route = node("route", "Page", "Page", "src/ui.tsx");
    route.kind = NodeKind::Route;
    route.framework = Some("next".to_owned());
    route.details = Some(NodeDetails::Route(RouteNodeDetails {
        operation: "GET".to_owned(),
        path: "/".to_owned(),
        original_path: None,
        declaring_scope: "src/app/page.tsx".to_owned(),
        resolution: ResolutionState::Exact,
        middleware_count: 0,
        stages: vec![RouteStageDetails {
            stage: RouteStage::RouteComponent,
            position: 0,
            reference: "Page".to_owned(),
            resolution: ResolutionState::Exact,
            source_anchor: Some(anchor("src/ui.tsx")),
            target: Some("component".to_owned()),
            candidates: Vec::new(),
        }],
    }));
    let mut page_component = node(
        "page-component",
        "PageComponent",
        "PageComponent",
        "src/ui.tsx",
    );
    page_component.kind = NodeKind::Component;
    page_component.framework = Some("react".to_owned());
    let mut render = edge(
        "component",
        EdgeKind::Renders,
        "page-component",
        "src/ui.tsx",
    );
    render.details = Some(EdgeDetails::Render(RenderEdgeDetails {
        render_kind: RenderKind::Jsx,
        boundary: None,
    }));
    graph.nodes = vec![component, route, page_component];
    graph.links = vec![
        render,
        edge("route", EdgeKind::RoutesTo, "page-component", "src/ui.tsx"),
    ];
    let graph_path = directory.path().join("graph.json");
    let engine = open_with_document(graph, &graph_path, None, &directory.path().join("cache"))?;
    let context = build_task_context(
        &engine,
        &TaskContextRequest {
            intent: TaskContextIntent::Modify,
            target: "page-component".to_owned(),
            repository_root: directory.path().to_string_lossy().into_owned(),
            limits: TaskContextLimits::default(),
        },
        &[],
    )?;
    let framework = context
        .framework
        .as_ref()
        .ok_or("framework context missing")?;
    assert!(framework.packs.iter().any(|pack| pack.id == "react-ui"));
    assert!(framework
        .packs
        .iter()
        .any(|pack| pack.qualification == compass_core::FrameworkQualificationState::Qualifying));
    assert!(framework.renders.is_empty());
    assert_eq!(framework.rendered_by.len(), 1);
    assert_eq!(framework.routes.len(), 1);
    assert_eq!(
        framework.routes[0].stages[0].stage,
        RouteStage::RouteComponent
    );

    let mut encoded = serde_json::to_value(&context)?;
    encoded["schema"] = serde_json::Value::String(compass_core::TASK_CONTEXT_SCHEMA_V1.to_owned());
    let error = TaskContext::from_json(&serde_json::to_vec(&encoded)?)
        .err()
        .ok_or("v1 task context unexpectedly accepted")?;
    assert!(matches!(
        error,
        compass_core::TaskContextError::UnsupportedSchema(_)
    ));
    Ok(())
}
