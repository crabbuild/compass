mod support;
use compass_model::code_graph::{GraphDocument, NodeKind};
use compass_model::query_contract::{CodeQueryLimits, ConceptMatchMethod, SearchRequest};
use compass_query::{EngineSelection, NaturalQueryRequest, open_with_engine};
use std::{error::Error, fs};

fn request(question: &str) -> NaturalQueryRequest {
    NaturalQueryRequest {
        question: question.into(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    }
}

#[test]
fn thirty_business_questions_find_expected_modules() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    // Business vocabulary is deliberately absent from these code identifiers.
    // The oracle names the intended declaration, not just a nonempty response.
    let cases = [
        ("How does authorization work?", "AuthzGate"),
        ("Where is access control enforced?", "AuthzGate"),
        ("What handles permissions?", "PermissionDeniedError"),
        ("How does authentication work?", "LoginHandler"),
        ("Where can a user sign in?", "LoginHandler"),
        ("Where is configuration read?", "SettingsLoader"),
        ("How are environment variables read?", "SettingsLoader"),
        ("What handles persistence?", "RecordRepository"),
        ("Where is database access implemented?", "RecordRepository"),
        ("What handles input checking?", "SchemaValidator"),
        ("Where does validation happen?", "SchemaValidator"),
        ("How does rate limiting work?", "QuotaLimiter"),
        ("What implements request throttling?", "QuotaLimiter"),
        ("Where do background jobs run?", "QueueWorker"),
        ("What handles job scheduling?", "QueueWorker"),
        ("How is caching implemented?", "CacheEvictor"),
        ("Where does cache invalidation happen?", "CacheEvictor"),
        ("What handles database transactions?", "UowSession"),
        ("Where are feature flags checked?", "ToggleReader"),
        ("How does observability work?", "TelemetryMetrics"),
        ("Where is synchronization implemented?", "MutexLock"),
        ("What handles serialization?", "WireCodec"),
        ("What handles transient failures?", "BackoffRetry"),
        ("How are timeouts handled?", "DeadlineGuard"),
        ("Where is encryption implemented?", "CipherBox"),
        ("What handles file uploads?", "MultipartUpload"),
        ("How does email delivery work?", "SmtpMail"),
        ("What implements pagination?", "CursorPage"),
        ("Where are payments handled?", "StripeInvoice"),
        ("What handles container orchestration?", "KubernetesDeploy"),
    ];
    for (_, name) in cases {
        if graph.nodes.iter().all(|node| node.id != name) {
            graph.nodes.push(support::node(
                name,
                NodeKind::Class,
                name,
                &format!("production.{name}"),
            ));
        }
    }
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open_with_engine(
        &path,
        None,
        &dir.path().join("cache"),
        EngineSelection::Json,
    )?;
    let mut failures = vec![];
    for (question, expected) in cases {
        let response = engine.query_natural(request(question))?;
        if !response
            .results
            .iter()
            .take(5)
            .any(|hit| hit.node_id == expected)
        {
            failures.push(format!(
                "{question}: expected {expected}; hits={:?}",
                response.results
            ));
        }
        assert!(
            response
                .concept_matches
                .iter()
                .any(|matched| matched.method == ConceptMatchMethod::Synonym),
            "{question}"
        );
        assert_eq!(
            response,
            engine.query_natural(request(question))?,
            "{question}"
        );
    }
    assert!(
        failures.is_empty(),
        "{} / 30 missed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // Strict lookup never admits concept alternatives, even with semantic opt-in.
    let exact = engine.with_semantic_search(true).search_exact(
        SearchRequest {
            query: "authorization".into(),
            limits: Default::default(),
        },
        Default::default(),
    )?;
    assert!(exact.nodes.is_empty());
    assert!(exact.concept_matches.is_empty());
    Ok(())
}

#[test]
fn markdown_prose_reaches_only_witnessed_code_mentions_with_backend_parity()
-> Result<(), Box<dyn Error>> {
    use compass_graph::{
        GraphSnapshotBuilder, RawNodeRecord, build_from_extraction, normalize_document_v1,
    };
    use compass_languages::Engine;
    use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore};
    use serde_json::{Map, json};
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let doc = root.join("README.md");
    fs::write(root.join("impl.rs"), "pub struct DeactivationEvaluator;\n")?;
    // The concept occurs past the display-name limit and must be indexed from
    // source-backed Markdown content rather than the paragraph's short label.
    fs::write(
        &doc,
        format!(
            "# Account lifecycle\n\n{} Account suspension is implemented by `DeactivationEvaluator`. Unavailable `GhostEvaluator` is not linked.\n",
            "Context. ".repeat(75)
        ),
    )?;
    let mut raw = Engine::default().extract(&doc)?;
    raw.nodes.push(RawNodeRecord { id: "implementation".into(), attributes: Map::from_iter([
        ("label".into(), json!("DeactivationEvaluator")), ("qualified_name".into(), json!("app::DeactivationEvaluator")),
        ("symbol_kind".into(), json!("struct")), ("file_type".into(), json!("code")),
        ("language".into(), json!("rust")), ("extractor".into(), json!("test.rust")),
        ("source_file".into(), json!("impl.rs")),
        ("source_anchor".into(), json!({"file":"impl.rs","startByte":11,"endByte":32,"startLine":1,"startColumn":11,"endLine":1,"endColumn":32})),
    ]) });
    let document = build_from_extraction(&raw, true, Some(root));
    let graph = normalize_document_v1(&document, root, "sha256:test", None)?;
    let target = graph
        .nodes
        .iter()
        .find(|node| node.name == "DeactivationEvaluator")
        .ok_or("missing code")?
        .id
        .clone();
    assert!(
        graph.links.iter().any(|edge| edge.target == target),
        "published links missing: {:?}",
        graph.links
    );
    let path = root.join("graph.json");
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let store = SqliteStore::open(root.join(STORE_FILE_NAME))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    fs::write(
        root.join(STORE_REF_FILE_NAME),
        serde_json::to_vec(&store.snapshot_reference()?)?,
    )?;
    store.checkpoint()?;
    let mut responses = vec![];
    for kind in [EngineSelection::Json, EngineSelection::Store] {
        let engine = open_with_engine(&path, None, &root.join(format!("{kind:?}-cache")), kind)?;
        let response = engine.query_natural(request("How does account suspension work?"))?;
        assert!(
            response
                .results
                .iter()
                .take(3)
                .any(|hit| hit.node_id == target),
            "{:?}",
            response.results
        );
        assert!(
            response
                .concept_matches
                .iter()
                .any(|matched| matched.node_id == target
                    && matched.method == ConceptMatchMethod::DocumentLink
                    && !matched.via.is_empty())
        );
        assert!(response.edges.iter().any(|edge| edge.target == target));
        assert!(
            !response
                .results
                .iter()
                .any(|hit| hit.node_id.contains("GhostEvaluator"))
        );
        responses.push(response);
    }
    assert_eq!(responses[0], responses[1]);
    Ok(())
}

