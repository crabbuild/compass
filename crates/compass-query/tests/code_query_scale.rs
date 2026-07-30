mod support;

use std::fs;
use std::time::{Duration, Instant};

use compass_model::code_graph::{EdgeKind, GraphDocument};
use compass_model::identity::edge_id;
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, ImpactRequest, NodeTrailRequest,
};
use compass_query::open;

const SCALE_NODES: usize = 100_000;
const QUERY_CEILING: Duration = Duration::from_secs(5);

#[test]
fn enterprise_queries_stay_within_in_process_ceiling() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let node_template = graph
        .nodes
        .iter()
        .find(|node| node.id == "n:caller")
        .cloned()
        .ok_or("missing node template")?;
    let edge_template = graph
        .links
        .iter()
        .find(|edge| edge.kind == EdgeKind::Calls)
        .cloned()
        .ok_or("missing edge template")?;

    for index in 0..SCALE_NODES {
        let mut node = node_template.clone();
        node.id = scale_id(index);
        node.name = format!("f{index:05}");
        node.qualified_name = format!("scale::f{index:05}");
        graph.nodes.push(node);
        if index == 0 {
            continue;
        }
        let source = scale_id(index - 1);
        let target = scale_id(index);
        let mut edge = edge_template.clone();
        edge.source.clone_from(&source);
        edge.target.clone_from(&target);
        edge.id = edge_id(
            &source,
            EdgeKind::Calls,
            &target,
            edge.relationship_site.as_ref(),
            None,
        );
        edge.key.clone_from(&edge.id);
        graph.links.push(edge);
    }
    fs::write(&graph_path, serde_json::to_vec(&graph)?)?;

    // Index construction and artifact validation are deliberately outside the
    // query latency measurement: this ceiling governs an already-open
    // long-lived service, not process startup or first-load I/O.
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let mut limits = CodeQueryLimits {
        max_depth: 256,
        max_nodes: 512,
        max_edges: 1_024,
        ..CodeQueryLimits::default()
    };
    let started = Instant::now();

    let callers_started = Instant::now();
    let callers = engine.callers(CallRequest {
        symbol: format!("scale::f{:05}", SCALE_NODES - 1),
        limits: limits.clone(),
    })?;
    assert!(
        callers
            .nodes
            .iter()
            .any(|node| { node.qualified_name == format!("scale::f{:05}", SCALE_NODES - 2) })
    );
    let callers_elapsed = callers_started.elapsed();

    let impact_started = Instant::now();
    let impact = engine.impact(ImpactRequest {
        symbol: format!("scale::f{:05}", SCALE_NODES - 1),
        include_heuristic: false,
        limits: limits.clone(),
    })?;
    assert_eq!(impact.nodes.len(), 257);
    assert_eq!(impact.edges.len(), 256);
    let impact_elapsed = impact_started.elapsed();

    limits.max_depth = 128;
    let trail_started = Instant::now();
    let trail = engine.node_trail(NodeTrailRequest {
        source: "scale::f00000".to_owned(),
        target: "scale::f00128".to_owned(),
        include_heuristic: false,
        limits,
    })?;
    assert_eq!(
        trail.paths.first().map(|path| path.edge_ids.len()),
        Some(128)
    );
    let trail_elapsed = trail_started.elapsed();

    let elapsed = started.elapsed();
    assert!(
        elapsed < QUERY_CEILING,
        "indexed in-process queries took {elapsed:?}, exceeding {QUERY_CEILING:?}; \
         callers={callers_elapsed:?}, impact={impact_elapsed:?}, trail={trail_elapsed:?}"
    );
    Ok(())
}

fn scale_id(index: usize) -> String {
    format!("n:scale:{index:05}")
}
