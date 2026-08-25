use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::{Engine, Extraction, FrameworkLimits};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::{FrameworkResolutionError, resolve_routes};
use compass_resolve::resolve;

fn extract(path: &str, source: &[u8]) -> Result<Extraction, Box<dyn Error>> {
    Ok(Engine::default().extract_source(Path::new(path), source)?)
}

fn resolved_project(sources: &[(&str, &[u8])]) -> Result<Extraction, Box<dyn Error>> {
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut source_map = HashMap::new();
    for (path, source) in sources {
        extractions.push(engine.extract_source(Path::new(path), source)?);
        source_map.insert((*path).to_owned(), String::from_utf8((*source).to_vec())?);
    }
    Ok(resolve(&extractions, &source_map))
}

#[test]
fn nested_and_repeated_fastapi_mounts_preserve_receiver_identity_and_multiplicity()
-> Result<(), Box<dyn Error>> {
    let leaf = br#"from fastapi import APIRouter
leaf = APIRouter()

@leaf.get("/leaf")
def handler():
    return None
"#;
    let middle = br#"from fastapi import APIRouter
from .leaf import leaf
middle = APIRouter()
middle.include_router(leaf, prefix="/middle")
"#;
    let app = br#"from fastapi import FastAPI
from .middle import middle
app = FastAPI()
app.include_router(middle, prefix="/v1")
app.include_router(middle, prefix="/v2")
"#;
    let extraction = resolved_project(&[
        ("pkg/leaf.py", leaf),
        ("pkg/middle.py", middle),
        ("pkg/app.py", app),
    ])?;
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    let paths = routes
        .iter()
        .map(|route| route.route.normalized_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["/v1/middle/leaf", "/v2/middle/leaf"]);
    assert!(
        routes
            .iter()
            .all(|route| route.state == ResolutionState::Exact)
    );
    assert!(routes.iter().all(|route| {
        route
            .route
            .detail
            .get("mount_anchors")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|anchors| anchors.len() == 2)
    }));
    Ok(())
}

#[test]
fn receiver_mount_cycles_emit_no_fabricated_route_and_depth_overflow_is_explicit()
-> Result<(), Box<dyn Error>> {
    let a = br#"from fastapi import APIRouter
from .b import b
a = APIRouter()
a.include_router(b, prefix="/a")

@a.get("/route")
def handler():
    return None
"#;
    let b = br#"from fastapi import APIRouter
from .a import a
b = APIRouter()
b.include_router(a, prefix="/b")
"#;
    let cycle = resolved_project(&[("pkg/a.py", a), ("pkg/b.py", b)])?;
    assert!(resolve_routes(&cycle, FrameworkLimits::default())?.is_empty());

    let leaf = br#"from fastapi import APIRouter
leaf = APIRouter()
@leaf.get("/leaf")
def handler(): return None
"#;
    let middle = br#"from fastapi import APIRouter
from .leaf import leaf
middle = APIRouter()
middle.include_router(leaf, prefix="/middle")
"#;
    let app = br#"from fastapi import FastAPI
from .middle import middle
app = FastAPI()
app.include_router(middle, prefix="/api")
"#;
    let chain = resolved_project(&[
        ("pkg/leaf.py", leaf),
        ("pkg/middle.py", middle),
        ("pkg/app.py", app),
    ])?;
    let limits = FrameworkLimits {
        max_include_depth: 1,
        ..FrameworkLimits::default()
    };
    assert!(matches!(
        resolve_routes(&chain, limits),
        Err(FrameworkResolutionError::Limit(error))
            if error.limit == "max_include_depth"
    ));
    Ok(())
}

#[test]
fn ambiguous_receiver_identity_retains_only_the_unmounted_route() -> Result<(), Box<dyn Error>> {
    let first = br#"from fastapi import APIRouter
router = APIRouter()
@router.get("/first")
def first(): return None
"#;
    let second = br#"from fastapi import APIRouter

router = APIRouter()
@router.get("/second")
def second(): return None
"#;
    let app = br#"from fastapi import FastAPI
from .routes import router
app = FastAPI()
app.include_router(router, prefix="/api")
"#;
    let mut ambiguous = Extraction::default();
    for mut extraction in [
        extract("pkg/routes.py", first)?,
        extract("pkg/routes.py", second)?,
        extract("pkg/app.py", app)?,
    ] {
        ambiguous.nodes.append(&mut extraction.nodes);
        ambiguous.edges.append(&mut extraction.edges);
        ambiguous
            .framework_facts
            .append(&mut extraction.framework_facts);
    }
    let routes = resolve_routes(&ambiguous, FrameworkLimits::default())?;
    assert!(
        routes
            .iter()
            .all(|route| !route.route.normalized_path.starts_with("/api"))
    );
    assert_eq!(
        routes
            .iter()
            .map(|route| route.route.normalized_path.as_str())
            .collect::<Vec<_>>(),
        ["/first", "/second"]
    );
    Ok(())
}
