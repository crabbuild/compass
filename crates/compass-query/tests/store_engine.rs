mod support;

use std::fs;

use compass_graph::GraphSnapshotBuilder;
use compass_model::code_graph::{CODE_GRAPH_SCHEMA_V1, GraphDocument};
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, ExploreRequest, ImpactRequest, NodeTrailRequest, SearchRequest,
};
use compass_query::{EngineSelection, QueryEngineKind, open, open_with_engine, open_with_store};
use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore, StoreRef};
use compass_store_redb::RedbStore;

fn publish_phase2_snapshot(
    directory: &std::path::Path,
    graph_path: &std::path::Path,
) -> Result<SqliteStore, Box<dyn std::error::Error>> {
    let store = SqliteStore::open(directory.join(STORE_FILE_NAME))?;
    let graph = GraphDocument::load(graph_path)?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let reference = store.snapshot_reference()?;
    fs::write(
        directory.join(STORE_REF_FILE_NAME),
        serde_json::to_vec(&reference)?,
    )?;
    store.checkpoint()?;
    Ok(store)
}

#[test]
fn default_query_open_uses_json_and_explicit_store_matches_results()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    publish_phase2_snapshot(directory.path(), &graph_path)?;

    let cache = directory.path().join("cache");
    let json_engine = open(&graph_path, None, &cache)?;
    assert_eq!(json_engine.engine_kind(), QueryEngineKind::Json);
    let store_engine = open_with_engine(&graph_path, None, &cache, EngineSelection::Store)?;
    assert_eq!(store_engine.engine_kind(), QueryEngineKind::Store);

    let request = SearchRequest {
        query: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store_engine.search(request.clone())?)?,
        serde_json::to_value(json_engine.search(request)?)?,
    );
    assert_ne!(store_engine.index_path(), json_engine.index_path());
    assert_eq!(
        store_engine
            .index_path()
            .file_name()
            .and_then(|name| name.to_str()),
        Some(STORE_FILE_NAME)
    );

    drop(store_engine);
    let reopened = open_with_engine(&graph_path, None, &cache, EngineSelection::Store)?;
    assert_eq!(reopened.engine_kind(), QueryEngineKind::Store);
    Ok(())
}

#[test]
fn bounded_search_candidate_order_matches_store_postings() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let template = graph
        .nodes
        .first()
        .cloned()
        .ok_or("fixture graph has no template node")?;
    for ordinal in 0..600 {
        let mut node = template.clone();
        node.id = format!("n:bulk:{ordinal:04}");
        node.name = format!("symbol_{ordinal:04}_{}", "padding".repeat(ordinal % 7));
        node.qualified_name = format!("Bulk.{}", node.name);
        node.language = Some("django".to_owned());
        graph.nodes.push(node);
    }
    fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
    publish_phase2_snapshot(directory.path(), &graph_path)?;

    let cache = directory.path().join("cache");
    let store = open_with_engine(&graph_path, None, &cache, EngineSelection::Store)?;
    let json = open_with_engine(&graph_path, None, &cache, EngineSelection::Json)?;
    let mut limits = CodeQueryLimits::default();
    limits.max_candidates = 10;
    limits.max_nodes = 100;
    let request = SearchRequest {
        query: "django".to_owned(),
        limits,
    };
    assert_eq!(
        serde_json::to_value(store.search(request.clone())?)?,
        serde_json::to_value(json.search(request)?)?,
    );
    Ok(())
}

#[test]
fn store_engine_reads_the_immutable_phase2_snapshot_for_all_code_queries()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    publish_phase2_snapshot(directory.path(), &graph_path)?;

    let cache = directory.path().join("cache");
    let store = open_with_engine(&graph_path, None, &cache, EngineSelection::Store)?;
    let json = open_with_engine(&graph_path, None, &cache, EngineSelection::Json)?;
    assert_eq!(store.engine_kind(), QueryEngineKind::Store);
    assert_eq!(json.engine_kind(), QueryEngineKind::Json);
    assert_ne!(store.index_path(), json.index_path());

    let search = SearchRequest {
        query: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.search(search.clone())?)?,
        serde_json::to_value(json.search(search)?)?,
    );
    let callers = CallRequest {
        symbol: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.callers(callers.clone())?)?,
        serde_json::to_value(json.callers(callers)?)?,
    );
    let callees = CallRequest {
        symbol: "Api.caller".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.callees(callees.clone())?)?,
        serde_json::to_value(json.callees(callees)?)?,
    );
    let impact = ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.impact(impact.clone())?)?,
        serde_json::to_value(json.impact(impact)?)?,
    );
    let explore = ExploreRequest {
        symbols: vec!["Api.caller".to_owned(), "Store.callee".to_owned()],
        root: directory.path().to_string_lossy().into_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.explore(explore.clone())?)?,
        serde_json::to_value(json.explore(explore)?)?,
    );
    let trail = NodeTrailRequest {
        source: "Api.caller".to_owned(),
        target: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.node_trail(trail.clone())?)?,
        serde_json::to_value(json.node_trail(trail)?)?,
    );
    Ok(())
}

