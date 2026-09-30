mod support;

use compass_graph::GraphSnapshotBuilder;
use compass_model::code_graph::{EdgeKind, GraphDocument, NodeKind};
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, DiscoveryDirection, DiscoveryQueryRequest, DiscoveryScope,
    DiscoveryScopeKind, QueryDiagnosticCode,
};
use compass_query::{EngineSelection, open_with_engine};
use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore};
use std::fs;
use std::path::Path;

fn request(question: &str) -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: question.to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: vec![],
        scope: vec![],
        traversal: Default::default(),
        include_heuristic: false,
        limits: Default::default(),
    }
}

fn fixture(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    support::write_graph(path)?;
    let mut graph = GraphDocument::load(path)?;
    for (id, kind, name, qualified) in [
        (
            "svc",
            NodeKind::Class,
            "FieldSectionMutationService",
            "app.FieldSectionMutationService",
        ),
        (
            "mutate",
            NodeKind::Method,
            "mutate",
            "app.FieldSectionMutationService.mutate",
        ),
        ("repo", NodeKind::Method, "get", "app.FieldRepository.get"),
        (
            "validation",
            NodeKind::Function,
            "validate",
            "app.validation.validate",
        ),
        (
            "authz",
            NodeKind::Function,
            "authorization",
            "app.authz.authorization",
        ),
    ] {
        graph.nodes.push(support::node(id, kind, name, qualified));
    }
    let template = graph.links[0].clone();
    for (_id, source, kind, target) in [
        ("owns", "svc", EdgeKind::Contains, "mutate"),
        ("repo-call", "mutate", EdgeKind::Calls, "repo"),
        ("validate-call", "mutate", EdgeKind::Calls, "validation"),
        ("authz-call", "mutate", EdgeKind::Calls, "authz"),
        ("svc-import", "svc", EdgeKind::Imports, "n:dependent"),
    ] {
        let mut edge = template.clone();
        edge.id = compass_model::identity::edge_id(
            source,
            kind,
            target,
            edge.relationship_site.as_ref(),
            edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
        );
        edge.key = edge.id.clone();
        edge.source = source.to_owned();
        edge.target = target.to_owned();
        edge.kind = kind;
        graph.links.push(edge);
    }
    fs::write(path, serde_json::to_vec(&graph)?)?;
    let root = path.parent().ok_or("missing fixture root")?;
    let store = SqliteStore::open(root.join(STORE_FILE_NAME))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    fs::write(
        root.join(STORE_REF_FILE_NAME),
        serde_json::to_vec(&store.snapshot_reference()?)?,
    )?;
    store.checkpoint()?;
    Ok(())
}

#[test]
fn reviewed_natural_questions_return_witnessed_answers_with_backend_parity()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let graph_path = dir.path().join("graph.json");
    fixture(&graph_path)?;
    let json = open_with_engine(
        &graph_path,
        None,
        &dir.path().join("json-cache"),
        EngineSelection::Json,
    )?;
    let store = open_with_engine(
        &graph_path,
        None,
        &dir.path().join("store-cache"),
        EngineSelection::Store,
    )?;
    // Each oracle names an edge or declaration in the fixture above, rather
    // than accepting any nonempty answer as evidence of useful recall.
    let cases = [
        ("what does FieldSectionMutationService depend on", "repo"),
        ("what does FieldSectionMutationService use", "validation"),
        ("dependencies of FieldSectionMutationService", "n:dependent"),
        (
            "what does app.FieldSectionMutationService depend on?",
            "authz",
        ),
        ("what calls app.FieldRepository.get?", "mutate"),
        ("who uses app.FieldRepository.get?", "mutate"),
        ("where is app.FieldRepository.get used?", "mutate"),
        ("who calls app.validation.validate?", "mutate"),
        ("what uses app.authz.authorization?", "mutate"),
        ("what breaks if app.FieldRepository.get changes?", "mutate"),
        (
            "what would break if app.validation.validate changes?",
            "mutate",
        ),
        ("what depends on app.authz.authorization?", "mutate"),
        (
            "how does app.FieldSectionMutationService.mutate relate to app.FieldRepository.get?",
            "repo",
        ),
        (
            "how is app.FieldSectionMutationService.mutate connected to app.validation.validate?",
            "validation",
        ),
        (
            "path from app.FieldSectionMutationService.mutate to app.authz.authorization",
            "authz",
        ),
        (
            "what does app.FieldSectionMutationService.mutate call?",
            "repo",
        ),
        (
            "calls made by app.FieldSectionMutationService.mutate",
            "validation",
        ),
        (
            "what functions does app.FieldSectionMutationService.mutate invoke?",
            "authz",
        ),
        ("who calls list?", "n:caller"),
        ("who uses list?", "n:caller"),
        ("where is list used?", "n:caller"),
        ("what breaks if list changes?", "n:caller"),
        ("what handles validation", "validation"),
        ("how does authorization work", "authz"),
        ("what does FieldSectionMutationServcie depend on?", "repo"),
    ];
    let mut answered = 0;
    let mut failures = vec![];
    for (question, expected) in cases {
        let a = json.discover(request(question))?;
        let b = store.discover(request(question))?;
        // Backend work budgets can withhold different low-ranked broad
        // candidates; both must return the same witnessed neighborhood here.
        assert_eq!(a.nodes, b.nodes, "{question}");
        assert_eq!(a.edges, b.edges, "{question}");
        if question != "what handles validation" && question != "how does authorization work" {
            assert_eq!(
                compass_query::discovery_response_digest(&a)?,
                compass_query::discovery_response_digest(&b)?,
                "{question}"
            );
        }
        assert_eq!(
            a,
            json.discover(request(question))?,
            "{question}: nondeterministic answer"
        );
        if a.nodes.iter().any(|node| node.id == expected) {
            answered += 1;
        } else {
            failures.push(format!(
                "{question}: expected {expected}, seeds={:?}",
                a.seeds
            ));
        }
        assert!(
            !a.diagnostics
                .iter()
                .any(|d| d.code == QueryDiagnosticCode::AmbiguousMatch)
                || !a.nodes.is_empty(),
            "{question}"
        );
    }
    assert_eq!(
        answered,
        cases.len(),
        "{} of {} answered:\n{}",
        answered,
        cases.len(),
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn ambiguous_natural_question_executes_but_explicit_callers_stays_strict()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("graph.json");
    fixture(&path)?;
    for selection in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(
            &path,
            None,
            &dir.path().join(format!("{selection:?}")),
            selection,
        )?;
        let natural = engine.discover(request("who calls list?"))?;
        assert_eq!(natural.seeds[0].node_id, "n:list");
        assert!(
            natural.seeds[0]
                .alternatives
                .iter()
                .any(|other| other.node_id == "n:other")
        );
        assert!(
            natural
                .edges
                .iter()
                .any(|edge| edge.source == "n:caller" && edge.target == "n:list")
        );
        let strict = engine.callers(CallRequest {
            symbol: "list".to_owned(),
            include_heuristic: false,
            limits: CodeQueryLimits::default(),
        })?;
        assert!(strict.edges.is_empty());
        assert!(
            strict
                .diagnostics
                .iter()
                .any(|d| d.code == QueryDiagnosticCode::AmbiguousMatch)
        );
    }
    Ok(())
}

