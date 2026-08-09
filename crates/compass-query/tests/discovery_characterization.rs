use std::collections::HashMap;

use compass_model::{Graph, GraphDocument};
use compass_query::{
    TextPageOptions, TraversalMode, query_graph_text, query_graph_text_page, render_explanation,
};
use serde_json::json;

fn graph(nodes: serde_json::Value, links: serde_json::Value) -> Graph {
    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": true,
        "graph": {},
        "nodes": nodes,
        "links": links,
    }))
    .unwrap_or_else(|_| std::process::abort());
    Graph::from_document(document).unwrap_or_else(|_| std::process::abort())
}

#[test]
fn compatibility_discovery_is_outgoing_only() {
    let graph = graph(
        json!([
            {"id":"caller","label":"caller","source_file":"src/caller.rs"},
            {"id":"seed","label":"target","source_file":"src/target.rs"},
            {"id":"callee","label":"callee","source_file":"src/callee.rs"}
        ]),
        json!([
            {"source":"caller","target":"seed","relation":"calls","context":"call"},
            {"source":"seed","target":"callee","relation":"calls","context":"call"}
        ]),
    );
    let output = query_graph_text(
        &graph,
        "target",
        TraversalMode::Bfs,
        1,
        2_000,
        &[],
        &HashMap::new(),
    );
    assert!(output.contains("callee"));
    assert!(!output.contains("NODE caller"));
}

#[test]
fn compatibility_per_term_seed_additions_can_exceed_the_nominal_cap() {
    let graph = graph(
        json!([
            {"id":"alpha","label":"alpha"},
            {"id":"beta","label":"beta"},
            {"id":"gamma","label":"gamma"},
            {"id":"delta","label":"delta"}
        ]),
        json!([]),
    );
    let output = query_graph_text(
        &graph,
        "alpha beta gamma delta",
        TraversalMode::Bfs,
        0,
        2_000,
        &[],
        &HashMap::new(),
    );
    let start = output
        .split("Start: [")
        .nth(1)
        .and_then(|value| value.split(']').next())
        .unwrap_or_default();
    assert_eq!(start.matches('\'').count(), 8, "{output}");
    for label in ["alpha", "beta", "gamma", "delta"] {
        assert!(start.contains(label), "{output}");
    }
}

#[test]
fn compatibility_discovery_renders_only_one_parallel_edge() {
    let graph = graph(
        json!([
            {"id":"seed","label":"target"},
            {"id":"callee","label":"callee"}
        ]),
        json!([
            {"source":"seed","target":"callee","relation":"calls","context":"call"},
            {"source":"seed","target":"callee","relation":"registers","context":"registration"}
        ]),
    );
    let output = query_graph_text(
        &graph,
        "target",
        TraversalMode::Bfs,
        1,
        2_000,
        &[],
        &HashMap::new(),
    );
    assert_eq!(output.matches("EDGE ").count(), 1);
}

#[test]
fn compatibility_context_is_a_relationship_filter() {
    let graph = graph(
        json!([
            {"id":"seed","label":"target"},
            {"id":"call","label":"call neighbor"},
            {"id":"import","label":"import neighbor"}
        ]),
        json!([
            {"source":"seed","target":"call","relation":"calls","context":"call"},
            {"source":"seed","target":"import","relation":"imports","context":"import"}
        ]),
    );
    let output = query_graph_text(
        &graph,
        "target",
        TraversalMode::Bfs,
        1,
        2_000,
        &["call".to_owned()],
        &HashMap::new(),
    );
    assert!(output.contains("call neighbor"));
    assert!(!output.contains("import neighbor"));
}

#[test]
fn compatibility_explanation_reports_same_name_ambiguity() {
    let graph = graph(
        json!([
            {"id":"one","label":"target","source_file":"src/one.rs"},
            {"id":"two","label":"target","source_file":"src/two.rs"}
        ]),
        json!([]),
    );
    let output = render_explanation(&graph, "target", &HashMap::new());
    assert!(output.contains("Ambiguous:"));
    assert!(output.contains("src/one.rs"));
    assert!(output.contains("src/two.rs"));
}

#[test]
fn compatibility_pagination_is_applied_after_discovery() {
    let nodes = (0..20)
        .map(|index| json!({"id":format!("n{index}"),"label":format!("target {index}")}))
        .collect::<Vec<_>>();
    let graph = graph(json!(nodes), json!([]));
    let output = query_graph_text_page(
        &graph,
        "target",
        TraversalMode::Bfs,
        0,
        TextPageOptions {
            token_budget: 40,
            page: 1,
        },
        &[],
        &HashMap::new(),
    )
    .unwrap_or_else(|_| std::process::abort());
    assert!(output.contains("Pagination: page=1/"));
}

#[test]
fn compatibility_high_degree_seed_expands_the_entire_wide_frontier() {
    let mut nodes = vec![json!({"id":"hub","label":"target hub"})];
    let mut links = Vec::new();
    for index in 0..60 {
        nodes.push(json!({"id":format!("leaf-{index}"),"label":format!("leaf {index}")}));
        links.push(json!({"source":"hub","target":format!("leaf-{index}"),"relation":"calls"}));
    }
    let graph = graph(json!(nodes), json!(links));
    let output = query_graph_text(
        &graph,
        "target",
        TraversalMode::Bfs,
        1,
        10_000,
        &[],
        &HashMap::new(),
    );
    assert!(output.contains("61 nodes found"));
    assert_eq!(output.matches("NODE ").count(), 61);
    assert_eq!(output.matches("EDGE ").count(), 60);
}
