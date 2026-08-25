use std::process::Command;

use compass_model::code_graph::{
    BuildMetadata, EdgeDetails, EdgeKind, EdgeRecord, ExtractionStatus, FileRecord, GraphDocument,
    NodeDetails, NodeKind, NodeRecord, NodeRole, RenderEdgeDetails, RenderKind, RouteNodeDetails,
    RouteStage, RouteStageDetails,
};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{
    EvidenceConfidence, EvidenceOrigin, Provenance, ResolutionState, SourceAnchor,
};
use sha2::{Digest, Sha256};

const SOURCE_PATH: &str = "src/ui.tsx";
const SOURCE: &[u8] = b"component";

fn anchor() -> SourceAnchor {
    SourceAnchor {
        file: SOURCE_PATH.to_owned(),
        start_byte: 0,
        end_byte: SOURCE.len() as u64,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: SOURCE.len() as u32,
    }
}

fn provenance() -> Provenance {
    Provenance {
        origin: EvidenceOrigin::Ast,
        extractor: "task-context-cli-fixture".to_owned(),
        confidence: EvidenceConfidence::Exact,
        rule: None,
        anchors: vec![anchor()],
        wiring_site: None,
        score: None,
        candidates: Vec::new(),
    }
}

fn node(id: &str, name: &str, kind: NodeKind, framework: &str) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        kind,
        roles: Vec::new(),
        name: name.to_owned(),
        qualified_name: name.to_owned(),
        language: Some("tsx".to_owned()),
        framework: Some(framework.to_owned()),
        source: Some(anchor()),
        details: None,
        evidence: vec![provenance()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
        community: None,
    }
}

fn edge(source: &str, kind: EdgeKind, target: &str) -> EdgeRecord {
    let site = anchor();
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
        evidence: vec![provenance()],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    }
}

#[test]
fn json_context_exposes_exact_react_render_and_next_route_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    std::fs::create_dir(root.join("src"))?;
    std::fs::write(root.join(SOURCE_PATH), SOURCE)?;

    let mut graph = GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "sha256:schema".to_owned(),
        source_tree_digest: "sha256:tree".to_owned(),
        configuration_digest: "sha256:config".to_owned(),
        generation_id: "sha256:task-context-cli".to_owned(),
        source_commit: None,
    });
    graph.graph.files.push(FileRecord {
        id: file_id(SOURCE_PATH),
        path: SOURCE_PATH.to_owned(),
        language: Some("tsx".to_owned()),
        content_digest: format!("sha256:{:x}", Sha256::digest(SOURCE)),
        byte_size: SOURCE.len() as u64,
        generated: false,
        extraction_status: ExtractionStatus::Extracted,
        extractor_versions: vec!["test".to_owned()],
        coverage: Vec::new(),
        diagnostics: Vec::new(),
    });

    let mut renderer = node("renderer", "Card", NodeKind::Component, "react");
    renderer.roles = vec![NodeRole::UiComponent];
    let page = node("page", "Page", NodeKind::Component, "react");
    let mut route = node("route", "/", NodeKind::Route, "next");
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
            source_anchor: Some(anchor()),
            target: Some("page".to_owned()),
            candidates: Vec::new(),
        }],
    }));
    let mut render = edge("renderer", EdgeKind::Renders, "page");
    render.details = Some(EdgeDetails::Render(RenderEdgeDetails {
        render_kind: RenderKind::Jsx,
        boundary: None,
    }));
    graph.nodes = vec![renderer, page, route];
    graph.links = vec![render, edge("route", EdgeKind::RoutesTo, "page")];

    let graph_path = root.join("graph.json");
    std::fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "context",
            "modify",
            "page",
            "--graph",
            graph_path.to_str().ok_or("non-UTF-8 graph path")?,
            "--root",
            root.to_str().ok_or("non-UTF-8 project root")?,
            "--engine",
            "json",
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let context: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(context["schema"], "compass.task-context/2");
    assert_eq!(context["target"]["state"], "exact");
    assert_eq!(context["target"]["node_id"], "page");
    let framework = &context["framework"];
    assert_eq!(framework["schema"], "compass.framework-context/1");
    assert_eq!(framework["renderedBy"].as_array().map(Vec::len), Some(1));
    assert_eq!(framework["routes"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        framework["routes"][0]["stages"][0]["stage"],
        "route_component"
    );
    assert_eq!(framework["truncated"], false);
    Ok(())
}
