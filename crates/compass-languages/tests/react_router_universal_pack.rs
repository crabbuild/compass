use std::fs;
use std::sync::Arc;

use compass_languages::{Engine, ProjectEvidenceIndex, RawFrameworkFact, RawRouteStageRole};
use tempfile::tempdir;

#[test]
fn react_router_pack_requires_dependency_and_publishes_loader_action_stages() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let path = root.join("src/router.tsx");
    let source = br#"import { createBrowserRouter } from 'react-router-dom';
import Root from './Root';
import Orders from './Orders';
const router = createBrowserRouter([
  { path: '/', element: <Root />, loader: loadRoot },
  { path: '/orders/:id', Component: Orders, loader: loadOrders, action: saveOrder, errorElement: <ErrorView /> },
]);
function loadRoot() { return null; }
function loadOrders() { return null; }
function saveOrder() { return null; }
function ErrorView() { return null; }
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"react-router-dom":"7.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(path.parent().expect("source parent")).expect("source directory");
    fs::write(&path, source).expect("router source");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&path, "src/router.tsx", source)
        .expect("router extraction");
    let routes = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) if route.framework == "react-router" => Some(route),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 2);
    assert!(routes.iter().any(|route| {
        route.normalized_path == "/orders/{id}"
            && route
                .stages
                .iter()
                .any(|stage| stage.role == RawRouteStageRole::Loader)
            && route
                .stages
                .iter()
                .any(|stage| stage.role == RawRouteStageRole::Action)
            && route
                .stages
                .iter()
                .any(|stage| stage.role == RawRouteStageRole::Boundary)
    }));
}

#[test]
fn react_router_routes_are_not_claimed_without_a_router_dependency() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let path = root.join("src/router.tsx");
    let source = b"const router = createBrowserRouter([{ path: '/orders', element: <Orders /> }]);";
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"react":"19.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(path.parent().expect("source parent")).expect("source directory");
    fs::write(&path, source).expect("router source");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&path, "src/router.tsx", source)
        .expect("router extraction");
    assert!(!extraction.framework_facts.iter().any(
        |fact| matches!(fact, RawFrameworkFact::Route(route) if route.framework == "react-router")
    ));
}

#[test]
fn react_router_file_convention_publishes_route_and_data_stages_without_runtime_import() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let path = root.join("app/routes/products.$id.tsx");
    let source = br#"import type { Route } from './+types/products.$id';

export async function loader() { return { ok: true }; }
export async function action() { return null; }
export default function Product({ loaderData }: Route.ComponentProps) {
  return <h1>{loaderData.ok ? 'ok' : 'no'}</h1>;
}
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"react-router":"7.0.0","react":"19.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(path.parent().expect("source parent")).expect("source directory");
    fs::write(&path, source).expect("source file");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&path, "app/routes/products.$id.tsx", source)
        .expect("route extraction");
    let route = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) if route.framework == "react-router" => Some(route),
            _ => None,
        })
        .expect("file route fact");
    assert_eq!(route.normalized_path, "/products/{id}");
    assert!(
        route.stages.iter().any(|stage| {
            stage.role == RawRouteStageRole::Loader && stage.reference == "loader"
        })
    );
    assert!(
        route.stages.iter().any(|stage| {
            stage.role == RawRouteStageRole::Action && stage.reference == "action"
        })
    );
    assert!(route.stages.iter().any(|stage| {
        stage.role == RawRouteStageRole::RouteComponent && stage.reference == "Product"
    }));
}
