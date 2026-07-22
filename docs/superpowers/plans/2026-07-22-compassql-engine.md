# CompassQL Native Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the complete CompassQL 1 read-only openCypher subset and expose it through compass query --cql only after conformance, safety, and performance gates pass.

**Architecture:** A new compass-cypher crate owns source parsing, the Compass AST, semantic analysis, logical plans, and deterministic optimization. compass-query executes typed physical operators directly over immutable compass-model graph snapshots; compass-output renders typed rows; compass-cli adds an explicit --cql mode without changing Graphify-compatible natural-language query behavior.

**Tech Stack:** Rust 2024, Rust 1.97.1, serde/serde_json, regex, sha2, existing Compass graph/query/output/CLI crates, Apache-2.0 openCypher 2024.3 grammar and TCK fixtures.

## Global Constraints

- CompassQL is a documented read-only openCypher subset; never claim full openCypher, Neo4j Cypher, or ISO GQL compatibility.
- The production parser and AST are Compass-owned; no alpha parser crate is a runtime dependency.
- Execution is local and deterministic and never invokes a model, filesystem, network, process, environment variable, or external database.
- Do not add a secondary graph store or copy the full graph for each query.
- compass query without --cql remains byte-compatible with the existing Compass and Graphify behavior.
- The graphify compatibility executable must reject --cql unless the Python oracle independently gains that flag.
- Every variable-length path has an explicit upper bound no greater than 32.
- Default interactive limits are 5 seconds, 10,000 returned rows, 5,000,000 expanded relationships, and 256 MiB working memory.
- A limit, timeout, cancellation, or execution error never returns a successful partial result.
- Preserve unsafe_code = forbid, clippy all/unwrap_used/expect_used/panic = deny, Rust 1.97.1, and standalone cross-platform binaries.
- Cover Linux, macOS, and Windows on x86-64 and ARM64.
- Use red-green-refactor, keep commits task-sized, and run graphify update . after code changes are complete.

---

## Plan sequence

This is plan 1 of 3. Complete it before:

1. docs/superpowers/plans/2026-07-22-compassql-policy.md
2. docs/superpowers/plans/2026-07-22-compassql-integrations.md

This plan produces a complete testable compass query --cql command. The policy plan consumes its public query interface. The integration plan consumes both.

## File and crate map

Create compass-cypher with these responsibilities:

~~~text
crates/compass-cypher/
├── Cargo.toml
├── src/
│   ├── lib.rs          public compile and diagnostic interface
│   ├── span.rs         byte spans and source locations
│   ├── diagnostic.rs   stable CQL1xxx-CQL4xxx diagnostics
│   ├── token.rs        bounded token model
│   ├── lexer.rs        case-insensitive keyword lexer
│   ├── ast.rs          Compass-owned query AST
│   ├── parser.rs       clauses, patterns, and Pratt expressions
│   ├── value.rs        CompassQL value/type/null model
│   ├── semantic.rs     scopes, types, functions, and read-only validation
│   ├── plan.rs         logical operators and typed columns
│   ├── optimize.rs     deterministic rewrite rules
│   └── support.rs      generated support-matrix records
└── tests/
    ├── lexer.rs
    ├── parser.rs
    ├── semantic.rs
    ├── planning.rs
    └── unsupported.rs
~~~

Extend existing crates:

- crates/compass-model/src/query_index.rs: immutable label, relation, display-label, source-file, and typed adjacency indexes.
- crates/compass-query/src/cql/: budgets, rows, expression evaluation, physical operators, path search, execution, profiling, and plan cache.
- crates/compass-output/src/cql.rs: table, JSON, and JSONL typed-row rendering.
- crates/compass-cli/src/query_commands.rs: shared natural-language and --cql parsing, source selection, parameters, REPL, and dispatch.
- tests/opencypher-tck/: pinned Apache-2.0 read-only feature files and provenance.
- scripts/check_compassql_support.py: support-matrix/TCK coverage check.
- scripts/benchmark_compassql.sh: cold and warm release-mode gates.
- fuzz/fuzz_targets/cql_source.rs and cql_params.rs: hostile parser and parameter inputs.

## Shared public interfaces

Later tasks and plans use these exact interfaces:

~~~rust
pub const LANGUAGE_VERSION: u16 = 1;

pub struct CompileRequest<'a> {
    pub source_name: &'a str,
    pub source: &'a str,
    pub parameter_types: &'a ParameterTypes,
    pub schema: &'a compass_model::SchemaFingerprint,
    pub limits: CompileLimits,
}

pub struct CompiledQuery {
    pub plan: LogicalPlan,
    pub columns: Vec<Column>,
    pub profile: QueryProfileMode,
    pub cache_key: PlanCacheKey,
}

pub fn compile(request: CompileRequest<'_>) -> Result<CompiledQuery, Diagnostics>;

#[derive(Clone, Debug, PartialEq)]
pub enum CompassValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(std::sync::Arc<str>),
    List(std::sync::Arc<[CompassValue]>),
    Map(std::sync::Arc<std::collections::BTreeMap<String, CompassValue>>),
    Node(NodeRef),
    Relationship(RelationshipRef),
    Path(PathRef),
}

pub struct QueryLimits {
    pub deadline: std::time::Instant,
    pub max_rows: usize,
    pub max_path_depth: usize,
    pub max_expanded_relationships: u64,
    pub max_memory_bytes: usize,
}

pub struct QueryRequest<'a> {
    pub compiled: &'a CompiledQuery,
    pub graph: &'a compass_model::Graph,
    pub parameters: &'a Parameters,
    pub limits: QueryLimits,
    pub cancellation: &'a std::sync::atomic::AtomicBool,
}

pub struct QueryResult {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub profile: Option<QueryProfile>,
}

