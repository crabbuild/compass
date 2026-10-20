use compass_core::{BuildOptions, GraphStorage, build_local_graph};
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
};
use compass_query::{EngineSelection, open_with_engine};
use std::error::Error;
use std::fs;
fn request(question: &str) -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: question.to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}
#[test]
fn body_candidate_ceiling_is_explicit_on_both_native_backends() -> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    let source = (0..70)
        .map(|i| format!("fn opaque_{i}() {{ // Handle payload here.\n}}\n"))
        .collect::<String>();
    fs::write(root.join("sample.rs"), source)?;
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(d.path().join("artifacts"));
    options.graph_storage = GraphStorage::Sqlite;
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    let built = build_local_graph(&options)?;
    let path = built.output_dir.join("graph.json");
    for backend in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &path,
            None,
            &d.path().join(format!("cache-{backend:?}")),
            backend,
        )?;
        let response = engine.discover_ranked(request("Where is payload handled?"))?;
        assert!(response.bounded);
        assert_eq!(response.eligible_count, None);
        assert!(!response.candidates.is_empty());
        assert!(response.candidates.len() <= 64);
        assert!(
            response
                .discovery
                .diagnostics
                .iter()
                .any(|d| d.message.contains("bound"))
        );
    }
    Ok(())
}
#[test]
fn body_comments_on_nested_callables_do_not_nominate_the_outer_function()
-> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("repo");
    fs::create_dir(&root)?;
    fs::write(
        root.join("sample.py"),
        "def opaque(value):\n    def inner(value):\n        value = value\n        # Handle secret messages here.\n        return value\n    return inner(value)\n",
    )?;
    let mut options = BuildOptions::new(&root);
    options.output_root = Some(d.path().join("artifacts"));
    options.no_cluster = true;
    options.no_viz = true;
    options.max_workers = Some(1);
    let built = build_local_graph(&options)?;
    for backend in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &built.output_dir.join("graph.json"),
            None,
            &d.path().join(format!("cache-{backend:?}")),
            backend,
        )?;
        let response = engine.discover_ranked(request("Where is secret handled?"))?;
        assert!(!response.candidates.is_empty());
        assert!(
            response
                .candidates
                .iter()
                .all(|row| row.node.name.trim_end_matches("()") == "inner"),
            "{:?}",
            response.candidates
        );
    }
    Ok(())
}
