mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Instant;

use compass_graph::{GraphSnapshotBuilder, canonical_graph_json};
use compass_model::code_graph::GraphDocument;
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, NodeTrailRequest,
    SearchRequest,
};
use compass_query::{
    EdgeIdentity, EdgeJudgment, IdJudgment, JudgedQuery, JudgmentCorpus, ObservedEdge,
    ObservedPath, PathJudgment, PathPattern, QueryClass, QueryObservation, RelevanceError,
    WorkCounts, qualification_report, score,
};
use compass_query::{EngineSelection, open_with_engine};
use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore};
use sha2::{Digest, Sha256};

const FIXTURE_DIGEST: &str = "sha256:relevance-synthetic-v1";

fn corpus() -> Result<JudgmentCorpus, Box<dyn std::error::Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/relevance/judged.json");
    let corpus = serde_json::from_slice::<JudgmentCorpus>(&fs::read(path)?)?;
    Ok(corpus)
}

fn observation(query: &compass_query::JudgedQuery, ordinal: u64) -> QueryObservation {
    let mut node_ids = query
        .node_judgments
        .iter()
        .filter(|judgment| judgment.grade > 0)
        .map(|judgment| (std::cmp::Reverse(judgment.grade), judgment.id.clone()))
        .collect::<Vec<_>>();
    node_ids.sort();
    let mut node_ids = node_ids.into_iter().map(|(_, id)| id).collect::<Vec<_>>();
    node_ids.extend(query.acceptable_ambiguity.iter().cloned());
    node_ids.sort();
    node_ids.dedup();
    let edges = query
        .edge_judgments
        .iter()
        .filter(|judgment| judgment.grade >= 2)
        .map(|judgment| observed_edge(&judgment.edge))
        .collect();
    let paths = query
        .path_judgments
        .iter()
        .filter(|judgment| judgment.grade >= 2)
        .map(|judgment| ObservedPath {
            edge_kinds: judgment.pattern.edge_kinds.clone(),
            endpoint_ids: judgment.pattern.endpoint_ids.clone(),
        })
        .collect();
    QueryObservation {
        query_id: query.id.clone(),
        intent: query.expected_intent.clone(),
        slots: query.expected_slots.clone(),
        node_ids,
        edges,
        paths,
        no_answer: matches!(query.class, QueryClass::Negative),
        latency_micros: Some(ordinal.saturating_mul(10)),
        work: WorkCounts {
            candidates_read: 2,
            postings_decoded: 1,
            nodes_expanded: 1,
            edges_expanded: 1,
            response_bytes: 128,
        },
    }
}

fn observed_edge(edge: &EdgeIdentity) -> ObservedEdge {
    ObservedEdge {
        id: edge.id.clone().unwrap_or_default(),
        source: edge.source.clone().unwrap_or_default(),
        target: edge.target.clone().unwrap_or_default(),
        kind: edge.kind.clone().unwrap_or_default(),
        direction: edge.direction.clone().unwrap_or_default(),
    }
}

fn operation_intent(operation: CodeQueryOperation) -> &'static str {
    match operation {
        CodeQueryOperation::Search => "search",
        CodeQueryOperation::Callers => "callers",
        CodeQueryOperation::Callees => "callees",
        CodeQueryOperation::Impact => "impact",
        CodeQueryOperation::Explore => "explore",
        CodeQueryOperation::NodeTrail => "node_trail",
    }
}

