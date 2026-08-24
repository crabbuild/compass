use std::fs;
use std::sync::Arc;

use compass_languages::{Engine, ProjectEvidenceIndex, RawFrameworkFact, RawRouteStageRole};
use tempfile::tempdir;

#[test]
fn next_app_router_publishes_special_files_groups_slots_and_methods() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    fs::write(
        root.join("package.json"),
        br#"{"name":"next-fixture","dependencies":{"next":"15.0.0","react":"19.0.0"}}"#,
    )
    .expect("package manifest");
    let files = [
        (
            "src/app/(marketing)/@modal/products/layout.tsx",
            "export default function Layout({ children }: { children: unknown }) { return children; }",
        ),
        (
            "src/app/(marketing)/@modal/products/page.tsx",
            "export const generateStaticParams = () => []; export default function Products() { return null; }",
        ),
        (
            "src/app/(marketing)/@modal/products/loading.tsx",
            "export default function Loading() { return null; }",
        ),
        (
            "src/app/(marketing)/@modal/products/route.ts",
            "export function GET() { return new Response('ok'); }",
        ),
        (
            "src/middleware.ts",
            "export function middleware() { return undefined; }",
        ),
    ];
    let mut paths = Vec::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("file parent")).expect("route directory");
        fs::write(&path, source).expect("route source");
        paths.push(path);
    }
    let project = ProjectEvidenceIndex::build(root, &paths);
    let mut engine = Engine::with_project_evidence(Arc::new(project));

    let layout = engine
        .extract_source_graph_only(
            &paths[0],
            "src/app/(marketing)/@modal/products/layout.tsx",
            files[0].1.as_bytes(),
        )
        .expect("layout extraction");
    let layout_route = layout
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            _ => None,
        })
        .expect("layout route fact");
    assert_eq!(layout_route.normalized_path, "/products");
    assert_eq!(
        layout_route
            .detail
            .get("route_groups")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        layout_route
            .detail
            .get("parallel_slots")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(layout_route.stages[0].role, RawRouteStageRole::Layout);

    let page = engine
        .extract_source_graph_only(
            &paths[1],
            "src/app/(marketing)/@modal/products/page.tsx",
            files[1].1.as_bytes(),
        )
        .expect("page extraction");
    let page_route = page
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            _ => None,
        })
        .expect("page route fact");
    assert!(
        page_route
            .stages
            .iter()
            .any(|stage| stage.role == RawRouteStageRole::RouteComponent)
    );
    assert!(
        page_route
            .stages
            .iter()
            .any(|stage| stage.role == RawRouteStageRole::DataLoader)
    );
    assert_eq!(page_route.stages[0].reference, "Products");

    let endpoint = engine
        .extract_source_graph_only(
            &paths[3],
            "src/app/(marketing)/@modal/products/route.ts",
            files[3].1.as_bytes(),
        )
        .expect("endpoint extraction");
    assert!(endpoint.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Route(route) if route.operation == "GET"
            && route.normalized_path == "/products"
            && route.stages.iter().any(|stage| stage.role == RawRouteStageRole::Handler))
    }));

    let middleware = engine
        .extract_source_graph_only(&paths[4], "src/middleware.ts", files[4].1.as_bytes())
        .expect("middleware extraction");
    assert!(middleware.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "route_middleware")
    }));
}

