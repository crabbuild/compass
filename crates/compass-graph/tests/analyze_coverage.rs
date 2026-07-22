use std::collections::BTreeMap;
use std::error::Error;

use compass_graph::{
    Communities, find_import_cycles, god_nodes, graph_diff, suggest_questions,
    surprising_connections,
};
use compass_model::GraphDocument;
use serde_json::{Value, json};

fn document(nodes: Vec<Value>, links: Vec<Value>, directed: bool) -> GraphDocument {
    let parsed = serde_json::from_value(json!({
        "directed":directed,
        "multigraph":false,
        "graph":{},
        "nodes":nodes,
        "links":links
    }));
    match parsed {
        Ok(document) => document,
        Err(error) => std::panic::resume_unwind(Box::new(error.to_string())),
    }
}

fn node(id: &str, label: &str, source_file: &str) -> Value {
    json!({"id":id,"label":label,"source_file":source_file,"file_type":"code"})
}

fn edge(source: &str, target: &str, relation: &str, confidence: &str) -> Value {
    json!({
        "source":source,"target":target,"relation":relation,"confidence":confidence,
        "_src":source,"_tgt":target
    })
}

#[test]
fn questions_cover_no_signal_isolation_inference_ambiguity_bridge_and_low_cohesion()
-> Result<(), Box<dyn Error>> {
    let empty = document(Vec::new(), Vec::new(), true);
    let questions = suggest_questions(&empty, &Communities::new(), &BTreeMap::new(), 10);
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0].kind, "no_signal");
    assert!(god_nodes(&empty, 10).is_empty());
    assert!(surprising_connections(&empty, &Communities::new(), 10).is_empty());

    let nodes = vec![
        node("hub", "Hub", "src/hub.py"),
        node("left", "Left", "src/left.py"),
        node("right", "Right", "src/right.py"),
        node("bridge", "Bridge", "src/bridge.py"),
        node("other", "Other", "src/other.py"),
        node("isolated", "Isolated", "src/isolated.py"),
    ];
    let links = vec![
        edge("hub", "left", "uses", "INFERRED"),
        edge("hub", "right", "calls", "INFERRED"),
        edge("left", "bridge", "references", "AMBIGUOUS"),
        edge("bridge", "other", "calls", "EXTRACTED"),
    ];
    let graph = document(nodes, links, true);
    let communities = BTreeMap::from([
        (
            0,
            vec!["hub".to_owned(), "left".to_owned(), "right".to_owned()],
        ),
        (1, vec!["bridge".to_owned(), "other".to_owned()]),
        (2, vec!["isolated".to_owned()]),
    ]);
    let labels = BTreeMap::from([(0, "Core".to_owned()), (1, "Boundary".to_owned())]);
    let questions = suggest_questions(&graph, &communities, &labels, 20);
    for kind in [
        "ambiguous_edge",
        "bridge_node",
        "verify_inferred",
        "isolated_nodes",
    ] {
        assert!(
            questions.iter().any(|question| question.kind == kind),
            "{kind}"
        );
    }

    let loose_nodes = (0..5)
        .map(|index| node(&format!("n{index}"), &format!("N{index}"), "one/module.py"))
        .collect();
    let loose = document(loose_nodes, Vec::new(), true);
    let loose_communities = BTreeMap::from([(7, (0..5).map(|i| format!("n{i}")).collect())]);
    let questions = suggest_questions(&loose, &loose_communities, &BTreeMap::new(), 20);
    assert!(
        questions
            .iter()
            .any(|question| question.kind == "low_cohesion")
    );
    Ok(())
}