fn observe_execution(
    query_id: &str,
    response: &CodeQueryResponse,
    latency_micros: u64,
) -> Result<QueryObservation, Box<dyn std::error::Error>> {
    let edge_kinds = response
        .edges
        .iter()
        .map(|edge| (edge.id.as_str(), edge.kind.as_str()))
        .collect::<BTreeMap<_, _>>();
    let node_ids = if response.results.is_empty() {
        response.nodes.iter().map(|node| node.id.clone()).collect()
    } else {
        response
            .results
            .iter()
            .map(|result| result.node_id.clone())
            .collect()
    };
    let response_bytes = u64::try_from(serde_json::to_vec(response)?.len())?;
    let mut slots = BTreeMap::from([(
        "operation".to_owned(),
        operation_intent(response.operation).to_owned(),
    )]);
    slots.insert("truncated".to_owned(), response.truncated.to_string());
    Ok(QueryObservation {
        query_id: query_id.to_owned(),
        intent: Some(operation_intent(response.operation).to_owned()),
        slots,
        node_ids,
        edges: response
            .edges
            .iter()
            .map(|edge| ObservedEdge {
                id: edge.id.clone(),
                source: edge.source.clone(),
                target: edge.target.clone(),
                kind: edge.kind.as_str().to_owned(),
                // `compass.graph/1` edges are directed from `source` to `target`.
                direction: "source_to_target".to_owned(),
            })
            .collect(),
        paths: response
            .paths
            .iter()
            .map(|path| ObservedPath {
                edge_kinds: path
                    .edge_ids
                    .iter()
                    .filter_map(|id| edge_kinds.get(id.as_str()).map(|kind| (*kind).to_owned()))
                    .collect(),
                endpoint_ids: match (path.node_ids.first(), path.node_ids.last()) {
                    (Some(first), Some(last)) => vec![first.clone(), last.clone()],
                    _ => Vec::new(),
                },
            })
            .collect(),
        no_answer: response.results.is_empty()
            && response.nodes.is_empty()
            && response.edges.is_empty()
            && response.paths.is_empty(),
        latency_micros: Some(latency_micros),
        // The public query response exposes serialized response bytes, but not
        // candidate/posting/expansion counters. Leave unavailable counters at
        // zero rather than synthesizing implementation work.
        work: WorkCounts {
            response_bytes,
            ..WorkCounts::default()
        },
    })
}

fn executable_corpus(graph_digest: String) -> JudgmentCorpus {
    let node_judgment = |id: &str| IdJudgment {
        id: id.to_owned(),
        grade: 3,
    };
    let edge_judgment = |source: &str, target: &str, kind: &str| EdgeJudgment {
        edge: EdgeIdentity {
            id: None,
            source: Some(source.to_owned()),
            target: Some(target.to_owned()),
            kind: Some(kind.to_owned()),
            direction: Some("source_to_target".to_owned()),
        },
        grade: 3,
    };
    let query = |id: &str, text: &str, class, operation: &str| JudgedQuery {
        id: id.to_owned(),
        text: text.to_owned(),
        class,
        locale: None,
        expected_intent: Some(operation.to_owned()),
        expected_slots: BTreeMap::from([("operation".to_owned(), operation.to_owned())]),
        node_judgments: Vec::new(),
        edge_judgments: Vec::new(),
        path_judgments: Vec::new(),
        acceptable_ambiguity: Vec::new(),
        must_not_return: Vec::new(),
        notes: None,
    };
    let mut search_list = query(
        "exec-search-list",
        "UserService.list",
        QueryClass::Exact,
        "search",
    );
    search_list.node_judgments = vec![node_judgment("n:list")];
    let mut search_unicode = query("exec-search-cafe", "cafe", QueryClass::Lexical, "search");
    search_unicode.node_judgments = vec![node_judgment("n:unicode")];
    let mut callers = query(
        "exec-callers",
        "callers of UserService.list",
        QueryClass::Edge,
        "callers",
    );
    callers.node_judgments = vec![
        node_judgment("n:caller"),
        node_judgment("n:list"),
        node_judgment("n:route"),
    ];
    callers.edge_judgments = vec![
        edge_judgment("n:caller", "n:list", "calls"),
        edge_judgment("n:route", "n:list", "routes_to"),
    ];
    let mut trail = query(
        "exec-trail",
        "Api.caller to Store.callee",
        QueryClass::Path,
        "node_trail",
    );
    trail.node_judgments = vec![
        node_judgment("n:caller"),
        node_judgment("n:list"),
        node_judgment("n:callee"),
    ];
    trail.edge_judgments = vec![
        edge_judgment("n:caller", "n:list", "calls"),
        edge_judgment("n:list", "n:callee", "calls"),
    ];
    trail.path_judgments = vec![PathJudgment {
        pattern: PathPattern {
            edge_kinds: vec!["calls".to_owned(), "calls".to_owned()],
            endpoint_ids: vec!["n:caller".to_owned(), "n:callee".to_owned()],
        },
        grade: 3,
    }];
    let mut missing = query(
        "exec-missing",
        "definitely_missing",
        QueryClass::Negative,
        "search",
    );
    missing.notes = Some("Reviewed compact fixture no-answer case.".to_owned());
    JudgmentCorpus {
        schema: compass_query::QUERY_JUDGMENTS_SCHEMA_V1.to_owned(),
        corpus_id: "compass-query-executable-reviewed-v1".to_owned(),
        graph_schema: "compass.graph/1".to_owned(),
        graph_digest,
        repository_revision: "crates/compass-query/tests/support@v1".to_owned(),
        analyzer_version: "compass.search-term/1".to_owned(),
        queries: vec![search_list, search_unicode, callers, trail, missing],
    }
}

