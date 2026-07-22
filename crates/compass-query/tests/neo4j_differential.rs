use std::collections::BTreeMap;
use std::error::Error;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use compass_cypher::{
    CompassValue, CompileLimits, CompileRequest, ParameterTypes, Parameters, compile,
};
use compass_graphdb::{push_to_neo4j, query_neo4j};
use compass_model::{Graph, GraphDocument};
use compass_query::{QueryLimits, QueryRequest, execute};

#[test]
fn accepted_scalar_projection_matches_neo4j_when_configured() -> Result<(), Box<dyn Error>> {
    let uri = std::env::var("COMPASS_NEO4J_URI").ok();
    let user = std::env::var("COMPASS_NEO4J_USER").ok();
    let password = std::env::var("COMPASS_NEO4J_PASSWORD").ok();
    if uri.is_none() && user.is_none() && password.is_none() {
        eprintln!("skipped Neo4j differential: COMPASS_NEO4J_* credentials are absent");
        return Ok(());
    }
    let (Some(uri), Some(user), Some(password)) = (uri, user, password) else {
        return Err(
            "set all or none of COMPASS_NEO4J_URI, COMPASS_NEO4J_USER, and COMPASS_NEO4J_PASSWORD"
                .into(),
        );
    };
    let document = serde_json::from_str::<GraphDocument>(
        r#"{
          "directed":true,"multigraph":true,"graph":{},
          "nodes":[
            {"id":"compass-cql-differential-a","label":"a()","file_type":"function"},
            {"id":"compass-cql-differential-b","label":"b()","file_type":"function"}
          ],
          "links":[
            {"source":"compass-cql-differential-a","target":"compass-cql-differential-b","relation":"calls","confidence":"EXTRACTED"}
          ]
        }"#,
    )?;
    push_to_neo4j(&document, &uri, &user, &password, None)?;
    let graph = Graph::from_document(document)?;
    let source = "MATCH (a {id:'compass-cql-differential-a'})-[:CALLS]->(b) \
                  RETURN a.id AS source, b.id AS target ORDER BY source, target";
    let parameter_types = ParameterTypes::new();
    let compiled = compile(CompileRequest {
        source_name: "neo4j-differential.cypher",
        source,
        parameter_types: &parameter_types,
        schema: &graph.schema_fingerprint(),
        limits: CompileLimits::default(),
    })?;
    let cancellation = AtomicBool::new(false);
    let local = execute(QueryRequest {
        compiled: &compiled,
        graph: &graph,
        parameters: &Parameters::new(),
        limits: QueryLimits {
            deadline: Instant::now() + Duration::from_secs(5),
            max_rows: 100,
            max_path_depth: 4,
            max_expanded_relationships: 1_000,
            max_memory_bytes: 1024 * 1024,
        },
        cancellation: &cancellation,
    })?;
    let remote = query_neo4j(&uri, &user, &password, source, &BTreeMap::new())?;
    assert_eq!(
        local
            .columns
            .iter()
            .map(|column| &column.name)
            .collect::<Vec<_>>(),
        remote.columns.iter().collect::<Vec<_>>()
    );
    let local_rows = local
        .rows
        .iter()
        .map(|row| row.iter().map(stable_scalar).collect::<Result<Vec<_>, _>>())
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(local_rows, remote.rows);
    Ok(())
}

fn stable_scalar(value: &CompassValue) -> Result<serde_json::Value, Box<dyn Error>> {
    match value {
        CompassValue::Null => Ok(serde_json::Value::Null),
        CompassValue::Boolean(value) => Ok(serde_json::Value::Bool(*value)),
        CompassValue::Integer(value) => Ok(serde_json::Value::from(*value)),
        CompassValue::Float(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .ok_or_else(|| "non-finite local float".into()),
        CompassValue::String(value) => Ok(serde_json::Value::String(value.to_string())),
        _ => Err("differential query must project stable scalar values".into()),
    }
}
