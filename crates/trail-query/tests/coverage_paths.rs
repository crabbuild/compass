use std::collections::HashMap;
use std::error::Error;

use serde_json::{Map, Value, json};
use trail_model::{Graph, GraphDocument};
use trail_query::{
    TraversalMode, affected_nodes, find_node, format_affected, format_benchmark,
    normalize_context_filters, query_graph_text, query_terms, render_explanation,
    render_shortest_path, resolve_seed, run_benchmark, sanitize_label, score_nodes, search_tokens,
};

fn document(raw: &str) -> Result<GraphDocument, Box<dyn Error>> {
    Ok(serde_json::from_str(raw)?)
}

fn graph(raw: &str) -> Result<Graph, Box<dyn Error>> {
    Ok(Graph::from_document(document(raw)?)?)
}

const FIXTURE: &str = r#"{
  "directed": true,
  "multigraph": false,
  "graph": {},
  "nodes": [
    {"id":"src/foo.py","label":"foo.py","source_file":"src/foo.py","source_location":"L1","file_type":"code","community":1},
    {"id":"foo-type","label":"Fóo","source_file":"src/foo.py","source_location":"L3","file_type":"code","community_name":"Core"},
    {"id":"foo-member","label":"run()","source_file":"src/foo.py","source_location":"L4","file_type":"code","community":1},
    {"id":"caller","label":"callFoo()","source_file":"src/app.py","source_location":"L9","file_type":"code","community":2},
    {"id":"other","label":"OtherThing","source_file":"src/other.py","source_location":"L2","file_type":"code","community":2},
    {"id":"isolated","label":"Isolated","source_file":"src/isolated.py","source_location":"L1","file_type":"code","community":3}
  ],
  "links": [
    {"source":"foo-type","target":"foo-member","relation":"contains","context":"field","confidence":"EXTRACTED"},
    {"source":"caller","target":"foo-member","relation":"calls","context":"call","confidence":"EXTRACTED"},
    {"source":"other","target":"caller","relation":"imports","context":"import","confidence":"INFERRED"}
  ]
}"#;

#[test]
fn affected_resolution_covers_ids_labels_sources_members_and_misses() -> Result<(), Box<dyn Error>>
{
    let graph = graph(FIXTURE)?;
    assert_eq!(
        resolve_seed(&graph, "src/foo.py/"),
        graph.node_index("src/foo.py")
    );
    assert_eq!(resolve_seed(&graph, "fóo"), graph.node_index("foo-type"));
    assert_eq!(resolve_seed(&graph, "run"), graph.node_index("foo-member"));
    assert_eq!(
        resolve_seed(&graph, "src/isolated.py"),
        graph.node_index("isolated")
    );
    assert_eq!(resolve_seed(&graph, "Other"), graph.node_index("other"));
    assert_eq!(resolve_seed(&graph, "missing"), None);

    let hits = affected_nodes(
        &graph,
        graph.node_index("foo-type").ok_or("missing seed")?,
        &["calls".to_owned(), "imports".to_owned()],
        2,
    );
    assert!(
        hits.iter()
            .any(|hit| graph.node(hit.node).id == "caller" && hit.depth == 1)
    );
    assert!(
        hits.iter()
            .any(|hit| graph.node(hit.node).id == "other" && hit.depth == 2)
    );

    let rendered = format_affected(&graph, "Fóo", &["calls".to_owned()], 1);
    assert!(rendered.contains("callFoo() [calls] src/app.py:L9"));
    assert!(
        format_affected(&graph, "Isolated", &["calls".to_owned()], 1)
            .contains("No affected nodes found.")
    );
    assert!(
        format_affected(&graph, "absent", &["calls".to_owned()], 1)
            .contains("No unique node match")
    );
    Ok(())
}

