mod support;

use std::fs;

use compass_model::query_contract::{CodeQueryLimits, ExploreRequest, QueryDiagnosticCode};
use compass_query::open;

#[test]
fn explore_connects_symbols_and_groups_digest_verified_source()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = ExploreRequest {
        symbols: vec!["Api.caller".to_owned(), "Store.callee".to_owned()],
        root: directory.path().to_string_lossy().into_owned(),
        limits: CodeQueryLimits::default(),
    };
    let response = engine.explore(request.clone())?;
    assert_eq!(response.paths.len(), 1);
    assert_eq!(response.files.len(), 1);
    assert_eq!(response.files[0].source.as_deref(), Some("code"));

    fs::write(directory.path().join("src/lib.rs"), "changed")?;
    let stale = engine.explore(request)?;
    assert!(stale.files[0].source.is_none());
    assert!(
        stale
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == QueryDiagnosticCode::StaleSourceDigest })
    );
    Ok(())
}

#[test]
fn explore_derives_repository_root_from_a_generation_graph()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let generation = directory
        .path()
        .join("compass-out/.compass-generations/generation-test");
    fs::create_dir_all(&generation)?;
    let graph_path = generation.join("graph.json");
    support::write_graph(&graph_path)?;
    fs::create_dir_all(directory.path().join("src"))?;
    fs::rename(
        generation.join("src/lib.rs"),
        directory.path().join("src/lib.rs"),
    )?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.explore(ExploreRequest {
        symbols: vec!["Api.caller".to_owned(), "Store.callee".to_owned()],
        root: String::new(),
        limits: CodeQueryLimits::default(),
    })?;
    assert_eq!(response.files[0].source.as_deref(), Some("code"));
    Ok(())
}

#[test]
fn explore_applies_one_aggregate_graph_budget() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.explore(ExploreRequest {
        symbols: vec![
            "Api.caller".to_owned(),
            "dependent".to_owned(),
            "UserService.list".to_owned(),
            "GET /users".to_owned(),
        ],
        root: directory.path().to_string_lossy().into_owned(),
        limits: CodeQueryLimits {
            max_edges: 1,
            max_nodes: 4,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(response.edges.len() <= 1, "edges={:?}", response.edges);
    assert!(response.nodes.len() <= 4, "nodes={:?}", response.nodes);
    let node_ids = response
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(response.edges.iter().all(|edge| {
        node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
    }));
    Ok(())
}

#[test]
fn explore_never_reads_unsafe_relative_paths() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph: serde_json::Value = serde_json::from_slice(&fs::read(&graph_path)?)?;
    graph["graph"]["files"][0]["path"] = serde_json::json!("../secret");
    graph["nodes"][0]["source"]["file"] = serde_json::json!("../secret");
    graph["nodes"][0]["evidence"][0]["anchors"][0]["file"] = serde_json::json!("../secret");
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;
    assert!(open(&graph_path, None, &directory.path().join("cache")).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn explore_rejects_a_source_symlink_that_escapes_the_repository()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let outside = tempfile::tempdir()?;
    let outside_source = outside.path().join("lib.rs");
    fs::write(&outside_source, "code")?;
    fs::remove_file(directory.path().join("src/lib.rs"))?;
    symlink(outside_source, directory.path().join("src/lib.rs"))?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let result = engine.explore(ExploreRequest {
        symbols: vec!["Api.caller".to_owned(), "Store.callee".to_owned()],
        root: directory.path().to_string_lossy().into_owned(),
        limits: CodeQueryLimits::default(),
    });
    assert!(result.is_err());
    Ok(())
}
