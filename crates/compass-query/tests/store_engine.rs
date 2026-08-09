mod support;

use std::fs;

use compass_graph::{GraphSnapshotBuilder, GraphSnapshotReader};
use compass_model::code_graph::{CODE_GRAPH_SCHEMA_V1, GraphDocument};
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest,
    DiscoveryQueryResponse, DiscoveryScope, DiscoveryScopeKind, DiscoveryTraversal, ExploreRequest,
    ImpactRequest, MAX_DISCOVERY_CANDIDATE_NODES_READ, MAX_DISCOVERY_CANDIDATE_PROBES,
    NodeTrailRequest, SearchRequest,
};
use compass_query::{
    EngineSelection, QueryEngineKind, open, open_with_document, open_with_engine, open_with_store,
    open_with_store_selector,
};
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

fn discovery_request(question: &str) -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: question.to_owned(),
        direction: DiscoveryDirection::Both,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits::default(),
    }
}

fn assert_discovery_semantically_equal(
    actual: &DiscoveryQueryResponse,
    expected: &DiscoveryQueryResponse,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut actual_value = serde_json::to_value(actual)?;
    let mut expected_value = serde_json::to_value(expected)?;
    actual_value
        .as_object_mut()
        .ok_or("discovery response must serialize as an object")?
        .remove("stats");
    expected_value
        .as_object_mut()
        .ok_or("discovery response must serialize as an object")?
        .remove("stats");
    assert_eq!(actual_value, expected_value);
    for response in [actual, expected] {
        assert!(response.stats.candidate_nodes <= MAX_DISCOVERY_CANDIDATE_NODES_READ);
        assert!(response.stats.candidate_probes <= MAX_DISCOVERY_CANDIDATE_PROBES);
    }
    Ok(())
}

