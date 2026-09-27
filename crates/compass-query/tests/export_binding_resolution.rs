mod support;

use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::GraphSnapshotBuilder;
use compass_model::Graph;
use compass_model::code_graph::{EdgeKind, EdgeRecord, GraphDocument, NodeKind};
use compass_model::identity::edge_id;
use compass_model::provenance::{EvidenceConfidence, ResolutionCandidate};
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, NodeTrailRequest, QueryDiagnosticCode,
};
use compass_query::{
    EngineSelection, find_exact_nodes, open, open_with_engine, open_with_store,
    render_shortest_path_with_limit,
};
use compass_store::{STORE_FILE_NAME, STORE_REF_FILE_NAME, SqliteStore};

fn fixture(path: &Path) -> Result<GraphDocument, Box<dyn Error>> {
    support::write_graph(path)?;
    fs::create_dir_all(path.parent().ok_or("missing graph parent")?.join("cache"))?;
    let mut graph = GraphDocument::load(path)?;
    let template = graph
        .links
        .first()
        .cloned()
        .ok_or("missing edge template")?;
    graph.nodes = vec![
        support::node("module", NodeKind::Module, "api", "api"),
        support::node("binding", NodeKind::Export, "entry", "api.entry"),
        support::node("function", NodeKind::Function, "entry()", "api.entry"),
        support::node("leaf", NodeKind::Function, "leaf()", "api.leaf"),
    ];
    graph.links = vec![
        edge(&template, "module", EdgeKind::Contains, "binding"),
        edge(&template, "module", EdgeKind::Exports, "function"),
        edge(&template, "function", EdgeKind::Calls, "leaf"),
    ];
    Ok(graph)
}

fn edge(template: &EdgeRecord, source: &str, kind: EdgeKind, target: &str) -> EdgeRecord {
    let mut edge = template.clone();
    edge.source = source.to_owned();
    edge.target = target.to_owned();
    edge.kind = kind;
    edge.id = edge_id(source, kind, target, edge.relationship_site.as_ref(), None);
    edge.key.clone_from(&edge.id);
    edge
}

fn ids(graph: &Graph, query: &str) -> Vec<String> {
    let mut ids = find_exact_nodes(graph, query)
        .into_iter()
        .map(|index| graph.node(index).id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

#[test]
fn coincident_export_binding_resolves_to_its_proven_declaration() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut document = fixture(&path)?;
    for reverse in [false, true] {
        if reverse {
            document.nodes.reverse();
            document.links.reverse();
        }
        fs::write(&path, serde_json::to_vec(&document)?)?;
        // Full document, cold traversal projection, and warm compact cache
        // must retain enough evidence to make the same decision.
        for projection in [
            document.to_legacy_document()?,
            compass_model::GraphDocument::load_for_traversal(&path)?,
            compass_model::GraphDocument::load_for_traversal(&path)?,
        ] {
            let graph = Graph::from_document(projection)?;
            for query in ["entry", "entry()", "api.entry"] {
                assert_eq!(ids(&graph, query), ["function"], "{query}");
            }
            assert_eq!(ids(&graph, "binding"), ["binding"]);
            let answer = render_shortest_path_with_limit(&graph, "entry", "leaf", 2)?;
            assert!(answer.contains("1 hops"), "{answer}");
            assert!(!answer.contains("AMBIGUOUS"), "{answer}");
        }
        let store = SqliteStore::open(directory.path().join(STORE_FILE_NAME))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &document)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        fs::write(
            directory.path().join(STORE_REF_FILE_NAME),
            serde_json::to_vec(&store.snapshot_reference()?)?,
        )?;
        store.checkpoint()?;
        for engine in [
            open_with_engine(
                &path,
                None,
                &directory.path().join("cache"),
                EngineSelection::Json,
            )?,
            open_with_engine(
                &path,
                None,
                &directory.path().join("direct-cache"),
                EngineSelection::Store,
            )?,
            open_with_store(&store, &path, None, &directory.path().join("store-cache"))?,
        ] {
            let response = engine.callees(CallRequest {
                symbol: "entry".to_owned(),
                include_heuristic: false,
                limits: CodeQueryLimits::default(),
            })?;
            assert!(
                !response
                    .diagnostics
                    .iter()
                    .any(|d| d.code == QueryDiagnosticCode::AmbiguousMatch)
            );
            assert!(
                response
                    .edges
                    .iter()
                    .any(|e| e.source == "function" && e.target == "leaf")
            );
            let trail = engine.node_trail(NodeTrailRequest {
                source: "entry".to_owned(),
                target: "leaf".to_owned(),
                include_heuristic: false,
                limits: CodeQueryLimits::default(),
            })?;
            assert_eq!(trail.paths.len(), 1);
            let exact = engine.callees(CallRequest {
                symbol: "binding".to_owned(),
                include_heuristic: false,
                limits: CodeQueryLimits::default(),
            })?;
            assert!(exact.edges.is_empty(), "exact binding ID must not redirect");
        }
    }
    Ok(())
}

