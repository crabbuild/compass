# Compass Code Graph v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first supported `compass.graph/1` structural code graph,
including typed enterprise facts, framework routing, trusted provenance,
Program IR enrichment, shared queries, and VS Code presentation.

**Architecture:** Keep the NetworkX node-link envelope, but make its metadata,
nodes, and links strict typed Rust records. Language extractors continue to
produce per-file raw facts; a typed normalization boundary in `compass-graph`
converts them into v1 records after `compass-resolve` finishes cross-file and
framework resolution. `graph.json` and `program.json` remain authoritative;
`compass-query` builds a disposable SQLite/FTS5 index and returns one
`compass.query/1` contract to CLI, MCP, and VS Code.

**Tech Stack:** Rust 1.97, serde/serde_json, tree-sitter, SHA-256, rusqlite
0.31 with bundled modern SQLite/FTS5, TypeScript 5.9, Zod 4, React 19, Vitest,
Playwright, VS Code Extension API.

## Global Constraints

- The graph schema is exactly `compass.graph/1`.
- The durable envelope remains NetworkX-compatible with `directed`,
  `multigraph`, `graph`, `nodes`, and `links`.
- Every published graph is directed and multigraph.
- Unversioned graphs are pre-contract artifacts: reject them at query/load
  boundaries and rebuild them at index/update boundaries.
- Do not add a legacy graph adapter, translation mode, or compatibility
  relationship aliases.
- Do not change Graphify or invoke the embedded TypeScript CodeGraph at runtime.
- Keep Program IR under its existing
  `http://crab.build/compass/v1` schema.
- `graph.json`, `program.json`, manifest, and trusted build state publish as
  one generation.
- Every declared node and edge kind must have a validated producer before the
  release gate passes.
- Every heuristic edge must include its rule and wiring-site source anchor.
- Ambiguous resolution keeps bounded candidates; it never picks the first
  candidate silently.
- Source returned by explore must match the digest recorded in the graph.
- CLI, MCP, and VS Code must consume the same `compass.query/1` semantics.
- Preserve the user's existing uncommitted
  `editors/vscode/package.json` change unless it is intentionally superseded
  during the VS Code task.

---

## File and module structure

| Path | Responsibility |
|---|---|
| `crates/compass-model/src/code_graph.rs` | V1 node, role, edge, file, build, coverage, and diagnostic records |
| `crates/compass-model/src/provenance.rs` | Anchors, origins, confidence, and evidence invariants |
| `crates/compass-model/src/identity.rs` | Portable SHA-256 file, symbol, route, domain, and edge IDs |
| `crates/compass-model/src/query_contract.rs` | Transport-neutral `compass.query/1` requests and responses |
| `crates/compass-model/src/document.rs` | NetworkX envelope serialization and strict v1 loading |
| `crates/compass-model/src/validation.rs` | Closed vocabulary, endpoint matrix, anchor, path, and graph validation |
| `crates/compass-languages/src/facts.rs` | Flexible pre-publication raw extraction records and framework facts |
| `crates/compass-languages/src/frameworks/*.rs` | Local route/domain detection by ecosystem |
| `crates/compass-resolve/src/frameworks/*.rs` | Cross-file handler, middleware, event, job, and ORM resolution |
| `crates/compass-graph/src/v1.rs` | Raw-to-v1 normalization and canonical graph construction |
| `crates/compass-query/src/index.rs` | Content-addressed SQLite/FTS5 index |
| `crates/compass-query/src/code_query.rs` | Search, callers, callees, impact, explore, and node trail |
| `crates/compass-query/src/program_join.rs` | Optional Program IR evidence reconciliation |
| `crates/compass-cli/src/code_query_commands.rs` | JSON/text CLI adapters |
| `crates/compass-mcp/src/code_query.rs` | MCP tool schemas and structured result adapters |
| `packages/compass-viewer/src/contracts/codeQuery.ts` | Zod mirror of `compass.query/1` |
| `packages/compass-viewer/src/graph/CodeEvidence.tsx` | Provenance, coverage, conflict, and truncation presentation |
| `editors/vscode/src/views/codeQueryClient.ts` | CLI-backed query client |

## Milestone 1: Establish the graph contract and hard cutover

### Task 1: Add the closed v1 vocabulary and typed records

**Files:**

- Create: `crates/compass-model/src/code_graph.rs`
- Create: `crates/compass-model/src/provenance.rs`
- Modify: `crates/compass-model/src/lib.rs`
- Modify: `crates/compass-model/src/document.rs`
- Test: `crates/compass-model/tests/code_graph_v1.rs`

**Interfaces:**

- Produces:
  `NodeKind`, `NodeRole`, `EdgeKind`, `SourceAnchor`, `EvidenceOrigin`,
  `EvidenceConfidence`, `Provenance`, `FileRecord`, `CoverageRecord`,
  `GraphDiagnostic`, typed `NodeRecord`, typed `EdgeRecord`,
  `GraphMetadata`, and `GraphDocument`.
- Consumers: every later task.

- [ ] **Step 1: Write the failing serialization test**

```rust
#[test]
fn serializes_the_networkx_v1_envelope() -> Result<(), Box<dyn std::error::Error>> {
    let document = fixture_graph();
    let value = serde_json::to_value(document)?;
    assert_eq!(value["directed"], true);
    assert_eq!(value["multigraph"], true);
    assert_eq!(value["graph"]["schema"], "compass.graph/1");
    assert_eq!(value["nodes"][0]["kind"], "file");
    assert_eq!(value["links"][0]["kind"], "contains");
    assert_eq!(value["links"][0]["key"], value["links"][0]["id"]);
    assert!(value["links"][0].get("relation").is_none());
    Ok(())
}
```

Also assert serde rejects `"kind":"unknown"` for a node, edge, role, origin,
confidence, and diagnostic severity.

- [ ] **Step 2: Run the test and verify the contract is absent**

Run:

```bash
cargo test -p compass-model --test code_graph_v1 --locked
```

Expected: compilation fails because the v1 types and `fixture_graph` cannot be
constructed.

- [ ] **Step 3: Implement the exact vocabulary**

Define serde `snake_case` enums with these variants:

```rust
pub enum NodeKind {
    File, Module, Package, Namespace, Class, Struct, Interface, Trait,
    Protocol, Enum, EnumMember, TypeAlias, Function, Method, Constructor,
    Property, Field, Variable, Constant, Parameter, Import, Export, Macro,
    Annotation, Route, Component, Event, Message, Topic, Queue, Job, Resource,
    Schema, Query, Migration, ConfigKey, Database, DatabaseSchema,
    DatabaseTable, DatabaseView, DatabaseColumn, DatabaseIndex,
    DatabaseConstraint, DatabaseProcedure, DatabaseTrigger,
}

pub enum NodeRole {
    Controller, RouteHandler, Middleware, Service, Resolver, Consumer,
    Producer, Subscriber, Repository, Model, Test, Fixture, Generated,
}

pub enum ResourceKind {
    Document, Paper, Image, Concept, Rationale,
}

pub enum EdgeKind {
    Contains, Calls, Imports, Exports, Extends, Implements, References,
    TypeOf, Returns, Instantiates, Overrides, Decorates, RoutesTo, Reads,
    Writes, Aliases, Registers, Handles, Publishes, Subscribes, Produces,
    Consumes, Schedules, Triggers, Tests, DependsOn, Documents, MapsTo,
}
```

Use `#[serde(deny_unknown_fields)]` on structured records. Serialize edge
identity into both `id` and NetworkX `key`. Keep community fields typed and
optional on `NodeRecord`; represent `resource` details with `ResourceKind`;
do not reintroduce a flattened attributes map.

- [ ] **Step 4: Run the model tests**

Run:

```bash
cargo test -p compass-model --test code_graph_v1 --locked
cargo test -p compass-model --all-targets --locked
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-model
git commit -m "feat(model): define Compass graph v1"
```

### Task 2: Implement portable identities and provenance validation

**Files:**

- Create: `crates/compass-model/src/identity.rs`
- Modify: `crates/compass-model/src/provenance.rs`
- Modify: `crates/compass-model/src/validation.rs`
- Modify: `crates/compass-model/src/error.rs`
- Test: `crates/compass-model/tests/code_graph_identity.rs`
- Test: `crates/compass-model/tests/code_graph_validation.rs`