pub fn execute(request: QueryRequest<'_>) -> Result<QueryResult, QueryError>;
~~~

## Test fixture contract

Create these concrete helpers with the first task that uses them:

- crates/compass-cypher/tests/support/mod.rs exports empty_schema(), parse_source(), and compile_query(); compile_query derives a SchemaFingerprint from fixture_graph() and uses default types/limits.
- crates/compass-query/tests/support/mod.rs exports fixture_graph(), wide_graph(), compiled_query(), execute_query(), test_limits(), and a non-cancelled AtomicBool.
- crates/compass-cli/tests/support/mod.rs exports CliFixture::new, write_graph, run_compass, run_graphify, and a CommandResult exposing code(), stdout(), and stderr().
- All graphs use explicit deterministic node/relationship IDs and BTreeMap properties. Helpers return public production types and contain no query behavior of their own.

### Task 1: Scaffold compass-cypher and lock language contracts

**Files:**
- Modify: Cargo.toml
- Modify: Cargo.lock
- Create: crates/compass-cypher/Cargo.toml
- Create: crates/compass-cypher/src/lib.rs
- Create: crates/compass-cypher/src/span.rs
- Create: crates/compass-cypher/src/diagnostic.rs
- Create: crates/compass-cypher/src/value.rs
- Create: crates/compass-cypher/tests/contracts.rs

**Interfaces:**
- Consumes: compass-model node and edge indices.
- Produces: LANGUAGE_VERSION, Span, Diagnostic, Diagnostics, CompassType, ParameterTypes, CompileLimits, and the compile signature above.

- [ ] **Step 1: Write the failing crate contract test**

~~~rust
use compass_cypher::{
    CompileLimits, CompileRequest, LANGUAGE_VERSION, ParameterTypes, compile,
};

#[test]
fn language_version_and_empty_query_diagnostic_are_stable() {
    assert_eq!(LANGUAGE_VERSION, 1);
    let parameters = ParameterTypes::default();
    let error = compile(CompileRequest {
        source_name: "query.cypher",
        source: "",
        parameter_types: &parameters,
        schema: &empty_schema(),
        limits: CompileLimits::default(),
    })
    .expect_err("empty source must fail");
    assert_eq!(error.items()[0].code(), "CQL1001");
    assert_eq!(error.items()[0].span().start, 0);
}
~~~

- [ ] **Step 2: Run the test and verify the package is absent**

Run: cargo test -p compass-cypher --test contracts

Expected: FAIL because package compass-cypher does not exist.

- [ ] **Step 3: Add the workspace member and focused manifest**

Add crates/compass-cypher to workspace members. The crate depends on serde, serde_json, regex, sha2, thiserror, and compass-model through workspace/path dependencies and inherits workspace lints.

Define Span as two byte offsets, Diagnostic with code/message/span/help, Diagnostics as a non-empty vector, and CompassType as the exact value variants in the design. CompileLimits defaults to 1 MiB source, 100,000 tokens, nesting 256, and path ceiling 32.

- [ ] **Step 4: Add the minimal compile entry point**

~~~rust
pub fn compile(request: CompileRequest<'_>) -> Result<CompiledQuery, Diagnostics> {
    if request.source.trim().is_empty() {
        return Err(Diagnostics::single(Diagnostic::new(
            "CQL1001",
            "query source is empty",
            Span::new(0, 0),
        )));
    }
    parser::parse(request)
        .and_then(semantic::analyze)
        .map(optimize::optimize)
}
~~~

Keep parser, semantic, and optimize private. Define parse(CompileRequest) -> Result<QueryAst, Diagnostics>, analyze(QueryAst, &ParameterTypes, &SchemaFingerprint) -> Result<LogicalPlan, Diagnostics>, and optimize(LogicalPlan) -> LogicalPlan. Until Tasks 3-4 land, parse returns CQL1002 for every non-empty source; do not expose compass query --cql yet.

- [ ] **Step 5: Run focused quality gates**

Run: cargo test -p compass-cypher --test contracts && cargo clippy -p compass-cypher --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add Cargo.toml Cargo.lock crates/compass-cypher
git commit -m "feat(cql): establish language contracts"
~~~

### Task 2: Implement bounded lexing and source diagnostics

**Files:**
- Create: crates/compass-cypher/src/token.rs
- Create: crates/compass-cypher/src/lexer.rs
- Modify: crates/compass-cypher/src/lib.rs
- Create: crates/compass-cypher/tests/lexer.rs

**Interfaces:**
- Consumes: Span, Diagnostic, CompileLimits.
- Produces: Token, TokenKind, lex(source, limits) -> Result<Vec<Token>, Diagnostics>.

- [ ] **Step 1: Write lexer golden and hostile-input tests**

~~~rust
use compass_cypher::{CompileLimits, TokenKind, lex};

#[test]
fn keywords_are_case_insensitive_and_identifiers_preserve_text() {
    let tokens = lex(
        "mAtCh (n:Function)-[r:CALLS]->(x) RETURN n.label",
        CompileLimits::default(),
    )
    .expect("lex");
    assert_eq!(tokens[0].kind, TokenKind::Match);
    assert!(tokens.iter().any(|token| token.text == "Function"));
    assert!(tokens.iter().any(|token| token.text == "CALLS"));
}

#[test]
fn unterminated_string_and_token_limit_have_stable_codes() {
    let string_error = lex("'unterminated", CompileLimits::default()).expect_err("fail");
    assert_eq!(string_error.items()[0].code(), "CQL1003");
    let limits = CompileLimits { max_tokens: 2, ..CompileLimits::default() };
    let limit_error = lex("MATCH (n) RETURN n", limits).expect_err("fail");
    assert_eq!(limit_error.items()[0].code(), "CQL3001");
}
~~~

- [ ] **Step 2: Run the test and verify lex is absent**

Run: cargo test -p compass-cypher --test lexer

Expected: FAIL with unresolved imports.

- [ ] **Step 3: Implement the token model and scanner**

TokenKind includes punctuation/operators, Identifier, Parameter, String, Integer, Float, and every supported or recognized-for-rejection keyword. The scanner advances by UTF-8 char boundaries, stores byte spans, decodes Cypher string escapes, rejects NUL/control characters, bounds token and literal counts, and emits one EOF token.

Mutation and administration keywords must be distinct token kinds so semantic validation can report unsupported read-only syntax rather than a generic parse failure.

- [ ] **Step 4: Run lexer, clippy, and size-limit tests**

Run: cargo test -p compass-cypher --test lexer && cargo clippy -p compass-cypher --all-targets -- -D warnings

Expected: PASS with no panics for malformed UTF-8 boundaries because input is Rust str.

- [ ] **Step 5: Commit**

~~~bash
git add crates/compass-cypher/src crates/compass-cypher/tests/lexer.rs
git commit -m "feat(cql): add bounded lexer"
~~~

### Task 3: Parse CompassQL Core into a Compass-owned AST

**Files:**
- Create: crates/compass-cypher/src/ast.rs
- Create: crates/compass-cypher/src/parser.rs
- Modify: crates/compass-cypher/src/lib.rs
- Create: crates/compass-cypher/tests/parser.rs
- Create: crates/compass-cypher/tests/unsupported.rs

**Interfaces:**
- Consumes: lex output.
- Produces: QueryAst, QueryPart, Clause, Pattern, Expr, FunctionCall, Projection, SortItem, parse(CompileRequest) -> Result<QueryAst, Diagnostics>.

- [ ] **Step 1: Write representative full-surface parser tests**

~~~rust
use compass_cypher::{CompileLimits, CompileRequest, ParameterTypes, parse_only};

#[test]
fn parses_core_pipeline_and_bounded_path() {
    let parameters = ParameterTypes::default();
    let ast = parse_only(CompileRequest {
        source_name: "policy.cypher",
        source: "MATCH p=(a:File)-[:CALLS|IMPORTS_FROM*1..8]->(b:File) \
                 WHERE a.source_file STARTS WITH 'src/domain/' \
        WITH p, b WHERE b IS NOT NULL RETURN DISTINCT p AS witness ORDER BY b.id LIMIT 10",
        parameter_types: &parameters,
        schema: &empty_schema(),
        limits: CompileLimits::default(),
    })
    .expect("parse");
    assert_eq!(ast.parts.len(), 1);
    assert_eq!(ast.parts[0].clauses.len(), 5);
}

#[test]
fn recognizes_mutation_for_precise_rejection() {
    let error = parse_source("MATCH (n) DELETE n").expect_err("read only");
    assert_eq!(error.items()[0].code(), "CQL1007");
    assert!(error.items()[0].message().contains("DELETE"));
}
~~~

- [ ] **Step 2: Run parser tests and verify failures**

Run: cargo test -p compass-cypher --test parser --test unsupported

Expected: FAIL because AST and parser are absent.

- [ ] **Step 3: Define the complete CompassQL 1 AST**

Represent MATCH/OPTIONAL MATCH, WHERE, UNWIND, WITH, RETURN, UNION/UNION ALL, ORDER BY, SKIP, LIMIT, EXPLAIN, PROFILE, correlated EXISTS, patterns, bounded variable relationships, shortestPath/allShortestPaths, property/list/map literals, parameters, CASE, list predicates, aggregate calls, indexing, and slicing. Every AST node carries Span.

Use a Pratt parser with an explicit precedence table:

~~~rust
const PRECEDENCE: &[(BinaryOp, u8)] = &[
    (BinaryOp::Or, 1),
    (BinaryOp::Xor, 2),
    (BinaryOp::And, 3),
    (BinaryOp::Equal, 4),
    (BinaryOp::NotEqual, 4),
    (BinaryOp::Less, 4),
    (BinaryOp::LessOrEqual, 4),
    (BinaryOp::Greater, 4),
    (BinaryOp::GreaterOrEqual, 4),
    (BinaryOp::In, 4),
    (BinaryOp::StartsWith, 4),
    (BinaryOp::EndsWith, 4),
    (BinaryOp::Contains, 4),
    (BinaryOp::RegexMatch, 4),
    (BinaryOp::Add, 5),
    (BinaryOp::Subtract, 5),
    (BinaryOp::Multiply, 6),
    (BinaryOp::Divide, 6),
    (BinaryOp::Modulo, 6),
    (BinaryOp::Power, 7),
];
~~~

Treat comparisons as non-associative. Parse NOT between comparison and AND; numeric unary +/- above multiplicative; property access, labels, IS NULL/IS NOT NULL, indexing, and slicing as highest-precedence postfix operators. Power is right-associative and every other binary operator is left-associative. Tests cover one expression at every adjacent precedence boundary.

- [ ] **Step 4: Implement clause, pattern, and expression parsing**

Require an upper bound for every variable relationship during parsing and reject bounds above 32 with CQL3002. Parse valid but unsupported clauses into UnsupportedClause and immediately emit CQL1007 with the clause span. Reject trailing tokens, duplicate query prefixes, incompatible UNION columns at semantic time, and nested EXISTS during parse with CQL1008.

- [ ] **Step 5: Run the parser suite**

Run: cargo test -p compass-cypher --test parser --test unsupported && cargo clippy -p compass-cypher --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-cypher/src crates/compass-cypher/tests
git commit -m "feat(cql): parse read-only core"
~~~

### Task 4: Add semantic scopes, types, functions, and logical plans

**Files:**
- Create: crates/compass-cypher/src/semantic.rs
- Create: crates/compass-cypher/src/plan.rs
- Create: crates/compass-cypher/src/support.rs
- Modify: crates/compass-cypher/src/lib.rs
- Create: crates/compass-cypher/tests/semantic.rs
- Create: crates/compass-cypher/tests/planning.rs
- Create: docs/COMPASSQL_SUPPORT.md

**Interfaces:**
- Consumes: QueryAst and ParameterTypes.
- Produces: LogicalPlan, LogicalOperator, Column, CompassType, Nullability, FunctionId, compile.

- [ ] **Step 1: Write scope, null, aggregation, and plan tests**

~~~rust
#[test]
fn with_resets_scope_and_optional_values_are_nullable() {
    let error = compile_query(
        "MATCH (n) OPTIONAL MATCH (n)-[:CALLS]->(service) WITH n RETURN service"
    )
    .expect_err("service left scope");
    assert_eq!(error.items()[0].code(), "CQL2004");
}

#[test]
fn compiles_aggregate_pipeline_to_typed_operators() {
    let compiled = compile_query(
        "MATCH (m:File)-[:IMPORTS_FROM]->(d:File) \
         WITH m, count(DISTINCT d) AS dependencies \
         WHERE dependencies > 12 RETURN m.id, dependencies"
    )
    .expect("compile");
    assert!(compiled.plan.contains_operator("Aggregate"));
    assert!(compiled.plan.contains_operator("Filter"));
}
~~~

- [ ] **Step 2: Run semantic tests and verify failures**

Run: cargo test -p compass-cypher --test semantic --test planning

Expected: FAIL because semantic analysis and plan types are absent.

- [ ] **Step 3: Implement binding and type inference**

Track lexical scopes across clauses and EXISTS subqueries; bind repeated variables by graph identity; infer Null/Boolean/Integer/Float/String/List/Map/Node/Relationship/Path; preserve nullable OPTIONAL variables; apply openCypher three-valued logic and numeric coercion; enforce aggregate grouping; validate function arity and context; validate UNION names and compatible types.

Property access on Null yields Null. Access to an unknown variable is CQL2004. Unknown parameters are CQL2011. Out-of-range integer properties become CQL4006 only when read.

- [ ] **Step 4: Build typed logical operators**

Emit the exact logical operators in the design. Each operator records typed columns, estimated cardinality, memory estimate, ordering, and cancellation checkpoint frequency. compile returns CompiledQuery and PlanCacheKey derived from source digest, parameter type signature, language version, exact SchemaFingerprint, compile limits that affect planning, and planner version.

- [ ] **Step 5: Generate and test the support matrix**

docs/COMPASSQL_SUPPORT.md lists each accepted and rejected clause/expression/function/path feature with its CQL version and TCK scenario path. support.rs contains the machine-readable equivalent. A test fails if any parser feature lacks a support record.

Run: cargo test -p compass-cypher && cargo clippy -p compass-cypher --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-cypher docs/COMPASSQL_SUPPORT.md
git commit -m "feat(cql): analyze and plan typed queries"
~~~

### Task 5: Add immutable query indexes and graph-schema fingerprints

**Files:**
- Create: crates/compass-model/src/query_index.rs
- Modify: crates/compass-model/src/graph.rs
- Modify: crates/compass-model/src/lib.rs
- Modify: crates/compass-model/Cargo.toml
- Create: crates/compass-model/tests/query_index.rs

**Interfaces:**
- Consumes: Graph node/edge records.
- Produces: QueryIndexes, SchemaFingerprint, Graph::query_indexes, Graph::schema_fingerprint, typed relation adjacency accessors.

- [ ] **Step 1: Write index mapping and parallel-edge tests**

~~~rust
#[test]
fn query_indexes_match_export_labels_relations_and_defaults() {
    let graph = fixture_graph();
    let indexes = graph.query_indexes();
    assert_eq!(indexes.nodes_with_label("Function").collect::<Vec<_>>(), vec![0]);
    assert_eq!(indexes.nodes_with_source_file("src/auth.rs").collect::<Vec<_>>(), vec![0]);
    assert_eq!(
        indexes.outgoing_with_type(0, "CALLS").collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(indexes.edge_confidence(0), "EXTRACTED");
}
~~~

- [ ] **Step 2: Run and verify the query index is absent**

Run: cargo test -p compass-model --test query_index

Expected: FAIL with no query_indexes method.

- [ ] **Step 3: Build indexes once in Graph::from_parts**

Use stable BTreeMap keys with Vec index lists for normalized file_type labels, uppercase relation types, exact label, and exact source_file. Preserve input index order and parallel relationship indices. Add relation-filtered incoming/outgoing adjacency without cloning edge records.

Compute SchemaFingerprint as SHA-256 over graph directed/multigraph flags plus sorted label, relation, and property-name/type sets. Exclude node IDs and property values.

- [ ] **Step 4: Run model quality gates**

Run: cargo test -p compass-model --all-targets && cargo clippy -p compass-model --all-targets -- -D warnings

Expected: PASS and existing Graph behavior unchanged.

- [ ] **Step 5: Commit**

~~~bash
git add crates/compass-model
git commit -m "feat(cql): index graph snapshots"
~~~

### Task 6: Execute fixed patterns, predicates, projection, and parameters

**Files:**
- Create: crates/compass-query/src/cql/mod.rs
- Create: crates/compass-query/src/cql/row.rs
- Create: crates/compass-query/src/cql/budget.rs
- Create: crates/compass-query/src/cql/eval.rs
- Create: crates/compass-query/src/cql/execute.rs
- Modify: crates/compass-query/src/lib.rs
- Modify: crates/compass-query/Cargo.toml
- Create: crates/compass-query/tests/cql_fixed.rs

**Interfaces:**
- Consumes: CompiledQuery, Graph query indexes, Parameters, QueryLimits.
- Produces: Row, Parameters, QueryRequest, QueryResult, QueryError, execute.

- [ ] **Step 1: Write fixed-pattern execution tests**

~~~rust
#[test]
fn fixed_match_filters_projects_and_preserves_parallel_edges() {
    let graph = fixture_graph();
    let compiled = compile_query(
        "MATCH (a:Function)-[r:CALLS]->(b:Function) \
         WHERE r.confidence = $confidence RETURN a.id AS caller, b.id AS target"
    );
    let parameters = Parameters::from_iter([(
        "confidence".to_owned(),
        CompassValue::String("EXTRACTED".into()),
    )]);
    let result = execute_fixture(&graph, &compiled, &parameters).expect("execute");
    assert_eq!(result.columns[0].name, "caller");
    assert_eq!(result.rows.len(), 2);
}
~~~

- [ ] **Step 2: Run and verify execution interfaces are absent**

Run: cargo test -p compass-query --test cql_fixed

Expected: FAIL with unresolved CQL execution imports.

- [ ] **Step 3: Implement rows, budgets, and value conversion**

Row stores Arc<[CompassValue]>. MemoryBudget reserves before allocation and releases on drop. QueryBudget counts returned rows and relationship expansions, checks deadline and AtomicBool cancellation at operator checkpoints, and maps failures to CQL4xxx QueryError values.

Convert node/edge properties lazily. Missing properties return Null; edge confidence defaults to EXTRACTED; n.id and n.label are always available.

- [ ] **Step 4: Implement scan, expand, filter, and project**

Execute NodeScan from exact ID, label, label property, source_file, or stable full scan. RelationshipExpand uses typed adjacency and direction. Evaluate comparisons, boolean/null logic, IN, string predicates, property maps, parameters, and core scalar graph functions. Project named columns and apply canonical result ordering when no ORDER BY exists.

- [ ] **Step 5: Run focused and regression tests**

Run: cargo test -p compass-query --test cql_fixed && cargo test -p compass-query && cargo clippy -p compass-query --all-targets -- -D warnings

Expected: PASS; existing natural-language traversal tests remain unchanged.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-query
git commit -m "feat(cql): execute fixed graph patterns"
~~~

### Task 7: Execute bounded paths and shortest paths

**Files:**
- Create: crates/compass-query/src/cql/path.rs
- Create: crates/compass-query/src/cql/shortest.rs
- Modify: crates/compass-query/src/cql/execute.rs
- Create: crates/compass-query/tests/cql_paths.rs

**Interfaces:**
- Consumes: VariableExpand and ShortestPath logical operators.
- Produces: PathRef, bounded_expand, shortest_path, all_shortest_paths.

- [ ] **Step 1: Write direction, uniqueness, bounds, and witness tests**

~~~rust
#[test]
fn bounded_path_never_reuses_a_relationship() {
    let graph = cyclic_fixture();
    let result = run(
        &graph,
        "MATCH p=(a {id:'a'})-[:CALLS*1..4]->(b) RETURN p"
    )
    .expect("execute");
    for row in result.rows {
        let path = row.path(0).expect("path");
        let unique = path.relationships().iter().copied().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), path.relationships().len());
    }
}