#[test]
fn incomplete_or_conflicting_export_proof_preserves_ambiguity() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    for case in [
        "missing",
        "wrong_site",
        "reversed",
        "inferred",
        "mixed_confidence",
        "deferred",
        "missing_site",
        "ambiguous",
        "two_targets",
        "other_declaration",
        "wrong_qualified_name",
    ] {
        let mut document = fixture(&path)?;
        match case {
            "missing" => document.links.retain(|e| e.kind != EdgeKind::Exports),
            "wrong_site" => {
                let site = document.links[1]
                    .relationship_site
                    .as_mut()
                    .ok_or("missing site")?;
                site.end_byte = 3;
                site.end_column = 3;
            }
            "reversed" => {
                document.links[1].source = "function".to_owned();
                document.links[1].target = "module".to_owned();
            }
            "inferred" => document.links[1].evidence[0].confidence = EvidenceConfidence::Inferred,
            "mixed_confidence" => {
                let mut weaker = document.links[1].evidence[0].clone();
                weaker.confidence = EvidenceConfidence::Inferred;
                document.links[1].evidence.push(weaker);
            }
            "deferred" => document.links[1].deferred = true,
            "missing_site" => document.links[1].relationship_site = None,
            "ambiguous" => {
                let evidence = &mut document.links[1].evidence[0];
                evidence.confidence = EvidenceConfidence::Ambiguous;
                evidence.candidates = ["function", "leaf"]
                    .map(|id| ResolutionCandidate {
                        node_id: id.to_owned(),
                        reason: "two possible targets".to_owned(),
                        confidence: EvidenceConfidence::Exact,
                        score: None,
                        anchor: None,
                    })
                    .to_vec();
            }
            "two_targets" => {
                document.nodes.push(support::node(
                    "other",
                    NodeKind::Function,
                    "elsewhere",
                    "api.elsewhere",
                ));
                document.links.push(edge(
                    &document.links[1],
                    "module",
                    EdgeKind::Exports,
                    "other",
                ));
            }
            "other_declaration" => document.nodes.push(support::node(
                "other",
                NodeKind::Function,
                "entry()",
                "other.entry",
            )),
            "wrong_qualified_name" => document.nodes[2].qualified_name = "other.entry".to_owned(),
            _ => return Err("unexpected case".into()),
        }
        let graph = Graph::from_document(document.to_legacy_document()?)?;
        assert!(ids(&graph, "entry").len() >= 2, "{case}");
        assert!(
            render_shortest_path_with_limit(&graph, "entry", "leaf", 2).is_err(),
            "{case}"
        );
        // Graph validity is independent of proof. Mutation keys must follow
        // updated endpoints/site; all cases remain valid public graphs.
        for edge in &mut document.links {
            edge.id = edge_id(
                &edge.source,
                edge.kind,
                &edge.target,
                edge.relationship_site.as_ref(),
                None,
            );
            edge.key.clone_from(&edge.id);
        }
        // Reverse exports is deliberately an invalid endpoint contract, so
        // the permissive legacy reader alone exercises that negative.
        if case == "reversed" {
            continue;
        }
        fs::write(&path, serde_json::to_vec(&document)?)?;
        for _ in 0..2 {
            let graph =
                Graph::from_document(compass_model::GraphDocument::load_for_traversal(&path)?)?;
            assert!(ids(&graph, "entry").len() >= 2, "cached {case}");
        }
        let engine = open(&path, None, &directory.path().join("negative-cache"))?;
        let response = engine.callees(CallRequest {
            symbol: "entry".to_owned(),
            include_heuristic: true,
            limits: CodeQueryLimits::default(),
        })?;
        assert!(
            response
                .diagnostics
                .iter()
                .any(|d| d.code == QueryDiagnosticCode::AmbiguousMatch),
            "{case}"
        );
        assert!(response.edges.is_empty(), "{case}");
    }
    Ok(())
}

#[test]
fn truncated_candidate_or_export_evidence_never_selects_a_target() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let mut document = fixture(&path)?;
    fs::write(&path, serde_json::to_vec(&document)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let response = engine.callees(CallRequest {
        symbol: "entry".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_candidates: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(response.truncated);
    assert!(
        response
            .diagnostics
            .iter()
            .any(|d| d.code == QueryDiagnosticCode::AmbiguousMatch)
    );
    assert!(response.edges.is_empty());
    for index in 0..1024 {
        let id = format!("other:{index}");
        document.nodes.push(support::node(
            &id,
            NodeKind::Function,
            "other",
            &format!("api.other{index}"),
        ));
        document
            .links
            .push(edge(&document.links[1], "module", EdgeKind::Exports, &id));
    }
    fs::write(&path, serde_json::to_vec(&document)?)?;
    let graph = Graph::from_document(document.to_legacy_document()?)?;
    assert_eq!(ids(&graph, "entry"), ["binding", "function"]);
    let engine = open(&path, None, &directory.path().join("bounded-cache"))?;
    let response = engine.callees(CallRequest {
        symbol: "entry".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(response.truncated);
    assert!(response.diagnostics.iter().any(
        |d| d.code == QueryDiagnosticCode::AmbiguousMatch && d.message.contains("proof bound")
    ));
    assert!(response.edges.is_empty());
    Ok(())
}

#[test]
fn coarse_legacy_source_location_is_not_export_binding_proof() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    let document = fixture(&path)?;
    let mut legacy = document.to_legacy_document()?;
    let binding = legacy
        .nodes
        .iter_mut()
        .find(|node| node.id == "binding")
        .ok_or("missing binding")?;
    binding.attributes.insert(
        "source_location".to_owned(),
        serde_json::Value::String("L1".to_owned()),
    );
    let graph = Graph::from_document(legacy)?;
    assert_eq!(ids(&graph, "entry"), ["binding", "function"]);
    Ok(())
}
