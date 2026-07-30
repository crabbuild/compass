use std::time::{Duration, Instant};

use compass_languages::{
    Extraction, FrameworkLimits, RawDomainFact, RawFrameworkAnchor, RawFrameworkFact,
    RawFrameworkOrigin, RawNodeRecord, RawRouteFact,
};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::{resolve_domains, resolve_routes};
use serde_json::{Map, Value};

const TARGETS: usize = 20_000;
const ROUTES: usize = 2_000;
const DOMAIN_FACTS: usize = 2_000;
const RESOLUTION_CEILING: Duration = Duration::from_secs(5);

#[test]
fn indexed_framework_resolution_stays_within_enterprise_ceiling()
-> Result<(), Box<dyn std::error::Error>> {
    let mut extraction = Extraction::default();
    extraction.nodes.reserve(TARGETS);
    for index in 0..TARGETS {
        extraction.nodes.push(callable(index));
    }
    extraction.framework_facts.reserve(ROUTES + DOMAIN_FACTS);
    for index in 0..ROUTES {
        extraction
            .framework_facts
            .push(RawFrameworkFact::Route(route(index)));
    }
    for index in 0..DOMAIN_FACTS {
        extraction
            .framework_facts
            .push(RawFrameworkFact::Domain(domain_fact(index)));
    }

    let started = Instant::now();
    let routes = resolve_routes(&extraction, FrameworkLimits::default())?;
    let domains = resolve_domains(&extraction, FrameworkLimits::default())?;
    let elapsed = started.elapsed();

    assert_eq!(routes.len(), ROUTES);
    assert!(
        routes
            .iter()
            .all(|route| route.state == ResolutionState::Exact)
    );
    assert_eq!(domains.len(), DOMAIN_FACTS);
    assert!(
        domains
            .iter()
            .all(|fact| fact.state == ResolutionState::Exact)
    );
    assert!(
        elapsed < RESOLUTION_CEILING,
        "indexed framework resolution took {elapsed:?}, exceeding {RESOLUTION_CEILING:?}"
    );
    Ok(())
}

fn callable(index: usize) -> RawNodeRecord {
    let name = format!("handler_{index:05}");
    RawNodeRecord {
        id: format!("node:{index:05}"),
        attributes: Map::from_iter([
            ("label".to_owned(), Value::String(name.clone())),
            ("name".to_owned(), Value::String(name.clone())),
            (
                "qualified_name".to_owned(),
                Value::String(format!("app.routes.{name}")),
            ),
            (
                "symbol_kind".to_owned(),
                Value::String("function".to_owned()),
            ),
            ("file_type".to_owned(), Value::String("code".to_owned())),
            (
                "source_file".to_owned(),
                Value::String("src/routes.rs".to_owned()),
            ),
            ("line_start".to_owned(), Value::from(index + 1)),
            ("line_end".to_owned(), Value::from(index + 1)),
        ]),
    }
}

fn route(index: usize) -> RawRouteFact {
    RawRouteFact {
        framework: "synthetic".to_owned(),
        operation: "GET".to_owned(),
        raw_path: format!("/resource/{index}"),
        normalized_path: format!("/resource/{index}"),
        declaring_scope: "app.routes".to_owned(),
        anchor: anchor(index),
        handler_reference: format!("handler_{index:05}"),
        middleware_references: Vec::new(),
        origin: RawFrameworkOrigin::Ast,
        rule: None,
        detail: Map::new(),
    }
}

fn domain_fact(index: usize) -> RawDomainFact {
    RawDomainFact {
        framework: "synthetic".to_owned(),
        kind: "message".to_owned(),
        name: format!("message.{index}"),
        declaring_scope: "app.routes".to_owned(),
        anchor: anchor(ROUTES + index),
        origin: RawFrameworkOrigin::Ast,
        detail: Map::from_iter([(
            "handler_reference".to_owned(),
            Value::String(format!("handler_{index:05}")),
        )]),
    }
}

fn anchor(index: usize) -> RawFrameworkAnchor {
    let offset = u64::try_from(index).unwrap_or(0) * 8;
    let line = u32::try_from(index + 1).unwrap_or(0);
    RawFrameworkAnchor {
        source_file: "src/routes.rs".to_owned(),
        start_byte: offset,
        end_byte: offset + 7,
        start_line: line,
        start_column: 0,
        end_line: line,
        end_column: 7,
    }
}