#[test]
fn discovery_is_identical_for_json_store_direct_document_and_immutable_selectors()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let first_graph = GraphDocument::load(&graph_path)?;
    let store = publish_phase2_snapshot(directory.path(), &graph_path)?;
    let first_selector = GraphSnapshotReader::open_active(&store)?
        .ok_or("first snapshot missing")?
        .selector()
        .clone();

    let json = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("json-cache"),
        EngineSelection::Json,
    )?;
    let json_reopened = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("json-cache"),
        EngineSelection::Json,
    )?;
    assert_eq!(json.index_path(), json_reopened.index_path());
    let active = open_with_store(
        &store,
        &graph_path,
        None,
        &directory.path().join("active-cache"),
    )?;
    let local_direct = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("local-direct-cache"),
        EngineSelection::Store,
    )?;
    let selected = open_with_store_selector(
        &store,
        first_selector.clone(),
        &graph_path,
        None,
        &directory.path().join("selector-cache"),
    )?;
    let direct = open_with_document(
        first_graph.clone(),
        &graph_path,
        None,
        &directory.path().join("shared-direct-cache"),
    )?;
    let direct_reopened = open_with_document(
        first_graph.clone(),
        &graph_path,
        None,
        &directory.path().join("shared-direct-cache"),
    )?;
    assert_eq!(direct.engine_kind(), QueryEngineKind::Memory);
    assert_eq!(direct.index_path(), direct_reopened.index_path());
    let request = discovery_request("UserService.list");
    let expected = json.discover(request.clone())?;
    for actual in [
        active.discover(request.clone())?,
        local_direct.discover(request.clone())?,
        selected.discover(request.clone())?,
        direct.discover(request.clone())?,
    ] {
        assert_discovery_semantically_equal(&actual, &expected)?;
    }
    assert_eq!(json.discover(request.clone())?.stats, expected.stats);
    assert_eq!(
        active.discover(request.clone())?.stats,
        active.discover(request.clone())?.stats
    );
    assert_eq!(
        local_direct.discover(request.clone())?.stats,
        local_direct.discover(request.clone())?.stats
    );

    let mut scoped = discovery_request("UserService.list");
    scoped.scope = vec![DiscoveryScope {
        kind: DiscoveryScopeKind::Node,
        value: "UserService.list".to_owned(),
    }];
    let scoped_expected = json.discover(scoped.clone())?;
    for actual in [
        active.discover(scoped.clone())?,
        local_direct.discover(scoped.clone())?,
        selected.discover(scoped.clone())?,
        direct.discover(scoped)?,
    ] {
        assert_discovery_semantically_equal(&actual, &scoped_expected)?;
    }

    for scope in [
        DiscoveryScope {
            kind: DiscoveryScopeKind::Source,
            value: "src".to_owned(),
        },
        DiscoveryScope {
            kind: DiscoveryScopeKind::Community,
            value: "services".to_owned(),
        },
    ] {
        let mut scoped = discovery_request("UserService.list");
        scoped.scope = vec![scope];
        let expected = json.discover(scoped.clone())?;
        for actual in [
            json_reopened.discover(scoped.clone())?,
            active.discover(scoped.clone())?,
            local_direct.discover(scoped.clone())?,
            selected.discover(scoped.clone())?,
            direct.discover(scoped)?,
        ] {
            assert_discovery_semantically_equal(&actual, &expected)?;
        }
    }

    let ambiguous = discovery_request("list");
    let ambiguous_expected = json.discover(ambiguous.clone())?;
    for actual in [
        active.discover(ambiguous.clone())?,
        selected.discover(ambiguous.clone())?,
        direct.discover(ambiguous)?,
    ] {
        assert_discovery_semantically_equal(&actual, &ambiguous_expected)?;
    }
    assert_discovery_semantically_equal(&direct_reopened.discover(request.clone())?, &expected)?;
    assert_discovery_semantically_equal(&direct.discover(request.clone())?, &expected)?;

    let boolean_only = discovery_request("AND OR NOT NEAR");
    let empty_expected = json.discover(boolean_only.clone())?;
    for actual in [
        local_direct.discover(boolean_only.clone())?,
        selected.discover(boolean_only.clone())?,
        direct.discover(boolean_only)?,
    ] {
        assert_discovery_semantically_equal(&actual, &empty_expected)?;
    }

    let mut second_graph = first_graph;
    let mut added = second_graph.nodes[0].clone();
    added.id = "n:new-realization".to_owned();
    added.name = "new_realization".to_owned();
    added.qualified_name = "History.new_realization".to_owned();
    second_graph.nodes.push(added);
    second_graph
        .nodes
        .sort_by(|left, right| left.id.cmp(&right.id));
    let replacement = open_with_document(
        second_graph.clone(),
        &graph_path,
        None,
        &directory.path().join("shared-direct-cache"),
    )?;
    assert_ne!(direct.index_path(), replacement.index_path());
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &second_graph)?;
    let second_selector = GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let historical = open_with_store_selector(
        &store,
        first_selector.clone(),
        &graph_path,
        None,
        &directory.path().join("historical-cache"),
    )?;
    assert_discovery_semantically_equal(&historical.discover(request.clone())?, &expected)?;
    let current = open_with_store_selector(
        &store,
        second_selector,
        &graph_path,
        None,
        &directory.path().join("current-cache"),
    )?;
    let current_response = current.discover(discovery_request("new_realization"))?;
    assert!(
        current_response
            .nodes
            .iter()
            .any(|node| node.id == "n:new-realization")
    );
    assert_ne!(historical.index_path(), current.index_path());

    let mut corrupt = first_selector.clone();
    corrupt.schema = "corrupt-selector".to_owned();
    let error = open_with_store_selector(
        &store,
        corrupt,
        &graph_path,
        None,
        &directory.path().join("corrupt-selector-cache"),
    )
    .err()
    .ok_or("corrupt selector unexpectedly opened")?;
    assert_eq!(error.code(), "store_graph_snapshot_failed");

    let mut mismatched = first_selector;
    mismatched.snapshot_id = "0".repeat(64);
    let error = open_with_store_selector(
        &store,
        mismatched,
        &graph_path,
        None,
        &directory.path().join("mismatch-cache"),
    )
    .err()
    .ok_or("mismatched selector unexpectedly opened")?;
    assert_eq!(error.code(), "store_graph_snapshot_failed");
    Ok(())
}

