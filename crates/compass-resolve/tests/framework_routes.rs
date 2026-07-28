use std::collections::HashSet;
use std::fs;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{
    Extraction, FrameworkLimits, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin,
    RawNodeRecord, RawRouteFact,
};
use compass_model::code_graph::{EdgeDetails, EdgeKind, NodeKind, RouteStage};
use compass_model::provenance::{EvidenceOrigin, ResolutionState};
use compass_resolve::frameworks::{
    FrameworkResolutionError, RouteStageRole, resolve_and_publish_framework_routes, resolve_routes,
};
use serde_json::{Map, Value};

fn node(id: &str, name: &str, qualified_name: &str) -> RawNodeRecord {
    RawNodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
            ("label".into(), Value::String(name.to_owned())),
            ("name".into(), Value::String(name.to_owned())),
            (
                "qualified_name".into(),
                Value::String(qualified_name.to_owned()),
            ),
            ("symbol_kind".into(), Value::String("function".into())),
            ("file_type".into(), Value::String("code".into())),
            ("source_file".into(), Value::String("src/routes.rs".into())),
            ("line_start".into(), Value::from(1)),
            ("line_end".into(), Value::from(1)),
        ]),
    }
}

fn route(handler: &str) -> RawRouteFact {
    RawRouteFact {
        framework: "synthetic".to_owned(),
        operation: "GET".to_owned(),
        raw_path: "/users/:id".to_owned(),
        normalized_path: "/users/{id}".to_owned(),
        declaring_scope: "app.routes".to_owned(),
        anchor: RawFrameworkAnchor {
            source_file: "src/routes.rs".to_owned(),
            start_byte: 20,
            end_byte: 52,
            start_line: 2,
            start_column: 0,
            end_line: 2,
            end_column: 32,
        },
        handler_reference: handler.to_owned(),
        middleware_references: Vec::new(),
        origin: RawFrameworkOrigin::Ast,
        rule: None,
        detail: Map::new(),
    }
}

#[test]
fn exact_routes_publish_ordered_middleware_and_handler_stages() {
    let mut route = route("show_user");
    route.middleware_references = vec!["authenticate".into(), "authorize".into()];
    let mut extraction = Extraction {
        nodes: vec![
            node("auth", "authenticate", "app.routes.authenticate"),
            node("authorize", "authorize", "app.routes.authorize"),
            node("handler", "show_user", "app.routes.show_user"),
        ],
        framework_facts: vec![RawFrameworkFact::Route(route.clone())],
        ..Extraction::default()
    };
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())
            .unwrap_or_else(|_| std::process::abort());

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].state, ResolutionState::Exact);
    assert_eq!(
        resolved[0]
            .stages
            .iter()
            .map(|stage| (stage.position, stage.role, stage.target.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, RouteStageRole::Middleware, "auth"),
            (1, RouteStageRole::Middleware, "authorize"),
            (2, RouteStageRole::Handler, "handler"),
        ]
    );
    let route_nodes = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "route")
        .count();
    assert_eq!(route_nodes, 1);
    let route_edges = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "routes_to")
        .collect::<Vec<_>>();
    assert_eq!(route_edges.len(), 3);
    assert_eq!(route_edges[0].string("stage"), "middleware");
    assert_eq!(route_edges[2].string("stage"), "handler");
}