**Interfaces:**

- Produces:
  `file_id(path)`, `symbol_id(SymbolIdentity)`, `route_id(RouteIdentity)`,
  `domain_id(DomainIdentity)`, `edge_id(EdgeIdentity)`, and
  `validate_graph_v1(&GraphDocument)`.
- Consumes: v1 types from Task 1.

- [ ] **Step 1: Write failing identity and provenance tests**

```rust
#[test]
fn identity_is_checkout_root_independent() {
    let left = symbol_id(&SymbolIdentity {
        language: "rust",
        kind: NodeKind::Function,
        source_path: "src/lib.rs",
        qualified_name: "crate::run",
        overload: None,
    });
    let right = symbol_id(&SymbolIdentity {
        source_path: "./src/../src/lib.rs",
        ..same_symbol()
    });
    assert_eq!(left, right);
    assert!(!left.contains('/'));
}

#[test]
fn heuristic_edges_require_a_wiring_site() {
    let mut edge = calls_edge();
    edge.provenance.origin = EvidenceOrigin::Heuristic;
    edge.provenance.rule = Some("event-dispatch".to_owned());
    edge.provenance.wiring_site = None;
    assert!(matches!(
        validate_graph_v1(&graph_with(edge)),
        Err(GraphError::InvalidProvenance { .. })
    ));
}
```

Add tests for out-of-bounds anchors, absolute paths, `..` escape, conflicting
duplicate IDs, dangling endpoints, and invalid endpoint-kind combinations.

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-model --test code_graph_identity --locked
cargo test -p compass-model --test code_graph_validation --locked
```

Expected: tests fail because identity and strict validation are missing.

- [ ] **Step 3: Implement identity and invariants**

Use length-prefixed identity components and SHA-256:

```rust
fn digest_identity(namespace: &str, fields: &[&str]) -> String {
    let mut bytes = Vec::new();
    push_field(&mut bytes, "compass.graph/1");
    push_field(&mut bytes, namespace);
    for field in fields {
        push_field(&mut bytes, field);
    }
    format!("{namespace}:{}", hex_sha256(&bytes))
}
```

Normalize separators to `/`, collapse `.` components, reject `..`, absolute
paths, prefixes, and empty paths. Validate the full endpoint matrix from the
approved design. A heuristic provenance record requires non-empty `rule` and
`wiring_site`; all other origins require at least one anchor.

- [ ] **Step 4: Verify identity and validation**

Run:

```bash
cargo test -p compass-model --test code_graph_identity --locked
cargo test -p compass-model --test code_graph_validation --locked
cargo test -p compass-model --all-targets --locked
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-model
git commit -m "feat(model): validate graph identity and evidence"
```

### Task 3: Enforce the graph v1 hard cutover

**Files:**

- Modify: `crates/compass-model/src/document.rs`
- Modify: `crates/compass-model/src/error.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/build_state.rs`
- Modify: `crates/compass-history/src/store.rs`
- Modify: `crates/compass-history/src/artifacts.rs`
- Modify: `crates/compass-history/src/validate.rs`
- Test: `crates/compass-model/tests/contracts_coverage.rs`
- Test: `crates/compass-core/tests/loading_coverage.rs`
- Test: `crates/compass-history/tests/roundtrip.rs`

**Interfaces:**

- Produces:
  `GraphError::UnsupportedSchema`, strict `GraphDocument::load`, and
  build-only `graph_needs_full_rebuild`.
- Consumers: all graph readers and the history store.

- [ ] **Step 1: Write failing hard-cutover tests**

Add three cases:

```rust
#[test]
fn loader_rejects_unversioned_graphs() {
    let error = GraphDocument::from_slice(
        br#"{"directed":true,"multigraph":true,"graph":{},"nodes":[],"links":[]}"#
    ).unwrap_err();
    assert!(matches!(error, GraphError::UnsupportedSchema { found: None }));
}

#[test]
fn update_rebuilds_a_precontract_graph_from_source() -> Result<(), Box<dyn Error>> {
    let repo = seeded_repo("fn run() {}")?;
    write_precontract_graph(&repo)?;
    let result = run_update(&repo)?;
    assert_eq!(load_graph(&result)?.metadata.schema, "compass.graph/1");
    Ok(())
}

