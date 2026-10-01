mod support;

use std::collections::HashSet;
use std::fs;

use compass_graph::GraphSnapshotBuilder;
use compass_model::code_graph::{EdgeKind, GraphDocument};
use compass_model::identity::{edge_id, file_id};
use compass_model::provenance::{OccurrenceRule, SourceAnchor};
use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, ImpactRequest, NodeTrailRequest, QueryDiagnosticCode,
};
use compass_query::{open, open_with_store};
use compass_store::SqliteStore;

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
    assert!(caller_ids.contains("n:alias"));
    assert!(
        callers
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Aliases)
    );
    assert!(!callers.nodes.iter().any(|node| node.id == "n:heuristic"));

    let enriched = engine.callers(CallRequest {
        include_heuristic: true,
        ..request.clone()
    })?;
    assert!(enriched.nodes.iter().any(|node| node.id == "n:heuristic"));

    let callees = engine.callees(request)?;
    assert!(
        callees.call_summary.is_none(),
        "producers without call inventories retain relationship counts"
    );
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
fn callers_recover_source_backed_importers_that_target_a_tsconfig_alias_owner()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let module = graph
        .nodes
        .iter()
        .find(|node| node.id == "n:list")
        .cloned()
        .ok_or("missing module template")?;
    let mut module = module;
    module.id = "n:list-module".to_owned();
    module.kind = compass_model::code_graph::NodeKind::Module;
    // This is the shape emitted after a TypeScript `paths` alias such as
    // `@scope/pkg/* -> packages/pkg/src/*` has been resolved: the importer
    // edge targets the module owner while the queried declaration is nested
    // inside that owner.
    module.name = "@scope/pkg".to_owned();
    module.qualified_name = "@scope/pkg/index".to_owned();
    if let Some(source) = module.source.as_mut() {
        source.file = "packages/pkg/src/index.ts".to_owned();
    }
    let mut module_file = graph
        .graph
        .files
        .first()
        .cloned()
        .ok_or("missing file template")?;
    module_file.id = file_id("packages/pkg/src/index.ts");
    module_file.path = "packages/pkg/src/index.ts".to_owned();
    graph.graph.files.push(module_file);
    graph.nodes.push(module);
    let edge_template = graph
        .links
        .iter()
        .find(|edge| edge.source == "n:caller")
        .cloned()
        .ok_or("missing edge template")?;
    for index in 0..8 {
        let source_id = format!("n:importer-{index}");
        let mut source = graph
            .nodes
            .iter()
            .find(|node| node.id == "n:caller")
            .cloned()
            .ok_or("missing source template")?;
        source.id = source_id.clone();
        source.name = format!("importer_{index}");
        source.qualified_name = format!("Api.importer_{index}");
        graph.nodes.push(source);
        let mut edge = edge_template.clone();
        edge.source = source_id;
        edge.target = "n:list-module".to_owned();
        edge.kind = EdgeKind::Imports;
        let id = edge_id(
            &edge.source,
            edge.kind,
            &edge.target,
            edge.relationship_site.as_ref(),
            None,
        );
        edge.id.clone_from(&id);
        edge.key = id;
        graph.links.push(edge);
    }
    let mut contains = edge_template.clone();
    contains.source = "n:list-module".to_owned();
    contains.target = "n:list".to_owned();
    contains.kind = EdgeKind::Contains;
    let id = edge_id(
        &contains.source,
        contains.kind,
        &contains.target,
        contains.relationship_site.as_ref(),
        None,
    );
    contains.id.clone_from(&id);
    contains.key = id;
    graph.links.push(contains);
    // A different module can share a display name or search term. Its import
    // must not become evidence for the queried declaration.
    let mut other_module = graph
        .nodes
        .iter()
        .find(|node| node.id == "n:list-module")
        .cloned()
        .ok_or("missing module")?;
    other_module.id = "n:other-module".to_owned();
    other_module.qualified_name = "@other/pkg/index".to_owned();
    if let Some(source) = other_module.source.as_mut() {
        source.file = "packages/other/src/index.ts".to_owned();
    }
    graph.nodes.push(other_module);
    let mut other_file = graph
        .graph
        .files
        .iter()
        .find(|file| file.path == "packages/pkg/src/index.ts")
        .cloned()
        .ok_or("missing module file")?;
    other_file.id = file_id("packages/other/src/index.ts");
    other_file.path = "packages/other/src/index.ts".to_owned();
    graph.graph.files.push(other_file);
    let mut unrelated_importer = graph
        .nodes
        .iter()
        .find(|node| node.id == "n:caller")
        .cloned()
        .ok_or("missing source template")?;
    unrelated_importer.id = "n:unrelated-importer".to_owned();
    unrelated_importer.name = "unrelated_importer".to_owned();
    unrelated_importer.qualified_name = "Other.unrelated_importer".to_owned();
    graph.nodes.push(unrelated_importer);
    let mut unrelated_edge = edge_template.clone();
    unrelated_edge.source = "n:unrelated-importer".to_owned();
    unrelated_edge.target = "n:other-module".to_owned();
    unrelated_edge.kind = EdgeKind::Imports;
    let id = edge_id(
        &unrelated_edge.source,
        unrelated_edge.kind,
        &unrelated_edge.target,
        unrelated_edge.relationship_site.as_ref(),
        None,
    );
    unrelated_edge.id.clone_from(&id);
    unrelated_edge.key = id;
    graph.links.push(unrelated_edge);
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.callers(CallRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    let importer_count = response
        .nodes
        .iter()
        .filter(|node| node.id.starts_with("n:importer-"))
        .count();
    assert_eq!(importer_count, 8);
    assert!(
        !response
            .nodes
            .iter()
            .any(|node| node.id == "n:unrelated-importer")
    );
    assert!(
        response
            .edges
            .iter()
            .any(|edge| { edge.kind == EdgeKind::Imports && edge.target == "n:list-module" })
    );
    assert!(
        !response
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::RelationshipInconsistency)
    );
    assert!(response.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == QueryDiagnosticCode::IncompleteCoverage
            && diagnostic.message.contains("owner-level dependency")
    }));

    let bounded = engine.callers(CallRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_edges: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(bounded.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == QueryDiagnosticCode::RelationshipInconsistency
            && diagnostic.message.contains("search found ")
            && diagnostic
                .message
                .contains("source-backed usages for n:list")
    }));

    let affected = engine.affected(
        ImpactRequest {
            symbol: "UserService.list".to_owned(),
            include_heuristic: false,
            limits: CodeQueryLimits::default(),
        },
        &[EdgeKind::Imports],
    )?;
    assert_eq!(
        affected
            .nodes
            .iter()
            .filter(|node| node.id.starts_with("n:importer-"))
            .count(),
        8
    );
    assert!(
        !affected
            .nodes
            .iter()
            .any(|node| node.id == "n:unrelated-importer")
    );
    assert!(!affected.paths.is_empty());
    assert!(affected.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == QueryDiagnosticCode::IncompleteCoverage
            && diagnostic.message.contains("owner-level dependency")
    }));
    for path in &affected.paths {
        assert_eq!(path.node_ids.len(), path.edge_ids.len() + 1);
        for (pair, edge_id) in path.node_ids.windows(2).zip(&path.edge_ids) {
            let edge = affected
                .edges
                .iter()
                .find(|edge| &edge.id == edge_id)
                .ok_or("missing path edge")?;
            assert_eq!(edge.target, pair[0]);
            assert_eq!(edge.source, pair[1]);
        }
    }
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
        calls_only: false,
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
        calls_only: false,
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
        calls_only: false,
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

