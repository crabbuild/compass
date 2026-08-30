use std::fs;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{
    Extraction, FrameworkLimits, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin,
    RawFrameworkRoleFact, RawNodeRecord, RawRouteFact, RawRouteStageFact, RawRouteStageRole,
};
use compass_model::code_graph::{
    EdgeDetails, EdgeKind, NodeDetails, NodeKind, RouteStage, RouteStageDetails,
};
use compass_model::provenance::{EvidenceOrigin, ResolutionState, SourceAnchor};
use compass_resolve::frameworks::{
    FrameworkResolutionError, RouteStageRole, resolve_and_publish_framework_domains,
    resolve_and_publish_framework_routes, resolve_routes,
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

fn import_alias(id: &str, source_file: &str, local_name: &str) -> RawNodeRecord {
    RawNodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
            ("label".into(), Value::String(local_name.to_owned())),
            ("symbol_kind".into(), Value::String("import".into())),
            ("file_type".into(), Value::String("code".into())),
            ("source_file".into(), Value::String(source_file.to_owned())),
            ("local_name".into(), Value::String(local_name.to_owned())),
            ("module".into(), Value::String("./handler".into())),
            ("imported_name".into(), Value::String("handler".into())),
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
        stages: Vec::new(),
        origin: RawFrameworkOrigin::Ast,
        rule: None,
        detail: Map::new(),
    }
}

