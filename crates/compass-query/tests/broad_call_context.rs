mod support;

use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;

use compass_graph::GraphSnapshotBuilder;
use compass_model::code_graph::{EdgeKind, EdgeRecord, GraphDocument, NodeKind};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance};
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryLimits, DiscoveryQueryRequest, DiscoveryScope, DiscoveryScopeKind,
    DiscoveryTraversal, MAX_INDEXED_CANDIDATE_NODES_READ, MAX_INDEXED_CANDIDATE_PROBES,
    QueryDiagnosticCode,
};
use compass_query::{CodeQueryEngine, open, open_with_store};
use compass_store::SqliteStore;

fn request(depth: u32) -> DiscoveryQueryRequest {
    DiscoveryQueryRequest {
        question: "Where are protocol errors formatted?".to_owned(),
        direction: DiscoveryDirection::Auto,
        relation_contexts: Vec::new(),
        scope: Vec::new(),
        traversal: DiscoveryTraversal::Bfs,
        include_heuristic: false,
        limits: DiscoveryLimits {
            max_depth: depth,
            ..DiscoveryLimits::default()
        },
    }
}

fn graph(path: &Path) -> Result<GraphDocument, Box<dyn Error>> {
    support::write_graph(path)?;
    let mut graph = GraphDocument::load(path)?;
    graph.nodes = vec![
        support::node(
            "seed",
            NodeKind::Function,
            "format_protocol_errors",
            "fixture.format_protocol_errors",
        ),
        support::node("entry", NodeKind::Function, "opaque", "fixture.opaque"),
        support::node("leaf", NodeKind::Function, "helper", "fixture.helper"),
    ];
    let template = graph
        .links
        .first()
        .cloned()
        .ok_or("missing edge template")?;
    graph.links = vec![
        call(&template, "seed", "entry"),
        call(&template, "entry", "leaf"),
    ];
    Ok(graph)
}

fn call(template: &EdgeRecord, source: &str, target: &str) -> EdgeRecord {
    let mut edge = template.clone();
    edge.source = source.to_owned();
    edge.target = target.to_owned();
    edge.kind = EdgeKind::Calls;
    edge.context = Some("call".to_owned());
    identify(&mut edge);
    edge
}

fn identify(edge: &mut EdgeRecord) {
    edge.id = compass_model::identity::edge_id(
        &edge.source,
        edge.kind,
        &edge.target,
        edge.relationship_site.as_ref(),
        edge.occurrence_rule.as_ref().map(|rule| rule.as_str()),
    );
    edge.key = edge.id.clone();
}

