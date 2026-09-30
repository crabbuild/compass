mod support;

use std::fs;

use compass_model::code_graph::{EdgeKind, GraphDocument};
use compass_model::code_graph::{EdgeRecord, NodeKind};
use compass_model::identity::edge_id;
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};
use compass_model::query_contract::{CodeQueryLimits, ImpactRequest};
use compass_query::open;

fn site() -> SourceAnchor {
    SourceAnchor {
        file: "src/lib.rs".to_owned(),
        start_byte: 0,
        end_byte: 4,
        start_line: 1,
        start_column: 0,
        end_line: 1,
        end_column: 4,
    }
}

fn owner_level_edge(source: &str, kind: EdgeKind) -> EdgeRecord {
    edge(source, kind, "n:callee")
}

fn edge(source: &str, kind: EdgeKind, target: &str) -> EdgeRecord {
    let anchor = site();
    let id = edge_id(source, kind, target, Some(&anchor), None);
    EdgeRecord {
        id: id.clone(),
        key: id,
        source: source.to_owned(),
        target: target.to_owned(),
        kind,
        occurrence_rule: None,
        relationship_site: Some(anchor.clone()),
        details: None,
        evidence: vec![Provenance {
            origin: EvidenceOrigin::Ast,
            extractor: "test".to_owned(),
            confidence: EvidenceConfidence::Exact,
            rule: None,
            anchors: vec![anchor],
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        }],
        weight: None,
        context: None,
        deferred: false,
        diagnostics: Vec::new(),
    }
}

#[test]
fn impact_retains_the_direct_caller_trail_ahead_of_calls_to_the_owning_module()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    // Drop the fixture's stronger direct call so the only competition left is
    // between two same-strength reference edges: one that names the target and
    // one that only reaches its containing owner.
    graph.links.retain(|edge| {
        !(edge.source == "n:list" && edge.target == "n:callee" && edge.kind == EdgeKind::Calls)
    });
    let direct = edge("n:dependent", EdgeKind::References, "n:callee");
    let direct_id = direct.id.clone();
    graph.links.push(direct);
    // `n:other` references the class that contains the target rather than
    // naming the target. The reverse walk resolves that owner spelling through
    // the containment chain, and owner-level evidence must not displace the
    // direct reference from the bounded trail ledger.
    graph
        .nodes
        .push(support::node("n:owner", NodeKind::Class, "Store", "Store"));
    graph
        .links
        .push(edge("n:owner", EdgeKind::Contains, "n:callee"));
    let owner_reference = [
        "n:listing",
        "n:other",
        "n:unicode",
        "n:resume",
        "n:snake",
        "n:camel",
        "n:caller",
        "n:heuristic",
    ]
    .into_iter()
    .map(|source| edge(source, EdgeKind::References, "n:owner"))
    .find(|candidate| candidate.id < direct_id)
    .ok_or("fixture needs an owner-level reference whose ID sorts before the direct one")?;
    let owner_source = owner_reference.source.clone();
    graph.links.push(owner_reference);
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let impact = engine.impact(ImpactRequest {
        symbol: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_paths: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    let trail = impact.paths.first().ok_or("expected a retained trail")?;
    assert!(
        impact.nodes.iter().any(|node| node.id == owner_source),
        "the owner-level reference must be resolved through the containment chain"
    );
    assert_eq!(
        trail.node_ids.last().map(String::as_str),
        Some("n:dependent")
    );
    Ok(())
}

#[test]
fn impact_retains_the_direct_caller_trail_ahead_of_owner_level_references()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let call_id = graph
        .links
        .iter()
        .find(|edge| {
            edge.source == "n:list" && edge.target == "n:callee" && edge.kind == EdgeKind::Calls
        })
        .map(|edge| edge.id.clone())
        .ok_or("fixture is missing the n:list -> n:callee call edge")?;
    // Owner-level reference and import edges carry the fixture's other nodes
    // into the reverse walk. Some of their deterministic IDs sort before the
    // call edge, so an unsorted traversal spends its whole trail ledger on
    // them and never records the caller that changing the symbol would break.
    let mut sorted_earlier = 0;
    for source in [
        "n:listing",
        "n:other",
        "n:unicode",
        "n:resume",
        "n:snake",
        "n:camel",
        "n:alias",
        "n:caller",
        "n:dependent",
        "n:heuristic",
    ] {
        let edge = owner_level_edge(source, EdgeKind::References);
        if edge.id < call_id {
            sorted_earlier += 1;
        }
        graph.links.push(edge);
    }
    assert!(
        sorted_earlier > 0,
        "fixture needs at least one reference edge that sorts before the call edge"
    );
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let impact = engine.impact(ImpactRequest {
        symbol: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_paths: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    let trail = impact.paths.first().ok_or("expected a retained trail")?;
    assert_eq!(trail.node_ids.last().map(String::as_str), Some("n:list"));
    assert!(impact.nodes.iter().any(|node| node.id == "n:list"));
    Ok(())
}

#[test]
fn impact_walks_the_approved_reverse_family_and_gates_heuristics()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = |include_heuristic| ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic,
        limits: CodeQueryLimits::default(),
    };
    let exact = engine.impact(request(false))?;
    assert!(exact.nodes.iter().any(|node| node.id == "n:route"));
    assert!(exact.nodes.iter().any(|node| node.id == "n:dependent"));
    assert!(exact.nodes.iter().any(|node| node.id == "n:alias"));
    assert!(
        exact
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Aliases)
    );
    assert!(!exact.nodes.iter().any(|node| node.id == "n:heuristic"));
    let enriched = engine.impact(request(true))?;
    assert!(enriched.nodes.iter().any(|node| node.id == "n:heuristic"));
    assert!(enriched.paths.iter().any(|path| {
        path.weakest_resolution == compass_model::provenance::ResolutionState::Unresolved
    }));
    Ok(())
}

