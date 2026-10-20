use compass_core::{BuildOptions, GraphStorage, build_local_graph};
use compass_model::code_graph::GraphDocument;
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryTraversal,
};
use compass_model::source_query::SOURCE_QUERY_FILE;
use compass_query::{
    EngineSelection, QueryEngineCache, open_with_engine, open_with_verified_document,
};
use std::error::Error;
use std::fs;
use std::path::Path;
fn request() -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: "Where is payload handled?".to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}
fn build(root: &Path, output: &Path, topic: &str) -> Result<std::path::PathBuf, Box<dyn Error>> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join("sample.rs"),
        format!(
            "fn opaque(value: &str) -> &str {{\nlet returned = value;\n// Handle {topic} here.\nreturned }}\n"
        ),
    )?;
    let mut o = BuildOptions::new(root);
    o.output_root = Some(output.to_path_buf());
    o.graph_storage = GraphStorage::Sqlite;
    o.no_cluster = true;
    o.no_viz = true;
    o.max_workers = Some(1);
    Ok(build_local_graph(&o)?.output_dir)
}
#[test]
fn current_engine_caches_detect_auxiliary_corruption_and_content_restoration()
-> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let output = build(&d.path().join("repo"), &d.path().join("out"), "payload")?;
    let graph = output.join("graph.json");
    let path = output.join(SOURCE_QUERY_FILE);
    let original = fs::read(&path)?;
    let cache = QueryEngineCache::default();
    let first = cache.open_published_store(&graph)?;
    assert!(
        !first
            .lock()
            .map_err(|_| "poisoned engine")?
            .discover_ranked(request())?
            .candidates
            .is_empty()
    );
    fs::write(&path, b"corrupt")?;
    assert!(
        cache.open_published_store(&graph).is_err(),
        "cache must not return a stale body snapshot"
    );
    fs::write(&path, &original)?;
    let restored = cache.open_published_store(&graph)?;
    assert!(std::sync::Arc::ptr_eq(&first, &restored));
    let (document, identity) = GraphDocument::load_with_artifact_digest(&graph)?;
    let json_cache = QueryEngineCache::default();
    let first = json_cache.open_verified_document(
        &document,
        &identity,
        &graph,
        &d.path().join("json-cache"),
    )?;
    fs::write(&path, b"corrupt")?;
    assert!(
        json_cache
            .open_verified_document(&document, &identity, &graph, &d.path().join("json-cache"))
            .is_err()
    );
    fs::write(&path, &original)?;
    let restored = json_cache.open_verified_document(
        &document,
        &identity,
        &graph,
        &d.path().join("json-cache"),
    )?;
    assert!(std::sync::Arc::ptr_eq(&first, &restored));
    Ok(())
}
#[test]
fn mismatched_or_unknown_artifacts_fail_current_queries_but_detached_graphs_stay_isolated()
-> Result<(), Box<dyn Error>> {
    let d = tempfile::tempdir()?;
    let a = build(&d.path().join("a"), &d.path().join("out-a"), "payload")?;
    let b = build(&d.path().join("b"), &d.path().join("out-b"), "transport")?;
    let graph = a.join("graph.json");
    let path = a.join(SOURCE_QUERY_FILE);
    let original = fs::read(&path)?;
    fs::copy(b.join(SOURCE_QUERY_FILE), &path)?;
    for mode in [EngineSelection::Json, EngineSelection::Store] {
        assert!(open_with_engine(&graph, None, &d.path().join("bad-cache"), mode).is_err());
    }
    let (document, identity) = GraphDocument::load_with_artifact_digest(&graph)?;
    let detached =
        open_with_verified_document(document, identity, &graph, None, &d.path().join("detached"))?;
    assert!(
        detached.discover_ranked(request())?.candidates.is_empty(),
        "a detached graph must not borrow current body evidence"
    );
    let mut value: serde_json::Value = serde_json::from_slice(&original)?;
    value["schema"] = "compass.source-query/2".into();
    fs::write(&path, serde_json::to_vec(&value)?)?;
    assert!(
        open_with_engine(
            &graph,
            None,
            &d.path().join("version-cache"),
            EngineSelection::Default
        )
        .is_err()
    );
    fs::remove_file(&path)?;
    let older = open_with_engine(
        &graph,
        None,
        &d.path().join("older-cache"),
        EngineSelection::Default,
    )?;
    let answer = older.discover_ranked(request())?;
    assert!(answer.candidates.is_empty());
    assert!(
        answer
            .discovery
            .diagnostics
            .iter()
            .any(|d| d.message.contains("no immutable callable-body index"))
    );
    Ok(())
}