#[test]
fn semantic_fallback_is_opt_in_cached_and_follows_document_evidence() -> Result<(), Box<dyn Error>>
{
    use compass_model::code_graph::{EdgeKind, NodeDetails, ResourceKind, ResourceNodeDetails};
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    let mut edge = graph.links.first().ok_or("missing fixture edge")?.clone();
    graph.nodes.clear();
    graph.links.clear();
    for (id, content) in [
        ("doc-a", "orchard apples harvest"),
        ("doc-b", "fruit apples harvest"),
    ] {
        let mut doc = support::node(id, NodeKind::Resource, "Notes", id);
        doc.details = Some(NodeDetails::Resource(ResourceNodeDetails {
            resource_kind: ResourceKind::Document,
            uri: None,
            media_type: Some("text/markdown".into()),
            content: Some(content.into()),
        }));
        graph.nodes.push(doc);
    }
    graph.nodes.push(support::node(
        "impl",
        NodeKind::Class,
        "FruitHandler",
        "production.FruitHandler",
    ));
    graph.nodes.push(support::node(
        "other",
        NodeKind::Class,
        "InvoicePayment",
        "production.InvoicePayment",
    ));
    edge.kind = EdgeKind::References;
    edge.source = "doc-b".into();
    edge.target = "impl".into();
    edge.details = None;
    edge.occurrence_rule = None;
    edge.id = compass_model::identity::edge_id(
        "doc-b",
        EdgeKind::References,
        "impl",
        edge.relationship_site.as_ref(),
        None,
    );
    edge.key = edge.id.clone();
    graph.links.push(edge);
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let mut engine = open_with_engine(
        &path,
        None,
        &dir.path().join("cache"),
        EngineSelection::Json,
    )?;
    let default = engine.query_natural(request("orchard"))?;
    assert!(!default.results.iter().any(|hit| hit.node_id == "impl"));
    assert!(default.concept_matches.is_empty());
    engine.set_semantic_search(true);
    let approximate = engine.query_natural(request("orchard"))?;
    assert!(
        approximate.results.iter().any(|hit| hit.node_id == "impl"),
        "{approximate:?}"
    );
    assert!(
        approximate
            .concept_matches
            .iter()
            .any(|item| item.method == ConceptMatchMethod::SemanticLsa)
    );
    assert!(
        approximate
            .concept_matches
            .iter()
            .any(|item| item.node_id == "impl" && item.method == ConceptMatchMethod::DocumentLink)
    );
    assert!(approximate.results.iter().all(|hit| hit.node_id != "other"));
    assert_eq!(approximate, engine.query_natural(request("orchard"))?);
    engine.set_semantic_search(false);
    assert_eq!(default, engine.query_natural(request("orchard"))?);
    engine.set_semantic_search(true);
    assert!(
        engine
            .query_natural(request("absentvocabulary"))?
            .results
            .is_empty()
    );
    Ok(())
}

