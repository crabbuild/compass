//! Executable evidence for the selected scenarios in `tests/opencypher-tck/manifest.toml`.

use std::error::Error;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use compass_cypher::{
    CompassValue, CompileLimits, CompileRequest, ParameterTypes, Parameters, compile,
};
use compass_model::{Graph, GraphDocument};
use compass_query::{QueryLimits, QueryRequest, QueryResult, execute};

const MATCH: &str =
    include_str!("../../../tests/opencypher-tck/features/clauses/match/Match1.feature");
const RETURN: &str =
    include_str!("../../../tests/opencypher-tck/features/clauses/return/Return1.feature");
const UNION: &str =
    include_str!("../../../tests/opencypher-tck/features/clauses/union/Union1.feature");
const UNWIND: &str =
    include_str!("../../../tests/opencypher-tck/features/clauses/unwind/Unwind1.feature");
const WITH: &str =
    include_str!("../../../tests/opencypher-tck/features/clauses/with/With1.feature");
const AGGREGATION: &str = include_str!(
    "../../../tests/opencypher-tck/features/expressions/aggregation/Aggregation1.feature"
);
const BOOLEAN: &str =
    include_str!("../../../tests/opencypher-tck/features/expressions/boolean/Boolean1.feature");
const LIST: &str =
    include_str!("../../../tests/opencypher-tck/features/expressions/list/List1.feature");
const PATH: &str =
    include_str!("../../../tests/opencypher-tck/features/expressions/path/Path1.feature");
const EXISTS: &str = include_str!(
    "../../../tests/opencypher-tck/features/expressions/existentialSubqueries/ExistentialSubquery1.feature"
);

fn scenario_query(feature: &str, id: usize) -> Result<&str, Box<dyn Error>> {
    let marker = format!("Scenario: [{id}]");
    let scenario = feature
        .split_once(&marker)
        .ok_or_else(|| format!("scenario {id} is absent"))?
        .1;
    let body = scenario
        .split_once("When executing query:")
        .ok_or_else(|| format!("scenario {id} has no query"))?
        .1;
    let quoted = body
        .split_once("\"\"\"")
        .ok_or_else(|| format!("scenario {id} has no opening query delimiter"))?
        .1;
    Ok(quoted
        .split_once("\"\"\"")
        .ok_or_else(|| format!("scenario {id} has no closing query delimiter"))?
        .0
        .trim())
}

fn graph(json: &str) -> Result<Graph, Box<dyn Error>> {
    Ok(Graph::from_document(
        serde_json::from_str::<GraphDocument>(json)?,
    )?)
}