#[test]
fn python_framework_dependency_and_schema_edges_reach_the_owning_route()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let dependency = graph
        .links
        .iter_mut()
        .find(|edge| edge.source == "n:list" && edge.target == "n:callee")
        .ok_or("missing dependency template")?;
    dependency.kind = EdgeKind::DependsOn;
    dependency.context = Some("request_model".to_owned());
    let id = edge_id(
        &dependency.source,
        dependency.kind,
        &dependency.target,
        dependency.relationship_site.as_ref(),
        dependency
            .occurrence_rule
            .as_ref()
            .map(compass_model::provenance::OccurrenceRule::as_str),
    );
    dependency.id.clone_from(&id);
    dependency.key = id;
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let impact = engine.impact(ImpactRequest {
        symbol: "Store.callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(impact.nodes.iter().any(|node| node.id == "n:list"));
    assert!(impact.nodes.iter().any(|node| node.id == "n:route"));
    assert!(
        impact
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::DependsOn)
    );
    assert!(
        impact
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::RoutesTo)
    );
    Ok(())
}

#[test]
fn django_drf_model_impact_reaches_serializer_viewset_action_and_url()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    for (id, name, qualified_name) in [
        ("n:list", "publish", "api.ItemViewSet.publish"),
        ("n:callee", "ItemSerializer", "api.ItemSerializer"),
        ("n:dependent", "Item", "api.models.Item"),
    ] {
        let node = graph
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .ok_or("missing DRF impact template node")?;
        node.name = name.to_owned();
        node.qualified_name = qualified_name.to_owned();
    }
    let viewset_serializer = graph
        .links
        .iter_mut()
        .find(|edge| edge.source == "n:list" && edge.target == "n:callee")
        .ok_or("missing viewset serializer template")?;
    viewset_serializer.kind = EdgeKind::DependsOn;
    viewset_serializer.context = Some("serializer_class".to_owned());
    let id = edge_id(
        &viewset_serializer.source,
        viewset_serializer.kind,
        &viewset_serializer.target,
        viewset_serializer.relationship_site.as_ref(),
        None,
    );
    viewset_serializer.id.clone_from(&id);
    viewset_serializer.key = id;

    let serializer_model = graph
        .links
        .iter_mut()
        .find(|edge| edge.source == "n:dependent" && edge.target == "n:caller")
        .ok_or("missing serializer model template")?;
    serializer_model.source = "n:callee".to_owned();
    serializer_model.target = "n:dependent".to_owned();
    serializer_model.kind = EdgeKind::DependsOn;
    serializer_model.context = Some("serializer_model".to_owned());
    let id = edge_id(
        &serializer_model.source,
        serializer_model.kind,
        &serializer_model.target,
        serializer_model.relationship_site.as_ref(),
        None,
    );
    serializer_model.id.clone_from(&id);
    serializer_model.key = id;
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let impact = engine.impact(ImpactRequest {
        symbol: "api.models.Item".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    for id in ["n:callee", "n:list", "n:route"] {
        assert!(
            impact.nodes.iter().any(|node| node.id == id),
            "missing {id}; impact={impact:#?}"
        );
    }
    assert!(impact.edges.iter().any(|edge| {
        edge.kind == EdgeKind::DependsOn
            && edge.source == "n:callee"
            && edge.target == "n:dependent"
    }));
    assert!(impact.edges.iter().any(|edge| {
        edge.kind == EdgeKind::DependsOn && edge.source == "n:list" && edge.target == "n:callee"
    }));
    assert!(
        impact
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::RoutesTo)
    );
    Ok(())
}