#[test]
fn path_expansion_limit_is_failure_not_partial_success() {
    let error = run_with_expansion_limit(&wide_fixture(), 3).expect_err("limit");
    assert_eq!(error.code(), "CQL4003");
}
~~~

- [ ] **Step 2: Run and verify path operators are absent**

Run: cargo test -p compass-query --test cql_paths

Expected: FAIL.

- [ ] **Step 3: Implement bounded variable expansion**

Use iterative DFS for enumerating bounded paths, store relationship indices in the current path, and reject repeated relationships. Apply relation type, direction, endpoint, and relationship predicates during expansion. Charge every examined relationship before filtering and check cancellation at least every 1,024 expansions.

- [ ] **Step 4: Implement shortest path selectors**

Use bidirectional BFS when source and target are single anchored nodes; otherwise use bounded BFS per source in stable node order. shortestPath returns one canonical shortest path. allShortestPaths retains only paths at the first discovered depth and remains bounded by row/memory/expansion limits.

- [ ] **Step 5: Run path and Miri-friendly tests**

Run: cargo test -p compass-query --test cql_paths && cargo test -p compass-query && cargo clippy -p compass-query --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-query/src/cql crates/compass-query/tests/cql_paths.rs
git commit -m "feat(cql): execute bounded paths"
~~~