fn empty_graph() -> Result<Graph, Box<dyn Error>> {
    graph(r#"{"directed":true,"multigraph":true,"graph":{},"nodes":[],"links":[]}"#)
}

fn run(graph: &Graph, query: &str, parameters: &Parameters) -> Result<QueryResult, Box<dyn Error>> {
    let parameter_types = parameters
        .iter()
        .map(|(name, value)| (name.clone(), value.compass_type()))
        .collect::<ParameterTypes>();
    let compiled = compile(CompileRequest {
        source_name: "openCypher-TCK.feature",
        source: query,
        parameter_types: &parameter_types,
        schema: &graph.schema_fingerprint(),
        limits: CompileLimits::default(),
    })?;
    let cancellation = AtomicBool::new(false);
    Ok(execute(QueryRequest {
        compiled: &compiled,
        graph,
        parameters,
        limits: QueryLimits {
            deadline: Instant::now() + Duration::from_secs(2),
            max_rows: 10_000,
            max_path_depth: 32,
            max_expanded_relationships: 100_000,
            max_memory_bytes: 32 * 1024 * 1024,
        },
        cancellation: &cancellation,
    })?)
}

fn integers(result: &QueryResult) -> Result<Vec<i64>, Box<dyn Error>> {
    result
        .rows
        .iter()
        .map(|row| match row.first() {
            Some(CompassValue::Integer(value)) => Ok(*value),
            value => Err(format!("expected integer, got {value:?}").into()),
        })
        .collect()
}

#[test]
fn selected_union_and_unwind_scenarios_execute_verbatim() -> Result<(), Box<dyn Error>> {
    let graph = empty_graph()?;
    let params = Parameters::new();
    for (id, expected) in [(1, vec![1, 2]), (2, vec![1, 2]), (3, vec![1, 2, 3, 4])] {
        let mut actual = integers(&run(&graph, scenario_query(UNION, id)?, &params)?)?;
        actual.sort_unstable();
        assert_eq!(actual, expected, "Union1 scenario {id}");
    }
    let union_error = compile(CompileRequest {
        source_name: "Union1.feature",
        source: scenario_query(UNION, 5)?,
        parameter_types: &ParameterTypes::new(),
        schema: &graph.schema_fingerprint(),
        limits: CompileLimits::default(),
    });
    let Err(union_error) = union_error else {
        return Err("different UNION columns compiled successfully".into());
    };
    assert_eq!(union_error.items()[0].code(), "CQL2014");

    for (id, rows) in [
        (1, 3),
        (3, 6),
        (7, 6),
        (8, 0),
        (9, 0),
        (10, 10),
        (11, 3),
        (13, 8),
    ] {
        let result = run(&graph, scenario_query(UNWIND, id)?, &params)?;
        assert_eq!(result.rows.len(), rows, "Unwind1 scenario {id}");
    }
    let wildcard = run(&graph, scenario_query(UNWIND, 11)?, &params)?;
    assert_eq!(
        wildcard
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["list", "x"]
    );
    Ok(())
}

#[test]
fn selected_boolean_and_list_scenarios_execute_verbatim() -> Result<(), Box<dyn Error>> {
    let graph = empty_graph()?;
    for id in 1..=5 {
        let result = run(&graph, scenario_query(BOOLEAN, id)?, &Parameters::new())?;
        assert!(!result.rows.is_empty(), "Boolean1 scenario {id}");
        assert!(result.rows.iter().flatten().all(|value| {
            matches!(
                value,
                CompassValue::Boolean(true | false) | CompassValue::Null
            )
        }));
    }
    let first = run(&graph, scenario_query(LIST, 1)?, &Parameters::new())?;
    assert_eq!(first.rows[0][0], CompassValue::Integer(1));
    let nested = run(&graph, scenario_query(LIST, 2)?, &Parameters::new())?;
    assert_eq!(nested.columns[0].name, "[[1]][0][0]");
    assert_eq!(nested.rows[0][0], CompassValue::Integer(1));

    let mut both = Parameters::new();
    both.insert(
        "expr".to_owned(),
        CompassValue::List(vec![CompassValue::String("Apa".into())].into()),
    );
    both.insert("idx".to_owned(), CompassValue::Integer(0));
    for id in [3, 5] {
        let result = run(&graph, scenario_query(LIST, id)?, &both)?;
        assert_eq!(result.rows[0][0], CompassValue::String("Apa".into()));
    }
    let mut index = Parameters::new();
    index.insert("idx".to_owned(), CompassValue::Integer(0));
    let result = run(&graph, scenario_query(LIST, 4)?, &index)?;
    assert_eq!(result.rows[0][0], CompassValue::String("Apa".into()));
    Ok(())
}

#[test]
fn selected_match_return_and_aggregation_scenarios_execute_verbatim() -> Result<(), Box<dyn Error>>
{
    let empty = empty_graph()?;
    assert!(
        run(&empty, scenario_query(MATCH, 1)?, &Parameters::new())?
            .rows
            .is_empty()
    );

    let names = graph(
        r#"{"directed":true,"multigraph":true,"graph":{},"nodes":[
          {"id":"bar","name":"bar"},{"id":"monkey","name":"monkey"},{"id":"first","firstname":"bar"}
        ],"links":[]}"#,
    )?;
    let matched = run(&names, scenario_query(MATCH, 4)?, &Parameters::new())?;
    assert_eq!(matched.rows.len(), 1);

    let numbers = graph(
        r#"{"directed":true,"multigraph":true,"graph":{},"nodes":[
          {"id":"one","num":1},{"id":"two","num":2},{"id":"three","num":3}
        ],"links":[]}"#,
    )?;
    assert_eq!(
        run(&numbers, scenario_query(MATCH, 5)?, &Parameters::new())?
            .rows
            .len(),
        9
    );

    let list_property = graph(
        r#"{"directed":true,"multigraph":true,"graph":{},"nodes":[
          {"id":"list","numbers":[1,2,3]}
        ],"links":[]}"#,
    )?;
    assert_eq!(
        run(
            &list_property,
            scenario_query(RETURN, 1)?,
            &Parameters::new()
        )?
        .rows
        .len(),
        1
    );
    let undefined = compile(CompileRequest {
        source_name: "Return1.feature",
        source: scenario_query(RETURN, 2)?,
        parameter_types: &ParameterTypes::new(),
        schema: &empty.schema_fingerprint(),
        limits: CompileLimits::default(),
    });
    let Err(undefined) = undefined else {
        return Err("undefined variable compiled successfully".into());
    };
    assert_eq!(undefined.items()[0].code(), "CQL2004");

    let aggregate = graph(
        r#"{"directed":true,"multigraph":true,"graph":{},"nodes":[
          {"id":"a1","name":"a","num":33},{"id":"a2","name":"a"},{"id":"b","name":"b","num":42}
        ],"links":[]}"#,
    )?;
    let result = run(
        &aggregate,
        scenario_query(AGGREGATION, 1)?,
        &Parameters::new(),
    )?;
    assert_eq!(result.columns[1].name, "count(n.num)");
    assert_eq!(result.rows.len(), 2);
    assert!(
        result
            .rows
            .iter()
            .all(|row| row[1] == CompassValue::Integer(1))
    );
    Ok(())
}

