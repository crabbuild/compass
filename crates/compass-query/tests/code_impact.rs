mod support;

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