fn engines(root: &Path, graph: &GraphDocument) -> Result<[CodeQueryEngine; 2], Box<dyn Error>> {
    let path = root.join("graph.json");
    std::fs::write(&path, serde_json::to_vec(graph)?)?;
    let json = open(&path, None, &root.join("json-cache"))?;
    let store = SqliteStore::open(root.join("store.sqlite3"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored = open_with_store(&store, &path, None, &root.join("store-cache"))?;
    Ok([json, stored])
}

#[test]
fn continuation_needs_exact_calls_at_every_hop_and_exact_targets() -> Result<(), Box<dyn Error>> {
    for case in [
        "exact",
        "reference-root",
        "inferred-root",
        "inferred-inner",
        "reference-inner",
        "heuristic-inner",
        "non-ast-inner",
        "reverse-inner",
        "source-free-target",
        "misanchored-inner",
        "inferred-target",
    ] {
        let directory = tempfile::tempdir()?;
        let mut graph = graph(&directory.path().join("graph.json"))?;
        match case {
            "reference-root" => {
                graph.links[0].kind = EdgeKind::References;
                identify(&mut graph.links[0]);
            }
            "reference-inner" => {
                graph.links[1].kind = EdgeKind::References;
                identify(&mut graph.links[1]);
            }
            "heuristic-inner" => {
                let edge = &mut graph.links[1];
                edge.evidence = vec![Provenance::heuristic(
                    "test",
                    "fixture-heuristic",
                    edge.relationship_site.clone().ok_or("missing site")?,
                    Vec::new(),
                )?];
            }
            "non-ast-inner" => graph.links[1].evidence[0].origin = EvidenceOrigin::Config,
            "reverse-inner" => {
                graph.links[1] = call(&graph.links[1], "leaf", "entry");
            }
            "source-free-target" => graph.nodes[2].source = None,
            "inferred-root" => graph.links[0].evidence[0].confidence = EvidenceConfidence::Inferred,
            "inferred-inner" => {
                graph.links[1].evidence[0].confidence = EvidenceConfidence::Inferred
            }
            "misanchored-inner" => {
                let edge = &mut graph.links[1];
                let site = edge.relationship_site.as_mut().ok_or("missing site")?;
                site.file = "src/payments/gateway.rs".to_owned();
                edge.evidence[0].anchors = vec![site.clone()];
                identify(edge);
            }
            "inferred-target" => {
                graph.nodes[2].evidence[0].confidence = EvidenceConfidence::Inferred
            }
            _ => {}
        }
        for engine in engines(directory.path(), &graph)? {
            let query = request(2);
            let ranked = engine.discover_ranked(query.clone())?;
            assert_eq!(ranked.candidates.len(), 1, "{case}");
            assert_eq!(ranked.candidates[0].node.id, "seed");
            let contextual = engine.discover_ranked_with_context(query, true)?;
            assert_eq!(contextual.candidates, ranked.candidates, "{case}");
            assert_eq!(
                contextual.discovery.nodes.iter().any(|n| n.id == "leaf"),
                case == "exact",
                "{case}"
            );
            assert_eq!(
                contextual
                    .discovery
                    .edges
                    .iter()
                    .any(|e| e.source == "entry" && e.target == "leaf"),
                case == "exact",
                "{case}"
            );
        }
    }
    Ok(())
}

#[test]
fn continuation_preserves_scopes_and_finite_depth_through_cycles() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let mut graph = graph(&directory.path().join("graph.json"))?;
    let mut middle = support::node("middle", NodeKind::Function, "step", "fixture.step");
    middle.source = graph.nodes[1].source.clone();
    graph.nodes.push(middle);
    let template = graph.links[0].clone();
    graph.links = vec![
        call(&template, "seed", "entry"),
        call(&template, "entry", "middle"),
        call(&template, "middle", "leaf"),
        call(&template, "leaf", "entry"),
    ];
    let leaf = &mut graph.nodes[2];
    let site = leaf.source.as_mut().ok_or("missing leaf source")?;
    site.file = "src/payments/gateway.rs".to_owned();
    leaf.evidence[0].anchors = vec![site.clone()];
    // Keep the cycle's invocation anchor inside its actual source owner.
    let cycle = &mut graph.links[3];
    cycle.relationship_site = Some(site.clone());
    cycle.evidence[0].anchors = vec![site.clone()];
    identify(cycle);
    for engine in engines(directory.path(), &graph)? {
        for traversal in [DiscoveryTraversal::Bfs, DiscoveryTraversal::Dfs] {
            for depth in [2, 3, 8] {
                let mut query = request(depth);
                query.traversal = traversal;
                let answer = engine.discover(query.clone())?;
                assert_eq!(answer.nodes.iter().any(|n| n.id == "leaf"), depth >= 3);
                assert!(
                    answer.stats.expanded_relationships <= query.limits.max_expanded_relationships
                );
                query.scope = vec![DiscoveryScope {
                    kind: DiscoveryScopeKind::Source,
                    value: "src/lib.rs".to_owned(),
                }];
                let scoped = engine.discover(query)?;
                assert!(!scoped.nodes.iter().any(|n| n.id == "leaf"));
                assert!(
                    scoped
                        .edges
                        .iter()
                        .all(|e| e.source != "leaf" && e.target != "leaf")
                );
            }
        }
    }
    Ok(())
}

#[test]
fn continuation_bounds_dense_graphs_and_preserves_endpoint_coherence() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let mut graph = graph(&directory.path().join("graph.json"))?;
    let template = graph.links[0].clone();
    graph.nodes.truncate(1);
    graph.links.clear();
    for index in 0..16 {
        let mut previous = "seed".to_owned();
        for depth in 1..=3 {
            let id = format!("branch:{index:02}:{depth}");
            graph.nodes.push(support::node(
                &id,
                NodeKind::Function,
                "opaque",
                &format!("fixture.{id}"),
            ));
            graph.links.push(call(&template, &previous, &id));
            previous = id;
        }
    }
    for engine in engines(directory.path(), &graph)? {
        for (nodes, edges, work) in [(64, 128, 10000), (4, 3, 10000), (64, 128, 1)] {
            let mut query = request(4);
            query.limits.max_nodes = nodes;
            query.limits.max_edges = edges;
            query.limits.max_expanded_relationships = work;
            let answer = engine.discover(query)?;
            assert!(answer.truncated);
            assert!(answer.nodes.len() <= nodes as usize);
            assert!(answer.edges.len() <= edges as usize);
            assert!(answer.stats.expanded_relationships <= work);
            assert!(answer.stats.candidate_nodes <= MAX_INDEXED_CANDIDATE_NODES_READ);
            assert!(answer.stats.candidate_probes <= MAX_INDEXED_CANDIDATE_PROBES);
            assert!(
                answer.nodes.len() < graph.nodes.len(),
                "the fixed continuation state cap must be exercised"
            );
            let ids: BTreeSet<_> = answer.nodes.iter().map(|n| n.id.as_str()).collect();
            assert!(
                answer
                    .edges
                    .iter()
                    .all(|e| ids.contains(e.source.as_str()) && ids.contains(e.target.as_str()))
            );
            assert!(
                answer
                    .diagnostics
                    .iter()
                    .any(|d| d.code == QueryDiagnosticCode::BoundedTruncation)
            );
            assert!(
                !answer
                    .diagnostics
                    .iter()
                    .any(|d| d.code == QueryDiagnosticCode::NoMatch)
            );
        }
    }
    Ok(())
}