#[test]
fn natural_dependencies_keep_imports_member_calls_and_limits_coherent()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("graph.json");
    fixture(&path)?;
    let engine = open_with_engine(
        &path,
        None,
        &dir.path().join("cache"),
        EngineSelection::Json,
    )?;
    let answer = engine.discover(request("what does FieldSectionMutationService depend on?"))?;
    assert!(answer.edges.iter().any(|e| e.kind == EdgeKind::Imports));
    assert!(
        answer
            .edges
            .iter()
            .any(|e| e.source == "mutate" && e.target == "repo")
    );
    let mut bounded = request("what does FieldSectionMutationService depend on?");
    bounded.limits.max_nodes = 2;
    let bounded = engine.discover(bounded)?;
    assert!(bounded.truncated);
    assert!(bounded.nodes.len() <= 2);
    assert!(bounded.edges.iter().all(
        |edge| bounded.nodes.iter().any(|node| node.id == edge.source)
            && bounded.nodes.iter().any(|node| node.id == edge.target)
    ));
    let mut scoped = request("what does FieldSectionMutationService depend on?");
    scoped.scope.push(DiscoveryScope {
        kind: DiscoveryScopeKind::Node,
        value: "svc".to_owned(),
    });
    let scoped = engine.discover(scoped)?;
    assert!(scoped.nodes.iter().all(|node| node.id == "svc"));
    for question in [
        "QuantumBananaMissingService",
        "what does QuantumBananaMissingService depend on?",
    ] {
        let absent = engine.discover(request(question))?;
        assert!(absent.nodes.is_empty(), "{question}: {:?}", absent.seeds);
        assert!(absent.edges.is_empty());
    }
    Ok(())
}

#[test]
fn natural_selection_ranks_connectivity_and_respects_test_intent()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("graph.json");
    fixture(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    let template = graph.links[0].clone();
    for index in 0..10 {
        let id = format!("handler-{index:02}");
        graph.nodes.push(support::node(
            &id,
            NodeKind::Function,
            "handle",
            &format!("app.Handler{index}.handle"),
        ));
        if index == 9 {
            for target in [
                "repo",
                "validation",
                "authz",
                "mutate",
                "n:caller",
                "n:list",
            ] {
                let mut edge = template.clone();
                edge.source = id.clone();
                edge.target = target.to_owned();
                edge.kind = EdgeKind::Calls;
                edge.id = compass_model::identity::edge_id(
                    &edge.source,
                    edge.kind,
                    &edge.target,
                    edge.relationship_site.as_ref(),
                    edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
                );
                edge.key = edge.id.clone();
                graph.links.push(edge);
            }
        }
    }
    let mut test_node = support::node("test-handle", NodeKind::Function, "handle", "tests.handle");
    if let Some(source) = &mut test_node.source {
        source.file = "tests/generated/payment_gateway.rs".to_owned();
    }
    graph.nodes.push(test_node);
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open_with_engine(
        &path,
        None,
        &dir.path().join("cache"),
        EngineSelection::Json,
    )?;
    let production = engine.discover(request("what does handle depend on?"))?;
    assert_eq!(production.seeds[0].node_id, "handler-09");
    assert!(production.edges.iter().any(|edge| edge.target == "repo"));
    let tests = engine.discover(request("who calls handle in tests?"))?;
    assert_eq!(tests.seeds[0].node_id, "test-handle");
    assert!(tests.seeds[0].ambiguous);
    Ok(())
}
