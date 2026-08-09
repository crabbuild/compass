use std::collections::HashMap;

use compass_model::Graph;
use compass_query::{TraversalMode, query_graph_text, query_terms, score_nodes};
use serde_json::json;

fn golden_graph() -> Result<Graph, Box<dyn std::error::Error>> {
    Ok(Graph::from_document(serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {},
        "nodes": [
            {
                "id": "n:routes",
                "label": "route_register",
                "kind": "function",
                "source_file": "src/router.py",
                "source_location": "L10"
            },
            {
                "id": "n:dependencies",
                "label": "dependency_solve",
                "kind": "function",
                "source_file": "src/dependencies.py",
                "source_location": "L20"
            },
            {
                "id": "n:collections",
                "label": "collection_map",
                "kind": "function",
                "source_file": "src/collections.js",
                "source_location": "L30"
            },
            {
                "id": "n:records",
                "label": "log_record_represent",
                "kind": "function",
                "source_file": "src/log.rs",
                "source_location": "L40"
            },
            {
                "id": "n:noise",
                "label": "unrelated_helper",
                "kind": "function",
                "source_file": "src/noise.rs",
                "source_location": "L50"
            }
        ],
        "links": []
    }))?)?)
}

fn compound_identifier_graph() -> Result<Graph, Box<dyn std::error::Error>> {
    Ok(Graph::from_document(serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {},
        "nodes": [
            {
                "id": "n:map-values",
                "label": "mapValues",
                "kind": "function",
                "source_file": "src/collections.js",
                "source_location": "L10"
            },
            {
                "id": "n:map",
                "label": "map",
                "kind": "function",
                "source_file": "src/utility.js",
                "source_location": "L20"
            },
            {
                "id": "n:object",
                "label": "object",
                "kind": "type",
                "source_file": "src/types.js",
                "source_location": "L30"
            }
        ],
        "links": []
    }))?)?)
}

#[test]
fn natural_query_golden_cases_retrieve_reviewed_symbols() -> Result<(), Box<dyn std::error::Error>>
{
    let graph = golden_graph()?;
    let cases = [
        ("how are routes registered", "n:routes", "route_register"),
        (
            "how are dependencies solved",
            "n:dependencies",
            "dependency_solve",
        ),
        (
            "how are collections mapped",
            "n:collections",
            "collection_map",
        ),
        (
            "how are log records represented",
            "n:records",
            "log_record_represent",
        ),
    ];

    for (question, expected_id, expected_label) in cases {
        let terms = query_terms(question);
        let scores = score_nodes(&graph, &terms, true);
        assert_eq!(
            graph
                .node(
                    scores
                        .ranked
                        .first()
                        .ok_or("golden query had no result")?
                        .node
                )
                .id,
            expected_id,
            "question: {question:?}"
        );
        let rendered = query_graph_text(
            &graph,
            question,
            TraversalMode::Bfs,
            0,
            2_000,
            &[],
            &HashMap::new(),
        );
        assert!(
            rendered.contains(expected_label),
            "question {question:?} did not render {expected_label:?}: {rendered}"
        );
    }
    Ok(())
}

#[test]
fn natural_query_golden_prefers_a_matching_compound_identifier()
-> Result<(), Box<dyn std::error::Error>> {
    let graph = compound_identifier_graph()?;
    let question = "how are object values mapped";
    let scores = score_nodes(&graph, &query_terms(question), true);
    let top = scores.ranked.first().ok_or("golden query had no result")?;
    assert_eq!(graph.node(top.node).id, "n:map-values");

    let rendered = query_graph_text(
        &graph,
        question,
        TraversalMode::Bfs,
        0,
        2_000,
        &[],
        &HashMap::new(),
    );
    assert!(
        rendered.contains("mapValues"),
        "unexpected output: {rendered}"
    );
    Ok(())
}
