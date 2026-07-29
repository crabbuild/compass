use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{
    Engine, Extraction, FrameworkLimits, RawDomainFact, RawFrameworkAnchor, RawFrameworkFact,
    RawFrameworkOrigin, RawNodeRecord,
};
use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::resolve_and_publish_framework_domains;
use compass_resolve::resolve;

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/code-graph/domain")
        .join(relative)
}

fn extract(relative: &str) -> Result<Extraction, Box<dyn Error>> {
    let path = fixture(relative);
    let extraction = Engine::default().extract(&path)?;
    if let Some(error) = &extraction.error {
        return Err(error.clone().into());
    }
    Ok(extraction)
}

fn merge(target: &mut Extraction, mut source: Extraction) {
    target.nodes.append(&mut source.nodes);
    target.edges.append(&mut source.edges);
    target.framework_facts.append(&mut source.framework_facts);
}

#[test]
fn nest_and_spring_message_facts_preserve_direction_and_transport() -> Result<(), Box<dyn Error>> {
    let mut extraction = extract("messaging/nest.ts")?;
    merge(&mut extraction, extract("messaging/spring.java")?);
    let resolved =
        resolve_and_publish_framework_domains(&mut extraction, FrameworkLimits::default())?;
    assert!(resolved.iter().any(|fact| {
        fact.fact.kind == "message"
            && fact.fact.name == "orders.created"
            && fact.state == ResolutionState::Exact
    }));
    assert!(resolved.iter().any(|fact| {
        fact.fact.kind == "topic"
            && fact.fact.name == "orders.created"
            && fact.state == ResolutionState::Exact
    }));
    assert!(resolved.iter().any(|fact| {
        fact.fact.kind == "queue"
            && fact.fact.name == "orders.queue"
            && fact.state == ResolutionState::Exact
    }));
    for relation in [
        "handles",
        "subscribes",
        "registers",
        "publishes",
        "produces",
        "consumes",
    ] {
        assert!(
            extraction
                .edges
                .iter()
                .any(|edge| edge.string("relation") == relation),
            "missing {relation} edge"
        );
    }
    assert!(
        extract("messaging/dynamic-near-matches.ts")?
            .framework_facts
            .is_empty()
    );
    Ok(())
}

#[test]
fn scheduled_and_hosted_jobs_publish_schedules_and_triggers() -> Result<(), Box<dyn Error>> {
    let mut extraction = Extraction::default();
    for fixture in ["jobs/spring.java", "jobs/aspnet.cs", "jobs/celery.py"] {
        merge(&mut extraction, extract(fixture)?);
    }
    let resolved =
        resolve_and_publish_framework_domains(&mut extraction, FrameworkLimits::default())?;
    assert_eq!(
        resolved
            .iter()
            .filter(|fact| fact.fact.kind == "job")
            .count(),
        4
    );
    assert!(
        resolved
            .iter()
            .filter(|fact| fact.fact.kind == "job")
            .all(|fact| fact.state == ResolutionState::Exact)
    );
    for relation in ["schedules", "triggers"] {
        assert!(
            extraction
                .edges
                .iter()
                .any(|edge| edge.string("relation") == relation)
        );
    }
    assert!(
        extract("jobs/dynamic-near-matches.py")?
            .framework_facts
            .is_empty()
    );
    Ok(())
}

#[test]
fn every_approved_orm_maps_only_to_existing_database_tables() -> Result<(), Box<dyn Error>> {
    let database = extract("orm/database.sql")?;
    for fixture in [
        "orm/django.py",
        "orm/sqlalchemy.py",
        "orm/typeorm.ts",
        "orm/jpa.java",
        "orm/entity-framework.cs",
        "orm/active-record.rb",
        "orm/eloquent.php",
        "orm/gorm.go",
        "orm/diesel.rs",
    ] {
        let mut extraction = extract(fixture)?;
        merge(&mut extraction, database.clone());
        let resolved =
            resolve_and_publish_framework_domains(&mut extraction, FrameworkLimits::default())?;
        let mapping = resolved
            .iter()
            .find(|fact| fact.fact.kind == "orm_mapping")
            .ok_or_else(|| format!("missing ORM mapping for {fixture}"))?;
        assert_eq!(
            mapping.state,
            ResolutionState::Exact,
            "unresolved mapping for {fixture}: {mapping:#?}"
        );
        assert!(extraction.edges.iter().any(|edge| {
            edge.string("relation") == "maps_to" && edge.string("_origin") == "ast"
        }));
    }
    assert!(
        extract("orm/dynamic-near-matches.ts")?
            .framework_facts
            .is_empty()
    );
    Ok(())
}

