#![no_main]

use compass_cypher::{CompileLimits, CompileRequest, ParameterTypes, compile};
use compass_model::SchemaFingerprint;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|source: &str| {
    let parameters = ParameterTypes::new();
    let schema = SchemaFingerprint::empty();
    let _result = compile(CompileRequest {
        source_name: "fuzz.cypher",
        source,
        parameter_types: &parameters,
        schema: &schema,
        limits: CompileLimits::default(),
    });
});