### Task 8: Execute joins, OPTIONAL MATCH, EXISTS, and UNWIND

**Files:**
- Create: crates/compass-query/src/cql/join.rs
- Create: crates/compass-query/src/cql/optional.rs
- Create: crates/compass-query/src/cql/exists.rs
- Modify: crates/compass-query/src/cql/execute.rs
- Create: crates/compass-query/tests/cql_composition.rs

**Interfaces:**
- Consumes: Join, Optional, Exists, Unwind logical operators.
- Produces: bounded join strategies and correlated execution.

- [ ] **Step 1: Write composition semantics tests**

~~~rust
#[test]
fn optional_match_emits_null_and_exists_short_circuits() {
    let graph = fixture_graph();
    let optional = run(
        &graph,
        "MATCH (f:Function) OPTIONAL MATCH (t:Test)-[:TESTS]->(f) \
         WITH f,t WHERE t IS NULL RETURN f.id"
    )
    .expect("optional");
    assert_eq!(optional.rows.len(), 1);

    let missing = run(
        &graph,
        "MATCH (f:Function) WHERE NOT EXISTS { MATCH (f)-[:CALLS]->(:Function {label:'authorize()'}) } RETURN f.id"
    )
    .expect("exists");
    assert_eq!(missing.rows.len(), 1);
}
~~~

