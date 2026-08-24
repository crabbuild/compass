use std::fs;
use std::sync::Arc;

use compass_languages::{Engine, ProjectEvidenceIndex, RawFrameworkFact, RawRouteStageRole};
use tempfile::tempdir;

#[test]
fn tanstack_router_requires_import_identity_and_preserves_route_stages() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let path = root.join("src/routes/orders.tsx");
    let source = br#"import { createFileRoute as makeRoute } from '@tanstack/react-router';
export const Route = makeRoute('/orders')({
  component: Orders,
  loader: loadOrders,
  pendingComponent: Pending,
  errorComponent: ErrorView,
});
function Orders() { return null; }
function loadOrders() { return []; }
function Pending() { return null; }
function ErrorView() { return null; }
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"@tanstack/react-router":"1.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(path.parent().expect("route parent")).expect("route directory");
    fs::write(&path, source).expect("route source");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&path, "src/routes/orders.tsx", source)
        .expect("tanstack extraction");
    let route = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            _ => None,
        })
        .expect("tanstack route");
    assert_eq!(route.framework, "tanstack-router");
    assert_eq!(route.normalized_path, "/orders");
    assert!(
        route
            .stages
            .iter()
            .any(|stage| stage.role == RawRouteStageRole::RouteComponent)
    );
    assert!(
        route
            .stages
            .iter()
            .any(|stage| stage.role == RawRouteStageRole::Loader)
    );
    assert!(
        route
            .stages
            .iter()
            .any(|stage| stage.role == RawRouteStageRole::Boundary)
    );
}

#[test]
fn tanstack_start_is_separate_and_shadowed_factories_are_ignored() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let path = root.join("src/server.ts");
    let source = br#"import { createServerFn } from '@tanstack/start';
export const save = createServerFn({ method: 'POST' });
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"@tanstack/start":"1.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(path.parent().expect("source parent")).expect("source directory");
    fs::write(&path, source).expect("server source");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&path, "src/server.ts", source)
        .expect("start extraction");
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Role(role)
            if role.pack_id == "tanstack-start" && role.role == "server_function")
    }));

    let shadow_path = root.join("src/shadow.ts");
    let shadow = b"function createRoute() {}\ncreateRoute('/not-a-route');\n";
    fs::write(&shadow_path, shadow).expect("shadow source");
    let shadow_extraction = engine
        .extract_source_graph_only(&shadow_path, "src/shadow.ts", shadow)
        .expect("shadow extraction");
    assert!(
        !shadow_extraction
            .framework_facts
            .iter()
            .any(|fact| matches!(fact, RawFrameworkFact::Route(_)))
    );
}

#[test]
fn tanstack_file_route_convention_is_bounded_and_skips_generated_tree() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let path = root.join("src/routes/admin/$id.tsx");
    let source = b"export const Route = { component: Admin };\n";
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"@tanstack/react-router":"1.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(path.parent().expect("route parent")).expect("route directory");
    fs::write(&path, source).expect("route source");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&path, "src/routes/admin/$id.tsx", source)
        .expect("tanstack convention extraction");
    let route = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            _ => None,
        })
        .expect("TanStack file route");
    assert_eq!(route.normalized_path, "/admin/{id}");
    assert_eq!(
        route.origin,
        compass_languages::RawFrameworkOrigin::Convention
    );
    assert_eq!(
        route.rule.as_deref(),
        Some("tanstack-file-route-convention")
    );

    let generated = root.join("src/routes/routeTree.gen.ts");
    let generated_source = b"export const Route = {};\n";
    fs::write(&generated, generated_source).expect("generated route tree");
    let generated_extraction = engine
        .extract_source_graph_only(&generated, "src/routes/routeTree.gen.ts", generated_source)
        .expect("generated tree extraction");
    assert!(
        !generated_extraction
            .framework_facts
            .iter()
            .any(|fact| matches!(fact, RawFrameworkFact::Route(_)))
    );
}

#[test]
fn tanstack_file_route_convention_does_not_claim_react_router_modules() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let path = root.join("src/routes/home.tsx");
    let source = br#"import { Route } from 'react-router-dom';
export const routes = <Route path='/home' element={<Home />} />;
function Home() { return null; }
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"@tanstack/react-router":"1.0.0","react-router-dom":"7.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(path.parent().expect("route parent")).expect("route directory");
    fs::write(&path, source).expect("route source");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&path, "src/routes/home.tsx", source)
        .expect("react router extraction");
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Route(route) if route.framework == "tanstack-router")
    }));
}

#[test]
fn tanstack_file_route_requires_a_parser_backed_route_binding() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let path = root.join("src/routes/notes.tsx");
    // The word Route occurs only in a string.  A source-text substring check
    // would incorrectly publish this file as a TanStack route.
    let source = br#"const documentation = 'Route is exported by TanStack';
export const routes = { component: Notes };
function Notes() { return null; }
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"@tanstack/react-router":"1.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(path.parent().expect("route parent")).expect("route directory");
    fs::write(&path, source).expect("route source");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&path, "src/routes/notes.tsx", source)
        .expect("tanstack extraction");
    assert!(!extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Route(route) if route.framework == "tanstack-router")
    }));
}