#[test]
fn selected_path_with_and_exists_scenarios_execute_verbatim() -> Result<(), Box<dyn Error>> {
    let empty = empty_graph()?;
    assert!(
        run(&empty, scenario_query(WITH, 5)?, &Parameters::new())?
            .rows
            .is_empty()
    );
    let path = run(&empty, scenario_query(PATH, 1)?, &Parameters::new())?;
    assert_eq!(
        path.columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["nodes(p)", "nodes(null)"]
    );
    assert_eq!(path.rows[0], [CompassValue::Null, CompassValue::Null]);

    let exists_graph = graph(
        r#"{"directed":true,"multigraph":true,"graph":{},"nodes":[
          {"id":"a","prop":1},{"id":"b","prop":1},{"id":"c","prop":2},{"id":"d","prop":3}
        ],"links":[
          {"source":"a","target":"b","relation":"R"},
          {"source":"a","target":"c","relation":"R"},
          {"source":"a","target":"d","relation":"R"}
        ]}"#,
    )?;
    for (id, expected) in [(1, 1), (3, 0), (4, 0)] {
        let result = run(
            &exists_graph,
            scenario_query(EXISTS, id)?,
            &Parameters::new(),
        )?;
        assert_eq!(
            result.rows.len(),
            expected,
            "ExistentialSubquery1 scenario {id}"
        );
    }
    let exists_where_graph = graph(
        r#"{"directed":true,"multigraph":true,"graph":{},"nodes":[
          {"id":"a","prop":1},{"id":"b","prop":1},{"id":"c","prop":2},{"id":"d"}
        ],"links":[
          {"source":"a","target":"b","relation":"R"},
          {"source":"a","target":"c","relation":"R"},
          {"source":"a","target":"d","relation":"R"},
          {"source":"b","target":"d","relation":"R"}
        ]}"#,
    )?;
    assert_eq!(
        run(
            &exists_where_graph,
            scenario_query(EXISTS, 2)?,
            &Parameters::new()
        )?
        .rows
        .len(),
        1,
    );
    Ok(())
}
