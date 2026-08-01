use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::{CandidateRelation, Engine, EvidenceLimits, validate_evidence};
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

#[test]
fn java_imports_receivers_and_overload_arity_resolve_on_the_universal_path()
-> Result<(), Box<dyn Error>> {
    let mut engine = Engine::default();
    let repository_source = br#"package org.example.data;
public class Repository {
    public Result load(String key) { return new Result(); }
    public Result load(String key, int limit) { return new Result(); }
}
class Result {}
"#;
    let service_source = br#"package org.example.app;
import org.example.data.Repository;
public class Service {
    private final Repository repository;
    public Service(Repository repository) { this.repository = repository; }
    public void run() { repository.load("one"); }
}
"#;
    let repository = engine.extract_source(
        Path::new("src/main/java/org/example/data/Repository.java"),
        repository_source,
    )?;
    let service = engine.extract_source(
        Path::new("src/main/java/org/example/app/Service.java"),
        service_source,
    )?;
    assert!(repository.nodes.is_empty() && repository.edges.is_empty());
    assert!(service.nodes.is_empty() && service.edges.is_empty());
    let sources = HashMap::from([
        (
            "src/main/java/org/example/data/Repository.java".to_owned(),
            String::from_utf8(repository_source.to_vec())?,
        ),
        (
            "src/main/java/org/example/app/Service.java".to_owned(),
            String::from_utf8(service_source.to_vec())?,
        ),
    ]);
    let merged = resolve(&[repository, service], &sources);
    assert!(merged.error.is_none(), "{:#?}", merged.error);
    let run = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "org.example.app.Service::run")
        .ok_or("missing Service::run")?;
    let loads = merged
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "org.example.data.Repository::load")
        .collect::<Vec<_>>();
    assert_eq!(loads.len(), 2);
    let one_argument_load = loads
        .iter()
        .find(|node| node.string("signature") == "load(String)")
        .ok_or("missing one-argument overload")?;
    assert!(
        merged.edges.iter().any(|edge| {
            edge.source == run.id
                && edge.target == one_argument_load.id
                && edge.string("relation") == "calls"
                && edge.string("resolution_rule") == "explicit-binding"
        }),
        "one-argument overload was not selected"
    );
    assert!(!merged.edges.iter().any(|edge| {
        edge.source == run.id
            && loads
                .iter()
                .any(|load| load.id == edge.target && load.id != one_argument_load.id)
            && edge.string("relation") == "calls"
    }));
    Ok(())
}

#[test]
fn java_contains_references_and_declarations_survive_universal_publication()
-> Result<(), Box<dyn Error>> {
    let source = br#"package demo.catalog;

interface Repository {
    void enqueue(Catalog item);
}

class Catalog {}

class Worker extends Catalog implements Repository {
    public void enqueue(Catalog item) {
        this.track(item);
    }

    void track(Catalog item) {}
}

class Service {
    private final Catalog catalog;

    public Service(Catalog catalog) {
        this.catalog = catalog;
    }

    public Catalog run(Catalog item) {
        return new Catalog();
    }
}
"#;
    let source_file = "src/main/java/demo/catalog/Service.java";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;

    assert!(extracted.nodes.is_empty());
    assert!(extracted.edges.is_empty());
    assert!(extracted.raw_calls.is_none());
    let evidence = extracted
        .semantic_evidence
        .as_ref()
        .ok_or("missing Java universal evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    assert_eq!(evidence.adapter.id, "compass.java");
    assert_eq!(evidence.adapter.version, 1);

    let declaration_id = |qualified_name: &str| {
        evidence
            .declarations
            .iter()
            .find(|declaration| declaration.qualified_name == qualified_name)
            .map(|declaration| declaration.id.as_str())
    };
    let service_id = declaration_id("demo.catalog.Service").ok_or("missing Service evidence")?;
    let worker_id = declaration_id("demo.catalog.Worker").ok_or("missing Worker evidence")?;
    let run_id = declaration_id("demo.catalog.Service::run").ok_or("missing run evidence")?;
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Contains
            && candidate.source_declaration_id == service_id
            && candidate.target_spelling == "run"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Extends
            && candidate.source_declaration_id == worker_id
            && candidate.target_spelling == "Catalog"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Implements
            && candidate.source_declaration_id == worker_id
            && candidate.target_spelling == "Repository"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::References
            && candidate.source_declaration_id == run_id
            && candidate.target_spelling == "Catalog"
    }));

    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let node = |qualified_name: &str| {
        resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
    };
    let service = node("demo.catalog.Service").ok_or("missing published Service")?;
    let worker = node("demo.catalog.Worker").ok_or("missing published Worker")?;
    let catalog = node("demo.catalog.Catalog").ok_or("missing published Catalog")?;
    let repository = node("demo.catalog.Repository").ok_or("missing published Repository")?;
    let run = node("demo.catalog.Service::run").ok_or("missing published run")?;
    for published in [service, worker, catalog, repository, run] {
        assert_eq!(published.string("language"), "java");
        assert_eq!(
            published.string("extractor"),
            "compass.languages.java.universal"
        );
        assert_eq!(published.string("source_file"), source_file);
        assert!(!published.string("evidence_declaration_id").is_empty());
    }
    for (source_id, target_id, relation) in [
        (service.id.as_str(), run.id.as_str(), "contains"),
        (worker.id.as_str(), catalog.id.as_str(), "inherits"),
        (worker.id.as_str(), repository.id.as_str(), "implements"),
        (run.id.as_str(), catalog.id.as_str(), "references"),
    ] {
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.source == source_id
                    && edge.target == target_id
                    && edge.string("relation") == relation
                    && edge.string("extractor") == "compass.resolve.java.universal"
            }),
            "missing {relation} edge {source_id} -> {target_id}"
        );
    }
    Ok(())
}

#[test]
fn java_collection_source_paths_keep_duplicate_class_copies_distinct() -> Result<(), Box<dyn Error>>
{
    let source = br#"package org.example.shared;
public class Duplicate { public void run() {} }
"#;
    let first_source = "module-a/src/main/java/org/example/shared/Duplicate.java";
    let second_source = "module-b/src/test/java/org/example/shared/Duplicate.java";
    let mut engine = Engine::default();
    let first = engine
        .extract_source_combined(
            Path::new("/repo/module-a/src/main/java/org/example/shared/Duplicate.java"),
            first_source,
            source,
        )?
        .graph;
    let second = engine
        .extract_source_combined(
            Path::new("/repo/module-b/src/test/java/org/example/shared/Duplicate.java"),
            second_source,
            source,
        )?
        .graph;
    let sources = HashMap::from([
        (first_source.to_owned(), String::from_utf8(source.to_vec())?),
        (
            second_source.to_owned(),
            String::from_utf8(source.to_vec())?,
        ),
    ]);

    let merged = resolve(&[first, second], &sources);
    assert!(merged.error.is_none(), "{:#?}", merged.error);
    let duplicates = merged
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "org.example.shared.Duplicate")
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 2, "{:#?}", merged.nodes);
    assert_ne!(duplicates[0].id, duplicates[1].id);
    assert_eq!(
        duplicates
            .iter()
            .map(|node| node.string("source_file"))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([first_source.to_owned(), second_source.to_owned()])
    );
    Ok(())
}
