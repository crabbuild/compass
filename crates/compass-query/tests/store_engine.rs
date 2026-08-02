mod support;

use std::fs;

use compass_model::code_graph::{CODE_GRAPH_SCHEMA_V1, GraphDocument};
use compass_model::query_contract::{CodeQueryLimits, SearchRequest};
use compass_query::{EngineSelection, QueryEngineKind, open, open_with_engine};
use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore};

#[test]
fn default_query_open_prefers_store_and_matches_json_results()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let graph_bytes = fs::read(&graph_path)?;
    let graph = GraphDocument::load(&graph_path)?;
    SqliteStore::open(directory.path().join(STORE_FILE_NAME))?.publish_snapshot(
        &graph_bytes,
        CODE_GRAPH_SCHEMA_V1,
        graph.nodes.len(),
        graph.links.len(),
    )?;

    let cache = directory.path().join("cache");
    let store_engine = open(&graph_path, None, &cache)?;
    assert_eq!(store_engine.engine_kind(), QueryEngineKind::Store);
    let json_engine = open_with_engine(&graph_path, None, &cache, EngineSelection::Json)?;
    assert_eq!(json_engine.engine_kind(), QueryEngineKind::Json);

    let request = SearchRequest {
        query: "UserService.list".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_value(store_engine.search(request.clone())?)?,
        serde_json::to_value(json_engine.search(request)?)?,
    );

    drop(store_engine);
    let reopened = open_with_engine(&graph_path, None, &cache, EngineSelection::Store)?;
    assert_eq!(reopened.engine_kind(), QueryEngineKind::Store);
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
    assert_eq!(error.code(), "store_open_failed");
    Ok(())
}

#[test]
fn explicit_json_selection_survives_a_corrupt_store_sidecar()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    fs::write(
        directory.path().join(STORE_FILE_NAME),
        b"not a compass sqlite database",
    )?;
    let cache = directory.path().join("cache");
    let json_engine = open_with_engine(&graph_path, None, &cache, EngineSelection::Json)?;
    assert_eq!(json_engine.engine_kind(), QueryEngineKind::Json);
    let result = open(&graph_path, None, &cache);
    let error = match result {
        Ok(_) => return Err("default selection ignored a corrupt store".into()),
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
    let graph_bytes = fs::read(&graph_path)?;
    let graph = GraphDocument::load(&graph_path)?;
    SqliteStore::open(directory.path().join(STORE_FILE_NAME))?.publish_snapshot(
        &graph_bytes,
        CODE_GRAPH_SCHEMA_V1,
        graph.nodes.len(),
        graph.links.len(),
    )?;
    fs::write(directory.path().join(STORE_REF_FILE_NAME), b"{}")?;
    let error = match open(&graph_path, None, &directory.path().join("cache")) {
        Ok(_) => return Err("malformed store reference was accepted".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "store_ref_decode_failed");
    Ok(())
}
