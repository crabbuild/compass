use std::error::Error;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use compass_cypher::{
    CompassValue, CompileLimits, CompileRequest, ParameterTypes, Parameters, compile,
};
use compass_model::{Graph, GraphDocument};
use compass_query::{PlanCache, QueryErrorKind, QueryLimits, QueryRequest, execute};

fn graph() -> Result<Graph, Box<dyn Error>> {
    let document = serde_json::from_str::<GraphDocument>(
        r#"{
          "directed": true,
          "multigraph": true,
          "graph": {},
          "nodes": [
            {"id":"a","label":"a()","file_type":"function","source_file":"src/a.rs"},
            {"id":"b","label":"b()","file_type":"function","source_file":"src/b.rs"},
            {"id":"c","label":"c()","file_type":"function","source_file":"src/c.rs"},
            {"id":"test","label":"test_a()","file_type":"function","source_file":"tests/a.rs"}
          ],
          "links": [
            {"source":"a","target":"b","relation":"calls","confidence":"EXTRACTED"},
            {"source":"b","target":"c","relation":"calls","confidence":"EXTRACTED"},
            {"source":"test","target":"a","relation":"tests","confidence":"INFERRED"}
          ]
        }"#,
    )?;
    Ok(Graph::from_document(document)?)
}

fn run(source: &str) -> Result<compass_query::QueryResult, Box<dyn Error>> {
    let graph = graph()?;
    let parameters = Parameters::new();
    let parameter_types = ParameterTypes::new();
    let schema = graph.schema_fingerprint();
    let compiled = compile(CompileRequest {
        source_name: "test.cypher",
        source,
        parameter_types: &parameter_types,
        schema: &schema,
        limits: CompileLimits::default(),
    })?;
    let cancellation = AtomicBool::new(false);
    Ok(execute(QueryRequest {
        compiled: &compiled,
        graph: &graph,
        parameters: &parameters,
        limits: QueryLimits {
            deadline: Instant::now() + Duration::from_secs(2),
            max_rows: 1_000,
            max_path_depth: 32,
            max_expanded_relationships: 10_000,
            max_memory_bytes: 16 * 1024 * 1024,
        },
        cancellation: &cancellation,
    })?)
}

#[test]
fn executes_indexed_fixed_patterns_and_properties() -> Result<(), Box<dyn Error>> {
    let result = run("MATCH (a:Function)-[r:CALLS]->(b:Function) \
         WHERE r.confidence = 'EXTRACTED' \
         RETURN a.id AS caller, b.id AS callee ORDER BY caller")?;
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], CompassValue::String("a".into()));
    assert_eq!(result.rows[1][1], CompassValue::String("c".into()));
    Ok(())
}

#[test]
fn executes_bounded_paths_list_predicates_and_path_functions() -> Result<(), Box<dyn Error>> {
    let result = run("MATCH p=(a {id:'a'})-[:CALLS*1..2]->(b) \
         WHERE all(r IN relationships(p) WHERE r.confidence = 'EXTRACTED') \
         RETURN b.id AS id, length(p) AS hops ORDER BY hops, id")?;
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][1], CompassValue::Integer(1));
    assert_eq!(result.rows[1][1], CompassValue::Integer(2));
    Ok(())
}

#[test]
fn executes_optional_exists_and_aggregation() -> Result<(), Box<dyn Error>> {
    let optional = run("MATCH (n {id:'c'}) OPTIONAL MATCH (n)-[:TESTS]->(t) \
         RETURN coalesce(t.id, 'none') AS test")?;
    assert_eq!(optional.rows[0][0], CompassValue::String("none".into()));

    let exists = run("MATCH (n) WHERE EXISTS { MATCH (n)-[:CALLS]->(m) } \
         RETURN count(DISTINCT n) AS callers")?;
    assert_eq!(exists.rows[0][0], CompassValue::Integer(2));
    Ok(())
}

#[test]
fn executes_scalar_list_case_and_three_valued_expressions() -> Result<(), Box<dyn Error>> {
    let result = run("RETURN [1,2,3][1] AS item, [1,2,3][1..3] AS tail, \
         CASE WHEN 2 IN [1,2] THEN 'yes' ELSE 'no' END AS verdict, \
         [null] = [1] AS unknown, 5 / 2 AS quotient")?;
    assert_eq!(result.rows[0][0], CompassValue::Integer(2));
    assert_eq!(
        result.rows[0][1],
        CompassValue::List(vec![CompassValue::Integer(2), CompassValue::Integer(3)].into())
    );
    assert_eq!(result.rows[0][2], CompassValue::String("yes".into()));
    assert_eq!(result.rows[0][3], CompassValue::Null);
    assert_eq!(result.rows[0][4], CompassValue::Integer(2));
    Ok(())
}

