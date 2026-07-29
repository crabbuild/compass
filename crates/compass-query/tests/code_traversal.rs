mod support;

use std::collections::HashSet;
use std::fs;

use compass_languages::Engine;
use compass_model::code_graph::EdgeKind;
use compass_model::query_contract::{CallRequest, CodeQueryLimits, NodeTrailRequest};
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
fn callees_return_each_exact_source_site_for_parallel_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("src/lib.rs");
    fs::create_dir_all(source_path.parent().ok_or("missing source parent")?)?;
    fs::write(
        &source_path,
        b"fn callee(){} fn caller(){callee();callee();}",
    )?;
    let extraction = Engine::default().extract(&source_path)?;
    let flexible = compass_graph::build_from_extraction(&extraction, true, Some(directory.path()));
    let graph =
        compass_graph::normalize_document_v1(&flexible, directory.path(), "sha256:test", None)?;
    let serialized = serde_json::to_vec_pretty(&graph)?;
    let graph_path = directory.path().join("graph.json");
    fs::write(&graph_path, &serialized)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.callees(CallRequest {
        symbol: "caller".to_owned(),
        limits: CodeQueryLimits::default(),
    })?;
    let mut sites = response
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::Calls)
        .map(|edge| {
            let site = edge
                .relationship_site
                .as_ref()
                .ok_or("missing relationship site")?;
            assert_eq!(site.file, "src/lib.rs");
            assert_eq!(site.start_line, 1);
            assert_eq!(site.end_line, 1);
            assert!(
                edge.evidence
                    .iter()
                    .all(|item| item.extractor == "compass.languages.rust")
            );
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