- [ ] **Step 2: Run and verify missing physical operators**

Run: cargo test -p compass-query --test cql_composition

Expected: FAIL.

- [ ] **Step 3: Implement deterministic joins and optional rows**

Use index nested-loop join when one side is graph-anchored and bounded hash join otherwise. Reserve the complete hash-side estimate before building. Repeated node/relationship variables compare graph identity. Optional emits one null-extended row only when its right plan returns no match.

- [ ] **Step 4: Implement correlated EXISTS and UNWIND**

Pass outer bindings into the subplan, short-circuit the first match, and prohibit result leakage from subquery variables. UNWIND emits values in list order, emits no rows for an empty list, and follows selected TCK null semantics.

- [ ] **Step 5: Run composition and regression tests**

Run: cargo test -p compass-query --test cql_composition && cargo test -p compass-query && cargo clippy -p compass-query --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 6: Commit**

~~~bash
git add crates/compass-query/src/cql crates/compass-query/tests/cql_composition.rs
git commit -m "feat(cql): compose graph matches"
~~~

### Task 9: Execute aggregation, WITH, UNION, lists, CASE, and safe regex

**Files:**
- Create: crates/compass-query/src/cql/aggregate.rs
- Create: crates/compass-query/src/cql/sort.rs
- Create: crates/compass-query/src/cql/functions.rs
- Modify: crates/compass-query/src/cql/eval.rs
- Modify: crates/compass-query/src/cql/execute.rs
- Create: crates/compass-query/tests/cql_advanced.rs