#[test]
fn executes_list_predicate_null_semantics_and_string_functions() -> Result<(), Box<dyn Error>> {
    let result = run("RETURN all(x IN [true,null] WHERE x) AS every, \
         any(x IN [false,null] WHERE x) AS some, \
         none(x IN [false,null] WHERE x) AS none_value, \
         single(x IN [true,null] WHERE x) AS one, \
         toUpper(trim(' compass ')) AS name, 'compass' =~ 'comp.*' AS regex")?;
    assert_eq!(result.rows[0][0], CompassValue::Null);
    assert_eq!(result.rows[0][1], CompassValue::Null);
    assert_eq!(result.rows[0][2], CompassValue::Null);
    assert_eq!(result.rows[0][3], CompassValue::Null);
    assert_eq!(result.rows[0][4], CompassValue::String("COMPASS".into()));
    assert_eq!(result.rows[0][5], CompassValue::Boolean(true));
    Ok(())
}

#[test]
fn executes_unwind_aggregates_union_and_graph_functions() -> Result<(), Box<dyn Error>> {
    let aggregate = run("UNWIND [1,2,2] AS value \
         RETURN count(DISTINCT value) AS count, sum(value) AS total, avg(value) AS average")?;
    assert_eq!(aggregate.rows[0][0], CompassValue::Integer(2));
    assert_eq!(aggregate.rows[0][1], CompassValue::Integer(5));

    let union = run("RETURN 1 AS value UNION RETURN 1 AS value UNION ALL RETURN 2 AS value")?;
    assert_eq!(union.rows.len(), 2);

    let graph_values = run("MATCH p=(a {id:'a'})-[r:CALLS]->(b) \
         RETURN id(a) AS id, labels(a) AS labels, type(r) AS relation, \
                length(p) AS hops, size(nodes(p)) AS nodes, properties(a).source_file AS file")?;
    assert_eq!(graph_values.rows[0][0], CompassValue::Integer(0));
    assert_eq!(
        graph_values.rows[0][2],
        CompassValue::String("CALLS".into())
    );
    assert_eq!(graph_values.rows[0][3], CompassValue::Integer(1));
    Ok(())
}

#[test]
fn shortest_paths_are_breadth_first_and_retain_intermediate_nodes() -> Result<(), Box<dyn Error>> {
    let result = run(
        "MATCH p=shortestPath((a {id:'a'})-[:CALLS*1..3]->(c {id:'c'})) \
         RETURN length(p) AS hops, size(nodes(p)) AS nodes",
    )?;
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], CompassValue::Integer(2));
    assert_eq!(result.rows[0][1], CompassValue::Integer(3));
    Ok(())
}

#[test]
fn cancellation_memory_limits_and_plan_cache_are_enforced() -> Result<(), Box<dyn Error>> {
    let graph = graph()?;
    let parameters = Parameters::new();
    let parameter_types = ParameterTypes::new();
    let request = CompileRequest {
        source_name: "limits.cypher",
        source: "PROFILE RETURN 'a deliberately long result value' AS value",
        parameter_types: &parameter_types,
        schema: &graph.schema_fingerprint(),
        limits: CompileLimits::default(),
    };
    let compiled = compile(request)?;
    let cache = PlanCache::default();
    assert!(cache.get(&compiled.cache_key).is_none());
    cache.insert(compiled.cache_key, std::sync::Arc::new(compiled.clone()));
    assert!(cache.get(&compiled.cache_key).is_some());
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().misses, 1);

    let cancelled = AtomicBool::new(true);
    let error = execute(QueryRequest {
        compiled: &compiled,
        graph: &graph,
        parameters: &parameters,
        limits: QueryLimits::interactive(),
        cancellation: &cancelled,
    })
    .err()
    .ok_or("cancelled execution unexpectedly succeeded")?;
    assert_eq!(error.kind(), QueryErrorKind::Cancelled);

    let active = AtomicBool::new(false);
    let error = execute(QueryRequest {
        compiled: &compiled,
        graph: &graph,
        parameters: &parameters,
        limits: QueryLimits {
            max_memory_bytes: 1,
            ..QueryLimits::interactive()
        },
        cancellation: &active,
    })
    .err()
    .ok_or("memory-limited execution unexpectedly succeeded")?;
    assert_eq!(error.kind(), QueryErrorKind::MemoryLimit);
    Ok(())
}

#[test]
fn graph_mapping_separates_portable_and_snapshot_local_ids() -> Result<(), Box<dyn Error>> {
    let result =
        run("MATCH (n {id:'a'}) RETURN n.id AS portable, id(n) AS local, labels(n) AS labels")?;
    assert_eq!(result.rows[0][0], CompassValue::String("a".into()));
    assert_eq!(result.rows[0][1], CompassValue::Integer(0));
    assert_eq!(
        result.rows[0][2],
        CompassValue::List(vec![CompassValue::String("Function".into())].into())
    );
    Ok(())
}