fn execute_subset(
    engine: &compass_query::CodeQueryEngine,
) -> Result<Vec<QueryObservation>, Box<dyn std::error::Error>> {
    let execute =
        |id: &str, operation: &dyn Fn() -> Result<CodeQueryResponse, compass_query::QueryError>| {
            let started = Instant::now();
            let response = operation()?;
            observe_execution(id, &response, u64::try_from(started.elapsed().as_micros())?)
        };
    Ok(vec![
        execute("exec-search-list", &|| {
            engine.search(SearchRequest {
                query: "UserService.list".to_owned(),
                limits: CodeQueryLimits::default(),
            })
        })?,
        execute("exec-search-cafe", &|| {
            engine.search(SearchRequest {
                query: "cafe".to_owned(),
                limits: CodeQueryLimits::default(),
            })
        })?,
        execute("exec-callers", &|| {
            engine.callers(CallRequest {
                symbol: "UserService.list".to_owned(),
                include_heuristic: false,
                limits: CodeQueryLimits::default(),
            })
        })?,
        execute("exec-trail", &|| {
            engine.node_trail(NodeTrailRequest {
                source: "Api.caller".to_owned(),
                target: "Store.callee".to_owned(),
                include_heuristic: false,
                limits: CodeQueryLimits::default(),
            })
        })?,
        execute("exec-missing", &|| {
            engine.search(SearchRequest {
                query: "definitely_missing".to_owned(),
                limits: CodeQueryLimits::default(),
            })
        })?,
    ])
}

