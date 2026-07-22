#![no_main]

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use compass_cypher::{
    CompassType, CompassValue, CompileLimits, CompileRequest, ParameterTypes, Parameters, compile,
};
use compass_model::{Graph, GraphDocument};
use compass_query::{QueryLimits, QueryRequest, execute};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 16 * 1024 * 1024 {
        return;
    }
    if let Ok(Value::Object(values)) = serde_json::from_slice::<Value>(bytes) {
        let converted = values
            .into_iter()
            .map(|(key, value)| convert(value).map(|value| (key, value)))
            .collect::<Option<BTreeMap<_, _>>>();
        let Some(converted) = converted else {
            return;
        };
        let Some(graph) = graph() else {
            return;
        };
        let parameters = Parameters::from([(
            "payload".to_owned(),
            CompassValue::Map(converted.into()),
        )]);
        let parameter_types = ParameterTypes::from([("payload".to_owned(), CompassType::Map)]);
        let Ok(compiled) = compile(CompileRequest {
            source_name: "fuzz-params.cypher",
            source: "RETURN $payload AS payload",
            parameter_types: &parameter_types,
            schema: &graph.schema_fingerprint(),
            limits: CompileLimits::default(),
        }) else {
            return;
        };
        let cancellation = AtomicBool::new(false);
        let _result = execute(QueryRequest {
            compiled: &compiled,
            graph,
            parameters: &parameters,
            limits: QueryLimits {
                deadline: Instant::now() + Duration::from_millis(100),
                max_rows: 10,
                max_path_depth: 4,
                max_expanded_relationships: 100,
                max_memory_bytes: 16 * 1024 * 1024,
            },
            cancellation: &cancellation,
        });
    }
});

fn graph() -> Option<&'static Graph> {
    static GRAPH: OnceLock<Option<Graph>> = OnceLock::new();
    GRAPH
        .get_or_init(|| {
            let document = serde_json::from_str::<GraphDocument>(
                r#"{"directed":true,"multigraph":true,"graph":{},"nodes":[],"links":[]}"#,
            )
            .ok()?;
            Graph::from_document(document).ok()
        })
        .as_ref()
}

fn convert(value: Value) -> Option<CompassValue> {
    match value {
        Value::Null => Some(CompassValue::Null),
        Value::Bool(value) => Some(CompassValue::Boolean(value)),
        Value::Number(value) => value
            .as_i64()
            .map(CompassValue::Integer)
            .or_else(|| {
                value
                    .as_u64()
                    .and_then(|value| i64::try_from(value).ok())
                    .map(CompassValue::Integer)
            })
            .or_else(|| {
                (!value.is_u64())
                    .then(|| value.as_f64())
                    .flatten()
                    .filter(|value| value.is_finite())
                    .map(CompassValue::Float)
            }),
        Value::String(value) => Some(CompassValue::String(value.into())),
        Value::Array(values) => values
            .into_iter()
            .map(convert)
            .collect::<Option<Vec<_>>>()
            .map(|values| CompassValue::List(values.into())),
        Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| convert(value).map(|value| (key, value)))
            .collect::<Option<BTreeMap<_, _>>>()
            .map(|values| CompassValue::Map(values.into())),
    }
}