#[test]
fn structural_context_preserves_prior_expansion_work_without_a_second_pass()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = graph(&directory.path().join("graph.json"))?;
    for engine in engines(directory.path(), &graph)? {
        for work in [1, 3, 20] {
            let mut query = request(2);
            query.question = "seed".to_owned();
            query.direction = DiscoveryDirection::Outgoing;
            query.limits.max_expanded_relationships = work;
            let original = engine.discover_ranked(query.clone())?;
            assert!(original.discovery.stats.expanded_relationships > 0);
            assert!(original.discovery.stats.expanded_relationships <= work);
            let contextual = engine.discover_ranked_with_context(query, true)?;
            assert_eq!(contextual.candidates, original.candidates);
            assert_eq!(
                contextual.discovery, original.discovery,
                "structural discovery already owns its neighborhood and cost; optional presentation must not spend another allowance"
            );
        }
    }
    Ok(())
}

#[test]
fn dfs_revisits_a_convergent_state_when_a_shorter_path_arrives() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let mut graph = graph(&directory.path().join("graph.json"))?;
    let template = graph.links[0].clone();
    graph.nodes.truncate(1);
    graph.links.clear();
    for id in ["a", "b", "detour", "join", "tail", "terminal"] {
        graph.nodes.push(support::node(
            id,
            NodeKind::Function,
            "opaque",
            &format!("fixture.{id}"),
        ));
    }
    let a = call(&template, "seed", "a");
    let b = call(&template, "seed", "b");
    let (long, short) = if a.id > b.id { ("a", "b") } else { ("b", "a") };
    graph.links = vec![
        a,
        b,
        call(&template, long, "detour"),
        call(&template, "detour", "join"),
        call(&template, short, "join"),
        call(&template, "join", "tail"),
        call(&template, "tail", "terminal"),
    ];
    for engine in engines(directory.path(), &graph)? {
        for traversal in [DiscoveryTraversal::Bfs, DiscoveryTraversal::Dfs] {
            let mut query = request(4);
            query.traversal = traversal;
            let answer = engine.discover(query)?;
            assert!(answer.nodes.iter().any(|n| n.id == "terminal"));
            assert!(
                answer
                    .edges
                    .iter()
                    .any(|e| e.source == "tail" && e.target == "terminal")
            );
            assert!(!answer.truncated);
        }
    }
    Ok(())
}

