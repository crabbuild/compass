use std::error::Error;
use std::fs;

use compass_model::{EdgeRecord, Graph, GraphDocument, NodeRecord};
use serde_json::{Value, json};

#[test]
fn document_loading_serialization_and_python_strings_cover_boundary_shapes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let wrong_extension = directory.path().join("graph.txt");
    fs::write(&wrong_extension, "{}")?;
    assert!(GraphDocument::load(&wrong_extension).is_err());
    assert!(GraphDocument::load(&directory.path().join("missing.json")).is_err());

    let corrupt = directory.path().join("corrupt.json");
    fs::write(&corrupt, "{")?;
    assert!(GraphDocument::load(&corrupt).is_err());

    let graph_path = directory.path().join("graph.json");
    fs::write(
        &graph_path,
        serde_json::to_vec(&json!({
            "directed":false,
            "multigraph":false,
            "graph":{"name":"fixture"},
            "nodes":[
                {"id":"a","label":"Alpha","null":null,"bool":true,"number":7,"array":[1],"object":{"x":1}},
                {"id":"a","label":"Merged","extra":"kept"}
            ],
            "edges":[
                {"source":"a","target":"ghost","relation":"first","context":"call"},
                {"source":"a","target":"ghost","relation":"merged","weight":2}
            ],
            "custom":"value"
        }))?,
    )?;
    let document = GraphDocument::load(&graph_path)?;
    assert!(document.used_legacy_edges_key);
    assert_eq!(document.extras["custom"], "value");
    assert_eq!(document.nodes[0].string("null"), "");
    assert_eq!(document.nodes[0].string("bool"), "True");
    assert_eq!(document.nodes[0].string("number"), "7");
    assert_eq!(document.nodes[0].string("array"), "[1]");
    assert_eq!(document.nodes[0].string("object"), "{\"x\":1}");
    assert_eq!(document.nodes[0].label(), "Alpha");
    assert_eq!(
        NodeRecord {
            id: "fallback".to_owned(),
            attributes: serde_json::Map::new(),
        }
        .label(),
        "fallback"
    );
    let encoded = serde_json::to_value(&document)?;
    assert!(encoded.get("edges").is_some());
    assert!(encoded.get("links").is_none());

    let graph = Graph::from_document(document)?;
    assert_eq!((graph.node_count(), graph.edge_count()), (2, 1));
    let a = graph.node_index("a").ok_or("missing a")?;
    let ghost = graph.node_index("ghost").ok_or("missing ghost")?;
    assert_eq!(graph.node(a).string("extra"), "kept");
    assert_eq!(graph.edge(0).string("relation"), "merged");
    assert_eq!(graph.degree(a), 1);
    assert_eq!(graph.successors(a).collect::<Vec<_>>(), vec![ghost]);
    assert_eq!(graph.predecessors(a).collect::<Vec<_>>(), vec![ghost]);
    assert_eq!(graph.edge_between(ghost, a), Some(0));
    assert_eq!(
        graph
            .first_edge_attributes(a, ghost)
            .ok_or("missing edge attributes")?["weight"],
        2
    );
    assert_eq!(graph.with_edge_contexts(&[]).edge_count(), 1);
    assert_eq!(
        graph
            .with_edge_contexts(&["missing".to_owned()])
            .edge_count(),
        0
    );
    Ok(())
}

#[test]
fn directed_and_multigraph_loading_preserve_parallel_edges_and_direction()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "directed":false,"multigraph":true,"nodes":[],
            "links":[
                {"source":"left","target":"right","relation":"one"},
                {"source":"left","target":"right","relation":"two"}
            ]
        }))?,
    )?;
    let undirected = Graph::load(&path)?;
    assert_eq!((undirected.node_count(), undirected.edge_count()), (2, 2));
    let left = undirected.node_index("left").ok_or("missing left")?;
    let right = undirected.node_index("right").ok_or("missing right")?;
    assert_eq!(undirected.degree(left), 2);
    assert_eq!(undirected.successors(right).collect::<Vec<_>>(), vec![left]);

    let directed = Graph::load_directed(&path)?;
    assert_eq!(directed.degree(left), 2);
    assert!(directed.successors(right).next().is_none());
    assert_eq!(directed.predecessors(right).collect::<Vec<_>>(), vec![left]);

    let edge = EdgeRecord {
        source: "left".to_owned(),
        target: "right".to_owned(),
        attributes: serde_json::Map::from_iter([("value".to_owned(), Value::Bool(false))]),
    };
    assert_eq!(edge.string("value"), "False");
    assert_eq!(edge.string("missing"), "");
    Ok(())
}