#[test]
fn neutral_framework_roles_publish_existing_node_roles_and_reject_unknown_values()
-> Result<(), Box<dyn std::error::Error>> {
    let anchor = route("service").anchor;
    let role = |role: &str| {
        RawFrameworkFact::Role(RawFrameworkRoleFact {
            pack_id: "python-test".to_owned(),
            framework: "synthetic".to_owned(),
            role: role.to_owned(),
            subject_reference: Some("service".to_owned()),
            context: Some("app".to_owned()),
            anchor: anchor.clone(),
            origin: RawFrameworkOrigin::Ast,
            evidence_class: "exact".to_owned(),
            detail: Map::new(),
        })
    };
    let mut extraction = Extraction {
        nodes: vec![node("service", "Service", "app.Service")],
        framework_facts: vec![role("service"), role("not_a_node_role")],
        ..Extraction::default()
    };
    let resolved =
        resolve_and_publish_framework_domains(&mut extraction, FrameworkLimits::default())?;
    assert_eq!(resolved.len(), 2);
    let service = extraction
        .nodes
        .iter()
        .find(|candidate| candidate.id == "service")
        .ok_or("missing service node")?;
    assert_eq!(
        service.attributes.get("roles"),
        Some(&Value::Array(vec![Value::String("service".to_owned())]))
    );
    assert!(
        extraction.extensions["framework_domain_diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics
                .iter()
                .any(|diagnostic| { diagnostic["kind"] == "invalid_framework_role" }))
    );
    Ok(())
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
            .map(|stage| (stage.position, stage.role, stage.target.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            (0, RouteStageRole::Middleware, Some("auth")),
            (1, RouteStageRole::Middleware, Some("authorize")),
            (2, RouteStageRole::Handler, Some("handler")),
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
fn dependency_and_security_stages_round_trip_without_becoming_http_middleware()
-> Result<(), Box<dyn std::error::Error>> {
    let mut route = route("handler");
    route.stages = [
        (RawRouteStageRole::Dependency, "provide_session"),
        (RawRouteStageRole::Dependency, "provide_session"),
        (RawRouteStageRole::Security, "require_user"),
        (RawRouteStageRole::Handler, "handler"),
    ]
    .into_iter()
    .enumerate()
    .map(|(position, (role, reference))| RawRouteStageFact {
        role,
        position: u32::try_from(position).unwrap_or_else(|_| std::process::abort()),
        reference: reference.to_owned(),
        anchor: route.anchor.clone(),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::new(),
    })
    .collect();
    let mut extraction = Extraction {
        nodes: vec![
            node(
                "dependency",
                "provide_session",
                "app.routes.provide_session",
            ),
            node("security", "require_user", "app.routes.require_user"),
            node("handler", "handler", "app.routes.handler"),
        ],
        framework_facts: vec![RawFrameworkFact::Route(route)],
        ..Extraction::default()
    };
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert_eq!(
        resolved[0]
            .stages
            .iter()
            .map(|stage| stage.role)
            .collect::<Vec<_>>(),
        vec![
            RouteStageRole::Dependency,
            RouteStageRole::Dependency,
            RouteStageRole::Security,
            RouteStageRole::Handler,
        ]
    );
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| edge.string("stage") == "dependency")
            .map(|edge| edge.string("position"))
            .collect::<Vec<_>>(),
        vec!["0", "1"],
        "repeated providers at distinct stage positions retain multiplicity"
    );
    let dependency = extraction
        .nodes
        .iter()
        .find(|candidate| candidate.id == "dependency")
        .ok_or("missing dependency node")?;
    assert_eq!(
        dependency.attributes["roles"],
        Value::Array(vec![Value::String("service".to_owned())])
    );
    let security = extraction
        .nodes
        .iter()
        .find(|candidate| candidate.id == "security")
        .ok_or("missing security node")?;
    assert_eq!(
        security.attributes["roles"],
        Value::Array(vec![Value::String("middleware".to_owned())])
    );
    let directory = tempfile::tempdir()?;
    let source_path = directory.path().join("src/routes.rs");
    fs::create_dir_all(source_path.parent().ok_or("missing source parent")?)?;
    fs::write(&source_path, vec![b'x'; 100])?;
    let build = BuildEvidence::from_extraction(
        directory.path(),
        &extraction,
        "sha256:dependency-security-stages",
    )?;
    let graph = normalize_v1(extraction, build)?;
    let mut dependency_positions = graph
        .links
        .iter()
        .filter_map(|edge| match edge.details.as_ref() {
            Some(EdgeDetails::Route(details)) if details.stage == RouteStage::Dependency => {
                details.position
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    dependency_positions.sort_unstable();
    assert_eq!(dependency_positions, vec![0, 1]);
    let route = graph
        .nodes
        .iter()
        .find_map(|node| match node.details.as_ref() {
            Some(NodeDetails::Route(route)) => Some(route),
            _ => None,
        })
        .ok_or("missing route details")?;
    assert_eq!(route.middleware_count, 0);
    assert_eq!(
        route
            .stages
            .iter()
            .map(|stage| stage.stage)
            .collect::<Vec<_>>(),
        vec![
            RouteStage::Dependency,
            RouteStage::Dependency,
            RouteStage::Security,
            RouteStage::Handler
        ]
    );
    Ok(())
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
    assert_eq!(ambiguous.stages.len(), 1);
    assert_eq!(ambiguous.stages[0].state, ResolutionState::Ambiguous);
    assert!(ambiguous.stages[0].target.is_none());
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
fn ambiguous_handlers_do_not_taint_exact_middleware_edges() {
    let mut ambiguous = route("show_user");
    ambiguous.middleware_references = vec!["authenticate".into()];
    let mut extraction = Extraction {
        nodes: vec![
            node("middleware", "authenticate", "app.routes.authenticate"),
            node("handler-a", "show_user", "app.show_user"),
            node("handler-b", "show_user", "other.show_user"),
        ],
        framework_facts: vec![RawFrameworkFact::Route(ambiguous)],
        ..Extraction::default()
    };

    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())
            .unwrap_or_else(|_| std::process::abort());

    assert_eq!(resolved[0].state, ResolutionState::Ambiguous);
    let route_edges = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "routes_to")
        .collect::<Vec<_>>();
    assert_eq!(route_edges.len(), 1);
    assert_eq!(route_edges[0].target, "middleware");
    assert_eq!(route_edges[0].string("confidence"), "EXTRACTED");
}

#[test]
fn every_declared_stage_retains_resolution_without_publishing_ambiguous_edges() {
    let mut declared = route("show_user");
    declared.middleware_references = vec!["authenticate".into(), "audit".into(), "missing".into()];
    let mut audit = node("audit-node", "audit", "other.audit");
    audit.attributes.insert(
        "source_file".to_owned(),
        Value::String("other/routes.rs".to_owned()),
    );
    let mut extraction = Extraction {
        nodes: vec![
            node("authenticate", "authenticate", "app.routes.authenticate"),
            audit,
            node("handler", "show_user", "app.routes.show_user"),
        ],
        framework_facts: vec![RawFrameworkFact::Route(declared)],
        ..Extraction::default()
    };

    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())
            .unwrap_or_else(|_| std::process::abort());
    let stages = &resolved[0].stages;
    assert_eq!(stages.len(), 4);
    assert_eq!(
        stages
            .iter()
            .map(|stage| (
                stage.position,
                stage.reference.as_str(),
                stage.state,
                stage.target.as_deref(),
                stage.candidates.len(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                0,
                "authenticate",
                ResolutionState::Exact,
                Some("authenticate"),
                1,
            ),
            (1, "audit", ResolutionState::Unresolved, None, 1),
            (2, "missing", ResolutionState::Unresolved, None, 0),
            (3, "show_user", ResolutionState::Exact, Some("handler"), 1,),
        ]
    );
    assert_eq!(
        stages[1].provenance.confidence,
        compass_model::provenance::EvidenceConfidence::Inferred
    );
    let route_edges = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "routes_to")
        .collect::<Vec<_>>();
    assert_eq!(route_edges.len(), 2);
    assert!(
        route_edges
            .iter()
            .all(|edge| edge.target != "audit" && edge.target != "missing")
    );
}

#[test]
fn conflicting_route_registrations_at_distinct_sites_are_retained() {
    let first = route("first_handler");
    let duplicate = first.clone();
    let mut second = route("second_handler");
    second.anchor.start_byte = 80;
    second.anchor.end_byte = 112;
    second.anchor.start_line = 5;
    second.anchor.end_line = 5;
    let mut extraction = Extraction {
        nodes: vec![
            node("first-handler", "first_handler", "app.routes.first_handler"),
            node(
                "second-handler",
                "second_handler",
                "app.routes.second_handler",
            ),
        ],
        framework_facts: vec![
            RawFrameworkFact::Route(first),
            RawFrameworkFact::Route(duplicate),
            RawFrameworkFact::Route(second),
        ],
        ..Extraction::default()
    };

    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())
            .unwrap_or_else(|_| std::process::abort());
    assert_eq!(resolved.len(), 2);
    let route_nodes = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "route")
        .collect::<Vec<_>>();
    assert_eq!(route_nodes.len(), 2);
    assert_ne!(route_nodes[0].id, route_nodes[1].id);

    for route_node in route_nodes {
        let anchor = route_node
            .attributes
            .get("source_anchor")
            .cloned()
            .and_then(|value| serde_json::from_value::<SourceAnchor>(value).ok())
            .unwrap_or_else(|| std::process::abort());
        let stages = route_node
            .attributes
            .get("stages")
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<RouteStageDetails>>(value).ok())
            .unwrap_or_else(|| std::process::abort());
        let [stage] = stages.as_slice() else {
            std::process::abort();
        };
        let (expected_reference, expected_target, expected_line) =
            if stage.reference == "first_handler" {
                ("first_handler", "first-handler", 2)
            } else {
                ("second_handler", "second-handler", 5)
            };
        assert_eq!(anchor.start_line, expected_line);
        assert_eq!(stage.reference, expected_reference);
        assert_eq!(stage.resolution, ResolutionState::Exact);
        assert_eq!(stage.target.as_deref(), Some(expected_target));
        assert_eq!(stage.candidates.len(), 1);
        assert_eq!(stage.candidates[0].node_id, expected_target);

        let edges = extraction
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "routes_to" && edge.source == route_node.id)
            .collect::<Vec<_>>();
        let [edge] = edges.as_slice() else {
            std::process::abort();
        };
        assert_eq!(edge.target, expected_target);
        assert_eq!(edge.string("confidence"), "EXTRACTED");
        let edge_anchor = edge
            .attributes
            .get("source_anchor")
            .cloned()
            .and_then(|value| serde_json::from_value::<SourceAnchor>(value).ok())
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(edge_anchor, anchor);
    }
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
fn candidate_limits_publish_deterministically_bounded_ambiguity() {
    let nodes = (0..21)
        .map(|index| node(&format!("handler-{index:02}"), "show_user", "show_user"))
        .collect();
    let mut extraction = Extraction {
        nodes,
        framework_facts: vec![RawFrameworkFact::Route(route("show_user"))],
        ..Extraction::default()
    };
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())
            .unwrap_or_else(|_| std::process::abort());
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].state, ResolutionState::Ambiguous);
    assert_eq!(resolved[0].candidates.len(), 20);
    assert_eq!(resolved[0].candidates[0].node_id, "handler-00");
    assert_eq!(resolved[0].candidates[19].node_id, "handler-19");
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "route")
            .count(),
        1
    );
    assert!(
        extraction
            .edges
            .iter()
            .all(|edge| edge.string("relation") != "routes_to"),
        "ambiguous candidates must not publish an exact route edge"
    );
}