#[test]
fn redb_store_runs_the_same_typed_queries_as_json() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let redb = RedbStore::open(directory.path().join(compass_store_redb::REDB_FILE_NAME))?;
    let graph = GraphDocument::load(&graph_path)?;
    let prepared = GraphSnapshotBuilder::new().prepare(&redb, &graph)?;
    GraphSnapshotBuilder::new().activate(&redb, &prepared)?;

    let store = open_with_store(
        &redb,
        &graph_path,
        None,
        &directory.path().join("redb-cache"),
    )?;
    let json = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("json-cache"),
        EngineSelection::Json,
    )?;
    assert_eq!(store.engine_kind(), QueryEngineKind::Store);

    let search = SearchRequest {
        query: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.search(search.clone())?)?,
        serde_json::to_value(json.search(search)?)?,
    );
    let callers = CallRequest {
        symbol: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.callers(callers.clone())?)?,
        serde_json::to_value(json.callers(callers)?)?,
    );
    let callees = CallRequest {
        symbol: "Api.caller".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.callees(callees.clone())?)?,
        serde_json::to_value(json.callees(callees)?)?,
    );
    let impact = ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.impact(impact.clone())?)?,
        serde_json::to_value(json.impact(impact)?)?,
    );
    let explore = ExploreRequest {
        symbols: vec!["Api.caller".to_owned(), "Store.callee".to_owned()],
        root: directory.path().to_string_lossy().into_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.explore(explore.clone())?)?,
        serde_json::to_value(json.explore(explore)?)?,
    );
    let trail = NodeTrailRequest {
        source: "Api.caller".to_owned(),
        target: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.node_trail(trail.clone())?)?,
        serde_json::to_value(json.node_trail(trail)?)?,
    );
    Ok(())
}

#[test]
fn phase2_snapshot_is_authoritative_over_the_legacy_payload_selector()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let store = SqliteStore::open(directory.path().join(STORE_FILE_NAME))?;
    store.publish_snapshot(b"not the phase2 graph", CODE_GRAPH_SCHEMA_V1, 0, 0)?;
    drop(store);
    publish_phase2_snapshot(directory.path(), &graph_path)?;

    let engine = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("cache"),
        EngineSelection::Store,
    )?;
    let response = engine.search(SearchRequest {
        query: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    })?;
    assert!(response.results.iter().any(|hit| hit.node_id == "n:list"));
    Ok(())
}

#[test]
fn active_phase2_snapshot_requires_a_matching_store_reference()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    publish_phase2_snapshot(directory.path(), &graph_path)?;
    fs::remove_file(directory.path().join(STORE_REF_FILE_NAME))?;

    let error = match open_with_engine(
        &graph_path,
        None,
        &directory.path().join("cache"),
        EngineSelection::Store,
    ) {
        Ok(_) => return Err("active snapshot opened without store.ref".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "store_ref_missing");
    Ok(())
}

#[test]
fn active_phase2_snapshot_rejects_a_stale_store_reference() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    publish_phase2_snapshot(directory.path(), &graph_path)?;
    let reference_path = directory.path().join(STORE_REF_FILE_NAME);
    let mut reference: StoreRef = serde_json::from_slice(&fs::read(&reference_path)?)?;
    reference.graph_digest = "0".repeat(64);
    fs::write(reference_path, serde_json::to_vec(&reference)?)?;

    let error = match open_with_engine(
        &graph_path,
        None,
        &directory.path().join("cache"),
        EngineSelection::Store,
    ) {
        Ok(_) => return Err("stale store.ref was accepted".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "store_ref_mismatch");
    Ok(())
}

#[test]
fn opened_store_reader_remains_pinned_when_the_active_selector_changes()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let store = publish_phase2_snapshot(directory.path(), &graph_path)?;
    let cache = directory.path().join("cache");
    let old = open_with_engine(&graph_path, None, &cache, EngineSelection::Store)?;

    let mut updated = GraphDocument::load(&graph_path)?;
    updated.nodes[0].qualified_name = "UserService.renamed".to_owned();
    fs::write(&graph_path, serde_json::to_vec(&updated)?)?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &updated)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    fs::write(
        directory.path().join(STORE_REF_FILE_NAME),
        serde_json::to_vec(&store.snapshot_reference()?)?,
    )?;

    let current = open_with_engine(&graph_path, None, &cache, EngineSelection::Store)?;
    let old_response = old.search(SearchRequest {
        query: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    })?;
    let current_response = current.search(SearchRequest {
        query: "UserService.renamed".to_owned(),
        limits: CodeQueryLimits::default(),
    })?;
    assert!(
        old_response
            .results
            .iter()
            .any(|hit| hit.node_id == "n:list")
    );
    assert!(
        current_response
            .results
            .iter()
            .any(|hit| hit.node_id == "n:list")
    );
    Ok(())
}

#[test]
fn explicit_store_selection_reports_a_missing_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let result = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("cache"),
        EngineSelection::Store,
    );
    let error = match result {
        Ok(_) => return Err("store selection silently fell back to JSON".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "store_ref_missing");
    Ok(())
}

#[test]
fn explicit_json_selection_survives_a_corrupt_store_sidecar()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    publish_phase2_snapshot(directory.path(), &graph_path)?;
    fs::write(
        directory.path().join(STORE_FILE_NAME),
        b"not a compass sqlite database",
    )?;
    let cache = directory.path().join("cache");
    let json_engine = open(&graph_path, None, &cache)?;
    assert_eq!(json_engine.engine_kind(), QueryEngineKind::Json);
    let result = open_with_engine(&graph_path, None, &cache, EngineSelection::Store);
    let error = match result {
        Ok(_) => return Err("explicit store selection ignored a corrupt store".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "store_open_failed");
    Ok(())
}

#[test]
fn a_present_malformed_store_reference_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    publish_phase2_snapshot(directory.path(), &graph_path)?;
    fs::write(directory.path().join(STORE_REF_FILE_NAME), b"{}")?;
    let cache = directory.path().join("cache");
    assert_eq!(
        open(&graph_path, None, &cache)?.engine_kind(),
        QueryEngineKind::Json
    );
    let error = match open_with_engine(&graph_path, None, &cache, EngineSelection::Store) {
        Ok(_) => return Err("explicit store selection accepted a malformed reference".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "store_ref_decode_failed");
    Ok(())
}
