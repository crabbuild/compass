use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::{CandidateRelation, Engine, EvidenceLimits, validate_evidence};
use compass_resolve::{resolve, resolve_with_root};

#[test]
fn markdown_documents_edges_resolve_to_universal_file_inventory() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let markdown_path = directory.path().join("guide.md");
    let rust_path = directory.path().join("documented.rs");
    std::fs::write(&markdown_path, "# Guide\n[Implementation](documented.rs)\n")?;
    std::fs::write(&rust_path, "pub fn documented() {}\n")?;

    let mut engine = Engine::default();
    let markdown = engine.extract(&markdown_path)?;
    let rust = engine.extract(&rust_path)?;
    let merged = resolve_with_root(&[markdown, rust], &HashMap::new(), directory.path());

    let edge = merged
        .edges
        .iter()
        .find(|edge| edge.string("relation") == "documents")
        .ok_or("missing documents edge")?;
    let target = merged
        .nodes
        .iter()
        .find(|node| node.id == edge.target)
        .ok_or("missing documents target")?;
    assert_eq!(target.string("symbol_kind"), "file");
    assert_eq!(target.string("source_file"), "documented.rs");

    let graph = compass_graph::build(&[merged], true, true, Some(directory.path()))?;
    assert!(graph.links.iter().any(|edge| {
        edge.attributes
            .get("relation")
            .and_then(serde_json::Value::as_str)
            == Some("documents")
    }));
    let inventory = [(&markdown_path, "markdown"), (&rust_path, "rust")]
        .into_iter()
        .map(|(path, language)| compass_graph::InventoryEvidence {
            path: path.clone(),
            language: Some(language.to_owned()),
            producer: format!("compass.languages.{language}"),
            status: compass_model::code_graph::ExtractionStatus::Extracted,
            reason: None,
        })
        .collect();
    let published = compass_graph::normalize_document_v1_with_inventory_best_effort(
        &graph,
        directory.path(),
        "test",
        None,
        inventory,
    )?;
    assert!(
        published
            .document
            .links
            .iter()
            .any(|edge| edge.kind.as_str() == "documents"),
        "published={:#?}",
        published.document
    );
    Ok(())
}

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
fn rust_call_resolution_distinguishes_same_named_fields_and_methods() -> Result<(), Box<dyn Error>>
{
    let source = br#"
struct Entry { path: String }
impl Entry { fn path(&self) -> &str { &self.path } }
fn read(entry: &Entry) { entry.path(); }
"#;
    let source_file = "src/lib.rs";
    let extracted = Engine::default().extract_source(Path::new(source_file), source)?;
    let call = extracted
        .semantic_evidence
        .as_ref()
        .and_then(|evidence| {
            evidence.candidates.iter().find(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == "path"
            })
        })
        .ok_or("missing path call evidence")?;
    assert_eq!(
        call.constraints.allowed_target_kinds,
        ["enum_member", "function", "method", "struct"]
    );
    let merged = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    let read = merged
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::read")
        .ok_or("missing read")?;
    let named = merged
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "crate::Entry::path")
        .collect::<Vec<_>>();
    assert_eq!(named.len(), 2, "nodes={named:#?}");
    assert_ne!(named[0].id, named[1].id, "nodes={named:#?}");
    let call_edges = merged
        .edges
        .iter()
        .filter(|edge| edge.source == read.id && edge.string("relation") == "calls")
        .collect::<Vec<_>>();
    let targets = call_edges
        .iter()
        .filter_map(|edge| merged.nodes.iter().find(|node| node.id == edge.target))
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 1, "edges={:#?}", merged.edges);
    assert_eq!(
        targets[0].string("symbol_kind"),
        "method",
        "target={:#?} edges={call_edges:#?}",
        targets[0],
    );
    assert_eq!(targets[0].string("qualified_name"), "crate::Entry::path");
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
    for (target_name, resolution_rule) in [
        ("crate::api::make", "explicit-binding"),
        ("crate::api::Widget::new", "member-binding"),
    ] {
        let target = merged
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing {target_name}: {:#?}", merged.nodes))?;
        assert!(merged.edges.iter().any(|edge| {
            edge.source == run.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("resolution_rule") == resolution_rule
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
    assert_eq!(evidence.adapter.version, 3);

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
    declaration_id("demo.catalog.Service::<init>").ok_or("missing Service constructor evidence")?;
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Contains
            && candidate.source_declaration_id == service_id
            && candidate.target_spelling == "run"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Contains
            && candidate.source_declaration_id == service_id
            && candidate.target_spelling == "<init>"
            && candidate.constraints.exact_target_declaration_id.is_some()
            && candidate.constraints.argument_count == Some(1)
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
    let constructor =
        node("demo.catalog.Service::<init>").ok_or("missing published Service constructor")?;
    for published in [service, worker, catalog, repository, run, constructor] {
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
        (service.id.as_str(), constructor.id.as_str(), "contains"),
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
fn java_same_arity_overloads_keep_exact_ownership() -> Result<(), Box<dyn Error>> {
    let source = br#"package demo;
class Parser {
    String parse(String value) { return value; }
    String parse(Object value) { return value.toString(); }
}
"#;
    let source_file = "src/main/java/demo/Parser.java";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let evidence = extracted
        .semantic_evidence
        .as_ref()
        .ok_or("missing Java universal evidence")?;
    let overloads = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.qualified_name == "demo.Parser::parse")
        .collect::<Vec<_>>();
    assert_eq!(overloads.len(), 2);
    assert!(overloads.iter().all(|overload| {
        evidence.candidates.iter().any(|candidate| {
            candidate.relation == CandidateRelation::Contains
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(overload.id.as_str())
        })
    }));

    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let parser = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "demo.Parser")
        .ok_or("missing Parser declaration")?;
    let published_overloads = resolved
        .nodes
        .iter()
        .filter(|node| node.string("qualified_name") == "demo.Parser::parse")
        .collect::<Vec<_>>();
    assert_eq!(published_overloads.len(), 2);
    assert!(published_overloads.iter().all(|overload| {
        resolved.edges.iter().any(|edge| {
            edge.source == parser.id
                && edge.target == overload.id
                && edge.string("relation") == "contains"
        })
    }));
    Ok(())
}

#[test]
fn java_exact_argument_types_select_one_overload_and_unknown_arguments_fail_closed()
-> Result<(), Box<dyn Error>> {
    let source = br#"package demo;
class Selector {
    String choose(String value) { return value; }
    String choose(Object value) { return value.toString(); }
    Object factory() { return new Object(); }
    void run() {
        choose("exact");
        choose(factory());
    }
}
"#;
    let source_file = "src/main/java/demo/Selector.java";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let evidence = extracted
        .semantic_evidence
        .as_ref()
        .ok_or("missing Java universal evidence")?;
    let choose_candidates = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "choose"
        })
        .collect::<Vec<_>>();
    assert_eq!(choose_candidates.len(), 2);
    assert!(choose_candidates.iter().any(|candidate| {
        candidate.constraints.argument_types == [Some("java.lang.String".to_owned())]
    }));
    assert!(
        choose_candidates
            .iter()
            .any(|candidate| candidate.constraints.argument_types == [None])
    );

    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "demo.Selector::run")
        .ok_or("missing run declaration")?;
    let selected = resolved
        .edges
        .iter()
        .filter(|edge| edge.source == run.id && edge.string("relation") == "calls")
        .filter_map(|edge| resolved.nodes.iter().find(|node| node.id == edge.target))
        .filter(|target| target.string("qualified_name") == "demo.Selector::choose")
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 1, "selected={selected:#?}");
    assert_eq!(selected[0].string("signature"), "choose(String)");
    Ok(())
}

