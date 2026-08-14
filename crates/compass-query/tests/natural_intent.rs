mod support;

use std::fs;
use std::path::Path;

use compass_graph::GraphSnapshotBuilder;
use compass_model::query_contract::{
    CodeQueryLimits, CodeQueryOperation, MAX_INDEXED_CANDIDATE_NODES_READ, QueryDiagnosticCode,
};
use compass_query::{
    EngineSelection, NaturalQueryIntent, NaturalQueryRequest, ProfiledCodeQueryResponse,
    QUERY_EXECUTION_PROFILE_V1, QUERY_PLANNER_PROFILE_V1, QUERY_RANKER_PROFILE_V1,
    open_with_engine, plan_natural_query,
};
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
fn structural_intents_use_bounded_fuzzy_recall_and_relation_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = compass_model::code_graph::GraphDocument::load(&graph_path)?;
    let mut distractor = graph
        .nodes
        .iter()
        .find(|node| node.id == "n:list")
        .cloned()
        .ok_or("missing list fixture node")?;
    distractor.id = "n:ilts".to_owned();
    distractor.name = "ilts".to_owned();
    distractor.qualified_name = "Noise.ilts".to_owned();
    graph.nodes.push(distractor);
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;
    publish_snapshot(directory.path(), &graph_path)?;

    for selection in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &graph_path,
            None,
            &directory.path().join(format!("{selection:?}-cache")),
            selection,
        )?;
        let response = engine.query_natural(request("who calls UserService.lits?"))?;
        assert_eq!(response.operation, CodeQueryOperation::Callers);
        assert!(
            response.nodes.iter().any(|node| node.id == "n:list"),
            "{response:#?}"
        );
        assert!(!response.nodes.iter().any(|node| node.id == "n:ilts"));
        assert!(response.edges.iter().any(|edge| edge.target == "n:list"));
    }
    Ok(())
}

#[test]
fn profiled_natural_queries_report_real_stage_work_without_changing_the_response()
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
    let profiled = engine.query_natural_profiled(request("who calls UserService.list?"))?;
    let repeated = engine.query_natural_profiled(request("who calls UserService.list?"))?;
    let ordinary = engine.query_natural(request("who calls UserService.list?"))?;

    assert_eq!(profiled.response, ordinary);
    assert_eq!(profiled.response, repeated.response);
    assert_eq!(profiled.profile.schema, QUERY_EXECUTION_PROFILE_V1);
    assert_eq!(profiled.profile.planner_profile, QUERY_PLANNER_PROFILE_V1);
    assert_eq!(profiled.profile.ranker_profile, QUERY_RANKER_PROFILE_V1);
    assert!(profiled.profile.work.candidates_read > 0);
    assert_eq!(
        profiled.profile.work.candidates_read,
        repeated.profile.work.candidates_read
    );
    assert!(profiled.profile.work.candidates_read <= MAX_INDEXED_CANDIDATE_NODES_READ);
    assert!(profiled.profile.work.nodes_expanded > 0);
    assert!(profiled.profile.work.edges_expanded > 0);
    assert_eq!(
        profiled.profile.work.response_bytes,
        u64::try_from(serde_json::to_vec(&ordinary)?.len())?
    );
    assert!(profiled.profile.timings.total_micros >= profiled.profile.timings.intent_micros);
    assert!(profiled.profile.timings.total_micros >= profiled.profile.timings.recall_micros);
    assert!(profiled.profile.timings.total_micros >= profiled.profile.timings.execution_micros);
    let encoded = serde_json::to_value(&profiled)?;
    assert_eq!(
        serde_json::from_value::<ProfiledCodeQueryResponse>(encoded.clone())?,
        profiled
    );
    let mut unknown = encoded;
    unknown
        .as_object_mut()
        .ok_or("profile envelope must be an object")?
        .insert("unexpected".to_owned(), serde_json::Value::Bool(true));
    assert!(serde_json::from_value::<ProfiledCodeQueryResponse>(unknown).is_err());
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

#[test]
fn planner_profile_covers_reviewed_phrase_variants_and_safe_fallbacks()
-> Result<(), Box<dyn std::error::Error>> {
    let cases = [
        ("who calls Target?", NaturalQueryIntent::Callers, true),
        ("what calls Target", NaturalQueryIntent::Callers, true),
        (
            "which functions call Target",
            NaturalQueryIntent::Callers,
            true,
        ),
        (
            "which methods call Target",
            NaturalQueryIntent::Callers,
            true,
        ),
        ("where is Target called", NaturalQueryIntent::Callers, true),
        ("callers of Target", NaturalQueryIntent::Callers, true),
        ("what does Source call?", NaturalQueryIntent::Callees, true),
        (
            "what functions does Source call",
            NaturalQueryIntent::Callees,
            true,
        ),
        (
            "what methods does Source invoke",
            NaturalQueryIntent::Callees,
            true,
        ),
        ("callees of Source", NaturalQueryIntent::Callees, true),
        ("calls made by Source", NaturalQueryIntent::Callees, true),
        ("what depends on Target?", NaturalQueryIntent::Impact, true),
        (
            "what is impacted by Target",
            NaturalQueryIntent::Impact,
            true,
        ),
        (
            "what is the impact of Target",
            NaturalQueryIntent::Impact,
            true,
        ),
        (
            "what would break if Target changes",
            NaturalQueryIntent::Impact,
            true,
        ),
        (
            "if Target changes, what breaks",
            NaturalQueryIntent::Impact,
            true,
        ),
        ("dependents of Target", NaturalQueryIntent::Impact, true),
        (
            "path from Source to Target",
            NaturalQueryIntent::NodeTrail,
            true,
        ),
        (
            "shortest path from Source to Target",
            NaturalQueryIntent::NodeTrail,
            true,
        ),
        (
            "route from Source to Target",
            NaturalQueryIntent::NodeTrail,
            true,
        ),
        (
            "connection from Source to Target",
            NaturalQueryIntent::NodeTrail,
            true,
        ),
        (
            "how can Source reach Target",
            NaturalQueryIntent::NodeTrail,
            true,
        ),
        ("where is Target defined?", NaturalQueryIntent::Search, true),
        (
            "find definition of Target",
            NaturalQueryIntent::Search,
            true,
        ),
        ("search for Target", NaturalQueryIntent::Search, true),
        ("show me Target", NaturalQueryIntent::Search, true),
        (
            "where is authentication enforced?",
            NaturalQueryIntent::Search,
            false,
        ),
        ("authentication flow", NaturalQueryIntent::Fallback, false),
        (
            "show callers and callees of Target",
            NaturalQueryIntent::Fallback,
            false,
        ),
        (
            "show incoming and outgoing calls for Target",
            NaturalQueryIntent::Fallback,
            false,
        ),
        ("", NaturalQueryIntent::Fallback, false),
    ];
    for (question, expected, auto_route) in cases {
        let plan = plan_natural_query(question)?;
        assert_eq!(plan.profile(), QUERY_PLANNER_PROFILE_V1, "{question:?}");
        assert_eq!(plan.intent(), expected, "{question:?}");
        assert_eq!(plan.routes_to_typed_query(), auto_route, "{question:?}");
        assert_eq!(
            plan.confidence() > 0,
            expected != NaturalQueryIntent::Fallback,
            "{question:?}"
        );
    }
    Ok(())
}