#[test]
fn fact_limits_fail_without_partial_publication() {
    let extraction = Extraction {
        nodes: vec![node("handler", "show_user", "show_user")],
        framework_facts: vec![RawFrameworkFact::Route(route("show_user"))],
        ..Extraction::default()
    };
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
fn import_alias_limit_is_enforced_per_source_file() {
    let limits = FrameworkLimits {
        max_alias_expansions: 1,
        ..FrameworkLimits::default()
    };
    let mut extraction = Extraction {
        nodes: vec![
            node("handler", "show_user", "app.routes.show_user"),
            import_alias("first", "src/first.ts", "first"),
            import_alias("second", "src/second.ts", "second"),
        ],
        framework_facts: vec![RawFrameworkFact::Route(route("show_user"))],
        ..Extraction::default()
    };
    assert!(
        resolve_routes(&extraction, limits).is_ok(),
        "aliases in independent source files do not share an expansion budget"
    );

    extraction
        .nodes
        .push(import_alias("third", "src/first.ts", "third"));
    assert!(matches!(
        resolve_routes(&extraction, limits),
        Err(FrameworkResolutionError::AliasLimit {
            observed: 2,
            maximum: 1
        })
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

#[test]
fn single_low_confidence_route_candidate_normalizes_as_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/routes.rs"), vec![b'x'; 256])?;
    fs::write(root.join("src/handlers.rs"), vec![b'x'; 256])?;
    let mut candidate = node("handler", "show_user", "other.show_user");
    candidate.attributes.insert(
        "source_file".into(),
        Value::String("src/handlers.rs".into()),
    );
    let mut extraction = Extraction {
        nodes: vec![candidate],
        framework_facts: vec![RawFrameworkFact::Route(route("show_user"))],
        ..Extraction::default()
    };
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert_eq!(resolved[0].state, ResolutionState::Unresolved);
    assert_eq!(resolved[0].candidates.len(), 1);

    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test")?;
    let graph = normalize_v1(extraction, evidence)?;
    let published = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Route)
        .ok_or("missing typed route")?;
    assert!(matches!(
        published.details,
        Some(NodeDetails::Route(ref details))
            if details.resolution == ResolutionState::Unresolved
    ));
    Ok(())
}

#[test]
fn sourceless_route_candidates_remain_anchorless_and_publishable()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/routes.rs"), vec![b'x'; 256])?;
    let mut candidate = node("external", "show_user", "external.show_user");
    candidate
        .attributes
        .insert("source_file".into(), Value::String(String::new()));
    let mut extraction = Extraction {
        nodes: vec![candidate],
        framework_facts: vec![RawFrameworkFact::Route(route("show_user"))],
        ..Extraction::default()
    };
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert_eq!(resolved[0].candidates.len(), 1);
    assert!(resolved[0].candidates[0].anchor.is_none());
    extraction.nodes.retain(|node| node.id != "external");

    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test")?;
    let graph = normalize_v1(extraction, evidence)?;
    assert!(graph.nodes.iter().any(|node| node.kind == NodeKind::Route));
    Ok(())
}

#[test]
fn incremental_resolution_replaces_the_changed_handler_without_stale_targets() {
    let mut first = route("first_handler");
    first.normalized_path = "/incremental".to_owned();
    first.raw_path = "/incremental".to_owned();
    let base = Extraction {
        nodes: vec![
            node("first", "first_handler", "app.routes.first_handler"),
            node("second", "second_handler", "app.routes.second_handler"),
        ],
        framework_facts: vec![RawFrameworkFact::Route(first.clone())],
        ..Extraction::default()
    };
    let initial =
        resolve_routes(&base, FrameworkLimits::default()).unwrap_or_else(|_| std::process::abort());
    assert_eq!(initial[0].stages[0].target.as_deref(), Some("first"));

    first.handler_reference = "second_handler".to_owned();
    let changed = Extraction {
        framework_facts: vec![RawFrameworkFact::Route(first)],
        ..base
    };
    let refreshed = resolve_routes(&changed, FrameworkLimits::default())
        .unwrap_or_else(|_| std::process::abort());
    assert_eq!(refreshed[0].stages[0].target.as_deref(), Some("second"));
    assert!(
        refreshed[0]
            .stages
            .iter()
            .all(|stage| stage.target.as_deref() != Some("first"))
    );
}