#[test]
fn scoring_and_find_node_cover_match_tiers_and_seed_collection() -> Result<(), Box<dyn Error>> {
    let graph = graph(FIXTURE)?;
    assert!(score_nodes(&graph, &[], true).ranked.is_empty());
    let scores = score_nodes(
        &graph,
        &["foo".to_owned(), "src".to_owned(), "foo".to_owned()],
        true,
    );
    assert!(!scores.ranked.is_empty());
    let expected = [
        ("foo-type", 230.503_783_409_003_52),
        ("src/foo.py", 24.338_368_737_318_614),
        ("caller", 1.889_245_806_401_811_8),
        ("foo-member", 1.431_100_440_464_734_3),
        ("isolated", 0.972_955_074_527_656_6),
        ("other", 0.972_955_074_527_656_6),
    ];
    assert_eq!(scores.ranked.len(), expected.len());
    for (actual, (id, score)) in scores.ranked.iter().zip(expected) {
        assert_eq!(graph.node(actual.node).id, id);
        assert!((actual.score - score).abs() < 1e-12);
    }
    assert!(scores.best_seed_by_term.contains_key("foo"));
    assert_eq!(
        scores
            .best_seed_by_term
            .get("foo")
            .map(|index| graph.node(*index).id.as_str()),
        Some("foo-type")
    );
    assert_eq!(graph.node(scores.ranked[0].node).id, "foo-type");
    type ScoreCase<'a> = (&'a [&'a str], &'a [(&'a str, f64)]);
    let cases: &[ScoreCase<'_>] = &[
        (
            &["foo"],
            &[
                ("foo-type", 10_916.748_877_240_092),
                ("src/foo.py", 1_092.087_218_553_352_6),
                ("caller", 0.916_290_731_874_155_1),
                ("foo-member", 0.458_145_365_937_077_55),
            ],
        ),
        (
            &["fo"],
            &[
                ("foo-type", 1_092.087_218_553_352_6),
                ("src/foo.py", 1_092.087_218_553_352_6),
                ("caller", 0.916_290_731_874_155_1),
                ("foo-member", 0.458_145_365_937_077_55),
            ],
        ),
        (&["thing"], &[("other", 1.386_294_361_119_890_6)]),
        (&["app"], &[("caller", 0.972_955_074_527_656_6)]),
    ];
    for (terms, expected) in cases {
        let terms = terms.iter().map(ToString::to_string).collect::<Vec<_>>();
        let actual = score_nodes(&graph, &terms, true);
        assert_eq!(actual.ranked.len(), expected.len());
        for (actual, (id, score)) in actual.ranked.iter().zip(*expected) {
            assert_eq!(graph.node(actual.node).id, *id);
            assert!((actual.score - score).abs() < 1e-12);
        }
    }

    assert_eq!(
        find_node(&graph, "src/foo.py").first().copied(),
        graph.node_index("src/foo.py")
    );
    assert_eq!(
        find_node(&graph, "fóo").first().copied(),
        graph.node_index("foo-type")
    );
    assert!(find_node(&graph, "call").contains(&graph.node_index("caller").ok_or("caller")?));
    assert!(find_node(&graph, "thing").contains(&graph.node_index("other").ok_or("other")?));
    assert!(find_node(&graph, "   ").is_empty());
    Ok(())
}

#[test]
fn traversal_path_and_explanation_cover_success_and_error_rendering() -> Result<(), Box<dyn Error>>
{
    let graph = graph(FIXTURE)?;
    let empty = query_graph_text(
        &graph,
        "unfindable symbol",
        TraversalMode::Bfs,
        2,
        200,
        &[],
        &HashMap::new(),
    );
    assert_eq!(empty, "No matching nodes found.");

    let bfs = query_graph_text(
        &graph,
        "which calls run",
        TraversalMode::Bfs,
        3,
        200,
        &[],
        &HashMap::new(),
    );
    assert!(bfs.contains("Traversal: BFS"));
    assert!(bfs.contains("Context: call (heuristic)"));

    let dfs = query_graph_text(
        &graph,
        "OtherThing",
        TraversalMode::Dfs,
        3,
        10,
        &["imports".to_owned()],
        &HashMap::new(),
    );
    assert!(dfs.contains("Traversal: DFS"));
    assert!(dfs.contains("Context: import (explicit)"));

    assert!(render_shortest_path(&graph, "absent", "run").is_err());
    assert!(render_shortest_path(&graph, "run", "absent").is_err());
    assert!(render_shortest_path(&graph, "run", "run").is_err());
    assert!(render_shortest_path(&graph, "run", "Isolated")?.contains("No path found"));
    assert!(render_shortest_path(&graph, "OtherThing", "run")?.contains("2 hops"));

    assert!(render_explanation(&graph, "absent", &HashMap::new()).contains("No node matching"));
    let contested = HashMap::from([(
        "foo-member".to_owned(),
        Map::from_iter([
            ("status".to_owned(), Value::String("contested".to_owned())),
            ("uses".to_owned(), json!(4)),
            ("neg".to_owned(), json!(2)),
            ("stale".to_owned(), json!(true)),
        ]),
    )]);
    let explained = render_explanation(&graph, "run", &contested);
    assert!(explained.contains("Lesson: contested"));
    assert!(explained.contains("[code changed since"));
    assert!(explained.contains("<-- callFoo()"));
    Ok(())
}

#[test]
fn benchmark_and_text_helpers_cover_unicode_ascii_and_error_modes() -> Result<(), Box<dyn Error>> {
    let document = document(FIXTURE)?;
    let questions = vec!["run foo".to_owned(), "OtherThing".to_owned()];
    let result = run_benchmark(&document, Some(12_345), Some(&questions));
    assert!(result.error.is_none());
    assert_eq!(result.corpus_words, 12_345);
    assert_eq!(result.per_question.len(), 2);
    assert!(format_benchmark(&result, true).contains("12,345 words →"));
    assert!(format_benchmark(&result, false).contains("12,345 words ->"));

    let missing = run_benchmark(&document, None, Some(&["zzzzzz".to_owned()]));
    assert!(missing.error.is_some());
    assert!(format_benchmark(&missing, false).starts_with("Benchmark error:"));

    assert_eq!(search_tokens("Crème brûlée"), vec!["creme", "brulee"]);
    assert!(query_terms("知识图谱如何工作").len() >= 2);
    assert_eq!(query_terms("how what why"), vec!["how", "what", "why"]);
    assert_eq!(
        normalize_context_filters(&[
            " Args ".to_owned(),
            "returns".to_owned(),
            "calls".to_owned(),
            "CALLS".to_owned(),
            "".to_owned(),
        ]),
        vec!["parameter_type", "return_type", "call"]
    );
    let dirty = format!("ok{}bad", char::from(7));
    assert_eq!(sanitize_label(&dirty), "okbad");
    assert_eq!(sanitize_label(&"x".repeat(300)).chars().count(), 256);
    Ok(())
}
