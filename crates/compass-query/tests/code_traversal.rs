mod support;

use std::collections::HashSet;
use std::fs;

use compass_model::code_graph::{EdgeKind, GraphDocument};
use compass_model::identity::edge_id;
use compass_model::provenance::{OccurrenceRule, SourceAnchor};
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, NodeTrailRequest, QueryDiagnosticCode,
};
use compass_query::open;

#[test]
fn callers_include_calls_and_route_bindings_while_callees_follow_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = CallRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    let callers = engine.callers(request.clone())?;
    let caller_ids = callers
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert!(caller_ids.contains("n:caller"));
    assert!(caller_ids.contains("n:route"));
    assert!(
        callers
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::RoutesTo)
    );
    assert!(!callers.nodes.iter().any(|node| node.id == "n:heuristic"));

    let enriched = engine.callers(CallRequest {
        include_heuristic: true,
        ..request.clone()
    })?;
    assert!(enriched.nodes.iter().any(|node| node.id == "n:heuristic"));

    let callees = engine.callees(request)?;
    assert!(callees.nodes.iter().any(|node| node.id == "n:callee"));
    assert!(
        callees
            .edges
            .iter()
            .all(|edge| edge.kind == EdgeKind::Calls)
    );
    Ok(())
}

#[test]
fn call_queries_never_publish_edges_with_truncated_endpoints()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.callers(CallRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_nodes: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    let node_ids = response
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert!(response.truncated);
    assert!(response.edges.iter().all(|edge| {
        node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
    }));
    Ok(())
}

#[test]
fn callees_return_each_exact_source_site_for_parallel_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    graph.graph.files[0].byte_size = 43;
    let template = graph
        .links
        .iter()
        .find(|edge| edge.source == "n:list" && edge.target == "n:callee")
        .cloned()
        .ok_or("missing call edge fixture")?;
    for (id, start_byte, end_byte) in [("parallel:1", 26, 34), ("parallel:2", 35, 43)] {
        let mut edge = template.clone();
        edge.source = "n:caller".to_owned();
        edge.target = "n:callee".to_owned();
        edge.occurrence_rule = OccurrenceRule::new(id);
        edge.relationship_site = Some(SourceAnchor {
            file: "src/lib.rs".to_owned(),
            start_byte,
            end_byte,
            start_line: 1,
            start_column: u32::try_from(start_byte)?,
            end_line: 1,
            end_column: u32::try_from(end_byte)?,
        });
        let identity = edge_id(
            &edge.source,
            EdgeKind::Calls,
            &edge.target,
            edge.relationship_site.as_ref(),
            edge.occurrence_rule.as_ref().map(OccurrenceRule::as_str),
        );
        edge.id.clone_from(&identity);
        edge.key = identity;
        for evidence in &mut edge.evidence {
            evidence.extractor = "compass.languages.rust".to_owned();
        }
        graph.links.push(edge);
    }
    let serialized = serde_json::to_vec_pretty(&graph)?;
    fs::write(&graph_path, &serialized)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.callees(CallRequest {
        symbol: "caller".to_owned(),
        include_heuristic: true,
        limits: CodeQueryLimits::default(),
    })?;
    let mut sites = response
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Calls && edge.target == "n:callee")
        .map(|edge| {
            let site = edge
                .relationship_site
                .as_ref()
                .ok_or("missing relationship site")?;
            assert_eq!(site.file, "src/lib.rs");
            assert_eq!(site.start_line, 1);
            assert_eq!(site.end_line, 1);
            Ok::<_, Box<dyn std::error::Error>>((
                site.start_byte,
                site.end_byte,
                site.start_column,
                site.end_column,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    sites.sort_unstable();

    assert_eq!(sites, [(26, 34, 26, 34), (35, 43, 35, 43)]);
    assert!(
        serialized
            .windows(b"compass.languages.unknown".len())
            .all(|window| window != b"compass.languages.unknown")
    );
    Ok(())
}

#[test]
fn node_trail_returns_a_stable_evidence_aware_path() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let trail = engine.node_trail(NodeTrailRequest {
        source: "dependent".to_owned(),
        target: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert_eq!(trail.paths.len(), 1);
    assert_eq!(
        trail.paths[0].node_ids,
        ["n:dependent", "n:caller", "n:list", "n:callee"]
    );
    assert_eq!(trail.paths[0].edge_ids.len(), 3);
    Ok(())
}

#[test]
fn node_trail_rejects_reverse_only_paths_with_a_typed_diagnostic()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let trail = engine.node_trail(NodeTrailRequest {
        source: "Store.callee".to_owned(),
        target: "dependent".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;

    assert!(trail.paths.is_empty());
    assert!(trail.nodes.is_empty());
    assert!(trail.edges.is_empty());
    assert!(trail.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == QueryDiagnosticCode::DirectionMismatch
            && diagnostic.message.contains("source-to-target direction")
    }));
    Ok(())
}

#[test]
fn node_trail_never_exceeds_node_or_edge_budgets() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let trail = engine.node_trail(NodeTrailRequest {
        source: "dependent".to_owned(),
        target: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_nodes: 2,
            max_edges: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(trail.truncated);
    assert!(trail.nodes.len() <= 2);
    assert!(trail.edges.len() <= 1);
    assert!(trail.paths.is_empty());
    Ok(())
}

#[test]
fn node_trail_excludes_graph_assembly_endpoint_remaps_by_default()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_endpoint_remap_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = |include_heuristic| NodeTrailRequest {
        source: "crate::Caller".to_owned(),
        target: "crate::Target".to_owned(),
        include_heuristic,
        limits: CodeQueryLimits::default(),
    };

    assert!(engine.node_trail(request(false))?.paths.is_empty());
    let enriched = engine.node_trail(request(true))?;
    assert_eq!(enriched.paths.len(), 1);
    assert!(enriched.edges.iter().any(|edge| {
        edge.evidence.iter().any(|evidence| {
            evidence.rule.as_deref() == Some("graph-ghost-endpoint-remap")
                && evidence.wiring_site.is_some()
        })
    }));
    Ok(())
}

#[test]
fn node_trail_excludes_deferred_external_inheritance_by_default()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_deferred_external_inheritance_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = |include_heuristic| NodeTrailRequest {
        source: "App\\Child".to_owned(),
        target: "Illuminate\\Database\\Eloquent\\Model".to_owned(),
        include_heuristic,
        limits: CodeQueryLimits::default(),
    };

    assert!(engine.node_trail(request(false))?.paths.is_empty());
    let enriched = engine.node_trail(request(true))?;
    assert_eq!(enriched.paths.len(), 1);
    assert!(enriched.edges.iter().all(|edge| {
        edge.evidence.iter().any(|evidence| {
            evidence.extractor == "compass.graph.external-placeholder"
                && evidence.rule.as_deref() == Some("external-symbol-placeholder")
        })
    }));
    Ok(())
}