#[test]
fn ambiguous_unresolved_duplicate_and_near_match_routes_are_conservative() {
    let duplicate = route("show_user");
    let mut unresolved = route("missing_handler");
    unresolved.raw_path = "/missing".to_owned();
    unresolved.normalized_path = "/missing".to_owned();
    unresolved.anchor.start_byte = 60;
    unresolved.anchor.end_byte = 84;
    unresolved.anchor.start_line = 3;
    unresolved.anchor.end_line = 3;
    let extraction = Extraction {
        nodes: vec![
            node("handler-b", "show_user", "other.show_user"),
            node("handler-a", "show_user", "app.show_user"),
            node("url-string", "/users/:id", "ordinary.constant"),
        ],
        framework_facts: vec![
            RawFrameworkFact::Route(duplicate.clone()),
            RawFrameworkFact::Route(duplicate),
            RawFrameworkFact::Route(unresolved),
        ],
        ..Extraction::default()
    };
    let resolved = resolve_routes(&extraction, FrameworkLimits::default())
        .unwrap_or_else(|_| std::process::abort());
    assert_eq!(resolved.len(), 2, "duplicate registrations must coalesce");
    let ambiguous = resolved
        .iter()
        .find(|route| route.route.handler_reference == "show_user")
        .unwrap_or_else(|| std::process::abort());
    assert_eq!(ambiguous.state, ResolutionState::Ambiguous);
    assert_eq!(
        ambiguous
            .candidates
            .iter()
            .map(|candidate| candidate.node_id.as_str())
            .collect::<Vec<_>>(),
        vec!["handler-a", "handler-b"]
    );
    assert!(ambiguous.stages.is_empty());
    assert!(
        resolved
            .iter()
            .any(|route| route.state == ResolutionState::Unresolved)
    );

    let no_framework_facts = Extraction {
        nodes: vec![node("url-string", "/admin", "ordinary.constant")],
        ..Extraction::default()
    };
    assert!(
        resolve_routes(&no_framework_facts, FrameworkLimits::default())
            .unwrap_or_default()
            .is_empty(),
        "URL-looking symbols are not routes"
    );
}

#[test]
fn heuristic_routes_surface_the_wiring_site_and_rule() {
    let mut heuristic = route("show_user");
    heuristic.origin = RawFrameworkOrigin::Heuristic;
    heuristic.rule = Some("dynamic-router-registration".to_owned());
    let extraction = Extraction {
        nodes: vec![node("handler", "show_user", "app.routes.show_user")],
        framework_facts: vec![RawFrameworkFact::Route(heuristic)],
        ..Extraction::default()
    };
    let resolved = resolve_routes(&extraction, FrameworkLimits::default())
        .unwrap_or_else(|_| std::process::abort());
    let evidence = &resolved[0].stages[0].provenance;
    assert_eq!(evidence.origin, EvidenceOrigin::Heuristic);
    assert_eq!(
        evidence.rule.as_deref(),
        Some("dynamic-router-registration")
    );
    assert!(evidence.wiring_site.is_some());
    assert!(evidence.anchors.is_empty());
}

#[test]
fn candidate_and_fact_limits_fail_without_partial_publication() {
    let nodes = (0..21)
        .map(|index| node(&format!("handler-{index:02}"), "show_user", "show_user"))
        .collect();
    let mut extraction = Extraction {
        nodes,
        framework_facts: vec![RawFrameworkFact::Route(route("show_user"))],
        ..Extraction::default()
    };
    let original_ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let error =
        match resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default()) {
            Ok(_) => std::process::abort(),
            Err(error) => error,
        };
    assert!(matches!(
        error,
        FrameworkResolutionError::Limit(error) if error.limit == "max_candidates"
    ));
    assert_eq!(
        extraction
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<HashSet<_>>(),
        original_ids
    );

    let limits = FrameworkLimits {
        max_facts_per_file: 0,
        ..FrameworkLimits::default()
    };
    assert!(matches!(
        resolve_routes(&extraction, limits),
        Err(FrameworkResolutionError::Limit(error))
            if error.limit == "max_facts_per_file"
    ));
}

#[test]
fn synthetic_framework_routes_normalize_to_the_shared_typed_shape()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/routes.rs"), vec![b'x'; 256])?;
    let mut extraction = Extraction {
        nodes: vec![node("handler", "show_user", "app.routes.show_user")],
        framework_facts: vec![RawFrameworkFact::Route(route("show_user"))],
        ..Extraction::default()
    };
    resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test")?;
    let graph = normalize_v1(extraction, evidence)?;

    let route = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Route)
        .ok_or("missing typed route")?;
    assert_eq!(route.framework.as_deref(), Some("synthetic"));
    let binding = graph
        .links
        .iter()
        .find(|edge| edge.kind == EdgeKind::RoutesTo)
        .ok_or("missing typed route binding")?;
    assert_eq!(binding.source, route.id);
    assert!(matches!(
        binding.details,
        Some(EdgeDetails::Route(ref details))
            if details.stage == RouteStage::Handler && details.position == Some(0)
    ));
    Ok(())
}
