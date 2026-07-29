mod support;

use std::collections::HashSet;

use compass_model::code_graph::EdgeKind;
use compass_model::query_contract::{CallRequest, CodeQueryLimits, NodeTrailRequest};
use compass_query::open;

#[test]
fn callers_include_calls_and_route_bindings_while_callees_follow_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = CallRequest {
        symbol: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    let callers = engine.callers(request.clone())?;
    let caller_ids = callers
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert!(caller_ids.contains("n:caller"));
    assert!(caller_ids.contains("n:route"));
    assert!(
        callers
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::RoutesTo)
    );

    let callees = engine.callees(request)?;
    assert!(callees.nodes.iter().any(|node| node.id == "n:callee"));
    assert!(
        callees
            .edges
            .iter()
            .all(|edge| edge.kind == EdgeKind::Calls)
    );
    Ok(())
}

#[test]
fn node_trail_returns_a_stable_evidence_aware_path() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let trail = engine.node_trail(NodeTrailRequest {
        source: "dependent".to_owned(),
        target: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert_eq!(trail.paths.len(), 1);
    assert_eq!(
        trail.paths[0].node_ids,
        ["n:dependent", "n:caller", "n:list", "n:callee"]
    );
    assert_eq!(trail.paths[0].edge_ids.len(), 3);
    Ok(())
}

#[test]
fn node_trail_never_exceeds_node_or_edge_budgets() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let trail = engine.node_trail(NodeTrailRequest {
        source: "dependent".to_owned(),
        target: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_nodes: 2,
            max_edges: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(trail.truncated);
    assert!(trail.nodes.len() <= 2);
    assert!(trail.edges.len() <= 1);
    assert!(trail.paths.is_empty());
    Ok(())
}