#[test]
fn impact_reports_bounds_as_typed_truncation() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.impact(ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: true,
        limits: CodeQueryLimits {
            max_nodes: 2,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(response.truncated);
    assert!(response.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == compass_model::query_contract::QueryDiagnosticCode::BoundedTruncation
    }));
    Ok(())
}

#[test]
fn impact_applies_edge_and_node_budgets_before_publishing_records()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.impact(ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: true,
        limits: CodeQueryLimits {
            max_nodes: 2,
            max_edges: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(response.truncated);
    assert!(response.nodes.len() <= 2);
    assert!(response.edges.len() <= 1);
    let node_ids = response
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(response.edges.iter().all(|edge| {
        node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str())
    }));
    Ok(())
}

#[test]
fn impact_excludes_graph_assembly_endpoint_remaps_by_default()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_endpoint_remap_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = |include_heuristic| ImpactRequest {
        symbol: "crate::Target".to_owned(),
        include_heuristic,
        limits: CodeQueryLimits::default(),
    };

    let exact = engine.impact(request(false))?;
    assert!(!exact.nodes.iter().any(|node| node.name == "Caller"));
    let enriched = engine.impact(request(true))?;
    assert!(enriched.nodes.iter().any(|node| node.name == "Caller"));
    assert!(enriched.edges.iter().any(|edge| {
        edge.evidence.iter().any(|evidence| {
            evidence.rule.as_deref() == Some("graph-ghost-endpoint-remap")
                && evidence.wiring_site.is_some()
        })
    }));
    Ok(())
}

#[test]
fn impact_includes_inbound_renderers_without_promoting_them_to_callers()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    let template = graph
        .links
        .iter()
        .find(|edge| edge.source == "n:caller" && edge.target == "n:list")
        .cloned()
        .ok_or("missing renderer template")?;
    let id = edge_id(
        "n:caller",
        EdgeKind::Renders,
        "n:list",
        template.relationship_site.as_ref(),
        Some("react-jsx-render"),
    );
    let mut render = template;
    render.id.clone_from(&id);
    render.key = id;
    render.kind = EdgeKind::Renders;
    render.occurrence_rule = compass_model::provenance::OccurrenceRule::new("react-jsx-render");
    graph.links.push(render);
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let impact = engine.impact(ImpactRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(
        impact
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Renders)
    );

    let callers = engine.callers(compass_model::query_contract::CallRequest {
        symbol: "UserService.list".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits::default(),
    })?;
    assert!(
        callers
            .edges
            .iter()
            .all(|edge| edge.kind != EdgeKind::Renders)
    );
    Ok(())
}

#[test]
fn partial_owner_coverage_does_not_stop_the_witnessed_direct_frontier()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    graph
        .nodes
        .push(support::node("n:owner", NodeKind::Class, "Store", "Store"));
    graph.nodes.push(support::node(
        "n:file",
        NodeKind::File,
        "lib.rs",
        "src/lib.rs",
    ));
    graph.links.extend([
        edge("n:file", EdgeKind::Contains, "n:owner"),
        edge("n:owner", EdgeKind::Contains, "n:callee"),
        // Three edges through the file and class exceed the requested depth.
        edge("n:dependent", EdgeKind::Imports, "n:file"),
    ]);
    fs::write(&path, serde_json::to_vec(&graph)?)?;
    let engine = open(&path, None, &directory.path().join("cache"))?;
    let response = engine.impact(ImpactRequest {
        symbol: "n:callee".to_owned(),
        include_heuristic: false,
        limits: CodeQueryLimits {
            max_depth: 2,
            ..CodeQueryLimits::default()
        },
    })?;
    assert!(
        response.truncated,
        "the excluded owner trail must stay disclosed"
    );
    assert!(
        response.nodes.iter().any(|node| node.id == "n:caller"),
        "the second-hop direct caller must remain reachable"
    );
    assert!(
        response
            .edges
            .iter()
            .any(|edge| edge.source == "n:caller" && edge.target == "n:list")
    );
    Ok(())
}
