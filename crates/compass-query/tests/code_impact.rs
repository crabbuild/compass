mod support;

use std::fs;

use compass_model::code_graph::{EdgeKind, GraphDocument};
use compass_model::identity::edge_id;
use compass_model::query_contract::{CodeQueryLimits, ImpactRequest};
use compass_query::open;

#[test]
fn impact_walks_the_approved_reverse_family_and_gates_heuristics()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = |include_heuristic| ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic,
        limits: CodeQueryLimits::default(),
    };
    let exact = engine.impact(request(false))?;
    assert!(exact.nodes.iter().any(|node| node.id == "n:route"));
    assert!(exact.nodes.iter().any(|node| node.id == "n:dependent"));
    assert!(!exact.nodes.iter().any(|node| node.id == "n:heuristic"));
    let enriched = engine.impact(request(true))?;
    assert!(enriched.nodes.iter().any(|node| node.id == "n:heuristic"));
    assert!(enriched.paths.iter().any(|path| {
        path.weakest_resolution == compass_model::provenance::ResolutionState::Unresolved
    }));
    Ok(())
}

#[test]
fn impact_reports_bounds_as_typed_truncation() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.impact(ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: true,
        limits: CodeQueryLimits {
            max_nodes: 2,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(response.truncated);
    assert!(response.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == compass_model::query_contract::QueryDiagnosticCode::BoundedTruncation
    }));
    Ok(())
}

#[test]
fn impact_applies_edge_and_node_budgets_before_publishing_records()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.impact(ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: true,
        limits: CodeQueryLimits {
            max_nodes: 2,
            max_edges: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(response.truncated);
    assert!(response.nodes.len() <= 2);
    assert!(response.edges.len() <= 1);
    let node_ids = response
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(response.edges.iter().all(|edge| {
        node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
    }));
    Ok(())
}

#[test]
fn impact_excludes_graph_assembly_endpoint_remaps_by_default()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_endpoint_remap_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = |include_heuristic| ImpactRequest {
        symbol: "crate::Target".to_owned(),
        include_heuristic,
        limits: CodeQueryLimits::default(),
    };

    let exact = engine.impact(request(false))?;
    assert!(!exact.nodes.iter().any(|node| node.name == "Caller"));
    let enriched = engine.impact(request(true))?;
    assert!(enriched.nodes.iter().any(|node| node.name == "Caller"));
    assert!(enriched.edges.iter().any(|edge| {
        edge.evidence.iter().any(|evidence| {
            evidence.rule.as_deref() == Some("graph-ghost-endpoint-remap")
                && evidence.wiring_site.is_some()
        })
    }));
    Ok(())
}

#[test]
fn impact_includes_inbound_renderers_without_promoting_them_to_callers()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let template = graph
        .links
        .iter()
        .find(|edge| edge.source == "n:caller" && edge.target == "n:list")
        .cloned()
        .ok_or("missing renderer template")?;
    let id = edge_id(
        "n:caller",
        EdgeKind::Renders,
        "n:list",
        template.relationship_site.as_ref(),
        Some("react-jsx-render"),
    );
    let mut render = template;
    render.id.clone_from(&id);
    render.key = id;
    render.kind = EdgeKind::Renders;
    render.occurrence_rule = compass_model::provenance::OccurrenceRule::new("react-jsx-render");
    graph.links.push(render);
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let impact = engine.impact(ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(
        impact
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Renders)
    );

    let callers = engine.callers(compass_model::query_contract::CallRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(
        callers
            .edges
            .iter()
            .all(|edge| edge.kind != EdgeKind::Renders)
    );
    Ok(())
}
