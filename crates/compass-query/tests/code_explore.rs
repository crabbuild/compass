mod support;

use std::fs;

use compass_model::query_contract::{CodeQueryLimits, ExploreRequest, QueryDiagnosticCode};
use compass_query::{QueryErrorKind, open};

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
fn explore_rejects_more_symbols_than_the_candidate_budget() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let Err(error) = engine.explore(ExploreRequest {
        symbols: vec![
            "Api.caller".to_owned(),
            "Store.callee".to_owned(),
            "UserService.list".to_owned(),
        ],
        root: directory.path().to_string_lossy().into_owned(),
        limits: CodeQueryLimits {
            max_candidates: 2,
            ..CodeQueryLimits::default()
        },
    }) else {
        return Err("the symbol fan-out must be bounded before resolution".into());
    };
    assert_eq!(error.kind(), QueryErrorKind::InvalidParameter);
    assert_eq!(error.code(), "too_many_explore_symbols");
    Ok(())
}

#[test]
fn code_query_limits_have_a_hard_candidate_ceiling() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let Err(error) = engine.explore(ExploreRequest {
        symbols: vec!["Api.caller".to_owned()],
        root: directory.path().to_string_lossy().into_owned(),
        limits: CodeQueryLimits {
            max_candidates: 10_000,
            ..CodeQueryLimits::default()
        },
    }) else {
        return Err("unbounded candidate limits must be rejected".into());
    };
    assert_eq!(error.kind(), QueryErrorKind::InvalidParameter);
    assert_eq!(error.code(), "code_query_limit_exceeded");
    Ok(())
}

#[test]
fn explore_rejects_source_files_above_the_hard_io_cap() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    fs::OpenOptions::new()
        .write(true)
        .open(directory.path().join("src/lib.rs"))?
        .set_len(16 * 1024 * 1024 + 1)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let Err(error) = engine.explore(ExploreRequest {
        symbols: vec!["Api.caller".to_owned(), "Store.callee".to_owned()],
        root: directory.path().to_string_lossy().into_owned(),
        limits: CodeQueryLimits::default(),
    }) else {
        return Err("oversized source must fail before hashing to EOF".into());
    };
    assert_eq!(error.kind(), QueryErrorKind::MemoryLimit);
    assert_eq!(error.code(), "source_file_too_large");
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

#[cfg(unix)]
#[test]
fn explore_rejects_a_symlink_as_the_repository_root() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let parent = tempfile::tempdir()?;
    let linked_root = parent.path().join("linked-root");
    symlink(directory.path(), &linked_root)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let Err(error) = engine.explore(ExploreRequest {
        symbols: vec!["Api.caller".to_owned(), "Store.callee".to_owned()],
        root: linked_root.to_string_lossy().into_owned(),
        limits: CodeQueryLimits::default(),
    }) else {
        return Err("repository roots must be opened without following links".into());
    };
    assert_eq!(error.kind(), QueryErrorKind::UnsafePath);
    assert_eq!(error.code(), "unsafe_source_path");
    Ok(())
}