**Interfaces:**
- Consumes: Aggregate, Distinct, Sort, Union logical operators and function IDs.
- Produces: complete CompassQL Core expression and composition execution.

- [ ] **Step 1: Write aggregate and function tests**

~~~rust
#[test]
fn aggregate_with_filters_and_union_preserves_contract() {
    let graph = fixture_graph();
    let result = run(
        &graph,
        "MATCH (m:File)-[:IMPORTS_FROM]->(d:File) \
         WITH m,count(DISTINCT d) AS dependencies WHERE dependencies >= 2 \
         RETURN m.id AS id, dependencies AS value \
         UNION ALL \
         MATCH (f:Function) RETURN f.id AS id, size(f.label) AS value"
    )
    .expect("advanced query");
    assert_eq!(result.columns.iter().map(|column| column.name.as_str()).collect::<Vec<_>>(), vec!["id", "value"]);
}
~~~

- [ ] **Step 2: Run and verify advanced operators are absent**

Run: cargo test -p compass-query --test cql_advanced

Expected: FAIL.

- [ ] **Step 3: Implement bounded aggregation and sorting**

Implement count/count distinct/min/max/sum/avg/collect/collect distinct with openCypher null behavior. Group by canonical CompassValue keys. Reject NaN generation. Reserve group, distinct, collect, and sort memory before growth. ORDER BY applies openCypher type/null ordering; SKIP and LIMIT use checked non-negative integers.

- [ ] **Step 4: Implement expressions and safe regex**

Implement any/all/none/single, size/head/last, coalesce, toLower/toUpper/trim/split/replace, simple/searched CASE, list indexing/slicing, and Rust regex matching. Reject look-around, backreferences, and other unsupported regex constructs with CQL2018 before compiling the Regex.

- [ ] **Step 5: Implement UNION and canonical rows**

UNION validates equal column names/types and deduplicates canonical rows. UNION ALL preserves branch order and multiplicity. Queries without ORDER BY canonicalize final bounded rows by the documented total value order.

- [ ] **Step 6: Run advanced, full query, and clippy tests**

Run: cargo test -p compass-query --test cql_advanced && cargo test -p compass-query --all-targets && cargo clippy -p compass-query --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 7: Commit**

~~~bash
git add crates/compass-query/src/cql crates/compass-query/tests/cql_advanced.rs
git commit -m "feat(cql): complete core execution"
~~~

### Task 10: Add deterministic optimization, EXPLAIN, PROFILE, and plan caching

**Files:**
- Create: crates/compass-cypher/src/optimize.rs
- Create: crates/compass-query/src/cql/profile.rs
- Create: crates/compass-query/src/cql/cache.rs
- Modify: crates/compass-query/src/cql/execute.rs
- Create: crates/compass-cypher/tests/optimizer.rs
- Create: crates/compass-query/tests/cql_profile.rs

**Interfaces:**
- Consumes: LogicalPlan and Graph schema/index statistics.
- Produces: optimize, QueryProfile, explain_text, ProfiledOperator, PlanCache.

- [ ] **Step 1: Write optimized/reference equivalence and profile tests**

~~~rust
#[test]
fn optimizer_pushes_predicates_without_changing_results() {
    let source = "MATCH (a)-[r:CALLS]->(b) WHERE a.id='a' AND r.confidence='EXTRACTED' RETURN b.id";
    let reference = execute_unoptimized(source).expect("reference");
    let optimized = execute_optimized(source).expect("optimized");
    assert_eq!(reference.rows, optimized.rows);
    assert!(optimized.profile.expect("profile").expanded_relationships <= 2);
}
~~~

- [ ] **Step 2: Run and verify optimizer/profile APIs are absent**

Run: cargo test -p compass-cypher --test optimizer && cargo test -p compass-query --test cql_profile

Expected: FAIL.

- [ ] **Step 3: Implement deterministic optimizer rules**

Implement exact-ID/label/source-file anchoring, predicate pushdown, relation/direction restriction, stable independent-pattern reordering, EXISTS short-circuit marking, safe LIMIT pushdown, projection pruning, bounded join choice, and bidirectional shortest-path selection. Each rewrite records a reason for EXPLAIN.

- [ ] **Step 4: Implement EXPLAIN and PROFILE**

