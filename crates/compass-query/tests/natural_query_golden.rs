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

fn reviewed_language_baseline_graph() -> Result<Graph, Box<dyn std::error::Error>> {
    Ok(Graph::from_document(serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {},
        "nodes": [
            {
                "id": "py:solve-dependencies",
                "label": "solve_dependencies",
                "kind": "function",
                "source_file": "fastapi/dependencies/utils.py",
                "source_location": "L540"
            },
            {
                "id": "rs:log-record",
                "label": "log_record_represent",
                "kind": "struct",
                "source_file": "src/record.rs",
                "source_location": "L18"
            },
            {
                "id": "ts:map-values",
                "label": "mapValues",
                "kind": "function",
                "source_file": "src/object/mapValues.ts",
                "source_location": "L11"
            },
            {
                "id": "ts:map-values-test",
                "label": "mapValuesFixture",
                "kind": "variable",
                "source_file": "tests/generated/mapValues.test.ts",
                "source_location": "L8"
            },
            {
                "id": "unicode:cafe-parser",
                "label": "CaféParser",
                "kind": "class",
                "source_file": "src/parsers/cafe.rs",
                "source_location": "L25"
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

#[test]
fn reviewed_language_baseline_preserves_ids_anchors_and_no_answer()
-> Result<(), Box<dyn std::error::Error>> {
    let graph = reviewed_language_baseline_graph()?;
    let cases = [
        ("how are dependencies solved", "py:solve-dependencies"),
        ("how are log records represented", "rs:log-record"),
        ("how are object values mapped", "ts:map-values"),
        ("where is cafe parsed", "unicode:cafe-parser"),
    ];

    for (question, expected_id) in cases {
        let expected = graph
            .node_index(expected_id)
            .ok_or("reviewed baseline node is missing")?;
        for _ in 0..3 {
            let scores = score_nodes(&graph, &query_terms(question), true);
            assert_eq!(
                scores.ranked.first().map(|candidate| candidate.node),
                Some(expected),
                "question: {question:?}"
            );
        }
    }

    let rendered = query_graph_text(
        &graph,
        "how are dependencies solved",
        TraversalMode::Bfs,
        0,
        2_000,
        &[],
        &HashMap::new(),
    );
    assert!(rendered.contains("src=fastapi/dependencies/utils.py"));
    assert!(!rendered.contains("mapValuesFixture"));

    let no_answer = score_nodes(&graph, &query_terms("quantum banana synchronization"), true);
    assert!(no_answer.ranked.is_empty());
    assert!(no_answer.best_seed_by_term.is_empty());
    Ok(())
}