#[test]
fn continuation_state_limit_signals_the_first_excess_state() -> Result<(), Box<dyn Error>> {
    for leaves in [15, 16] {
        let directory = tempfile::tempdir()?;
        let mut graph = graph(&directory.path().join("graph.json"))?;
        let template = graph.links[0].clone();
        graph.nodes.truncate(1);
        graph.links.clear();
        for index in 0..16 {
            let head = format!("head:{index}");
            graph.nodes.push(support::node(
                &head,
                NodeKind::Function,
                "opaque",
                &format!("fixture.{head}"),
            ));
            graph.links.push(call(&template, "seed", &head));
            if index < leaves {
                let leaf = format!("tip:{index}");
                graph.nodes.push(support::node(
                    &leaf,
                    NodeKind::Function,
                    "opaque",
                    &format!("fixture.{leaf}"),
                ));
                graph.links.push(call(&template, &head, &leaf));
            }
        }
        assert_eq!(graph.nodes.len(), 17 + leaves);
        for engine in engines(directory.path(), &graph)? {
            let answer = engine.discover(request(3))?;
            assert_eq!(answer.truncated, leaves == 16);
            assert_eq!(answer.nodes.len(), graph.nodes.len());
        }
    }
    Ok(())
}

#[test]
fn filtered_fanout_lookahead_is_charged_before_discarding_heuristics() -> Result<(), Box<dyn Error>>
{
    for fanout in [16, 17] {
        let directory = tempfile::tempdir()?;
        let mut graph = graph(&directory.path().join("graph.json"))?;
        let template = graph.links[0].clone();
        graph.nodes.truncate(2);
        graph.links.truncate(1);
        for index in 0..fanout {
            let id = format!("tip:{index}");
            graph.nodes.push(support::node(
                &id,
                NodeKind::Function,
                "opaque",
                &format!("fixture.{id}"),
            ));
            graph.links.push(call(&template, "entry", &id));
        }
        let last = graph
            .links
            .iter_mut()
            .filter(|e| e.source == "entry")
            .max_by(|a, b| a.id.cmp(&b.id))
            .ok_or("missing fanout")?;
        last.evidence = vec![Provenance::heuristic(
            "test",
            "fixture-guess",
            last.relationship_site.clone().ok_or("missing site")?,
            Vec::new(),
        )?];
        for engine in engines(directory.path(), &graph)? {
            let answer = engine.discover(request(2))?;
            assert_eq!(answer.truncated, fanout == 17);
            assert_eq!(
                answer.edges.iter().filter(|e| e.source == "entry").count(),
                if fanout == 16 { 15 } else { 16 }
            );
            assert_eq!(
                answer.stats.expanded_relationships,
                fanout + 2,
                "first-hop/root call reads and raw fanout, including the filtered lookahead, must remain charged"
            );
        }
    }
    Ok(())
}