EXPLAIN renders operator tree, columns, estimates, indexes, path bounds, memory estimate, and warnings without reading property values. PROFILE wraps operators with counters for input/output rows, candidate nodes, expanded relationships, memory, elapsed time, cancellation checks, and cache status.

- [ ] **Step 5: Implement a bounded LRU plan cache**

PlanCache keys on query digest, parameter type signature, CompassQL version, Graph::schema_fingerprint, and planner version. It stores only Arc<CompiledQuery>, evicts by entry count and estimated bytes, and never stores parameters, results, or physical state.

- [ ] **Step 6: Run optimizer and cache tests**

Run: cargo test -p compass-cypher --test optimizer && cargo test -p compass-query --test cql_profile && cargo clippy -p compass-cypher -p compass-query --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 7: Commit**

~~~bash
git add crates/compass-cypher crates/compass-query
git commit -m "feat(cql): optimize and profile plans"
~~~

### Task 11: Render typed rows and add compass query --cql

**Files:**
- Create: crates/compass-output/src/cql.rs
- Modify: crates/compass-output/src/lib.rs
- Create: crates/compass-output/tests/cql.rs
- Create: crates/compass-cli/src/query_commands.rs
- Modify: crates/compass-cli/src/lib.rs
- Modify: crates/compass-cli/Cargo.toml
- Create: crates/compass-cli/tests/cql_cli.rs
- Modify: crates/compass-cli/tests/coverage_paths.rs

**Interfaces:**
- Consumes: QueryResult, Diagnostics, QueryError.
- Produces: render_cql_table/json/jsonl, parsed CqlCliRequest, Compass-only --cql dispatch.

- [ ] **Step 1: Write CLI mode-isolation and source-selection tests**

~~~rust
#[test]
fn cql_is_compass_only_and_never_auto_detected() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = CliFixture::graph();
    let cql = fixture.run_compass(&["query", "--cql", "MATCH (n) RETURN n.id"])?;
    assert!(cql.status.success());
    let natural = fixture.run_compass(&["query", "MATCH (n) RETURN n.id"])?;
    assert!(natural.status.success());
    assert!(String::from_utf8_lossy(&natural.stdout).contains("No matching nodes"));
    let compat = fixture.run_graphify(&["query", "--cql", "MATCH (n) RETURN n"])?;
    assert!(!compat.status.success());
    Ok(())
}

#[test]
fn cql_requires_exactly_one_source() {
    let fixture = CliFixture::new();
    let outcome = fixture.run_compass(&["query", "--cql", "--stdin", "--file", "q.cypher"]);
    assert_eq!(outcome.code(), 2);
    assert!(outcome.stderr().contains("exactly one CompassQL source"));
}
~~~

- [ ] **Step 2: Run and verify --cql is rejected**

Run: cargo test -p compass-cli --test cql_cli

Expected: FAIL because --cql mode is not implemented.

- [ ] **Step 3: Add typed output renderers**

Table escapes controls and displays Node/Relationship/Path compactly; JSON emits a stable object with columns, typed rows, and optional profile; JSONL emits one typed row object per line. Renderers never truncate silently and fail before output if the row result exceeds limits.

- [ ] **Step 4: Extract query dispatch and parse sources**

Move existing command_query behavior unchanged into query_commands::command_natural_query. Add command_query(frontend, args) that recognizes --cql only for Frontend::Compass and selects exactly one positional source, --file, --stdin, or --repl. Parse --param strings, bounded --params-file JSON, --format, shared --graph, limits, EXPLAIN/PROFILE source prefixes, and output selection. Reject natural-only --dfs/--context/--budget in CQL mode.

- [ ] **Step 5: Add REPL without weakening script behavior**

--repl requires an interactive terminal, reads one complete semicolon-terminated query at a time, supports :help/:quit/:params/:clear, and writes diagnostics without terminating the session. --stdin reads the complete bounded stream once and never opens the REPL.

- [ ] **Step 6: Keep help hidden until the full gate passes**

During intermediate commits, tests invoke the internal parser entry through a cfg(test) route while compass query --help omits --cql. At the end of this task, expose --cql only because Tasks 1-10 are complete and green. The graphify help and parser remain unchanged.

- [ ] **Step 7: Run CLI, output, and compatibility tests**

Run: cargo test -p compass-output --test cql && cargo test -p compass-cli --test cql_cli --test coverage_paths && cargo test -p compass-parity && cargo clippy -p compass-output -p compass-cli --all-targets -- -D warnings

Expected: PASS.

- [ ] **Step 8: Commit**

~~~bash
git add crates/compass-output crates/compass-cli
git commit -m "feat(cql): expose deterministic query mode"
~~~

### Task 12: Integrate pinned openCypher TCK and Neo4j differential fixtures

**Files:**
- Create: tests/opencypher-tck/README.md
- Create: tests/opencypher-tck/LICENSE
- Create: tests/opencypher-tck/NOTICE
- Create: tests/opencypher-tck/manifest.toml
- Create: tests/opencypher-tck/features/
- Create: crates/compass-cypher/tests/tck.rs
- Create: crates/compass-query/tests/neo4j_differential.rs
- Create: scripts/check_compassql_support.py
- Modify: .github/workflows/compass-ci.yml
- Modify: .github/workflows/compass-hardening.yml
- Modify: THIRD_PARTY_NOTICES.md

**Interfaces:**
- Consumes: openCypher 2024.3 read-only Apache-2.0 feature files and existing Neo4j Bolt client.
- Produces: selected TCK runner, support manifest, optional live differential suite.

- [ ] **Step 1: Add a failing support-manifest test**