#[test]
fn java_proven_conversions_select_the_unique_most_specific_overload() -> Result<(), Box<dyn Error>>
{
    let source = br#"package demo;
import vendor.External;
class Base {}
class Child extends Base {}
class Other {}
class Selector {
    String select(Object value) { return "object"; }
    String select(Base value) { return "base"; }
    Object factory() { return new Object(); }
    void run(Other other, Child child, External external) {
        select(other);
        select(child);
        select(1);
        select(new int[1]);
        select(factory());
        select(external);
    }
}
"#;
    let source_file = "src/main/java/demo/Selector.java";
    let extracted = Engine::default()
        .extract_source_combined(Path::new(source_file), source_file, source)?
        .graph;
    let resolved = resolve(
        &[extracted],
        &HashMap::from([(source_file.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(resolved.error.is_none(), "{:#?}", resolved.error);
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "demo.Selector::run")
        .ok_or("missing run declaration")?;
    let selected = resolved
        .edges
        .iter()
        .filter(|edge| edge.source == run.id && edge.string("relation") == "calls")
        .filter_map(|edge| resolved.nodes.iter().find(|node| node.id == edge.target))
        .filter(|target| target.string("qualified_name") == "demo.Selector::select")
        .map(|target| target.string("signature"))
        .collect::<Vec<_>>();
    assert_eq!(
        selected,
        [
            "select(Object)",
            "select(Base)",
            "select(Object)",
            "select(Object)"
        ],
        "selected={selected:#?}"
    );
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
