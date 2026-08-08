mod support;

use std::fs;
use std::path::Path;

use compass_graph::GraphSnapshotBuilder;
use compass_model::query_contract::{CodeQueryLimits, CodeQueryOperation, QueryDiagnosticCode};
use compass_query::{EngineSelection, NaturalQueryRequest, open_with_engine};
use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore};

fn publish_snapshot(directory: &Path, graph_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::open(directory.join(STORE_FILE_NAME))?;
    let graph = compass_model::code_graph::GraphDocument::load(graph_path)?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    fs::write(
        directory.join(STORE_REF_FILE_NAME),
        serde_json::to_vec(&store.snapshot_reference()?)?,
    )?;
    store.checkpoint()?;
    Ok(())
}

fn request(question: &str) -> NaturalQueryRequest {
    NaturalQueryRequest {
        question: question.to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    }
}

#[test]
fn natural_intents_route_to_typed_operations_with_backend_parity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    publish_snapshot(directory.path(), &graph_path)?;
    let store = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("store-cache"),
        EngineSelection::Store,
    )?;
    let json = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("json-cache"),
        EngineSelection::Json,
    )?;

    for (question, operation, required_id) in [
        (
            "who calls UserService.list?",
            CodeQueryOperation::Callers,
            "n:caller",
        ),
        (
            "what does Api.caller call?",
            CodeQueryOperation::Callees,
            "n:list",
        ),
        (
            "what depends on Api.caller?",
            CodeQueryOperation::Impact,
            "n:dependent",
        ),
        (
            "path from Api.caller to Store.callee",
            CodeQueryOperation::NodeTrail,
            "n:callee",
        ),
        (
            "where is résumé defined?",
            CodeQueryOperation::Search,
            "n:resume",
        ),
    ] {
        let store_response = store.query_natural(request(question))?;
        let json_response = json.query_natural(request(question))?;
        assert_eq!(
            serde_json::to_vec(&store_response)?,
            serde_json::to_vec(&json_response)?,
            "backend mismatch for {question:?}"
        );
        assert_eq!(store_response.operation, operation, "{question:?}");
        assert!(
            store_response
                .nodes
                .iter()
                .any(|node| node.id == required_id),
            "{question:?} did not return {required_id}"
        );
    }
    Ok(())
}

#[test]
fn contradictory_and_ambiguous_questions_never_invent_direction()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("cache"),
        EngineSelection::Json,
    )?;

    let contradictory =
        engine.query_natural(request("show callers and callees of UserService.list"))?;
    assert_eq!(contradictory.operation, CodeQueryOperation::Search);

    let ambiguous = engine.query_natural(request("who calls list?"))?;
    assert_eq!(ambiguous.operation, CodeQueryOperation::Callers);
    assert!(ambiguous.nodes.is_empty());
    assert!(
        ambiguous
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == QueryDiagnosticCode::AmbiguousMatch })
    );
    Ok(())
}

#[test]
fn natural_question_size_is_bounded_before_graph_work() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open_with_engine(
        &graph_path,
        None,
        &directory.path().join("cache"),
        EngineSelection::Json,
    )?;
    assert!(engine.query_natural(request(&"x".repeat(4_097))).is_err());
    Ok(())
}