~~~rust
#[test]
fn every_supported_feature_has_tck_evidence() {
    let manifest = TckManifest::load("../../tests/opencypher-tck/manifest.toml")
        .expect("manifest");
    for feature in compass_cypher::supported_features() {
        assert!(manifest.scenarios_for(feature.id).next().is_some(), "{}", feature.id);
    }
}
~~~

- [ ] **Step 2: Pin and record TCK provenance**

Copy only the read-only 2024.3 feature/graph files required by docs/COMPASSQL_SUPPORT.md. manifest.toml records upstream repository, release, commit, SHA-256 per copied file, license, supported scenario IDs, and explicitly rejected mutation/admin scenario IDs. README documents the copying procedure and update review.

- [ ] **Step 3: Implement a focused Gherkin scenario loader**

The test-only loader parses Given graph fixtures, parameters, query text, expected rows/order, and expected errors needed by selected scenarios. It rejects unsupported TCK step forms so skipped evidence cannot appear to pass. Mutation scenarios assert CQL1007 before execution and zero graph changes.

- [ ] **Step 4: Add optional Neo4j differential execution**

neo4j_differential reads COMPASS_NEO4J_URI, COMPASS_NEO4J_USER, and COMPASS_NEO4J_PASSWORD. When all are absent it returns early with a visible skipped-test message; CI differential job supplies them using a pinned Neo4j service. Export the same fixture graph, run accepted source/parameters locally and remotely, and normalize stable id properties, relationship tuples, paths, multiplicity, column names, types, and explicit order.

- [ ] **Step 5: Add CI and support checks**

compass-ci runs selected TCK on every PR. hardening runs Neo4j differential, full unsupported syntax, and support-matrix coverage. THIRD_PARTY_NOTICES records Apache-2.0 openCypher grammar/TCK provenance.

- [ ] **Step 6: Run conformance**

Run: cargo test -p compass-cypher --test tck && python3 scripts/check_compassql_support.py && cargo test -p compass-query --test neo4j_differential

Expected: PASS; differential reports skipped only when credentials are absent.

- [ ] **Step 7: Commit**

~~~bash
git add tests/opencypher-tck crates/compass-cypher/tests/tck.rs crates/compass-query/tests/neo4j_differential.rs scripts/check_compassql_support.py .github/workflows THIRD_PARTY_NOTICES.md
git commit -m "test(cql): enforce openCypher conformance"
~~~

### Task 13: Add fuzzing, mutation, performance, and user documentation gates

**Files:**
- Modify: fuzz/Cargo.toml
- Create: fuzz/fuzz_targets/cql_source.rs
- Create: fuzz/fuzz_targets/cql_params.rs
- Create: fuzz/corpus/cql_source/
- Create: fuzz/corpus/cql_params/
- Create: scripts/benchmark_compassql.sh
- Modify: scripts/check_critical_coverage.sh
- Modify: .github/workflows/compass-hardening.yml
- Modify: PERFORMANCE.md
- Modify: COMPATIBILITY.md
- Modify: README.md
- Create: docs/COMPASSQL.md
- Modify: docs/COMPASSQL_SUPPORT.md

**Interfaces:**
- Consumes: complete engine and CLI.
- Produces: release evidence and user reference.

- [ ] **Step 1: Add parser and parameter fuzz targets**

~~~rust
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|source: &str| {
    let parameters = compass_cypher::ParameterTypes::default();
    let _ = compass_cypher::compile(compass_cypher::CompileRequest {
        source_name: "fuzz.cypher",
        source,
        parameter_types: &parameters,
        schema: &compass_model::SchemaFingerprint::empty(),
        limits: compass_cypher::CompileLimits::default(),
    });
});
~~~

cql_params feeds arbitrary JSON through the bounded parameter decoder and, when valid, executes a fixed parameterized query against a three-node fixture.

- [ ] **Step 2: Add performance qualification**

scripts/benchmark_compassql.sh generates anchored, scan, one-hop, bounded-path, aggregate, optional, and policy-shaped queries over small/medium/large/adversarial fixtures. It records cold parse/plan, warm cached execution, p50/p95, peak RSS, expanded edges, rows, and cancellation latency.

The script fails when cached-plan overhead exceeds 10% over the equivalent direct traversal, memory exceeds the declared budget, cancellation misses the next checkpoint by more than 100 ms, or existing read benchmarks regress by more than 10%.

- [ ] **Step 3: Add hardening jobs**

Add cql_source and cql_params to the fuzz matrix, compass-cypher and CQL execution to 95% critical-module coverage, parser/limit/fingerprint targets to mutation testing, and compass-cypher/compass-query CQL tests to Miri where supported.

- [ ] **Step 4: Document the complete public contract**

docs/COMPASSQL.md includes command modes, grammar, values/nulls, parameters, paths, limits, EXPLAIN/PROFILE, output schemas, diagnostics, portability, and examples. README links it. COMPATIBILITY states --cql is Compass-only. PERFORMANCE records reproducible commands and first approved baseline.

- [ ] **Step 5: Run the complete engine gate**

Run:

~~~bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
python3 scripts/check_compassql_support.py
scripts/benchmark_compassql.sh
~~~

Expected: every command passes; no CompassQL command or help surface is partially exposed.

- [ ] **Step 6: Refresh the repository graph and commit**

Run: graphify update .

~~~bash
git add fuzz scripts .github/workflows PERFORMANCE.md COMPATIBILITY.md README.md docs graphify-out
git commit -m "docs(cql): qualify CompassQL core"
~~~

## Engine completion gate

Do not begin the policy plan until:

- compass query --cql supports the complete documented CompassQL 1 surface.
- All selected TCK scenarios pass.
- Neo4j differential fixtures pass in configured CI.
- Graphify query parity remains unchanged.
- Fuzz, mutation, coverage, Miri, cross-platform, and performance gates pass.
- The command is documented and no hidden accepted syntax is absent from the support matrix.
