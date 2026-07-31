use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::Engine;
use compass_resolve::resolve;

#[test]
fn collection_resolution_consumes_each_rust_evidence_batch_once() -> Result<(), Box<dyn Error>> {
    let mut engine = Engine::default();
    let left_source =
        b"struct Left {} impl Left { fn new() -> Self { Self {} } } fn left() { Left::new(); }";
    let right_source =
        b"struct Right {} impl Right { fn new() -> Self { Self {} } } fn right() { Right::new(); }";
    let left = engine.extract_source(Path::new("src/left.rs"), left_source)?;
    let right = engine.extract_source(Path::new("src/right.rs"), right_source)?;
    let sources = HashMap::from([
        (
            "src/left.rs".to_owned(),
            String::from_utf8(left_source.to_vec())?,
        ),
        (
            "src/right.rs".to_owned(),
            String::from_utf8(right_source.to_vec())?,
        ),
    ]);

    let merged = resolve(&[left, right], &sources);
    assert!(merged.semantic_evidence.is_none());
    assert!(merged.raw_calls.as_ref().is_some_and(Vec::is_empty));
    for (caller_name, target_name) in [
        ("crate::left::left", "crate::left::Left::new"),
        ("crate::right::right", "crate::right::Right::new"),
    ] {
        let caller = merged
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == caller_name)
            .ok_or_else(|| format!("missing {caller_name}"))?;
        let target = merged
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing {target_name}"))?;
        assert!(merged.edges.iter().any(|edge| {
            edge.source == caller.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("extractor") == "compass.resolve.rust.universal"
        }));
    }
    Ok(())
}

#[test]
fn rust_import_aliases_resolve_cross_file_types_functions_and_methods() -> Result<(), Box<dyn Error>>
{
    let mut engine = Engine::default();
    let api_source = br#"
pub trait Render { fn render(&self); }
pub struct Widget {}
impl Widget { pub fn new() -> Self { Self {} } }
impl Render for Widget { fn render(&self) {} }
pub fn make() -> Widget { Widget::new() }
"#;
    let caller_source = br#"
mod api;
use crate::api::{make as build_widget, Widget};
fn run() { build_widget(); Widget::new(); }
"#;
    let api = engine.extract_source(Path::new("src/api.rs"), api_source)?;
    let caller = engine.extract_source(Path::new("src/lib.rs"), caller_source)?;
    let sources = HashMap::from([
        (
            "src/api.rs".to_owned(),
            String::from_utf8(api_source.to_vec())?,
        ),
        (
            "src/lib.rs".to_owned(),
            String::from_utf8(caller_source.to_vec())?,
        ),
    ]);
    let merged = resolve(&[api, caller], &sources);
    let run = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::run")
        .ok_or("missing run")?;
    for target_name in ["crate::api::make", "crate::api::Widget::new"] {
        let target = merged
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing {target_name}: {:#?}", merged.nodes))?;
        assert!(merged.edges.iter().any(|edge| {
            edge.source == run.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("resolution_rule") == "explicit-binding"
        }));
    }
    let widget = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Widget")
        .ok_or("missing Widget")?;
    let render = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::api::Render")
        .ok_or("missing Render")?;
    assert!(merged.edges.iter().any(|edge| {
        edge.source == widget.id
            && edge.target == render.id
            && edge.string("relation") == "implements"
    }));
    Ok(())
}