#[test]
fn dfs_frontier_limit_does_not_execute_an_unqueued_continuation() -> Result<(), Box<dyn Error>> {
    for final_fanout in [4, 5] {
        let directory = tempfile::tempdir()?;
        let mut graph = graph(&directory.path().join("graph.json"))?;
        let template = graph.links[0].clone();
        graph.nodes.truncate(1);
        graph.links.clear();
        let mut current = "seed".to_owned();
        for level in 0..5 {
            let fanout = if level == 4 { final_fanout } else { 16 };
            let mut choices = Vec::new();
            for index in 0..fanout {
                let id = format!("level:{level}:{index}");
                graph.nodes.push(support::node(
                    &id,
                    NodeKind::Function,
                    "opaque",
                    &format!("fixture.{id}"),
                ));
                let edge = call(&template, &current, &id);
                choices.push((edge.id.clone(), id));
                graph.links.push(edge);
            }
            choices.sort();
            current = choices.pop().ok_or("empty branch")?.1;
        }
        graph.nodes.push(support::node(
            "terminal",
            NodeKind::Function,
            "opaque",
            "fixture.terminal",
        ));
        graph.links.push(call(&template, &current, "terminal"));
        for engine in engines(directory.path(), &graph)? {
            let mut query = request(8);
            query.traversal = DiscoveryTraversal::Dfs;
            query.limits.max_nodes = 500;
            query.limits.max_edges = 1000;
            let answer = engine.discover(query)?;
            assert_eq!(
                answer.nodes.iter().any(|n| n.id == "terminal"),
                final_fanout == 4
            );
            assert!(answer.truncated);
        }
    }
    Ok(())
}

#[test]
fn inferred_incoming_broad_context_keeps_only_incoming_first_hop() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let mut graph = graph(&directory.path().join("graph.json"))?;
    let template = graph.links[0].clone();
    for id in ["predecessor", "upstream"] {
        graph.nodes.push(support::node(
            id,
            NodeKind::Function,
            "opaque",
            &format!("fixture.{id}"),
        ));
    }
    graph.links.push(call(&template, "predecessor", "seed"));
    graph.links.push(call(&template, "upstream", "predecessor"));
    for engine in engines(directory.path(), &graph)? {
        let mut query = request(8);
        // Existing relationship recall can also admit direct callers. Select
        // one root to isolate first-hop context from candidate admission.
        query.limits.max_seeds = 1;
        query.question = "Where are protocol errors formatted, referenced by callers?".to_owned();
        let original = engine.discover_ranked(query.clone())?;
        assert_eq!(
            original.discovery.selected_direction,
            DiscoveryDirection::Incoming
        );
        assert_eq!(
            original.discovery.direction_source,
            compass_model::query_contract::DiscoveryDirectionSource::Heuristic
        );
        assert_eq!(
            original.discovery.stats.expanded_relationships, 0,
            "this must exercise fielded routing, not a prior structural traversal"
        );
        assert_eq!(
            original.discovery.seeds.len(),
            1,
            "{:?}",
            original.discovery.seeds
        );
        assert_eq!(original.discovery.seeds[0].node_id, "seed");
        let contextual = engine.discover_ranked_with_context(query, true)?;
        assert_eq!(contextual.candidates, original.candidates);
        assert_eq!(contextual.discovery.seeds, original.discovery.seeds);
        let ids: BTreeSet<_> = contextual
            .discovery
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect();
        assert!(ids.contains("predecessor"));
        assert!(
            !ids.contains("upstream"),
            "incoming context is first-hop only"
        );
        assert!(
            !ids.contains("entry") && !ids.contains("leaf"),
            "inferred Incoming must never start outgoing continuation"
        );
        assert_eq!(contextual.discovery.edges.len(), 1);
        assert_eq!(contextual.discovery.edges[0].source, "predecessor");
        assert_eq!(contextual.discovery.edges[0].target, "seed");
        assert_eq!(
            contextual.discovery.truncated, original.discovery.truncated,
            "context must preserve any existing retrieval bound"
        );
    }
    Ok(())
}