fn write_weighted_trail_fixture(
    path: &std::path::Path,
    edges: &[(&str, EdgeKind, &str)],
) -> Result<(), Box<dyn std::error::Error>> {
    support::write_graph(path)?;
    let mut graph = GraphDocument::load(path)?;
    let template = graph
        .links
        .iter()
        .find(|edge| edge.kind == EdgeKind::Calls)
        .cloned()
        .ok_or("missing call template")?;
    graph.nodes = ["n:s", "n:a", "n:b", "n:t"]
        .into_iter()
        .map(|id| support::node(id, compass_model::code_graph::NodeKind::Function, id, id))
        .collect();
    graph.links = edges
        .iter()
        .map(|(source, kind, target)| {
            let mut edge = template.clone();
            edge.source = (*source).to_owned();
            edge.target = (*target).to_owned();
            edge.kind = *kind;
            edge.occurrence_rule = None;
            edge.id = edge_id(source, *kind, target, edge.relationship_site.as_ref(), None);
            edge.key.clone_from(&edge.id);
            edge
        })
        .collect();
    fs::write(path, serde_json::to_vec(&graph)?)?;
    Ok(())
}

#[test]
fn node_trail_keeps_a_costlier_shorter_prefix_that_can_reach_the_target()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    write_weighted_trail_fixture(
        &graph_path,
        &[
            ("n:s", EdgeKind::Calls, "n:a"),
            ("n:a", EdgeKind::Calls, "n:b"),
            ("n:a", EdgeKind::Calls, "n:s"),
            ("n:s", EdgeKind::References, "n:b"),
            ("n:b", EdgeKind::Calls, "n:t"),
        ],
    )?;
    for (max_depth, expected) in [
        (2, vec!["n:s", "n:b", "n:t"]),
        (3, vec!["n:s", "n:a", "n:b", "n:t"]),
    ] {
        for reverse in [false, true] {
            if reverse {
                let mut graph = GraphDocument::load(&graph_path)?;
                graph.nodes.reverse();
                graph.links.reverse();
                fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
            }
            let graph = GraphDocument::load(&graph_path)?;
            let store = SqliteStore::open(
                directory
                    .path()
                    .join(format!("store-{max_depth}-{reverse}.db")),
            )?;
            let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
            GraphSnapshotBuilder::new().activate(&store, &prepared)?;
            for engine in [
                open(&graph_path, None, &directory.path().join("cache"))?,
                open_with_store(
                    &store,
                    &graph_path,
                    None,
                    &directory.path().join("store-cache"),
                )?,
            ] {
                let response = engine.node_trail(NodeTrailRequest {
                    calls_only: false,
                    source: "n:s".to_owned(),
                    target: "n:t".to_owned(),
                    include_heuristic: false,
                    limits: CodeQueryLimits {
                        max_depth,
                        max_edges: 5,
                        ..CodeQueryLimits::default()
                    },
                })?;
                assert_eq!(response.paths.len(), 1);
                assert_eq!(response.paths[0].node_ids, expected);
                assert_eq!(response.paths[0].edge_ids.len(), expected.len() - 1);
            }
        }
    }
    Ok(())
}