#[test]
fn reviewed_corpus_generates_a_stable_finite_baseline() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus()?;
    corpus.validate_graph_digest(FIXTURE_DIGEST)?;
    assert_eq!(corpus.queries.len(), 80);
    let classes = corpus
        .queries
        .iter()
        .map(|query| format!("{:?}", query.class))
        .collect::<BTreeSet<_>>();
    assert_eq!(classes.len(), 8);
    assert!(corpus.queries.iter().all(|query| {
        matches!(query.class, QueryClass::Negative)
            || !query.node_judgments.is_empty()
            || !query.edge_judgments.is_empty()
            || !query.path_judgments.is_empty()
    }));
    assert!(
        corpus
            .queries
            .iter()
            .filter(|query| matches!(query.class, QueryClass::Negative))
            .all(|query| query.notes.is_some())
    );
    let observations = corpus
        .queries
        .iter()
        .enumerate()
        .map(|(index, query)| observation(query, u64::try_from(index + 1).unwrap_or(u64::MAX)))
        .collect::<Vec<_>>();
    let limits = BTreeMap::from([
        ("maxCandidates".to_owned(), 20),
        ("maxNodes".to_owned(), 500),
    ]);
    let first = qualification_report(
        &corpus,
        &observations,
        "fixture-ranker/1",
        "fixture-planner/1",
        "fixture",
        limits.clone(),
    )?;
    let second = qualification_report(
        &corpus,
        &observations,
        "fixture-ranker/1",
        "fixture-planner/1",
        "fixture",
        limits,
    )?;
    let first_json = serde_json::to_vec(&first)?;
    assert_eq!(first_json, serde_json::to_vec(&second)?);
    for metric in [
        &first.metrics.success_at_1,
        &first.metrics.mrr_at_10,
        &first.metrics.recall_at_5,
        &first.metrics.precision_at_10,
        &first.metrics.ndcg_at_10,
        &first.metrics.intent_macro_f1,
        &first.metrics.edge_precision,
        &first.metrics.edge_kind_precision,
        &first.metrics.edge_direction_precision,
        &first.metrics.path_acceptance_rate,
        &first.metrics.accepted_ambiguity_recall,
        &first.metrics.no_answer_precision,
        &first.metrics.latency_p50_micros,
        &first.metrics.latency_p95_micros,
    ] {
        assert!(metric.value.is_some_and(f64::is_finite));
    }
    assert_eq!(first.metrics.work.candidates_read, 160);
    Ok(())
}

#[test]
fn malformed_observations_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus()?;
    let one = observation(&corpus.queries[0], 1);
    assert!(matches!(
        score(&corpus, std::slice::from_ref(&one)),
        Err(RelevanceError::MissingObservation { .. })
    ));
    let all = corpus
        .queries
        .iter()
        .enumerate()
        .map(|(index, query)| observation(query, u64::try_from(index + 1).unwrap_or(u64::MAX)))
        .collect::<Vec<_>>();
    let mut duplicate = all.clone();
    duplicate.push(one);
    assert!(matches!(
        score(&corpus, &duplicate),
        Err(RelevanceError::DuplicateObservation { .. })
    ));
    Ok(())
}

#[test]
fn executable_baseline_is_digest_pinned_and_backend_deterministic()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let graph = GraphDocument::load(&graph_path)?;
    let graph_digest = format!("sha256:{:x}", Sha256::digest(canonical_graph_json(&graph)?));
    let corpus = executable_corpus(graph_digest.clone());
    corpus.validate_graph_digest(&graph_digest)?;
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
    let store_observations = execute_subset(&store)?;
    let json_observations = execute_subset(&json)?;
    let repeated_observations = execute_subset(&store)?;
    assert!(store_observations.iter().all(|observation| {
        observation.latency_micros.is_some() && observation.work.response_bytes > 0
    }));
    assert!(store_observations.iter().all(|observation| {
        observation.work.candidates_read == 0
            && observation.work.postings_decoded == 0
            && observation.work.nodes_expanded == 0
            && observation.work.edges_expanded == 0
    }));

    let normalize_timing = |mut observations: Vec<QueryObservation>| {
        for observation in &mut observations {
            observation.latency_micros = None;
        }
        serde_json::to_vec(&observations)
    };
    assert_eq!(
        normalize_timing(store_observations.clone())?,
        normalize_timing(json_observations)?
    );
    assert_eq!(
        normalize_timing(store_observations.clone())?,
        normalize_timing(repeated_observations)?
    );
    let mut deterministic_observations = store_observations.clone();
    for observation in &mut deterministic_observations {
        observation.latency_micros = None;
    }

    let limits = BTreeMap::from([
        (
            "maxCandidates".to_owned(),
            u64::from(CodeQueryLimits::default().max_candidates),
        ),
        (
            "maxNodes".to_owned(),
            u64::from(CodeQueryLimits::default().max_nodes),
        ),
    ]);
    let report = qualification_report(
        &corpus,
        &store_observations,
        "code-query/1",
        "typed-code-query/1",
        "store",
        limits,
    )?;
    assert_eq!(report.graph_digest, graph_digest);
    assert_eq!(report.metrics.success_at_1.value, Some(1.0));
    assert_eq!(report.metrics.edge_direction_precision.value, Some(1.0));
    assert_eq!(report.metrics.path_acceptance_rate.value, Some(1.0));
    assert_eq!(report.metrics.no_answer_precision.value, Some(1.0));
    assert!(
        report
            .metrics
            .latency_p50_micros
            .value
            .is_some_and(f64::is_finite)
    );
    assert!(
        report
            .metrics
            .latency_p95_micros
            .value
            .is_some_and(f64::is_finite)
    );
    assert!(report.metrics.work.response_bytes > 0);
    let deterministic_report = qualification_report(
        &corpus,
        &deterministic_observations,
        "code-query/1",
        "typed-code-query/1",
        "store",
        BTreeMap::from([
            ("maxCandidates".to_owned(), 20),
            ("maxNodes".to_owned(), 500),
        ]),
    )?;
    assert_eq!(
        serde_json::to_vec(&deterministic_report)?,
        serde_json::to_vec(&qualification_report(
            &corpus,
            &deterministic_observations,
            "code-query/1",
            "typed-code-query/1",
            "store",
            BTreeMap::from([
                ("maxCandidates".to_owned(), 20),
                ("maxNodes".to_owned(), 500),
            ]),
        )?)?
    );
    Ok(())
}