#[test]
fn surprises_cover_cross_file_scoring_cross_community_and_structural_fallbacks() {
    let mut nodes = vec![
        node("hub", "Hub", "backend/hub.py"),
        node("doc", "Guide", "docs/guide.md"),
        node("rust", "Rust", "native/lib.rs"),
        node("image", "Diagram", "assets/flow.png"),
        node("a", "A", "backend/a.py"),
        node("b", "B", "backend/b.py"),
        node("c", "C", "backend/c.py"),
        node("d", "D", "backend/d.py"),
    ];
    let mut links = vec![
        edge("hub", "doc", "uses", "INFERRED"),
        edge("hub", "rust", "semantically_similar_to", "AMBIGUOUS"),
        edge("hub", "image", "references", "EXTRACTED"),
    ];
    for id in ["a", "b", "c", "d"] {
        links.push(edge("hub", id, "calls", "EXTRACTED"));
    }
    let graph = document(nodes.clone(), links, true);
    let communities = BTreeMap::from([
        (0, vec!["hub".to_owned(), "a".to_owned(), "b".to_owned()]),
        (
            1,
            vec!["doc".to_owned(), "rust".to_owned(), "image".to_owned()],
        ),
    ]);
    let surprises = surprising_connections(&graph, &communities, 20);
    assert!(surprises.len() >= 3);
    assert!(surprises.iter().any(|item| {
        item.why
            .as_deref()
            .is_some_and(|why| why.contains("semantically similar"))
    }));
    assert!(surprises.iter().any(|item| {
        item.why
            .as_deref()
            .is_some_and(|why| why.contains("peripheral node"))
    }));

    for item in &mut nodes {
        item["source_file"] = json!("single/module.py");
    }
    let community_graph = document(
        nodes,
        vec![
            edge("hub", "doc", "relates", "AMBIGUOUS"),
            edge("hub", "rust", "relates", "INFERRED"),
            edge("a", "image", "relates", "EXTRACTED"),
            edge("b", "c", "imports", "EXTRACTED"),
        ],
        false,
    );
    let surprises = surprising_connections(&community_graph, &communities, 20);
    assert!(surprises.iter().any(|item| {
        item.note
            .as_deref()
            .is_some_and(|n| n.contains("community"))
    }));

    let structural = surprising_connections(&community_graph, &Communities::new(), 3);
    assert!(!structural.is_empty());
    assert!(
        structural[0]
            .note
            .as_deref()
            .is_some_and(|note| note.contains("betweenness"))
    );

    let huge = document(
        (0..5_001)
            .map(|index| node(&format!("x{index}"), "X", "same.py"))
            .collect(),
        vec![edge("x0", "x1", "calls", "EXTRACTED")],
        true,
    );
    assert!(surprising_connections(&huge, &Communities::new(), 2).is_empty());
}

#[test]
fn diff_and_cycle_analysis_cover_add_remove_direction_deferred_and_rotation() {
    let old = document(
        vec![node("a", "A", "a.py"), node("b", "B", "b.py")],
        vec![edge("a", "b", "calls", "EXTRACTED")],
        true,
    );
    let newer = document(
        vec![
            node("b", "B", "b.py"),
            node("c", "C", "c.py"),
            node("d", "D", "d.py"),
        ],
        vec![
            edge("b", "c", "uses", "INFERRED"),
            edge("d", "c", "references", "AMBIGUOUS"),
        ],
        true,
    );
    let diff = graph_diff(&old, &newer);
    assert_eq!(diff.new_nodes.len(), 2);
    assert_eq!(diff.removed_nodes.len(), 1);
    assert_eq!(diff.new_edges.len(), 2);
    assert_eq!(diff.removed_edges.len(), 1);
    assert!(diff.summary.contains("2 new nodes"));
    assert_eq!(graph_diff(&old, &old).summary, "no changes");

    let undirected_old = document(
        vec![node("a", "A", "a.py"), node("b", "B", "b.py")],
        vec![edge("a", "b", "calls", "")],
        false,
    );
    let undirected_new = document(
        vec![node("a", "A", "a.py"), node("b", "B", "b.py")],
        vec![edge("b", "a", "calls", "EXTRACTED")],
        false,
    );
    assert_eq!(
        graph_diff(&undirected_old, &undirected_new).summary,
        "no changes"
    );

    let cycle_graph = document(
        vec![
            node("a", "A", "a.py"),
            node("b", "B", "b.py"),
            node("c", "C", "c.py"),
            node("skip", "Skip", "skip.py"),
        ],
        vec![
            json!({"source":"a","target":"b","relation":"imports_from","confidence":"EXTRACTED","source_file":"a.py"}),
            json!({"source":"b","target":"c","relation":"re_exports","confidence":"EXTRACTED","source_file":"b.py"}),
            json!({"source":"c","target":"a","relation":"imports_from","confidence":"EXTRACTED","source_file":"c.py"}),
            json!({"source":"a","target":"skip","relation":"imports_from","confidence":"EXTRACTED","source_file":"a.py","deferred":true}),
            json!({"source":"skip","target":"c","relation":"calls","confidence":"EXTRACTED","source_file":"skip.py"}),
        ],
        true,
    );
    let cycles = find_import_cycles(&cycle_graph, 5, 10);
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].cycle, ["a.py", "b.py", "c.py"]);
    assert!(find_import_cycles(&cycle_graph, 2, 10).is_empty());
    assert!(find_import_cycles(&document(Vec::new(), Vec::new(), true), 5, 10).is_empty());
}