#[test]
fn default_query_open_prefers_published_store_and_matches_json()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    publish_phase2_snapshot(directory.path(), &graph_path)?;

    let cache = directory.path().join("cache");
    let default_engine = open(&graph_path, None, &cache)?;
    assert_eq!(default_engine.engine_kind(), QueryEngineKind::Store);
    let store_engine = open_with_engine(&graph_path, None, &cache, EngineSelection::Store)?;
    assert_eq!(store_engine.engine_kind(), QueryEngineKind::Store);
    let json_engine = open_with_engine(&graph_path, None, &cache, EngineSelection::Json)?;
    assert_eq!(json_engine.engine_kind(), QueryEngineKind::Json);

    let request = SearchRequest {
        query: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store_engine.search(request.clone())?)?,
        serde_json::to_value(default_engine.search(request.clone())?)?,
    );
    assert_eq!(
        serde_json::to_value(store_engine.search(request)?)?,
        serde_json::to_value(json_engine.search(SearchRequest {
            query: "UserService.list".to_owned(),
            limits: CodeQueryLimits::default(),
        })?)?,
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
        node.id = format!("n:bulk:{:04}", 599 - ordinal);
        node.name = format!("dja{ordinal:04}_{}", "padding".repeat(ordinal % 7));
        node.qualified_name = format!("Bulk.{}", node.name);
        graph.nodes.push(node);
    }
    fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
    publish_phase2_snapshot(directory.path(), &graph_path)?;

    let cache = directory.path().join("cache");
    let store = open_with_engine(&graph_path, None, &cache, EngineSelection::Store)?;
    let json = open_with_engine(&graph_path, None, &cache, EngineSelection::Json)?;
    let limits = CodeQueryLimits {
        max_candidates: 10,
        max_nodes: 100,
        ..CodeQueryLimits::default()
    };
    let request = SearchRequest {
        query: "dja".to_owned(),
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
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.callers(callers.clone())?)?,
        serde_json::to_value(json.callers(callers)?)?,
    );
    let callees = CallRequest {
        symbol: "Api.caller".to_owned(),
        include_heuristic: false,
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
        include_heuristic: false,
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
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store.callers(callers.clone())?)?,
        serde_json::to_value(json.callers(callers)?)?,
    );
    let callees = CallRequest {
        symbol: "Api.caller".to_owned(),
        include_heuristic: false,
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
        include_heuristic: false,
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
    let json_engine = open_with_engine(&graph_path, None, &cache, EngineSelection::Json)?;
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
fn json_index_v3_is_rebuilt_to_v4() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let cache = directory.path().join("cache");
    let engine = open_with_engine(&graph_path, None, &cache, EngineSelection::Json)?;
    let index_path = engine.index_path().to_path_buf();
    drop(engine);

    let connection = rusqlite::Connection::open(&index_path)?;
    connection.execute(
        "UPDATE metadata SET value='compass-code-index/3' WHERE key='format'",
        [],
    )?;
    drop(connection);

    let reopened = open_with_engine(&graph_path, None, &cache, EngineSelection::Json)?;
    assert_eq!(reopened.index_path(), index_path);
    let connection = rusqlite::Connection::open(index_path)?;
    let format: String =
        connection.query_row("SELECT value FROM metadata WHERE key='format'", [], |row| {
            row.get(0)
        })?;
    assert_eq!(format, "compass-code-index/4");
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
    let error = match open(&graph_path, None, &cache) {
        Ok(_) => return Err("default selection ignored a malformed store reference".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "store_ref_decode_failed");
    let error = match open_with_engine(&graph_path, None, &cache, EngineSelection::Store) {
        Ok(_) => return Err("explicit store selection accepted a malformed reference".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "store_ref_decode_failed");
    Ok(())
}