fn publish_snapshot(directory: &Path, graph_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::open(directory.join(STORE_FILE_NAME))?;
    let graph = GraphDocument::load(graph_path)?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    fs::write(
        directory.join(STORE_REF_FILE_NAME),
        serde_json::to_vec(&store.snapshot_reference()?)?,
    )?;
    store.checkpoint()?;
    Ok(())
}

#[test]
fn backend_parity_subset_preserves_normalized_search_ids_and_edges()
-> Result<(), Box<dyn std::error::Error>> {
    // The 80-question fixture is contract-only. This bounded subset exercises
    // real JSON/store execution against the compact shared graph fixture.
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
    let searches = [
        ("café", vec!["n:unicode"]),
        ("cafe", vec!["n:unicode"]),
        ("résumé", vec!["n:resume"]),
        ("resume", vec!["n:resume"]),
        ("ångström", vec!["n:unicode-case"]),
        ("cache_key", vec!["n:snake"]),
        ("fetchUserRecord", vec!["n:camel"]),
        ("definitely_missing", Vec::new()),
    ];
    for (query, expected_ids) in searches {
        let request = SearchRequest {
            query: query.to_owned(),
            limits: CodeQueryLimits::default(),
        };
        let store_result = store.search(request.clone())?;
        let json_result = json.search(request)?;
        assert_eq!(
            serde_json::to_vec(&store_result)?,
            serde_json::to_vec(&json_result)?
        );
        assert_eq!(
            store_result
                .results
                .iter()
                .map(|hit| hit.node_id.as_str())
                .collect::<Vec<_>>(),
            expected_ids,
        );
        let repeated = store.search(SearchRequest {
            query: query.to_owned(),
            limits: CodeQueryLimits::default(),
        })?;
        assert_eq!(
            serde_json::to_vec(&store_result)?,
            serde_json::to_vec(&repeated)?
        );
    }
    let callers = CallRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    let store_result = store.callers(callers.clone())?;
    let json_result = json.callers(callers)?;
    assert_eq!(
        serde_json::to_vec(&store_result)?,
        serde_json::to_vec(&json_result)?
    );
    assert!(
        store_result
            .edges
            .iter()
            .all(|edge| !edge.kind.as_str().is_empty())
    );
    let trail = NodeTrailRequest {
        source: "Api.caller".to_owned(),
        target: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    assert_eq!(
        serde_json::to_vec(&store.node_trail(trail.clone())?)?,
        serde_json::to_vec(&json.node_trail(trail)?)?
    );
    Ok(())
}
