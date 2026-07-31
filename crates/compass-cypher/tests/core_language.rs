use std::error::Error;

use compass_cypher::{
    Clause, CompileLimits, CompileRequest, ParameterTypes, QueryProfileMode, compile, parse_only,
};
use compass_model::SchemaFingerprint;

fn request<'a>(
    source: &'a str,
    parameters: &'a ParameterTypes,
    schema: &'a SchemaFingerprint,
) -> CompileRequest<'a> {
    CompileRequest {
        source_name: "test.cypher",
        source,
        parameter_types: parameters,
        schema,
        limits: CompileLimits::default(),
    }
}

#[test]
fn parses_and_plans_the_core_read_surface() -> Result<(), Box<dyn Error>> {
    let parameters = ParameterTypes::new();
    let schema = SchemaFingerprint::empty();
    let compiled = compile(request(
        "PROFILE MATCH p=shortestPath((a:Function)-[:CALLS*1..8]->(b:Function)) \
         WHERE all(r IN relationships(p) WHERE r.confidence = 'EXTRACTED') \
         WITH a, b, length(p) AS hops WHERE hops > 0 \
         RETURN a.id AS source, b.id AS target, hops ORDER BY hops LIMIT 10",
        &parameters,
        &schema,
    ))?;
    assert_eq!(compiled.profile, QueryProfileMode::Profile);
    assert!(compiled.plan.contains_operator("NodeScan"));
    assert!(compiled.plan.contains_operator("Expand"));
    assert!(compiled.plan.contains_operator("Filter"));
    assert!(compiled.plan.contains_operator("Sort"));
    assert!(compiled.plan.contains_operator("Limit"));
    Ok(())
}

#[test]
fn rejects_writes_and_unbounded_paths_with_stable_codes() {
    let parameters = ParameterTypes::new();
    let schema = SchemaFingerprint::empty();
    let write = parse_only(request("MATCH (n) DELETE n", &parameters, &schema));
    assert_eq!(
        write
            .err()
            .and_then(|value| value.items().first().map(|item| item.code().to_owned())),
        Some("CQL1007".to_owned())
    );
    let unbounded = parse_only(request(
        "MATCH (a)-[:CALLS*]->(b) RETURN b",
        &parameters,
        &schema,
    ));
    assert_eq!(
        unbounded
            .err()
            .and_then(|value| value.items().first().map(|item| item.code().to_owned())),
        Some("CQL3002".to_owned())
    );
}

#[test]
fn correlated_exists_does_not_require_a_return_clause() -> Result<(), Box<dyn Error>> {
    let parameters = ParameterTypes::new();
    let schema = SchemaFingerprint::empty();
    compile(request(
        "MATCH (n) WHERE EXISTS { MATCH (n)-[:CALLS]->(m) } RETURN n.id",
        &parameters,
        &schema,
    ))?;
    Ok(())
}

#[test]
fn removes_only_unreferenced_path_bindings() -> Result<(), Box<dyn Error>> {
    let parameters = ParameterTypes::new();
    let schema = SchemaFingerprint::empty();
    let unused = compile(request(
        "MATCH p=(a)-[:CALLS*1..2]->(b) \
         RETURN a.id AS source, b.id AS target ORDER BY source, target LIMIT 100",
        &parameters,
        &schema,
    ))?;
    assert!(unused.plan.optimizations.iter().any(|record| {
        record.rule == "unused-path-binding-elimination"
            && record.reason == "removed 1 unreferenced path binding(s) before execution"
    }));
    let Some(Clause::Match(unused_match)) = unused.plan.ast.parts[0].clauses.first() else {
        return Err("expected leading MATCH".into());
    };
    assert_eq!(unused_match.patterns[0].variable, None);

    for source in [
        "MATCH p=(a)-[:CALLS*1..2]->(b) RETURN length(p) AS hops",
        "MATCH p=(a)-[:CALLS*1..2]->(b) RETURN *",
    ] {
        let retained = compile(request(source, &parameters, &schema))?;
        let Some(Clause::Match(retained_match)) = retained.plan.ast.parts[0].clauses.first() else {
            return Err("expected leading MATCH".into());
        };
        assert_eq!(retained_match.patterns[0].variable.as_deref(), Some("p"));
        assert!(
            retained
                .plan
                .optimizations
                .iter()
                .all(|record| record.rule != "unused-path-binding-elimination")
        );
    }
    Ok(())
}