#[test]
fn scoped_discovery_retains_filters_and_labels_concept_seeds() -> Result<(), Box<dyn Error>> {
    use compass_model::query_contract::{
        DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryScope,
        DiscoveryScopeKind, DiscoveryTraversal,
    };
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    graph.nodes.push(support::node(
        "gate",
        NodeKind::Class,
        "AuthzGate",
        "production.AuthzGate",
    ));
    for i in 0..100 {
        let mut competitor = support::node(
            &format!("other-{i:03}"),
            NodeKind::Class,
            "AuthzGate",
            "unrelated.AuthzGate",
        );
        if let Some(source) = &mut competitor.source {
            source.file = "src/payments/gateway.rs".into();
        }
        graph.nodes.push(competitor);
    }
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open_with_engine(
        &path,
        None,
        &dir.path().join("cache"),
        EngineSelection::Json,
    )?;
    let response = engine.discover(DiscoveryQueryRequest {
        question: "How does authorization work?".into(),
        direction: DiscoveryDirection::Outgoing,
        include_heuristic: false,
        scope: vec![DiscoveryScope {
            kind: DiscoveryScopeKind::Source,
            value: "src/lib.rs".into(),
        }],
        relation_contexts: vec![],
        traversal: DiscoveryTraversal::Bfs,
        limits: DiscoveryLimits::default(),
    })?;
    assert!(
        response.seeds.iter().any(|seed| seed.node_id == "gate"
            && seed
                .matched_fields
                .iter()
                .any(|field| field.contains("Synonym"))),
        "{response:?}"
    );
    assert_eq!(response.selected_direction, DiscoveryDirection::Outgoing);
    assert!(response.nodes.iter().all(|node| {
        node.source
            .as_ref()
            .is_some_and(|source| source.file == "src/lib.rs")
    }));
    Ok(())
}

#[test]
fn concept_recall_prefers_implementations_over_fields_and_incidental_doc_links()
-> Result<(), Box<dyn Error>> {
    use compass_model::code_graph::{EdgeKind, NodeDetails, ResourceKind, ResourceNodeDetails};
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    for i in 0..80 {
        graph.nodes.push(support::node(
            &format!("parameter-{i}"),
            NodeKind::Parameter,
            "auth",
            &format!("fixture.{i}.auth"),
        ));
    }
    for name in ["AuthzGate", "DbConnection", "RateLimit"] {
        graph.nodes.push(support::node(
            name,
            NodeKind::Class,
            name,
            &format!("production.{name}"),
        ));
    }
    let mut doc = support::node(
        "incidental-doc",
        NodeKind::Resource,
        "A tool-specific policy authorization status",
        "Notes",
    );
    doc.details = Some(NodeDetails::Resource(ResourceNodeDetails {
        resource_kind: ResourceKind::Rationale,
        uri: None,
        media_type: None,
        content: Some(doc.name.clone()),
    }));
    graph.nodes.push(doc);
    let mut edge = graph.links[0].clone();
    edge.kind = EdgeKind::Documents;
    edge.source = "incidental-doc".into();
    edge.target = "n:listing".into();
    edge.occurrence_rule = None;
    edge.details = None;
    edge.id = compass_model::identity::edge_id(
        &edge.source,
        edge.kind,
        &edge.target,
        edge.relationship_site.as_ref(),
        None,
    );
    edge.key = edge.id.clone();
    graph.links.push(edge);
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open_with_engine(
        &path,
        None,
        &dir.path().join("cache"),
        EngineSelection::Json,
    )?;
    for (question, expected) in [
        ("How does authorization work?", "AuthzGate"),
        ("What handles persistence?", "DbConnection"),
        ("How does rate limiting work?", "RateLimit"),
    ] {
        let response = engine.query_natural(request(question))?;
        assert_eq!(
            response.results.first().map(|hit| hit.node_id.as_str()),
            Some(expected),
            "{question}: {:?}",
            response.results
        );
        assert!(
            response
                .concept_matches
                .iter()
                .any(|item| item.node_id == expected && item.method == ConceptMatchMethod::Synonym)
        );
    }
    Ok(())
}