#[test]
fn next_app_router_activates_from_a_local_config_marker() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let config = root.join("next.config.js");
    let page = root.join("test/e2e/foo/app/page.tsx");
    fs::create_dir_all(page.parent().expect("page parent")).expect("route directory");
    fs::write(&config, b"module.exports = {};").expect("next config");
    fs::write(
        &page,
        br#"import Link from 'next/link'

export default async function Home() {
  return (
    <div>
      <div>
        <Link href="/foo">Go to /foo (page & slot)</Link>
      </div>
      <div>
        <Link href="/bar">Go to /bar (page & no slot)</Link>
      </div>
      <div>
        <Link href="/baz">Go to /baz (no page & slot)</Link>
      </div>
      <div>
        <Link href="/quux">Go to /quux (no page & no slot)</Link>
      </div>
    </div>
  )
}
"#,
    )
    .expect("route page");
    let paths = vec![config.clone(), page.clone()];
    let project = ProjectEvidenceIndex::build(root, &paths);
    assert!(
        project
            .evidence_for(&page)
            .has_configuration("next.config.js")
    );
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(
            &page,
            "test/e2e/foo/app/page.tsx",
            br#"import Link from 'next/link'

export default async function Home() {
  return (
    <div>
      <div>
        <Link href="/foo">Go to /foo (page & slot)</Link>
      </div>
      <div>
        <Link href="/bar">Go to /bar (page & no slot)</Link>
      </div>
      <div>
        <Link href="/baz">Go to /baz (no page & slot)</Link>
      </div>
      <div>
        <Link href="/quux">Go to /quux (no page & no slot)</Link>
      </div>
    </div>
  )
}
"#,
        )
        .expect("page extraction");
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Route(route) if route.framework == "next")
    }));
}

#[test]
fn next_app_router_activates_for_nested_javascript_routes() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let config = root.join("next.config.js");
    let page = root.join("test/e2e/foo/app/page.js");
    fs::create_dir_all(page.parent().expect("page parent")).expect("route directory");
    fs::write(&config, b"module.exports = {}; ").expect("next config");
    let source = b"export default function Home() { return null; }";
    fs::write(&page, source).expect("route page");
    let paths = vec![config, page.clone()];
    let project = ProjectEvidenceIndex::build(root, &paths);
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&page, "test/e2e/foo/app/page.js", source)
        .expect("page extraction");
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Route(route) if route.framework == "next")
    }));
}

#[test]
fn next_app_default_reexport_preserves_handler_identity_and_module() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"next":"15.0.0"}}"#,
    )
    .expect("package manifest");
    let route = root.join("src/app/reexport/page.tsx");
    fs::create_dir_all(route.parent().expect("route parent")).expect("route directory");
    let source = b"export { SuspensePage as default } from '../../components/page'";
    fs::write(&route, source).expect("route source");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&route));
    let mut engine = Engine::with_project_evidence(Arc::new(project));

    let extraction = engine
        .extract_source_graph_only(&route, "src/app/reexport/page.tsx", source)
        .expect("route extraction");
    let route = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            _ => None,
        })
        .expect("route fact");
    assert_eq!(route.stages[0].reference, "SuspensePage");
    assert_eq!(
        route.detail.get("handler_module"),
        Some(&serde_json::Value::String(
            "../../components/page".to_owned()
        ))
    );
}

#[test]
fn next_app_router_ignores_app_text_inside_directory_names() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let config = root.join("next.config.js");
    let page = root.join("test/e2e/next-after-app/app/page.js");
    fs::create_dir_all(page.parent().expect("page parent")).expect("route directory");
    fs::write(&config, b"module.exports = {}; ").expect("next config");
    let source = b"export default function Home() { return null; }";
    fs::write(&page, source).expect("route page");
    let paths = vec![config, page.clone()];
    let project = ProjectEvidenceIndex::build(root, &paths);
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&page, "test/e2e/next-after-app/app/page.js", source)
        .expect("page extraction");
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Route(route) if route.framework == "next")
    }));
}

#[test]
fn next_app_unresolved_reexport_route_stays_in_route_tree() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"next":"15.0.0"}}"#,
    )
    .expect("package manifest");
    let route = root.join("src/app/edge/advanced/route.ts");
    fs::create_dir_all(route.parent().expect("route parent")).expect("route directory");
    let source = b"export * from '../../handlers/hello'";
    fs::write(&route, source).expect("route source");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&route));
    let mut engine = Engine::with_project_evidence(Arc::new(project));

    let extraction = engine
        .extract_source_graph_only(&route, "src/app/edge/advanced/route.ts", source)
        .expect("route extraction");
    let route = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            _ => None,
        })
        .expect("unresolved route fact");
    assert_eq!(route.operation, "ANY");
    assert_eq!(route.stages[0].role, RawRouteStageRole::Handler);
    assert_eq!(route.stages[0].reference, "default");
    assert!(!extraction.nodes.iter().any(|node| {
        node.attributes
            .get("synthetic_handler")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    }));
}