#[test]
fn node_trail_does_not_readmit_a_rejected_node_without_paying_its_budget()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    write_weighted_trail_fixture(
        &graph_path,
        &[
            ("n:s", EdgeKind::Calls, "n:a"),
            ("n:s", EdgeKind::Calls, "n:b"),
            ("n:a", EdgeKind::Calls, "n:b"),
        ],
    )?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.node_trail(NodeTrailRequest {
        calls_only: false,
        source: "n:s".to_owned(),
        target: "n:b".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_nodes: 2,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(response.truncated);
    assert!(
        response.paths.is_empty(),
        "a budget-rejected node was admitted on a second visit"
    );
    assert!(
        !response.nodes.iter().any(|node| node.id == "n:b"),
        "a budget-rejected node leaked into the response on a second visit"
    );
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
        calls_only: false,
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
        calls_only: false,
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

#[test]
fn node_trail_depth_bound_does_not_assert_direction_mismatch_for_a_longer_forward_route()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    write_weighted_trail_fixture(
        &graph_path,
        &[
            ("n:s", EdgeKind::Calls, "n:a"),
            ("n:a", EdgeKind::Calls, "n:b"),
            ("n:b", EdgeKind::Calls, "n:t"),
            ("n:t", EdgeKind::Calls, "n:s"),
        ],
    )?;
    for reverse in [false, true] {
        let mut graph = GraphDocument::load(&graph_path)?;
        if reverse {
            graph.nodes.reverse();
            graph.links.reverse();
            fs::write(&graph_path, serde_json::to_vec(&graph)?)?;
        }
        let store = SqliteStore::open(directory.path().join(format!("store-{reverse}.db")))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        for engine in [
            open(&graph_path, None, &directory.path().join("cache"))?,
            open_with_store(
                &store,
                &graph_path,
                None,
                &directory.path().join("store-cache"),
            )?,
        ] {
            for depth in [1, 2, 3] {
                let result = engine.node_trail(NodeTrailRequest {
                    calls_only: false,
                    source: "n:s".into(),
                    target: "n:t".into(),
                    include_heuristic: false,
                    limits: CodeQueryLimits {
                        max_depth: depth,
                        ..CodeQueryLimits::default()
                    },
                })?;
                if depth < 3 {
                    assert!(result.paths.is_empty());
                    assert!(result.truncated, "{result:?}");
                    assert!(
                        result
                            .diagnostics
                            .iter()
                            .any(|d| d.code == QueryDiagnosticCode::BoundedTruncation)
                    );
                    assert!(!result.diagnostics.iter().any(|d| matches!(
                        d.code,
                        QueryDiagnosticCode::DirectionMismatch | QueryDiagnosticCode::NoMatch
                    )));
                } else {
                    assert!(!result.truncated);
                    assert_eq!(result.paths.len(), 1);
                    assert_eq!(result.paths[0].node_ids, ["n:s", "n:a", "n:b", "n:t"]);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn node_trail_depth_frontier_proves_closed_dead_ends_and_cycles()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    for cycle in [false, true] {
        let path = directory.path().join(format!("closed-{cycle}.json"));
        let mut edges = vec![
            ("n:s", EdgeKind::Calls, "n:a"),
            ("n:a", EdgeKind::Calls, "n:b"),
        ];
        if cycle {
            edges.push(("n:b", EdgeKind::Calls, "n:a"));
        }
        write_weighted_trail_fixture(&path, &edges)?;
        let graph = GraphDocument::load(&path)?;
        let store = SqliteStore::open(directory.path().join(format!("closed-{cycle}.db")))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        for engine in [
            open(&path, None, &directory.path().join("cache"))?,
            open_with_store(&store, &path, None, &directory.path().join("store-cache"))?,
        ] {
            let response = engine.node_trail(NodeTrailRequest {
                calls_only: false,
                source: "n:s".into(),
                target: "n:t".into(),
                include_heuristic: false,
                limits: CodeQueryLimits {
                    max_depth: 2,
                    ..CodeQueryLimits::default()
                },
            })?;
            assert!(!response.truncated, "{response:?}");
            assert!(response.paths.is_empty());
            assert!(
                response
                    .diagnostics
                    .iter()
                    .any(|d| d.code == QueryDiagnosticCode::NoMatch)
            );
        }
    }
    Ok(())
}

#[test]
fn explore_keeps_depth_exhaustion_when_no_connecting_path_is_returned()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    write_weighted_trail_fixture(
        &path,
        &[
            ("n:s", EdgeKind::Calls, "n:a"),
            ("n:a", EdgeKind::Calls, "n:b"),
            ("n:b", EdgeKind::Calls, "n:t"),
        ],
    )?;
    let graph = GraphDocument::load(&path)?;
    let store = SqliteStore::open(directory.path().join("store.db"))?;
    let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    for engine in [
        open(&path, None, &directory.path().join("cache"))?,
        open_with_store(&store, &path, None, &directory.path().join("store-cache"))?,
    ] {
        let response = engine.explore(compass_model::query_contract::ExploreRequest {
            symbols: vec!["n:s".into(), "n:t".into()],
            root: directory.path().to_string_lossy().into_owned(),
            include_heuristic: false,
            limits: CodeQueryLimits {
                max_depth: 2,
                ..CodeQueryLimits::default()
            },
        })?;
        assert!(response.paths.is_empty());
        assert!(response.truncated, "{response:?}");
    }
    Ok(())
}

#[test]
fn calls_only_trails_preserve_bounds_occurrences_and_backend_parity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    write_weighted_trail_fixture(
        &path,
        &[
            ("n:s", EdgeKind::Contains, "n:t"),
            ("n:s", EdgeKind::Calls, "n:a"),
            ("n:a", EdgeKind::Calls, "n:b"),
            ("n:b", EdgeKind::Calls, "n:t"),
            ("n:a", EdgeKind::Calls, "n:s"),
        ],
    )?;
    let mut graph = GraphDocument::load(&path)?;
    let mut parallel = graph
        .links
        .iter()
        .find(|e| e.source == "n:s" && e.kind == EdgeKind::Calls)
        .cloned()
        .ok_or("missing call")?;
    let mut anchor = parallel.relationship_site.clone().ok_or("missing site")?;
    anchor.start_column += 1;
    anchor.start_byte += 1;
    parallel.relationship_site = Some(anchor.clone());
    parallel.id = edge_id(
        &parallel.source,
        parallel.kind,
        &parallel.target,
        Some(&anchor),
        None,
    );
    parallel.key.clone_from(&parallel.id);
    for evidence in &mut parallel.evidence {
        evidence.anchors = vec![anchor.clone()];
    }
    graph.links.push(parallel);
    let mut canonical = None;
    for reverse in [false, true] {
        if reverse {
            graph.nodes.reverse();
            graph.links.reverse();
        }
        fs::write(&path, serde_json::to_vec(&graph)?)?;
        let store = SqliteStore::open(directory.path().join(format!("calls-{reverse}.db")))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        for engine in [
            open(&path, None, &directory.path().join("cache"))?,
            open_with_store(&store, &path, None, &directory.path().join("store-cache"))?,
        ] {
            let request = NodeTrailRequest {
                source: "n:s".into(),
                target: "n:t".into(),
                calls_only: true,
                include_heuristic: false,
                limits: CodeQueryLimits {
                    max_depth: 3,
                    ..CodeQueryLimits::default()
                },
            };
            let structural = engine.node_trail(NodeTrailRequest {
                calls_only: false,
                ..request.clone()
            })?;
            assert_eq!(structural.paths[0].node_ids, ["n:s", "n:t"]);
            let result = engine.node_trail(request.clone())?;
            assert!(!result.truncated);
            assert_eq!(result.paths[0].node_ids, ["n:s", "n:a", "n:b", "n:t"]);
            assert!(result.edges.iter().all(|e| e.kind == EdgeKind::Calls));
            assert!(
                result.edges.iter().all(|e| graph
                    .links
                    .iter()
                    .any(|stored| stored.id == e.id
                        && stored.relationship_site == e.relationship_site))
            );
            assert!(
                engine
                    .node_trail(NodeTrailRequest {
                        limits: CodeQueryLimits {
                            max_response_bytes: 1,
                            ..request.limits.clone()
                        },
                        ..request.clone()
                    })
                    .is_err()
            );
            let bytes = serde_json::to_vec(&result)?;
            if let Some(expected) = &canonical {
                assert_eq!(expected, &bytes);
            } else {
                canonical = Some(bytes);
            }
            for limits in [
                CodeQueryLimits {
                    max_depth: 2,
                    ..request.limits.clone()
                },
                CodeQueryLimits {
                    max_edges: 1,
                    ..request.limits.clone()
                },
                CodeQueryLimits {
                    max_nodes: 2,
                    ..request.limits.clone()
                },
            ] {
                let result = engine.node_trail(NodeTrailRequest {
                    limits,
                    ..request.clone()
                })?;
                assert!(result.truncated && result.paths.is_empty(), "{result:?}");
                assert!(!result.diagnostics.iter().any(|d| matches!(
                    d.code,
                    QueryDiagnosticCode::NoMatch | QueryDiagnosticCode::DirectionMismatch
                )));
            }
            for question in ["call path from n:s to n:t", "call chain from n:s to n:t"] {
                let asked = engine.query_natural(compass_query::NaturalQueryRequest {
                    question: question.into(),
                    include_heuristic: false,
                    limits: request.limits.clone(),
                })?;
                assert_eq!(serde_json::to_vec(&asked)?, serde_json::to_vec(&result)?);
            }
        }
    }
    Ok(())
}

#[test]
fn calls_only_negative_diagnostics_ignore_structural_frontiers_and_reverse_shortcuts()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    for (name, edges, expected) in [
        (
            "structural-reverse",
            vec![("n:t", EdgeKind::Contains, "n:s")],
            QueryDiagnosticCode::NoMatch,
        ),
        (
            "call-reverse",
            vec![("n:t", EdgeKind::Calls, "n:s")],
            QueryDiagnosticCode::DirectionMismatch,
        ),
        (
            "closed-call-cycle",
            vec![
                ("n:s", EdgeKind::Calls, "n:a"),
                ("n:a", EdgeKind::Calls, "n:s"),
                ("n:a", EdgeKind::Contains, "n:t"),
            ],
            QueryDiagnosticCode::NoMatch,
        ),
    ] {
        let path = directory.path().join(format!("{name}.json"));
        write_weighted_trail_fixture(&path, &edges)?;
        let graph = GraphDocument::load(&path)?;
        let store = SqliteStore::open(directory.path().join(format!("{name}.db")))?;
        let prepared = GraphSnapshotBuilder::new().prepare(&store, &graph)?;
        GraphSnapshotBuilder::new().activate(&store, &prepared)?;
        for engine in [
            open(&path, None, &directory.path().join("cache"))?,
            open_with_store(&store, &path, None, &directory.path().join("store-cache"))?,
        ] {
            let result = engine.node_trail(NodeTrailRequest {
                calls_only: true,
                source: "n:s".into(),
                target: "n:t".into(),
                include_heuristic: false,
                limits: CodeQueryLimits {
                    max_depth: 1,
                    ..CodeQueryLimits::default()
                },
            })?;
            assert!(
                !result.truncated && result.paths.is_empty(),
                "{name}: {result:?}"
            );
            assert!(
                result.diagnostics.iter().any(|d| d.code == expected),
                "{name}: {result:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn calls_only_retains_heuristic_opt_in_and_exact_name_ambiguity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let request = NodeTrailRequest {
        source: "n:heuristic".into(),
        target: "n:list".into(),
        calls_only: true,
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    };
    assert!(engine.node_trail(request.clone())?.paths.is_empty());
    let enriched = engine.node_trail(NodeTrailRequest {
        include_heuristic: true,
        ..request
    })?;
    assert_eq!(enriched.paths.len(), 1);
    assert!(enriched.edges.iter().all(|e| e.kind == EdgeKind::Calls));
    let mut graph = GraphDocument::load(&path)?;
    let original = graph
        .nodes
        .iter()
        .find(|n| n.id == "n:list")
        .cloned()
        .ok_or("missing node")?;
    let mut duplicate = original.clone();
    duplicate.id = "n:duplicate".into();
    graph.nodes.push(duplicate);
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let ambiguous = engine.node_trail(NodeTrailRequest {
        source: original.name,
        target: "n:callee".into(),
        calls_only: true,
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(ambiguous.paths.is_empty());
    assert!(
        ambiguous
            .diagnostics
            .iter()
            .any(|d| d.code == QueryDiagnosticCode::AmbiguousMatch)
    );
    Ok(())
}
#[test]
fn class_callees_include_calls_owned_by_its_methods() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    graph.nodes.push(support::node(
        "n:service",
        compass_model::code_graph::NodeKind::Class,
        "UserService",
        "UserService",
    ));
    let mut contains = graph.links.first().cloned().ok_or("edge template")?;
    contains.source = "n:service".to_owned();
    contains.target = "n:list".to_owned();
    contains.kind = EdgeKind::Contains;
    contains.id = edge_id(
        &contains.source,
        contains.kind,
        &contains.target,
        contains.relationship_site.as_ref(),
        None,
    );
    contains.key.clone_from(&contains.id);
    graph.links.push(contains);
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let answer = engine.callees(CallRequest {
        symbol: "n:service".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(
        answer.edges.iter().any(|edge| edge.kind == EdgeKind::Calls
            && edge.source == "n:list"
            && edge.target == "n:callee"),
        "class must not claim zero outgoing calls: {answer:#?}"
    );
    Ok(())
}

#[test]
fn python_callable_includes_initializer_calls_but_not_nested_callable_bodies()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::NodeKind;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    let owner = graph
        .nodes
        .iter_mut()
        .find(|node| node.id == "n:list")
        .ok_or("owner")?;
    owner.language = Some("python".to_owned());
    let template = graph.links.first().cloned().ok_or("edge template")?;
    for (id, kind) in [
        ("n:initializer", NodeKind::Variable),
        ("n:nested", NodeKind::Function),
    ] {
        let mut node = support::node(id, kind, id, id);
        node.language = Some("python".to_owned());
        graph.nodes.push(node);
        for (source, target, kind) in [
            ("n:list", id, EdgeKind::Contains),
            (id, "n:callee", EdgeKind::Calls),
        ] {
            let mut edge = template.clone();
            edge.source = source.to_owned();
            edge.target = target.to_owned();
            edge.kind = kind;
            edge.id = edge_id(source, kind, target, edge.relationship_site.as_ref(), None);
            edge.key.clone_from(&edge.id);
            graph.links.push(edge);
        }
    }
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let answer = engine.callees(CallRequest {
        symbol: "n:list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(
        answer
            .edges
            .iter()
            .any(|edge| edge.source == "n:initializer")
    );
    assert!(!answer.edges.iter().any(|edge| edge.source == "n:nested"));
    Ok(())
}
