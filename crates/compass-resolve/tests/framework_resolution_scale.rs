use std::collections::HashMap;
use std::time::{Duration, Instant};

use compass_languages::{
    Extraction, RawDomainFact, RawFrameworkAnchor, RawFrameworkFact, RawFrameworkOrigin,
    RawNodeRecord, RawRouteFact,
};
use compass_resolve::resolve_owned_with_root;
use serde_json::{Map, Value};

const TARGETS: usize = 100_000;
const ROUTES: usize = 50_000;
const DOMAIN_FACTS: usize = 50_000;
// This end-to-end guard runs in an unoptimized test binary on shared CI
// hardware. Candidate expansion has separate deterministic budget assertions;
// this ceiling detects material regressions without encoding runner speed.
const RESOLUTION_CEILING: Duration = Duration::from_secs(90);

#[test]
fn shared_production_framework_resolution_stays_within_enterprise_ceiling()
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

    let root = tempfile::tempdir()?;
    let started = Instant::now();
    let resolved = resolve_owned_with_root(vec![extraction], &HashMap::new(), root.path());
    let elapsed = started.elapsed();

    let routes = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "routes_to")
        .count();
    let domains = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "handles")
        .count();
    assert_eq!(routes, ROUTES);
    assert_eq!(domains, DOMAIN_FACTS);
    assert_eq!(resolved.error, None);
    assert!(
        elapsed < RESOLUTION_CEILING,
        "production framework resolution took {elapsed:?}, exceeding {RESOLUTION_CEILING:?}"
    );
    println!(
        "{{\"targets\":{TARGETS},\"facts\":{},\"routes\":{},\"domains\":{},\"elapsedMs\":{}}}",
        ROUTES + DOMAIN_FACTS,
        routes,
        domains,
        elapsed.as_millis()
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