#[test]
fn history_store_format_names_compass_graph_v1() {
    assert!(store_format().contains(r#""graph_schema":"compass.graph/1""#));
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-model --test contracts_coverage --locked
cargo test -p compass-core --test loading_coverage --locked
cargo test -p compass-history --test roundtrip --locked
```

Expected: the unversioned graph loads and the history format still names
`networkx-node-link/v1`.

- [ ] **Step 3: Implement the hard cutover**

- Require `graph.schema == "compass.graph/1"` in every query/history loader.
- Let update/index detect `UnsupportedSchema` before loading incremental state,
  invalidate AST/query/output caches, and perform a clean source rebuild.
- Bump the trusted build-state and history store graph-schema fingerprints.
- Refuse mixed graph/program/manifest/build-state generations.
- Do not deserialize old `relation` attributes into `EdgeKind`.

- [ ] **Step 4: Verify the cutover**

Run:

```bash
cargo test -p compass-model -p compass-core -p compass-history --all-targets --locked
```

Expected: all tests pass, including explicit rejection of pre-contract graph
reads and source rebuild during update.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-model crates/compass-core crates/compass-history
git commit -m "feat: hard cut over to Compass graph v1"
```

## Milestone 2: Normalize extraction and publication

### Task 4: Separate raw extraction facts from published records

**Files:**

- Modify: `crates/compass-languages/src/facts.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Modify: `crates/compass-languages/src/apex.rs`
- Modify: `crates/compass-languages/src/bash.rs`
- Modify: `crates/compass-languages/src/cpp.rs`
- Modify: `crates/compass-languages/src/csharp.rs`
- Modify: `crates/compass-languages/src/dart.rs`
- Modify: `crates/compass-languages/src/dm.rs`
- Modify: `crates/compass-languages/src/dotnet_project.rs`
- Modify: `crates/compass-languages/src/elixir.rs`
- Modify: `crates/compass-languages/src/engine.rs`
- Modify: `crates/compass-languages/src/fortran.rs`
- Modify: `crates/compass-languages/src/go.rs`
- Modify: `crates/compass-languages/src/groovy.rs`
- Modify: `crates/compass-languages/src/json_config.rs`
- Modify: `crates/compass-languages/src/julia.rs`
- Modify: `crates/compass-languages/src/markdown.rs`
- Modify: `crates/compass-languages/src/mcp.rs`
- Modify: `crates/compass-languages/src/objc.rs`
- Modify: `crates/compass-languages/src/package_manifest.rs`
- Modify: `crates/compass-languages/src/pascal.rs`
- Modify: `crates/compass-languages/src/pascal_forms.rs`
- Modify: `crates/compass-languages/src/php.rs`
- Modify: `crates/compass-languages/src/powershell.rs`
- Modify: `crates/compass-languages/src/r.rs`
- Modify: `crates/compass-languages/src/rust_lang.rs`
- Modify: `crates/compass-languages/src/scip.rs`
- Modify: `crates/compass-languages/src/sql.rs`
- Modify: `crates/compass-languages/src/swift.rs`
- Modify: `crates/compass-languages/src/templates.rs`
- Modify: `crates/compass-languages/src/terraform.rs`
- Modify: `crates/compass-languages/src/verilog.rs`
- Modify: `crates/compass-languages/src/xaml.rs`
- Modify: `crates/compass-languages/src/zig.rs`
- Modify: `crates/compass-resolve/src/lib.rs`
- Modify: `crates/compass-resolve/src/members.rs`
- Create: `crates/compass-graph/src/v1.rs`
- Modify: `crates/compass-graph/src/lib.rs`
- Test: `crates/compass-languages/tests/typed_extraction.rs`
- Test: `crates/compass-graph/tests/graph_v1_normalization.rs`

**Interfaces:**

- Produces:
  `RawNodeRecord`, `RawEdgeRecord`, `RawFrameworkFact`, and
  `normalize_v1(Extraction, BuildEvidence) -> Result<GraphDocument, GraphError>`.
- Consumes: typed v1 model and current per-language extraction behavior.

- [ ] **Step 1: Write failing normalization tests**

Build raw facts using the current extractor vocabulary and assert exact v1
normalization:

```rust
#[test]
fn normalizes_old_internal_relations_without_publishing_aliases() {
    let raw = extraction_with_relations([
        ("imports_from", "imports"),
        ("re_exports", "exports"),
        ("inherits", "extends"),
        ("indirect_call", "calls"),
        ("reads_from", "reads"),
        ("references_constant", "references"),
        ("uses_static_prop", "references"),
    ]);
    let graph = normalize_v1(raw, evidence()).unwrap();
    assert!(graph.links.iter().all(|edge| {
        !["imports_from", "re_exports", "inherits", "indirect_call"]
            .contains(&edge.kind.as_str())
    }));
    let indirect = graph.links.iter()
        .find(|edge| edge.kind == EdgeKind::Calls
            && edge.provenance.rule.as_deref() == Some("indirect-call-resolution"))
        .unwrap();
    assert_eq!(indirect.provenance.origin, EvidenceOrigin::Heuristic);
}
```

Add a compile-time migration test proving extractors use raw records and only
`compass-graph` constructs published v1 records.

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-languages --test typed_extraction --locked
cargo test -p compass-graph --test graph_v1_normalization --locked
```

Expected: raw types and normalization do not exist.

- [ ] **Step 3: Move the flexible records to the extraction boundary**

Define:

```rust
pub struct RawNodeRecord {
    pub id: String,
    pub attributes: serde_json::Map<String, serde_json::Value>,
}

pub struct RawEdgeRecord {
    pub source: String,
    pub target: String,
    pub attributes: serde_json::Map<String, serde_json::Value>,
}
```

Change extractor imports mechanically from `compass_model` to
`crate::facts::{RawNodeRecord as NodeRecord, RawEdgeRecord as EdgeRecord}`.
Change `compass-resolve` to operate on raw records. Implement the closed
normalization table in `compass-graph/src/v1.rs`; any producer kind or relation
outside the table is a build error with producer file and anchor.

Normalize current semantic/media nodes to `resource` with a typed
`resource_kind` of `document`, `paper`, `image`, `concept`, or `rationale`.
Normalize `rationale_for` to `documents`, `configures` to `depends_on`,
`case_of`, `defines`, and `method` to `contains`, `uses` to `references`,
`embeds` to `contains` with evidence rule `embedded-member`, `mixes_in` to
`implements` with evidence rule `mixin-contract`, `reads_from` to `reads`,
`inherits` to `extends`, and `re_exports` to `exports`. Relations already in
the closed vocabulary retain their typed meaning. The producer spelling is
retained only in evidence detail; it is never serialized as an edge-kind
alias.

- [ ] **Step 4: Run language, resolver, and graph tests**

Run:

```bash
cargo test -p compass-languages -p compass-resolve -p compass-graph --all-targets --locked
```

Expected: all existing extraction behavior passes through the typed
publication boundary.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-languages crates/compass-resolve crates/compass-graph
git commit -m "refactor: type the graph publication boundary"
```

### Task 5: Publish file inventory, coverage, diagnostics, and canonical bytes

**Files:**

- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/build_state.rs`
- Modify: `crates/compass-files/src/manifest.rs`
- Modify: `crates/compass-graph/src/v1.rs`
- Modify: `crates/compass-history/src/artifacts.rs`
- Test: `crates/compass-core/tests/program_pipeline.rs`
- Test: `crates/compass-core/tests/loading_coverage.rs`
- Test: `crates/compass-history/tests/roundtrip.rs`

**Interfaces:**

- Produces:
  `BuildMetadata`, complete `FileRecord` inventory, canonical graph bytes, and
  one-generation publication.
- Consumes: Tasks 1-4 and existing file manifests.

- [ ] **Step 1: Write failing publication tests**

```rust
#[test]
fn clean_and_incremental_builds_are_byte_identical() -> Result<(), Box<dyn Error>> {
    let repo = seeded_repo("pub fn run() {}")?;
    let clean = build(&repo, true)?;
    let incremental = build(&repo, false)?;
    assert_eq!(fs::read(clean.graph_path)?, fs::read(incremental.graph_path)?);
    Ok(())
}

#[test]
fn file_inventory_reports_failed_extraction() -> Result<(), Box<dyn Error>> {
    let graph = build_fixture_with_invalid_source()?;
    let file = graph.metadata.files.iter().find(|f| f.path == "src/bad.py").unwrap();
    assert_eq!(file.extraction_status, ExtractionStatus::Failed);
    assert!(!file.diagnostics.is_empty());
    Ok(())
}
```

Also assert build metadata excludes timestamps, hostnames, and absolute roots.

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-core --test loading_coverage --locked
cargo test -p compass-history --test roundtrip --locked
```

Expected: file inventory and canonical v1 generation are absent.

- [ ] **Step 3: Implement deterministic metadata and atomic publication**

Compute:

```rust
BuildMetadata {
    builder_version,
    schema_fingerprint,
    source_tree_digest,
    configuration_digest,
    generation_id,
}
```

Derive `generation_id` from the preceding four fields plus Program provider
fingerprints. Sort files by normalized path, nodes by ID, and links by
`(source, target, kind, id)`. Validate graph and Program fingerprints before
publishing the guarded output directory.

- [ ] **Step 4: Verify publication**

Run:

```bash
cargo test -p compass-core -p compass-history --all-targets --locked
```

Expected: all tests pass and repeated builds are byte-identical.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-core crates/compass-files crates/compass-graph crates/compass-history
git commit -m "feat(core): publish canonical graph generations"
```

## Milestone 3: Add enterprise and framework producers

### Task 6: Emit SQL and database-domain nodes and edges

**Files:**

- Modify: `crates/compass-languages/src/sql.rs`
- Modify: `crates/compass-languages/src/package_manifest.rs`
- Modify: `crates/compass-languages/src/json_config.rs`
- Modify: `crates/compass-languages/src/terraform.rs`
- Create: `fixtures/code-graph/domain/database.sql`
- Create: `fixtures/code-graph/domain/database.expected.json`
- Test: `crates/compass-languages/tests/domain_extraction.rs`
- Test: `crates/compass-graph/tests/domain_normalization.rs`

**Interfaces:**

- Produces all database, schema, query, migration, config-key, package, and
  resource node kinds plus `reads`, `writes`, `depends_on`, and `maps_to`
  edges backed by direct syntax/configuration evidence.

- [ ] **Step 1: Write the failing domain fixture test**

The SQL fixture must contain one schema, table, view, index, constraint,
procedure, trigger, migration marker, `SELECT`, `INSERT`, and `UPDATE`.

```rust
#[test]
fn sql_emits_the_database_vocabulary() {
    let graph = build_fixture("fixtures/code-graph/domain/database.sql");
    assert_kinds(&graph, [
        NodeKind::DatabaseSchema, NodeKind::DatabaseTable,
        NodeKind::DatabaseView, NodeKind::DatabaseColumn,
        NodeKind::DatabaseIndex, NodeKind::DatabaseConstraint,
        NodeKind::DatabaseProcedure, NodeKind::DatabaseTrigger,
        NodeKind::Query, NodeKind::Migration,
    ]);
    assert_edge(&graph, EdgeKind::Reads, "recent_orders", "orders");
    assert_edge(&graph, EdgeKind::Writes, "insert_order", "orders");
}
```

- [ ] **Step 2: Verify the test fails**

Run:

```bash
cargo test -p compass-languages --test domain_extraction --locked
cargo test -p compass-graph --test domain_normalization --locked
```

Expected: current SQL facts do not cover the complete v1 domain vocabulary.

- [ ] **Step 3: Implement exact producers**

Emit a domain node only when its parser exposes a definition or direct
configuration key. Preserve SQL object qualification and query sites. Do not
infer ORM mappings in this task. Emit package/resource/config facts from
manifests, JSON configuration, and Terraform only when the key or block type
matches the closed producer registry.

- [ ] **Step 4: Verify domain extraction**

Run:

```bash
cargo test -p compass-languages -p compass-graph --all-targets --locked
```

Expected: every domain node/edge in this task has positive and negative fixture
coverage.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-languages crates/compass-graph fixtures/code-graph/domain
git commit -m "feat(languages): extract database domain facts"
```

### Task 7: Introduce the framework-fact and route-resolution interfaces

**Files:**

- Create: `crates/compass-languages/src/frameworks/mod.rs`
- Create: `crates/compass-languages/src/frameworks/model.rs`
- Modify: `crates/compass-languages/src/facts.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Create: `crates/compass-resolve/src/frameworks/mod.rs`
- Create: `crates/compass-resolve/src/frameworks/routes.rs`
- Modify: `crates/compass-resolve/src/lib.rs`
- Test: `crates/compass-resolve/tests/framework_routes.rs`

**Interfaces:**

- Produces:
  `FrameworkFact::Route`, `HandlerReference`, `MiddlewareReference`,
  `RouteResolution::{Exact, Ambiguous, Unresolved}`, and
  `resolve_framework_facts`.
- Consumers: Tasks 8-12.

- [ ] **Step 1: Write failing generic route-resolution tests**

```rust
#[test]
fn exact_route_resolution_emits_ordered_routes_to_edges() {
    let resolved = resolve_fixture(route_fact(
        "express", "GET", "/orders", ["auth", "list_orders"]
    ));
    assert_routes_to(&resolved, "GET /orders", [
        ("auth", "middleware", 0),
        ("list_orders", "handler", 1),
    ]);
}

#[test]
fn duplicate_handler_names_remain_ambiguous() {
    let resolved = resolve_fixture(route_to_name("show"));
    assert_eq!(route(&resolved).resolution, RouteResolution::Ambiguous);
    assert_eq!(route(&resolved).candidates.len(), 2);
    assert!(resolved.edges.iter().all(|e| e.provenance.confidence != Exact));
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-resolve --test framework_routes --locked
```

Expected: framework facts and route resolution do not exist.

- [ ] **Step 3: Implement the interfaces**

Use this route fact shape:

```rust
pub struct RouteFact {
    pub framework: Framework,
    pub method: String,
    pub original_path: String,
    pub normalized_path: String,
    pub scope: String,
    pub anchor: RawSourceAnchor,
    pub provenance_origin: EvidenceOrigin,
    pub handler: HandlerReference,
    pub middleware: Vec<MiddlewareReference>,
}
```

Resolve exact qualified IDs first, imported aliases second, unique scoped names
third. Multiple candidates remain bounded and sorted. Emit one `routes_to`
edge per stage with `stage` and `position`.

- [ ] **Step 4: Verify the resolver**

Run:

```bash
cargo test -p compass-resolve --all-targets --locked
```

Expected: exact, ambiguous, unresolved, middleware-order, and heuristic wiring
tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-languages crates/compass-resolve
git commit -m "feat(resolve): add framework route facts"
```

### Task 8: Add Python routing packs

**Files:**

- Create: `crates/compass-languages/src/frameworks/python.rs`
- Create: `crates/compass-resolve/src/frameworks/python.rs`
- Create: `fixtures/code-graph/routes/python/django_urls.py`
- Create: `fixtures/code-graph/routes/python/flask_app.py`
- Create: `fixtures/code-graph/routes/python/fastapi_app.py`
- Create: `fixtures/code-graph/routes/python/near_matches.py`
- Test: `crates/compass-resolve/tests/python_routes.rs`

**Interfaces:**

- Produces Django, Flask, and FastAPI route facts and resolved `routes_to`
  edges.

- [ ] **Step 1: Write failing framework tests**

Cover:

- Django `path`, `re_path`, legacy `url`, `include`, `.as_view`, and dotted
  paths;
- Flask app and blueprint decorators with methods;
- FastAPI app/router decorators for GET, POST, PUT, PATCH, DELETE, OPTIONS,
  HEAD, and WebSocket;
- strings and unrelated decorators that must emit zero routes.

```rust
#[test]
fn django_include_composes_prefix_and_child_path() {
    let graph = build_python_routes("django_urls.py");
    assert_route(&graph, "GET", "/api/users/:id", "UserDetail.as_view");
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-resolve --test python_routes --locked
```

Expected: no Python route facts are emitted.

- [ ] **Step 3: Implement Python detection and resolution**

Parse decorator/call arguments from tree-sitter nodes, never regular-expression
scan whole files. Normalize Django converters and regex routes without claiming
equivalence between distinct regexes. Blueprint/include composition is exact
only when the prefix is a static literal; otherwise keep an unresolved route
with exact local evidence.

- [ ] **Step 4: Verify Python packs**

Run:

```bash
cargo test -p compass-languages -p compass-resolve --all-targets --locked
```

Expected: Python route and near-match tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-languages crates/compass-resolve fixtures/code-graph/routes/python
git commit -m "feat(frameworks): resolve Python web routes"
```

### Task 9: Add TypeScript/JavaScript and file-route packs

**Files:**

- Create: `crates/compass-languages/src/frameworks/typescript.rs`
- Create: `crates/compass-languages/src/frameworks/file_routes.rs`
- Create: `crates/compass-resolve/src/frameworks/typescript.rs`
- Create: `fixtures/code-graph/routes/typescript/express.ts`
- Create: `fixtures/code-graph/routes/typescript/nest.ts`
- Create: `fixtures/code-graph/routes/typescript/react-router.tsx`
- Create: `fixtures/code-graph/routes/typescript/sveltekit/src/routes/users/[id]/+page.svelte`
- Create: `fixtures/code-graph/routes/typescript/sveltekit/src/routes/users/[id]/+server.ts`
- Create: `fixtures/code-graph/routes/typescript/vue-router.ts`
- Create: `fixtures/code-graph/routes/typescript/nuxt/pages/users/[id].vue`
- Create: `fixtures/code-graph/routes/typescript/nuxt/server/api/users.get.ts`
- Create: `fixtures/code-graph/routes/typescript/nuxt/middleware/auth.ts`
- Create: `fixtures/code-graph/routes/typescript/astro/src/pages/about.astro`
- Create: `fixtures/code-graph/routes/typescript/astro/src/pages/users/[id].ts`
- Create: `fixtures/code-graph/routes/typescript/near-matches.ts`
- Test: `crates/compass-resolve/tests/typescript_routes.rs`

**Interfaces:**

- Produces Express, NestJS, React Router, SvelteKit, Vue Router, Nuxt, and
  Astro routes, plus NestJS GraphQL/message/event/WebSocket domain facts.

- [ ] **Step 1: Write failing TypeScript route tests**

Assert:

- Express middleware order and router prefixes;
- NestJS controller prefixes and all approved HTTP decorators;
- NestJS GraphQL query/mutation/resolver, message pattern, event pattern, and
  subscribed-message nodes/edges;
- React Router route components;
- SvelteKit, Nuxt, and Astro convention provenance;
- Vue configured routes and route middleware;
- dynamic path expressions remain unresolved rather than exact.

```rust
#[test]
fn nest_controller_prefix_and_method_path_compose() {
    let graph = build_ts_routes("nest.ts");
    assert_route(&graph, "GET", "/admin/users/:id", "UsersController.show");
    assert_role(&graph, "UsersController.show", NodeRole::RouteHandler);
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-resolve --test typescript_routes --locked
```

Expected: TypeScript framework facts are absent.

- [ ] **Step 3: Implement TypeScript and convention packs**

Use AST decorators/calls for Express, NestJS, React Router, and Vue Router.
Use normalized repository paths for SvelteKit, Nuxt, and Astro. Mark file
routes `convention`, not `heuristic`. Mark reflective NestJS dispatch
`heuristic` only when a runtime registration boundary cannot be proven.

- [ ] **Step 4: Verify TypeScript packs**

Run:

```bash
cargo test -p compass-languages -p compass-resolve --all-targets --locked
```

Expected: all TypeScript route, domain, and near-match tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-languages crates/compass-resolve fixtures/code-graph/routes/typescript
git commit -m "feat(frameworks): resolve TypeScript web routes"
```

### Task 10: Add PHP, Ruby, and JVM routing packs

**Files:**

- Create: `crates/compass-languages/src/frameworks/php.rs`
- Create: `crates/compass-languages/src/frameworks/ruby.rs`
- Create: `crates/compass-languages/src/frameworks/java.rs`
- Create: `crates/compass-languages/src/frameworks/play.rs`
- Create: `crates/compass-resolve/src/frameworks/php.rs`
- Create: `crates/compass-resolve/src/frameworks/ruby.rs`
- Create: `crates/compass-resolve/src/frameworks/jvm.rs`
- Create: `fixtures/code-graph/routes/php/laravel.php`
- Create: `fixtures/code-graph/routes/php/drupal.routing.yml`
- Create: `fixtures/code-graph/routes/php/drupal.module`
- Create: `fixtures/code-graph/routes/php/near_matches.php`
- Create: `fixtures/code-graph/routes/ruby/rails.rb`
- Create: `fixtures/code-graph/routes/ruby/near_matches.rb`
- Create: `fixtures/code-graph/routes/jvm/SpringController.java`
- Create: `fixtures/code-graph/routes/jvm/play/conf/routes`
- Create: `fixtures/code-graph/routes/jvm/PlayController.java`
- Create: `fixtures/code-graph/routes/jvm/PlayController.scala`
- Create: `fixtures/code-graph/routes/jvm/NearMatches.java`
- Test: `crates/compass-resolve/tests/php_ruby_jvm_routes.rs`

**Interfaces:**

- Produces Laravel, Drupal, Rails, Spring, and Play route facts and handlers.

- [ ] **Step 1: Write failing ecosystem tests**

The fixtures must cover:

- Laravel route methods, resources, `Controller@action`, and tuple syntax;
- Drupal `*.routing.yml` `_controller`, `_form`, entity handlers, HTTP methods,
  and `hook_*` implementations;
- Rails `to:` and hash-rocket syntax;
- Spring class/method prefixes, `RequestMapping`, and composed method mappings;
- Play verb routes to Scala and Java actions;
- near-match files that emit zero routes.

```rust
#[test]
fn laravel_resource_expands_to_canonical_actions() {
    let graph = build_fixture("php/laravel.php");
    assert_route(&graph, "GET", "/users/:user", "UserController.show");
    assert_route(&graph, "DELETE", "/users/:user", "UserController.destroy");
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-resolve --test php_ruby_jvm_routes --locked
```

Expected: the five framework families are missing.

- [ ] **Step 3: Implement the packs**

Use tree-sitter for PHP, Ruby, Java, Scala, and controller symbols. Use bounded
YAML parsing for Drupal and line-oriented grammar-aware parsing for Play
`conf/routes`. Generated Laravel resource routes use `config` evidence anchored
to the resource declaration and distinct stable route identities.

- [ ] **Step 4: Verify PHP, Ruby, and JVM packs**

Run:

```bash
cargo test -p compass-languages -p compass-resolve --all-targets --locked
```

Expected: all route and near-match tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-languages crates/compass-resolve fixtures/code-graph/routes/php fixtures/code-graph/routes/ruby fixtures/code-graph/routes/jvm
git commit -m "feat(frameworks): resolve PHP Ruby and JVM routes"
```

### Task 11: Add Go, Rust, C#, and Swift routing packs

**Files:**

- Create: `crates/compass-languages/src/frameworks/go.rs`
- Create: `crates/compass-languages/src/frameworks/rust.rs`
- Create: `crates/compass-languages/src/frameworks/csharp.rs`
- Create: `crates/compass-languages/src/frameworks/swift.rs`
- Create: `crates/compass-resolve/src/frameworks/native.rs`
- Create: `fixtures/code-graph/routes/go/gin.go`
- Create: `fixtures/code-graph/routes/go/chi.go`
- Create: `fixtures/code-graph/routes/go/gorilla.go`
- Create: `fixtures/code-graph/routes/go/near_matches.go`
- Create: `fixtures/code-graph/routes/rust/axum.rs`
- Create: `fixtures/code-graph/routes/rust/actix.rs`
- Create: `fixtures/code-graph/routes/rust/rocket.rs`
- Create: `fixtures/code-graph/routes/rust/near_matches.rs`
- Create: `fixtures/code-graph/routes/csharp/AspNetController.cs`
- Create: `fixtures/code-graph/routes/csharp/NearMatches.cs`
- Create: `fixtures/code-graph/routes/swift/VaporRoutes.swift`
- Create: `fixtures/code-graph/routes/swift/NearMatches.swift`
- Test: `crates/compass-resolve/tests/native_routes.rs`

**Interfaces:**

- Produces Gin, chi, gorilla/mux, Axum, actix, Rocket, ASP.NET, and Vapor route
  facts and handlers.

- [ ] **Step 1: Write failing native-stack tests**

Cover:

- Go `GET`, `POST`, and `HandleFunc`, group prefixes, and middleware;
- Rust Axum chained `.route`, actix route builders/attributes, Rocket route
  attributes and mounting;
- ASP.NET controller/action prefixes and HTTP method attributes;
- Vapor application and grouped-route registrations;
- computed paths and receiver calls that are not router instances.

```rust
#[test]
fn axum_route_resolves_get_handler() {
    let graph = build_fixture("rust/axum.rs");
    assert_route(&graph, "GET", "/users/:id", "show_user");
    assert_edge_kind(&graph, "GET /users/:id", "show_user", EdgeKind::RoutesTo);
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-resolve --test native_routes --locked
```

Expected: native-stack route packs are missing.

- [ ] **Step 3: Implement native-stack packs**

Require router receiver/type evidence before treating method calls as routes.
Compose static group/mount prefixes. Preserve computed paths as unresolved
facts. ASP.NET attribute routes are AST evidence; route-convention expansion
is convention evidence.

- [ ] **Step 4: Verify native-stack packs**

Run:

```bash
cargo test -p compass-languages -p compass-resolve --all-targets --locked
```

Expected: route and near-match tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-languages crates/compass-resolve fixtures/code-graph/routes/go fixtures/code-graph/routes/rust fixtures/code-graph/routes/csharp fixtures/code-graph/routes/swift
git commit -m "feat(frameworks): resolve native web routes"
```

### Task 12: Resolve events, messages, jobs, and ORM mappings

**Files:**

- Create: `crates/compass-resolve/src/frameworks/domain.rs`
- Modify: `crates/compass-languages/src/frameworks/python.rs`
- Modify: `crates/compass-languages/src/frameworks/typescript.rs`
- Modify: `crates/compass-languages/src/frameworks/file_routes.rs`
- Modify: `crates/compass-languages/src/frameworks/php.rs`
- Modify: `crates/compass-languages/src/frameworks/ruby.rs`
- Modify: `crates/compass-languages/src/frameworks/java.rs`
- Modify: `crates/compass-languages/src/frameworks/play.rs`
- Modify: `crates/compass-languages/src/frameworks/go.rs`
- Modify: `crates/compass-languages/src/frameworks/rust.rs`
- Modify: `crates/compass-languages/src/frameworks/csharp.rs`
- Modify: `crates/compass-languages/src/frameworks/swift.rs`
- Modify: `crates/compass-resolve/src/frameworks/python.rs`
- Modify: `crates/compass-resolve/src/frameworks/typescript.rs`
- Modify: `crates/compass-resolve/src/frameworks/php.rs`
- Modify: `crates/compass-resolve/src/frameworks/ruby.rs`
- Modify: `crates/compass-resolve/src/frameworks/jvm.rs`
- Modify: `crates/compass-resolve/src/frameworks/native.rs`
- Create: `fixtures/code-graph/domain/messaging/nest.ts`
- Create: `fixtures/code-graph/domain/messaging/spring.java`
- Create: `fixtures/code-graph/domain/messaging/dynamic-near-matches.ts`
- Create: `fixtures/code-graph/domain/jobs/spring.java`
- Create: `fixtures/code-graph/domain/jobs/aspnet.cs`
- Create: `fixtures/code-graph/domain/jobs/celery.py`
- Create: `fixtures/code-graph/domain/jobs/dynamic-near-matches.py`
- Create: `fixtures/code-graph/domain/orm/django.py`
- Create: `fixtures/code-graph/domain/orm/sqlalchemy.py`
- Create: `fixtures/code-graph/domain/orm/typeorm.ts`
- Create: `fixtures/code-graph/domain/orm/jpa.java`
- Create: `fixtures/code-graph/domain/orm/entity-framework.cs`
- Create: `fixtures/code-graph/domain/orm/active-record.rb`
- Create: `fixtures/code-graph/domain/orm/eloquent.php`
- Create: `fixtures/code-graph/domain/orm/gorm.go`
- Create: `fixtures/code-graph/domain/orm/diesel.rs`
- Create: `fixtures/code-graph/domain/orm/dynamic-near-matches.ts`
- Test: `crates/compass-resolve/tests/domain_resolution.rs`

**Interfaces:**

- Produces:
  `event`, `message`, `topic`, `queue`, and `job` nodes;
  `registers`, `handles`, `publishes`, `subscribes`, `produces`, `consumes`,
  `schedules`, `triggers`, and exact `maps_to` edges.

- [ ] **Step 1: Write failing domain-resolution tests**

Fixtures must cover:

- NestJS message/event/WebSocket handlers;
- Spring events and scheduled jobs;
- ASP.NET hosted/background jobs;
- Django/Celery task registration;
- exact ORM table mappings for Django, SQLAlchemy, TypeORM, JPA/Hibernate,
  Entity Framework, ActiveRecord, Eloquent, GORM, and Diesel;
- dynamic subjects/table names that remain unresolved.

```rust
#[test]
fn scheduled_job_triggers_its_handler() {
    let graph = build_fixture("jobs/spring.java");
    assert_edge_kind(&graph, "nightly-cleanup", "Cleanup.run", EdgeKind::Triggers);
    assert_exact_anchor(&graph, "nightly-cleanup");
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-resolve --test domain_resolution --locked
```

Expected: domain dispatch and ORM mappings are absent.

- [ ] **Step 3: Implement evidence-gated resolution**

Emit exact edges only from literal registrations, annotations, generated
artifact facts, or exact ORM mapping declarations. Dynamic subject/table
expressions produce unresolved facts. Runtime registry synthesis uses
`heuristic` provenance with rule and wiring site.

- [ ] **Step 4: Verify domain resolution**

Run:

```bash
cargo test -p compass-resolve -p compass-graph --all-targets --locked
```

Expected: every enterprise node and edge kind has a positive producer and a
negative fixture.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-resolve crates/compass-languages fixtures/code-graph/domain
git commit -m "feat(resolve): connect enterprise domain facts"
```

## Milestone 4: Build the shared query engine

### Task 13: Define `compass.query/1` and reconcile Program IR

**Files:**

- Create: `crates/compass-model/src/query_contract.rs`
- Modify: `crates/compass-model/src/lib.rs`
- Create: `crates/compass-query/src/program_join.rs`
- Modify: `crates/compass-query/src/lib.rs`
- Modify: `crates/compass-query/Cargo.toml`
- Test: `crates/compass-query/tests/program_join.rs`
- Test: `crates/compass-query/tests/query_contract.rs`

**Interfaces:**

- Produces:
  `CodeQueryRequest`, `CodeQueryResponse`, `QueryNode`, `QueryEdge`,
  `QueryFile`, `QueryPath`, `QueryDiagnostic`, `CodeQueryLimits`,
  `ProgramEvidenceJoin`.
- Consumes: `Graph`, optional `compass_analysis::AnalysisBundle`.

- [ ] **Step 1: Write failing contract and reconciliation tests**

```rust
#[test]
fn response_schema_is_versioned_and_preserves_two_evidence_layers() {
    let response = query_fixture_with_program();
    let value = serde_json::to_value(response).unwrap();
    assert_eq!(value["schema"], "compass.query/1");
    assert_eq!(value["edges"][0]["evidence"].as_array().unwrap().len(), 2);
    assert_eq!(value["edges"][0]["evidence"][0]["layer"], "structural_graph");
    assert_eq!(value["edges"][0]["evidence"][1]["layer"], "program_ir");
}

#[test]
fn program_only_symbol_is_a_diagnostic_not_a_structural_node() {
    let response = query_fixture_with_orphan_program_symbol();
    assert!(response.nodes.iter().all(|node| node.id != "program-only"));
    assert!(response.diagnostics.iter().any(|d| d.code == "program_orphan"));
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-query --test query_contract --locked
cargo test -p compass-query --test program_join --locked
```

Expected: query v1 and Program join types are absent.

- [ ] **Step 3: Implement transport-neutral responses**

Add `compass-analysis` and `compass-ir` path dependencies to
`compass-query`. Join by `graph_node_id`; merge evidence without overwriting.
Represent conflicts and missing graph identities as diagnostics. Keep all
response collections sorted by stable identity.

Use one additive response envelope for every operation:

```rust
pub struct CodeQueryResponse {
    pub schema: String,                 // always "compass.query/1"
    pub operation: CodeQueryOperation,
    pub results: Vec<SearchHit>,        // populated by search
    pub nodes: Vec<QueryNode>,
    pub edges: Vec<QueryEdge>,
    pub files: Vec<QueryFile>,
    pub paths: Vec<QueryPath>,
    pub diagnostics: Vec<QueryDiagnostic>,
    pub limits: CodeQueryLimits,
    pub truncated: bool,
}
```

Non-search operations return an empty `results` array; search returns its
ranked hits there and includes the corresponding nodes in `nodes`.

- [ ] **Step 4: Verify query contracts**

Run:

```bash
cargo test -p compass-query --all-targets --locked
```

Expected: contract and Program reconciliation tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-model crates/compass-query
git commit -m "feat(query): define shared code query contract"
```

### Task 14: Build the disposable SQLite/FTS5 index and search

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/compass-query/Cargo.toml`
- Create: `crates/compass-query/src/index.rs`
- Create: `crates/compass-query/src/code_query.rs`
- Modify: `crates/compass-query/src/lib.rs`
- Test: `crates/compass-query/tests/code_search.rs`
- Test: `crates/compass-query/tests/index_recovery.rs`

**Interfaces:**

- Produces:
  `CodeQueryEngine::open(graph, program, cache_dir)`,
  `CodeQueryEngine::search(SearchRequest)`.
- Consumes: Tasks 1-13.

- [ ] **Step 1: Write failing FTS5 tests**

```rust
#[test]
fn ranking_is_exact_then_qualified_then_prefix_then_bm25() {
    let engine = fixture_engine();
    let hits = engine.search(SearchRequest::new("UserService")).unwrap();
    assert_eq!(names(&hits), [
        "UserService",
        "app.services.UserService",
        "UserServiceFactory",
        "LegacyUserServiceAdapter",
    ]);
}

#[test]
fn corrupt_cache_is_deleted_and_rebuilt() {
    let fixture = fixture_paths();
    fs::write(&fixture.index, b"not sqlite").unwrap();
    let engine = CodeQueryEngine::open(&fixture.graph, None, &fixture.cache).unwrap();
    assert_eq!(engine.search(SearchRequest::new("run")).unwrap().results.len(), 1);
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-query --test code_search --locked
cargo test -p compass-query --test index_recovery --locked
```

Expected: index and search APIs are missing.

- [ ] **Step 3: Implement the index**

Add the already-locked dependency:

```toml
rusqlite = { version = "0.31.0", features = ["bundled", "modern_sqlite"] }
```

Create `nodes_fts` with `name`, `qualified_name`, `kind`, `roles`, `path`,
`language`, `framework`, `signature`, and `documentation`. Cache identity is
SHA-256 over graph digest, Program digest, both schema fingerprints, and index
implementation version. Use parameterized queries and quote FTS terms.

- [ ] **Step 4: Verify search and recovery**

Run:

```bash
cargo test -p compass-query --test code_search --locked
cargo test -p compass-query --test index_recovery --locked
cargo test -p compass-query --all-targets --locked
```

Expected: search filters, ranking, cache invalidation, and corruption recovery
pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/compass-query
git commit -m "feat(query): index graph symbols with FTS5"
```

### Task 15: Implement callers, callees, and impact

**Files:**

- Modify: `crates/compass-query/src/code_query.rs`
- Modify: `crates/compass-query/src/affected.rs`
- Modify: `crates/compass-query/src/traversal.rs`
- Test: `crates/compass-query/tests/code_traversal.rs`
- Test: `crates/compass-query/tests/code_impact.rs`

**Interfaces:**

- Produces:
  `CodeQueryEngine::callers`, `CodeQueryEngine::callees`,
  `CodeQueryEngine::impact`.

- [ ] **Step 1: Write failing traversal tests**

```rust
#[test]
fn handler_callers_include_the_binding_route() {
    let engine = routing_engine();
    let response = engine.callers(CallRequest::one_hop("Users.show")).unwrap();
    assert!(response.nodes.iter().any(|node| node.kind == NodeKind::Route));
    assert!(response.edges.iter().any(|edge| edge.kind == EdgeKind::RoutesTo));
}

#[test]
fn exact_only_impact_excludes_heuristic_edges() {
    let engine = impact_engine();
    let response = engine.impact(ImpactRequest {
        exact_only: true,
        ..ImpactRequest::new("charge")
    }).unwrap();
    assert!(!response.edges.iter().any(|edge| edge.provenance.origin == Heuristic));
}
```

Add tests for one-hop default, bounded depth, middleware order, event/job
execution, parallel sites, containment entry, no sibling explosion, reason
paths, node caps, and truncation.

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-query --test code_traversal --locked
cargo test -p compass-query --test code_impact --locked
```

Expected: typed traversal methods are absent.

- [ ] **Step 3: Implement traversal families**

Execution edges are:

```rust
const EXECUTION: &[EdgeKind] = &[
    EdgeKind::Calls, EdgeKind::RoutesTo, EdgeKind::Handles,
    EdgeKind::Subscribes, EdgeKind::Schedules, EdgeKind::Triggers,
];
```

Impact uses incoming dependency edges and records the predecessor edge for
every hit. Enter a container's children at the same depth; never traverse an
incoming `contains` edge. Enforce `max_depth`, `max_nodes`, `max_edges`, and
return explicit truncation.

- [ ] **Step 4: Verify traversal**

Run:

```bash
cargo test -p compass-query --test code_traversal --locked
cargo test -p compass-query --test code_impact --locked
cargo test -p compass-query --all-targets --locked
```

Expected: all traversal tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-query
git commit -m "feat(query): add trusted traversal and impact"
```

### Task 16: Implement explore and node trail

**Files:**

- Create: `crates/compass-query/src/source.rs`
- Modify: `crates/compass-query/src/code_query.rs`
- Modify: `crates/compass-query/src/lib.rs`
- Test: `crates/compass-query/tests/code_explore.rs`
- Test: `crates/compass-query/tests/node_trail.rs`

**Interfaces:**

- Produces:
  `CodeQueryEngine::explore(ExploreRequest)` and
  `CodeQueryEngine::node_trail(NodeTrailRequest)`.

- [ ] **Step 1: Write failing source-integrity and path tests**

```rust
#[test]
fn explore_groups_verified_source_by_file_and_connects_symbols() {
    let engine = source_fixture_engine();
    let response = engine.explore(ExploreRequest::names(["route", "handler"])).unwrap();
    assert_eq!(response.files.len(), 2);
    assert_eq!(response.paths[0].edges[0].kind, EdgeKind::RoutesTo);
    assert!(response.files.iter().all(|file| file.source.is_some()));
}

#[test]
fn explore_omits_source_when_the_digest_is_stale() {
    let fixture = source_fixture();
    fs::write(fixture.root.join("src/handler.rs"), "changed").unwrap();
    let response = fixture.engine.explore(ExploreRequest::names(["handler"])).unwrap();
    assert!(response.files[0].source.is_none());
    assert!(response.diagnostics.iter().any(|d| d.code == "stale_source"));
}
```

Add tests for ambiguous names, Program signatures/types/effects, heuristic
wiring sites, route middleware, grouped slices, output budgets, and node-trail
ancestors/children/domain relationships.

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-query --test code_explore --locked
cargo test -p compass-query --test node_trail --locked
```

Expected: explore and node trail do not exist.

- [ ] **Step 3: Implement bounded source assembly**

Verify each file's digest before reading. Select the shortest evidence-rich
path, preferring exact over inferred over ambiguous/heuristic when hop counts
tie. Group source slices by normalized file path. Track `max_chars`,
`max_files`, `max_chars_per_file`, omitted files, and truncation in the
response.

- [ ] **Step 4: Verify explore and trail**

Run:

```bash
cargo test -p compass-query --test code_explore --locked
cargo test -p compass-query --test node_trail --locked
cargo test -p compass-query --all-targets --locked
```

Expected: all tests pass without returning stale source.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-query
git commit -m "feat(query): add explore and node trails"
```

## Milestone 5: Expose one contract through every client

### Task 17: Add CLI and MCP adapters

**Files:**

- Create: `crates/compass-cli/src/code_query_commands.rs`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/help.rs`
- Modify: `crates/compass-cli/src/query_commands.rs`
- Modify: `crates/compass-cli/src/call_graph_commands.rs`
- Create: `crates/compass-cli/tests/code_query_cli.rs`
- Create: `crates/compass-mcp/src/code_query.rs`
- Modify: `crates/compass-mcp/src/lib.rs`
- Test: `crates/compass-mcp/tests/code_query_tools.rs`

**Interfaces:**

- Produces CLI commands:
  `search`, `callers`, `callees`, `impact`, `explore`, `node`.
- Produces MCP tools:
  `search_symbols`, `get_callers`, `get_callees`, `get_impact`,
  `explore_code`, `get_node`.
- Existing `query`, `affected`, `explain`, and `call-graph` delegate to the new
  engine where their semantics overlap.

- [ ] **Step 1: Write failing CLI/MCP parity tests**

```rust
#[test]
fn cli_and_mcp_return_the_same_query_payload() {
    let cli = run_cli(["callers", "Users.show", "--format", "json"]);
    let mcp = invoke_mcp("get_callers", json!({"symbol":"Users.show"}));
    assert_eq!(
        canonicalize_query_payload(&cli.stdout),
        canonicalize_query_payload(&mcp)
    );
}

#[test]
fn expected_no_match_is_a_success_response() {
    let result = invoke_mcp("search_symbols", json!({"query":"absent"}));
    assert_eq!(result["schema"], "compass.query/1");
    assert_eq!(result["results"], json!([]));
    assert_eq!(result["diagnostics"][0]["code"], "no_match");
}
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
cargo test -p compass-cli --test code_query_cli --locked
cargo test -p compass-mcp --test code_query_tools --locked
```

Expected: commands/tools are unknown.

- [ ] **Step 3: Implement thin adapters**

CLI JSON output serializes `CodeQueryResponse` directly. Text output is a
renderer over that response. MCP returns structured JSON plus concise text
content and reserves MCP errors for schema corruption, unsafe paths, or engine
malfunctions. Add bounded request schemas with explicit defaults.

- [ ] **Step 4: Verify adapters**

Run:

```bash
cargo test -p compass-cli -p compass-mcp --all-targets --locked
```

Expected: CLI/MCP golden payloads match.

- [ ] **Step 5: Commit**

```bash
git add crates/compass-cli crates/compass-mcp
git commit -m "feat: expose Compass code queries"
```

### Task 18: Add the shared TypeScript contract and graph evidence UI

**Files:**

- Create: `packages/compass-viewer/src/contracts/codeQuery.ts`
- Create: `packages/compass-viewer/src/contracts/codeQuery.test.ts`
- Modify: `packages/compass-viewer/package.json`
- Modify: `packages/compass-viewer/src/contracts/graph.ts`
- Create: `packages/compass-viewer/src/graph/CodeEvidence.tsx`
- Create: `packages/compass-viewer/src/graph/CodeEvidence.test.tsx`
- Modify: `packages/compass-viewer/src/graph/GraphInspector.tsx`
- Modify: `packages/compass-viewer/src/graph/edgeLabels.ts`
- Modify: `packages/compass-viewer/src/graph/CompassGraph.tsx`

**Interfaces:**

- Produces strict Zod decoders and reusable provenance/coverage rendering.
- Consumes: `compass.query/1` and graph viewer projections.

- [ ] **Step 1: Write failing Zod and rendering tests**

```typescript
it("rejects heuristic edges without wiring evidence", () => {
  const parsed = CodeQueryResponseSchema.safeParse({
    ...fixtureResponse,
    edges: [{ ...fixtureEdge, provenance: {
      origin: "heuristic", rule: "event-dispatch", wiringSite: null
    }}]
  });
  expect(parsed.success).toBe(false);
});

it("renders the heuristic rule and wiring site", () => {
  render(<CodeEvidence edge={heuristicEdge} />);
  expect(screen.getByText("event-dispatch")).toBeVisible();
  expect(screen.getByText("src/events.ts:27")).toBeVisible();
});
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
npm run test -w @compass/viewer -- src/contracts/codeQuery.test.ts src/graph/CodeEvidence.test.tsx
```

Expected: contract and component files are missing.

- [ ] **Step 3: Implement strict decoders and evidence presentation**

Use `z.strictObject` for contract records. Mirror every Rust enum exactly.
Render exact, convention, ambiguous, and heuristic states with text and icons,
not color alone. Show rule, extractor, anchor, wiring site, coverage,
conflicts, stale state, and truncation.

- [ ] **Step 4: Verify viewer contracts**

Run:

```bash
npm run test -w @compass/viewer
npm run typecheck -w @compass/viewer
```

Expected: all viewer tests and type checks pass.

- [ ] **Step 5: Commit**

```bash
git add packages/compass-viewer
git commit -m "feat(viewer): display code graph evidence"
```

### Task 19: Integrate queries into the VS Code extension

**Files:**

- Create: `editors/vscode/src/views/codeQueryClient.ts`
- Create: `editors/vscode/src/views/codeQueryClient.test.ts`
- Modify: `editors/vscode/src/transport/messages.ts`
- Modify: `editors/vscode/src/transport/messages.test.ts`
- Modify: `editors/vscode/src/views/graphPanel.ts`
- Modify: `editors/vscode/src/webviews/graph.tsx`
- Modify: `editors/vscode/src/extension.ts`
- Modify: `editors/vscode/package.json`
- Modify: `editors/vscode/src/test/suite/extension.integration.ts`

**Interfaces:**

- Produces VS Code actions for search, callers, callees, impact, explore, and
  node trail; graph-schema rebuild guidance; source navigation.
- Consumes: CLI commands from Task 17 and contracts from Task 18.

- [ ] **Step 1: Write failing host/webview tests**

```typescript
it("requests callers for the selected node and validates the response", async () => {
  const client = fixtureClient();
  const response = await client.callers("node:user-show");
  expect(response.schema).toBe("compass.query/1");
  expect(client.invocations[0]?.args).toEqual([
    "callers", "node:user-show", "--format", "json"
  ]);
});

it("offers rebuild when graph v1 is missing", async () => {
  await openGraphWithCliError("unsupported graph schema: unversioned");
  expect(vscode.window.showWarningMessage).toHaveBeenCalledWith(
    expect.stringContaining("rebuild"),
    "Update Graph"
  );
});
```

- [ ] **Step 2: Verify the tests fail**

Run:

```bash
npm run test -w crabbuild-compass-vscode -- src/views/codeQueryClient.test.ts src/transport/messages.test.ts
```

Expected: client and query messages are missing.

- [ ] **Step 3: Implement VS Code actions**

Register:

- `compass.searchSymbols`
- `compass.showNodeTrail`
- `compass.showImpact`
- `compass.exploreSelection`

Reuse existing caller/callee commands but route them through
`codeQueryClient`. Post validated query results to the graph webview. Navigate
using returned source anchors. Never parse CLI text or raw `graph.json`
attributes.

- [ ] **Step 4: Verify the extension**

Run:

```bash
npm run test -w crabbuild-compass-vscode
npm run typecheck -w crabbuild-compass-vscode
npm run build -w crabbuild-compass-vscode
```

Expected: unit tests, type checks, and extension build pass.

- [ ] **Step 5: Commit**

Before staging, compare the existing user modification:

```bash
git diff -- editors/vscode/package.json
```

Preserve unrelated content, then commit only intentional extension changes:

```bash
git add editors/vscode/src editors/vscode/package.json
git commit -m "feat(vscode): explore trusted code graph evidence"
```

## Milestone 6: Qualify the complete release

### Task 20: Add cross-platform, determinism, and real-repository gates

**Files:**

- Create: `scripts/qualify_code_graph_v1.sh`
- Create: `scripts/check_code_graph_v1_coverage.py`
- Create: `tests/qualification/code-graph-v1-repositories.toml`
- Create: `docs/design/code-graph-v1-qualification.md`
- Modify: `.github/workflows/compass-ci.yml`
- Modify: `Makefile`
- Modify: `CHANGELOG.md`
- Test: `crates/compass-core/tests/code_graph_v1_determinism.rs`
- Test: `tests/viewer/query.spec.ts`

**Interfaces:**

- Produces one release qualification command and a machine-readable coverage
  report.
- Consumes every previous task.

- [ ] **Step 1: Write failing qualification checks**

The coverage script must fail when:

- a declared node/edge kind has zero producers;
- a framework lacks positive, near-match, exact, ambiguous, unresolved, or
  incremental coverage;
- clean and incremental graph bytes differ;
- a heuristic edge lacks rule/wiring site;
- CLI/MCP/VS Code schema fingerprints differ.

Add this shell contract:

```bash
./scripts/qualify_code_graph_v1.sh --fixtures-only
```

Expected before implementation: non-zero exit with a list of missing gates.

- [ ] **Step 2: Run the failing gate**

Run:

```bash
./scripts/qualify_code_graph_v1.sh --fixtures-only
```

Expected: failure listing all unimplemented checks.

- [ ] **Step 3: Implement the qualification pipeline**

The script runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
npm run test:js
npm run typecheck:js
npm run build
python3 scripts/check_code_graph_v1_coverage.py
```

The full mode additionally builds small, medium, and large repositories for
each supported language family and executes three route-to-handler flows for
every listed framework. Record repository revision, Compass revision, graph
digest, node/edge counts, query timings, unresolved counts, and false exact
resolutions in `docs/design/code-graph-v1-qualification.md`.

Commit the real-repository matrix in
`tests/qualification/code-graph-v1-repositories.toml`. Each entry contains
`name`, `url`, immutable 40-hex `commit`, `size_class`, `language_family`, and
`frameworks`. The script rejects branches, tags, shortened hashes, duplicate
size/language cells, and any framework with fewer than three declared
route-to-handler flows. It clones into a temporary directory and never writes
into the fixture or source trees.

- [ ] **Step 4: Run the fixture gate**

Run:

```bash
./scripts/qualify_code_graph_v1.sh --fixtures-only
```

Expected: exit 0.

- [ ] **Step 5: Run the real-repository and platform gates**

Run:

```bash
./scripts/qualify_code_graph_v1.sh \
  --repositories tests/qualification/code-graph-v1-repositories.toml
```

Expected: exit 0 and a qualification report containing results for every
locked repository and framework flow.

Run platform-sensitive suites on:

- macOS locally;
- Linux in the repository's documented Node/Rust Docker environment;
- the documented Windows VM for path, atomic publication, SQLite, and VS Code
  process behavior.

- [ ] **Step 6: Refresh the project graph**

Because implementation modifies code files, run from the outer Graphify
workspace:

```bash
cd /Users/haipingfu/graphify
graphify update .
```

Expected: `graphify-out/GRAPH_REPORT.md` records the implementation commit and
the update exits 0.

- [ ] **Step 7: Commit**

```bash
git add scripts/check_code_graph_v1_coverage.py scripts/qualify_code_graph_v1.sh tests/qualification/code-graph-v1-repositories.toml docs/design/code-graph-v1-qualification.md .github/workflows/compass-ci.yml Makefile CHANGELOG.md crates/compass-core/tests/code_graph_v1_determinism.rs tests/viewer/query.spec.ts
git commit -m "test: qualify Compass code graph v1"
```

## Final verification

- [ ] Run the full Rust test suite:

```bash
cargo test --workspace --all-targets --locked
```

- [ ] Run strict Rust linting:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

- [ ] Run JavaScript tests, type checks, and builds:

```bash
npm run test:js
npm run typecheck:js
npm run build
```

- [ ] Run the fixture qualification gate:

```bash
./scripts/qualify_code_graph_v1.sh --fixtures-only
```

- [ ] Run the locked real-repository gate:

```bash
./scripts/qualify_code_graph_v1.sh \
  --repositories tests/qualification/code-graph-v1-repositories.toml
```

- [ ] Inspect final scope:

```bash
git status --short
git diff --stat HEAD~20..HEAD
git log --oneline --decorate -20
```

- [ ] Confirm the original user-owned
  `editors/vscode/package.json` change is either preserved verbatim or
  intentionally incorporated and documented.

- [ ] Run the outer graph refresh after the final code change:

```bash
cd /Users/haipingfu/graphify
graphify update .
```

Expected: every command exits 0, the qualification document contains no
unresolved release gate, and only intentional files are modified.