#[test]
fn absent_orm_targets_remain_diagnostic_without_synthetic_tables() -> Result<(), Box<dyn Error>> {
    let mut extraction = extract("orm/typeorm.ts")?;
    let table_count = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "database_table")
        .count();
    let resolved =
        resolve_and_publish_framework_domains(&mut extraction, FrameworkLimits::default())?;
    assert!(resolved.iter().any(|fact| {
        fact.fact.kind == "orm_mapping" && fact.state == ResolutionState::Unresolved
    }));
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "database_table")
            .count(),
        table_count
    );
    assert!(
        extraction
            .extensions
            .get("framework_domain_diagnostics")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|diagnostics| !diagnostics.is_empty())
    );
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Domain(domain) if domain.kind == "orm_mapping")
    }));
    Ok(())
}

#[test]
fn sole_terminal_domain_candidate_remains_ambiguous_and_non_authoritative()
-> Result<(), Box<dyn Error>> {
    let fact = RawDomainFact {
        framework: "synthetic".to_owned(),
        kind: "message".to_owned(),
        name: "orders.created".to_owned(),
        declaring_scope: "app.handlers".to_owned(),
        anchor: RawFrameworkAnchor {
            source_file: "handlers.ts".to_owned(),
            start_byte: 10,
            end_byte: 30,
            start_line: 2,
            start_column: 0,
            end_line: 2,
            end_column: 20,
        },
        origin: RawFrameworkOrigin::Ast,
        detail: serde_json::Map::from_iter([
            (
                "handler_reference".to_owned(),
                serde_json::Value::String("handle".to_owned()),
            ),
            (
                "transport".to_owned(),
                serde_json::Value::String("synthetic".to_owned()),
            ),
        ]),
    };
    let mut extraction = Extraction {
        nodes: vec![RawNodeRecord {
            id: "other-handler".to_owned(),
            attributes: serde_json::Map::from_iter([
                (
                    "label".to_owned(),
                    serde_json::Value::String("handle".to_owned()),
                ),
                (
                    "name".to_owned(),
                    serde_json::Value::String("handle".to_owned()),
                ),
                (
                    "qualified_name".to_owned(),
                    serde_json::Value::String("other.handle".to_owned()),
                ),
                (
                    "symbol_kind".to_owned(),
                    serde_json::Value::String("function".to_owned()),
                ),
                (
                    "source_file".to_owned(),
                    serde_json::Value::String("other.ts".to_owned()),
                ),
            ]),
        }],
        framework_facts: vec![RawFrameworkFact::Domain(fact)],
        ..Extraction::default()
    };

    let resolved =
        resolve_and_publish_framework_domains(&mut extraction, FrameworkLimits::default())?;
    assert_eq!(resolved[0].state, ResolutionState::Ambiguous);
    assert_eq!(resolved[0].source_candidates.len(), 1);
    assert!(
        extraction
            .edges
            .iter()
            .all(|edge| edge.string("relation") != "handles")
    );
    Ok(())
}

#[test]
fn enterprise_facts_normalize_into_the_closed_v1_graph() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "orders.ts",
            r#"import { Controller } from '@nestjs/common';
import { MessagePattern } from '@nestjs/microservices';
import { Entity } from 'typeorm';
@Controller()
class Orders {
  @MessagePattern('orders.created')
  handle() {}
}
@Entity('orders')
class Order {}
"#,
        ),
        (
            "jobs.py",
            r#"from celery import shared_task
@shared_task
def refresh_orders():
    pass
"#,
        ),
        ("database.sql", "CREATE TABLE orders (id INTEGER);"),
    ];
    let mut engine = Engine::default();
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        fs::write(root.join(relative), source)?;
        extractions.push(engine.extract_source(Path::new(relative), source.as_bytes())?);
        sources.insert(relative.to_owned(), source.to_owned());
    }
    let mut extraction = resolve(&extractions, &sources);
    let ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    extraction
        .edges
        .retain(|edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str()));
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:test-config")?;
    let graph = normalize_v1(extraction, evidence)?;
    for kind in [NodeKind::Message, NodeKind::Job, NodeKind::DatabaseTable] {
        assert!(graph.nodes.iter().any(|node| node.kind == kind));
    }
    for kind in [
        EdgeKind::Handles,
        EdgeKind::Schedules,
        EdgeKind::Triggers,
        EdgeKind::MapsTo,
    ] {
        assert!(graph.links.iter().any(|edge| edge.kind == kind));
    }
    Ok(())
}
