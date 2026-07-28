# Compass Code Graph v1 Implementation-First Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first supported `compass.graph/1` code graph with a closed
vocabulary, trustworthy provenance, enterprise and framework facts, shared
queries, and first-class VS Code presentation.

**Architecture:** Preserve a versioned NetworkX node-link envelope for the
durable artifact while replacing its free-form node and edge attributes with
strict Compass records. Extractors keep a flexible internal fact model;
`compass-resolve` resolves cross-file facts, and `compass-graph` is the only
publication boundary allowed to create v1 nodes and edges. `graph.json` and
the independently versioned Program IR remain authoritative, while a
rebuildable SQLite/FTS5 projection powers one `compass.query/1` contract used
by CLI, MCP, the viewer, and the VS Code extension.

**Tech Stack:** Rust 1.97.1, serde/serde_json, tree-sitter, SHA-256, rusqlite
0.31 with bundled modern SQLite/FTS5, TypeScript 5.9, Zod 4, React 19, Vitest,
Playwright, and the VS Code Extension API.

## How to use this plan

Each task explains the current system, the reason for the change, the exact
implementation boundary, and the contract it hands to later tasks. Implement
the production path first, then add focused verification and run the listed
gates. Each task ends in a reviewable, working commit; do not leave the
workspace in a deliberately failing state between tasks.

Tasks are ordered by dependency. A later task may consume only contracts
explicitly produced by an earlier task. When a task lists a file that was
created earlier in this plan, “Modify” means the file exists by that point.

## Global constraints

- The durable structural graph schema is exactly `compass.graph/1`.
- The envelope remains NetworkX-compatible with `directed`, `multigraph`,
  `graph`, `nodes`, and `links`.
- Every published graph is directed and multigraph, even when it has no
  parallel edges.
- The new contract is called v1. Existing unversioned artifacts are
  pre-contract artifacts, not an earlier supported version.
- Do not add a legacy adapter, compatibility mode, dual writer, or
  relationship aliases to the published artifact.
- Query/load entry points reject pre-contract artifacts with rebuild guidance.
  Index/update entry points rebuild them.
- Keep Program IR under its existing
  `http://crab.build/compass/v1` schema. Do not merge Program IR records into
  the durable structural graph.
- `graph.json`, `program.json`, manifest, and trusted build state publish as
  one generation.
- Every declared node and edge kind must have a validated producer before the
  release gate passes.
- Every heuristic edge carries a non-empty rule and a wiring-site source
  anchor.
- Ambiguous resolution retains a bounded, deterministically sorted candidate
  set. It never silently chooses the first candidate.
- `explore` returns source only when the current file digest matches the
  digest recorded in the graph.
- CLI, MCP, viewer, and VS Code consume the same `compass.query/1` semantics.
- Do not change Graphify or invoke Graphify’s TypeScript CodeGraph at runtime.
- Preserve unrelated worktree changes, especially
  `editors/vscode/package.json` and the call-graph hardening documents.
- After any task that changes code, run `graphify update .` from
  `/Users/haipingfu/graphify` after the task’s Compass verification succeeds.

---

## Current system and target boundaries

| Area | Current state | v1 target |
|---|---|---|
| Durable model | `compass-model::NodeRecord` and `EdgeRecord` flatten arbitrary JSON maps | Closed typed node, edge, provenance, file, coverage, and diagnostic records |
| Extraction | `compass-languages::Extraction` directly contains durable records | Extractors emit `RawNodeRecord`, `RawEdgeRecord`, calls, and framework/domain facts |
| Resolution | `compass-resolve` mutates free-form facts | Resolver consumes raw facts and emits explicit resolution state and candidates |
| Construction | `compass-graph::build_*` canonicalizes strings and arbitrary attributes | One `normalize_v1` boundary validates and publishes canonical v1 records |
| Loading | `GraphDocument` accepts permissive node-link JSON and caches by file signature | Strict schema loading with content-addressed derived caches |
| Query | In-memory scoring, CQL, traversal, and affected-node commands use independent projections | One typed `compass.query/1` response for search, callers, callees, impact, explore, and node trail |
| Program analysis | Program IR is a separate `compass/v1` artifact | Remains separate and joins only as optional query evidence |
| Clients | CLI, MCP, viewer, and VS Code have separate graph/query shapes | Thin adapters and one TypeScript mirror of `compass.query/1` |

## Planned module ownership

| Path | Responsibility |
|---|---|
| `crates/compass-model/src/code_graph.rs` | Closed v1 vocabulary and durable records |
| `crates/compass-model/src/provenance.rs` | Evidence origin, confidence, anchors, resolution state, and invariants |
| `crates/compass-model/src/identity.rs` | Portable deterministic IDs |
| `crates/compass-model/src/query_contract.rs` | Transport-neutral `compass.query/1` requests and responses |
| `crates/compass-model/src/document.rs` | NetworkX envelope serialization and strict loading |
| `crates/compass-model/src/validation.rs` | Whole-document validation |
| `crates/compass-languages/src/facts.rs` | Flexible pre-publication facts |
| `crates/compass-languages/src/frameworks/` | Local framework and convention detection |
| `crates/compass-resolve/src/frameworks/` | Cross-file route, message, job, and ORM resolution |
| `crates/compass-graph/src/v1.rs` | Raw-to-v1 normalization and canonical publication |
| `crates/compass-query/src/index.rs` | Disposable content-addressed SQLite/FTS5 index |
| `crates/compass-query/src/code_query.rs` | Shared query execution |
| `crates/compass-query/src/program_join.rs` | Optional Program IR evidence reconciliation |
| `crates/compass-cli/src/code_query_commands.rs` | CLI request parsing and rendering |
| `crates/compass-mcp/src/code_query.rs` | MCP tool definitions and response adapters |
| `packages/compass-viewer/src/contracts/codeQuery.ts` | Zod mirror of Rust query records |
| `packages/compass-viewer/src/graph/CodeEvidence.tsx` | Provenance and coverage presentation |
| `editors/vscode/src/views/codeQueryClient.ts` | Validated CLI-backed query client |

## Requirement-to-task map

| Approved requirement | Implemented by |
|---|---|
| Closed node/edge vocabulary and NetworkX envelope | Tasks 1, 3, and 5 |
| Portable identity and trustworthy provenance | Task 3 |
| Removal of free-form durable attributes | Tasks 2 and 4 |
| Hard cutover, canonical publication, file inventory, coverage | Task 5 |
| Database, schema, query, migration, configuration producers | Task 6 |
| Shared route model and complete framework matrix | Tasks 7–11 |
| Events, messages, jobs, schedules, and ORM mappings | Task 12 |
| Separate Program IR with query-time reconciliation | Task 13 |
| FTS5 symbol search | Task 14 |
| Callers, callees, impact, explore, and node trail | Task 15 |
| Shared CLI and MCP semantics | Task 16 |
| Viewer evidence and TypeScript contract | Task 17 |
| VS Code enterprise integration | Task 18 |
| Determinism, platform, and real-repository release gates | Task 19 |

---

## Milestone 1: Establish the durable contract

### Task 1: Add the closed v1 model without breaking current extraction

**Objective:** Introduce all durable v1 types alongside the current flexible
records so the repository continues to compile while later tasks migrate
producers.

**Context:** Today `crates/compass-model/src/document.rs` owns both the
NetworkX envelope and permissive records with flattened JSON attributes.
Those records are imported directly by every language extractor. Replacing
them in one edit would mix the public contract change with a repository-wide
producer migration. This task therefore adds the new contract in a separate
module and leaves the current root exports intact until Task 2 moves raw
facts out of `compass-model`.

**Files:**

- Create: `crates/compass-model/src/code_graph.rs`
- Create: `crates/compass-model/src/provenance.rs`
- Modify: `crates/compass-model/src/lib.rs`
- Test: `crates/compass-model/tests/code_graph_v1.rs`

**Implementation:**

- [x] Freeze the serialized node values as:
  `file`, `module`, `package`, `namespace`, `class`, `struct`, `interface`,
  `trait`, `protocol`, `enum`, `enum_member`, `type_alias`, `function`,
  `method`, `constructor`, `property`, `field`, `variable`, `constant`,
  `parameter`, `import`, `export`, `macro`, `annotation`, `route`,
  `component`, `event`, `message`, `topic`, `queue`, `job`, `resource`,
  `schema`, `query`, `migration`, `config_key`, `database`,
  `database_schema`, `database_table`, `database_view`, `database_column`,
  `database_index`, `database_constraint`, `database_procedure`, and
  `database_trigger`.
- [x] Freeze the serialized edge values as:
  `contains`, `calls`, `imports`, `exports`, `extends`, `implements`,
  `references`, `type_of`, `returns`, `instantiates`, `overrides`,
  `decorates`, `routes_to`, `reads`, `writes`, `aliases`, `registers`,
  `handles`, `publishes`, `subscribes`, `produces`, `consumes`, `schedules`,
  `triggers`, `tests`, `depends_on`, `documents`, and `maps_to`.
- [x] Add serde `snake_case` enums with the exact closed vocabulary:

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

- [x] Define `SourceAnchor`, `EvidenceOrigin`, `EvidenceConfidence`,
  `ResolutionState`, `ResolutionCandidate`, and `Provenance`.
  `EvidenceOrigin` is exactly `ast`, `config`, `convention`, `artifact`, or
  `heuristic`. SCIP is an `artifact`; Program IR remains a separate query
  evidence layer and is not a durable structural origin. This task establishes
  the serialized shape; Task 3 adds construction and whole-graph invariants.
- [x] Define typed `NodeRecord`, `EdgeRecord`, `FileRecord`,
  `GraphMetadata`, `CoverageRecord`, and `GraphDiagnostic`. Put kind-specific
  fields in tagged detail enums instead of a flattened attributes map. Keep
  community IDs, scores, colors, and labels typed and optional so existing
  clustering can enrich v1 nodes without reopening the vocabulary.
- [x] Give each edge both `id` and NetworkX `key`; require them to contain the
  same deterministic identity. Use `kind`, never `relation`, as the published
  relationship field.
- [x] Apply `#[serde(deny_unknown_fields)]` to structured records. Unknown
  enum values and misspelled required fields must fail deserialization.
- [x] Export the module as `compass_model::code_graph` but do not replace the
  existing root-level flexible record exports yet.

**Contract produced:** Later tasks can build against
`compass_model::code_graph::{NodeRecord, EdgeRecord, NodeKind, EdgeKind,
GraphMetadata}` without changing current extractors. Task 3 constructs the v1
envelope in parallel; Task 4 switches existing consumers to it.

**Verification:**

- Add record round-trip fixtures that assert closed enum behavior,
  `routes_to` spelling, `id == key`, and absence of `relation`.
- Run:

```bash
cargo test -p compass-model --all-targets --locked
cargo check --workspace --all-targets --locked
```

**Done when:** The typed model can serialize and reject malformed v1
documents, while all current extractors and graph commands still compile
against the old internal records.

**Commit:** `feat(model): define Compass code graph v1`

### Task 2: Move flexible records to the raw extraction boundary

**Objective:** Make free-form maps explicitly internal so only
`compass-graph` can create durable v1 records.

**Context:** `compass-languages::Extraction` currently stores
`compass_model::{NodeRecord, EdgeRecord}`. `compass-resolve` and
`compass-graph` both depend on their arbitrary attribute maps. The v1 model
cannot be trustworthy while producers can write directly into the durable
shape. This task preserves existing extraction behavior but renames and
relocates the flexible types.

**Files:**

- Modify: `crates/compass-languages/src/facts.rs`
- Modify: `crates/compass-languages/Cargo.toml`
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
- Modify: `crates/compass-graph/src/dedup.rs`
- Modify: `crates/compass-graph/src/lib.rs`
- Modify: `crates/compass-graph/src/analyze.rs` (Rust 1.97 lint cleanup required by the verification gate)
- Modify: `crates/compass-cli/src/dedup_commands.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Test: `crates/compass-languages/tests/typed_extraction.rs`

**Implementation:**

- [x] Define `RawNodeRecord` and `RawEdgeRecord` in `facts.rs` with the current
  `id`/endpoint fields and flattened JSON maps. Change `Extraction` to:

```rust
pub struct Extraction {
    pub nodes: Vec<RawNodeRecord>,
    pub edges: Vec<RawEdgeRecord>,
    pub hyperedges: Vec<serde_json::Value>,
    pub raw_calls: Option<Vec<RawCall>>,
    pub error: Option<String>,
    pub extensions: serde_json::Map<String, serde_json::Value>,
}
```
- [x] Re-export the raw records from `compass-languages`. Update extractors
  mechanically to construct them; do not change producer semantics in this
  task.
- [x] Update resolution and deduplication signatures to consume raw records.
  Keep current string relations and attributes lossless through these phases.
- [x] Change `compass-graph::build`, `build_with_tiebreaker`, and
  `build_owned_with_tiebreaker` to accept raw extraction records. They may
  still return the current permissive document until Task 3 installs the v1
  normalization boundary.

**Contract produced:** Language and resolver crates own producer-facing raw
facts. Only the temporary graph-construction compatibility path still uses
the old permissive document, and Task 3 removes it. Existing extraction output
remains behaviorally equivalent.

**Verification:**

- Add a dependency-boundary check proving `compass-languages` and
  `compass-resolve` construct raw records while only `compass-graph` imports
  durable node and edge types.
- Run:

```bash
cargo test -p compass-languages -p compass-resolve -p compass-graph --all-targets --locked
cargo check --workspace --all-targets --locked
```

**Done when:** No extractor or resolver constructs
`compass_model::NodeRecord` or `EdgeRecord`, and the workspace still produces
the same pre-v1 behavior through the temporary graph compatibility path.

**Commit:** `refactor: isolate raw graph extraction facts`

### Task 3: Implement portable identity, provenance, and v1 normalization

**Objective:** Convert resolved raw facts into deterministic v1 records with
validated evidence.

**Context:** Current IDs and evidence are assembled differently by language
extractors, and edge confidence is carried in ad hoc strings such as
`EXTRACTED`. Current producer relations also include aliases such as
`imports_from`, `re_exports`, and `indirect_call`. This task creates the
single normalization table and identity scheme that all publication paths
must use.

**Files:**

- Create: `crates/compass-model/src/identity.rs`
- Modify: `crates/compass-model/src/code_graph.rs`
- Modify: `crates/compass-model/src/provenance.rs`
- Modify: `crates/compass-model/src/validation.rs`
- Modify: `crates/compass-model/src/error.rs`
- Create: `crates/compass-graph/src/v1.rs`
- Modify: `crates/compass-graph/src/lib.rs`
- Test: `crates/compass-model/tests/code_graph_identity.rs`
- Test: `crates/compass-model/tests/code_graph_validation.rs`
- Test: `crates/compass-graph/tests/graph_v1_normalization.rs`

**Implementation:**

- [x] Add constructors and validation for the Task 1 evidence records.
  Heuristic evidence requires both `rule` and `wiring_site`;
  non-heuristic structural evidence requires at least one anchor.
- [x] Implement length-prefixed SHA-256 identity functions so IDs are stable
  across checkout roots:

```rust
pub fn file_id(normalized_path: &str) -> String;
pub fn symbol_id(language: &str, normalized_path: &str, kind: NodeKind,
                 qualified_name: &str, disambiguator: &str) -> String;
pub fn route_id(framework: &str, normalized_path: &str, operation: &str,
                route_path: &str, declaring_scope: &str) -> String;
pub fn messaging_id(kind: NodeKind, transport: &str, subject: &str,
                    declaring_scope: &str) -> String;
pub fn database_entity_id(kind: NodeKind, logical_database: &str,
                          database_schema: &str, qualified_name: &str) -> String;
pub fn domain_id(kind: NodeKind, namespace: &str, qualified_name: &str) -> String;
pub fn edge_id(source: &str, kind: EdgeKind, target: &str,
               relationship_site: Option<&SourceAnchor>,
               rule: Option<&str>) -> String;
```

- [x] Normalize paths to repository-relative forward-slash form before
  hashing. Do not include an absolute checkout root, timestamps, extraction
  order, or machine-specific data.
- [x] Include the `compass.graph/1` schema identity in every hash preimage.
  Treat a rename or move as remove/add unless exact alias evidence establishes
  continuity; never infer continuity from name similarity.
- [x] Implement
  `normalize_v1(Extraction, BuildEvidence) -> Result<
  compass_model::code_graph::GraphDocument, GraphError>` in
  `compass-graph/src/v1.rs`. This is the only function allowed to turn raw
  records into durable records.
- [x] Define the v1 envelope in `code_graph.rs`:

```rust
pub struct GraphDocument {
    pub directed: bool,
    pub multigraph: bool,
    pub graph: GraphMetadata,
    pub nodes: Vec<NodeRecord>,
    pub links: Vec<EdgeRecord>,
}
```

  Keep it under `compass_model::code_graph` for this task. The existing
  crate-root loader and production builder remain on the temporary permissive
  document until Task 4 migrates all consumers in one compilable change.
- [x] Encode the current relation migration table:

| Raw relation | Published kind | Additional evidence |
|---|---|---|
| `imports_from` | `imports` | retain producer spelling only in evidence detail |
| `re_exports` | `exports` | retain producer spelling only in evidence detail |
| `inherits` | `extends` | none |
| `indirect_call` | `calls` | heuristic rule `indirect-call-resolution` and wiring site |
| `reads_from` | `reads` | none |
| `references_constant`, `uses_static_prop`, `uses` | `references` | none |
| `rationale_for` | `documents` | none |
| `configures` | `depends_on` | none |
| `case_of`, `defines`, `method` | `contains` | none |
| `embeds` | `contains` | rule `embedded-member` |
| `mixes_in` | `implements` | rule `mixin-contract` |

- [x] Normalize current semantic/media nodes to `resource` and set
  `resource_kind` to `document`, `paper`, `image`, `concept`, or `rationale`.
  Reject any raw kind or relation not present in an explicit normalization
  table; report its producer path and anchor.
- [x] Validate endpoint-kind compatibility, duplicate identities, missing
  files, invalid anchors, heuristic evidence, candidate bounds, and edge
  `id == key` before returning a document. The initial closed endpoint matrix
  includes:

  - `route routes_to function|method|class|component`;
  - callable nodes `calls` callable or constructible nodes;
  - callable nodes `publishes event|message|topic`;
  - callable nodes `handles event|message`;
  - callable nodes `reads|writes
    database|database_schema|database_table|database_view|database_column|config_key`;
  - type nodes `maps_to database_table|database_view`;
  - callable or configuration nodes `schedules job`;
  - `job triggers function|method`;
  - test-role code symbols `tests` any code or domain symbol;
  - container nodes `contains` their declared members.

  Reject invalid pairs rather than degrading them to `references`.

**Contract produced:** A resolved `Extraction` becomes one valid,
deterministically identified v1 document or a specific build error.

**Verification:**

- Add root-relocation, duplicate-ID, missing-wiring-site, ambiguous-candidate,
  raw-alias, and endpoint-matrix cases.
- Run:

```bash
cargo test -p compass-model -p compass-graph --all-targets --locked
```

**Done when:** Reordering raw facts or moving a checkout does not change IDs;
the v1 normalization path never emits an alias or unknown kind; malformed
provenance cannot enter its document; and the existing production path still
works until the consumer migration.

**Commit:** `feat(graph): normalize trusted v1 records`

### Task 4: Migrate durable graph consumers from attribute maps

**Objective:** Move every graph reader, algorithm, query adapter, and exporter
to typed v1 access before strict loading becomes mandatory.

**Context:** Replacing `NodeRecord.attributes` and `EdgeRecord.attributes`
changes more than extraction. Clustering, Graph/NetworkX projections,
CompassQL property lookup, history, output formats, PostgreSQL integration,
semantic diff, MCP, and CLI history currently read arbitrary keys directly.
Leaving those reads implicit would either break the workspace or encourage a
hidden compatibility map. This task provides typed accessors and explicit
property projections, then migrates all durable consumers.

**Files:**

- Modify: `crates/compass-model/src/code_graph.rs`
- Modify: `crates/compass-model/src/document.rs`
- Modify: `crates/compass-model/src/lib.rs`
- Modify: `crates/compass-model/src/graph.rs`
- Modify: `crates/compass-model/src/query_index.rs`
- Modify: `crates/compass-analysis/src/universal_call_graph.rs`
- Modify: `crates/compass-graph/src/analyze.rs`
- Modify: `crates/compass-graph/src/cluster.rs`
- Modify: `crates/compass-graph/src/dedup.rs`
- Modify: `crates/compass-graph/src/lib.rs`
- Modify: `crates/compass-core/src/lib.rs`
- Modify: `crates/compass-core/src/cluster_existing.rs`
- Modify: `crates/compass-core/src/history.rs`
- Modify: `crates/compass-core/src/merge.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-global/src/lib.rs`
- Modify: `crates/compass-graphdb/src/queries.rs`
- Modify: `crates/compass-history/src/artifacts.rs`
- Modify: `crates/compass-media/src/lib.rs`
- Modify: `crates/compass-output/src/callflow.rs`
- Modify: `crates/compass-output/src/callflow_model.rs`
- Modify: `crates/compass-output/src/canvas.rs`
- Modify: `crates/compass-output/src/cypher.rs`
- Modify: `crates/compass-output/src/graphml.rs`
- Modify: `crates/compass-output/src/history_bundle.rs`
- Modify: `crates/compass-output/src/history_viewer.rs`
- Modify: `crates/compass-output/src/html.rs`
- Modify: `crates/compass-output/src/json.rs`
- Modify: `crates/compass-output/src/obsidian.rs`
- Modify: `crates/compass-output/src/report.rs`
- Modify: `crates/compass-output/src/svg.rs`
- Modify: `crates/compass-output/src/tree.rs`
- Modify: `crates/compass-postgres/src/lib.rs`
- Modify: `crates/compass-prs/src/lib.rs`
- Modify: `crates/compass-query/src/cql/eval.rs`
- Modify: `crates/compass-semantic-diff/src/engine.rs`
- Modify: `crates/compass-semantic-diff/src/logic.rs`
- Modify: `crates/compass-mcp/src/lib.rs`
- Modify: `crates/compass-cli/src/history_build.rs`
- Test: `crates/compass-model/tests/typed_property_projection.rs`
- Modify: `crates/compass-graph/tests/build_coverage.rs`
- Test: `crates/compass-output/tests/code_graph_v1_exports.rs`

**Implementation:**

- [ ] Replace the permissive records and envelope in `document.rs` with
  re-exports of `code_graph::GraphDocument`, `NodeRecord`, and `EdgeRecord`.
  Remove the flattened durable definitions, export typed records at the crate
  root, and route `compass-graph::build*` through `normalize_v1`.
- [x] Add typed convenience accessors for frequently used structural fields:
  label, kind, roles, source anchor, language, framework, community, evidence,
  edge kind, and edge endpoints. These methods read typed fields and do not
  reconstruct a mutable attribute map.
- [x] Add read-only `NodePropertyProjection` and `EdgePropertyProjection`
  iterators for generic consumers such as CompassQL, Cypher, GraphML, and JSON
  export. The projection has an explicit key registry and converts typed
  fields to `GraphProperty` values; unknown keys return `None`.
- [ ] Update graph construction, clustering, deduplication, analysis, merge,
  and global-graph code to use typed fields and `EdgeKind` matching.
- [ ] Update `compass-model::Graph`, query indexes, CompassQL evaluation, and
  graph database queries to use the property projection where generic lookup
  is required. Preserve existing query-visible property names through the
  projection without storing them as arbitrary durable attributes.
- [ ] Update history, semantic diff, PR analysis, and universal call graph
  logic to compare typed records and evidence. Changes in provenance,
  resolution state, roles, or kind-specific detail must be visible changes.
- [ ] Update every output format to serialize from typed records. Exporters
  may map v1 fields into format-specific property bags at the boundary, but
  those bags are never written back into `GraphDocument`.
- [ ] Update MCP and CLI history code that directly reads attributes. The new
  code-query tools remain a later task; this change only keeps current
  commands working on typed records.
- [ ] Remove all durable Rust `.attributes` access outside raw extraction. Add
  a repository check that permits it only on `RawNodeRecord` and
  `RawEdgeRecord` inside language/resolver code and their extraction tests.

**Contract produced:** Every existing Compass consumer can read the typed v1
document without a flattened compatibility map, and generic query/export
features use an explicit read-only property projection.

**Verification:**

- Verify clustering, CQL property access, graph algorithms, history diff,
  semantic diff, MCP summaries, and each export family against one v1 fixture.
- Run:

```bash
cargo test -p compass-model -p compass-graph -p compass-query --all-targets --locked
cargo test -p compass-output -p compass-history -p compass-semantic-diff --all-targets --locked
cargo check --workspace --all-targets --locked
```

**Done when:** The workspace compiles with typed durable records, exports and
existing queries preserve their intended public behavior, and no durable
consumer reaches into an arbitrary attribute map.

**Commit:** `refactor: migrate graph consumers to typed records`

### Task 5: Hard-cut publication and loading to `compass.graph/1`

**Objective:** Make v1 the only supported durable graph and publish it
atomically with complete file and coverage metadata.

**Context:** `GraphDocument::load` currently accepts permissive node-link JSON
and maintains signature-based binary caches. The pipeline and history store
load graph artifacts independently. A hard cutover is trustworthy only if
every loader agrees on schema validation and every build publishes graph,
Program IR, manifest, and build state as one generation.

**Files:**

- Modify: `crates/compass-model/src/document.rs`
- Modify: `crates/compass-model/src/validation.rs`
- Modify: `crates/compass-model/src/error.rs`
- Modify: `crates/compass-graph/src/v1.rs`
- Modify: `crates/compass-core/src/pipeline.rs`
- Modify: `crates/compass-core/src/build_state.rs`
- Modify: `crates/compass-files/src/manifest.rs`
- Modify: `crates/compass-history/src/store.rs`
- Modify: `crates/compass-history/src/artifacts.rs`
- Modify: `crates/compass-history/src/validate.rs`
- Test: `crates/compass-core/tests/loading_coverage.rs`
- Test: `crates/compass-core/tests/program_pipeline.rs`
- Test: `crates/compass-history/tests/roundtrip.rs`

**Implementation:**

- [ ] Replace the permissive `GraphDocument` loader with strict v1
  deserialization and whole-document validation. A missing or different
  `graph.schema` returns `UnsupportedGraphSchema` with an explicit instruction
  to run `compass update`. Retain explicit graph and Program artifact size
  caps before allocation.
- [ ] At index/update entry points, detect an existing pre-contract graph and
  perform a clean rebuild. At read-only query, history, merge, recluster,
  affected, and MCP entry points, reject it. Do not translate it in memory.
- [ ] Populate `graph` metadata with:
  `schema`, `build`, `files`, `coverage`, and `diagnostics`. A `FileRecord`
  includes normalized path, language, content digest, size, parse state, and
  extractor versions. Do not copy source text into `graph.json`.
- [ ] Record explicit coverage for supported, unsupported, excluded, parse
  failure, partial, generated, and binary files. Strict mode fails on required
  incomplete coverage; default mode publishes diagnostics without pretending
  those files were indexed.
- [ ] Sanitize diagnostics so paths remain repository-relative and messages do
  not copy unrelated environment variables, credentials, or configuration
  secrets.
- [x] Sort files by normalized path, nodes by ID, and links by
  `(source, kind, target, key)`. Canonicalize maps before serialization so
  clean and incremental builds produce byte-identical JSON. Always publish
  `directed: true` and `multigraph: true`; do not derive either flag from the
  current edge inventory.
- [x] Replace signature-only graph caches with a content-addressed key over
  graph digest and schema fingerprint. Treat caches as disposable and rebuild
  them after corruption or mismatch.
- [ ] Stage `graph.json`, `program.json`, manifest, and build state under one
  generation directory, fsync files and directory, then atomically switch the
  active generation. A failed build leaves the prior generation readable.
- [ ] Bump history and trusted-build fingerprints to include
  `compass.graph/1`; history rematerialization must reproduce the same
  generation contract by checking out source and rebuilding v1. It must not
  load or translate an unversioned graph stored by an older commit.

**Contract produced:** All supported loaders see a validated v1 generation;
all builders either publish the complete generation or preserve the previous
one.

**Verification:**

- Cover unversioned load rejection, automatic rebuild on update, interrupted
  publication, corrupt cache recovery, missing Program IR, file coverage, and
  clean/incremental byte equality.
- Run:

```bash
cargo test -p compass-model -p compass-core -p compass-history --all-targets --locked
```

**Done when:** No product path can mistake a pre-contract graph for v1, and a
reader never observes a mixed graph/Program/manifest generation.

**Commit:** `feat(core): hard cut over to Compass graph v1`

---

## Milestone 2: Add enterprise and framework producers

### Task 6: Emit database, schema, query, migration, and configuration facts

**Objective:** Supply trustworthy first-release producers for the database
and configuration portion of the enterprise vocabulary.

**Context:** The v1 release gate forbids declaring unused kinds. Compass
already parses SQL, manifests, JSON configuration, and Terraform, but those
extractors currently emit generic symbols and relations. This task upgrades
only evidence that can be tied to exact syntax or configuration anchors.

**Files:**

- Modify: `crates/compass-languages/src/sql.rs`
- Modify: `crates/compass-languages/src/package_manifest.rs`
- Modify: `crates/compass-languages/src/json_config.rs`
- Modify: `crates/compass-languages/src/terraform.rs`
- Create: `fixtures/code-graph/domain/database.sql`
- Create: `fixtures/code-graph/domain/database.expected.json`
- Test: `crates/compass-languages/tests/domain_extraction.rs`
- Test: `crates/compass-graph/tests/domain_normalization.rs`

**Implementation:**

- [x] Extend SQL extraction to emit raw facts for database, schema, table,
  view, column, index, constraint, procedure, trigger, query, and migration
  when the parsed statement provides an exact name and anchor.
- [x] Model SQL containment explicitly:
  database → database_schema → table/view/procedure, and table/view → columns,
  indexes, constraints, and triggers.
- [x] Emit `reads` and `writes` from exact SQL table references. Preserve
  qualification and aliases in typed detail; do not resolve dynamic SQL
  strings as exact.
- [x] Emit packages, resources, and config keys from manifest, JSON, and
  Terraform syntax. Use `depends_on`, `contains`, and `references` only when
  the source declares that relation.
- [x] Add producer identifiers to coverage metadata so the release gate can
  prove which extractor emitted each declared kind and edge.

**Contract produced:** The v1 vocabulary has anchored database and
configuration producers independent of ORM inference.

**Verification:**

- Verify nested database containment, quoted identifiers, migration files,
  query read/write direction, configuration references, malformed SQL, and
  dynamic SQL non-resolution.
- Run:

```bash
cargo test -p compass-languages --test domain_extraction --locked
cargo test -p compass-graph --test domain_normalization --locked
```

**Done when:** Every database/configuration kind has a real positive fixture
and near matches never become exact nodes or edges.

**Commit:** `feat(languages): extract database domain facts`

### Task 7: Build the shared framework fact and route-resolution substrate

**Objective:** Give every routing pack one local fact model and one
deterministic cross-file resolver.

**Context:** Framework syntax differs, but route publication semantics do
not. A route always has a framework, operation, normalized path, declaration
anchor, provenance, resolution state, and ordered execution stages. Without a
shared substrate, each framework would invent incompatible route records and
candidate behavior.

**Files:**

- Create: `crates/compass-languages/src/frameworks/mod.rs`
- Create: `crates/compass-languages/src/frameworks/model.rs`
- Modify: `crates/compass-languages/src/facts.rs`
- Modify: `crates/compass-languages/src/lib.rs`
- Create: `crates/compass-resolve/src/frameworks/mod.rs`
- Create: `crates/compass-resolve/src/frameworks/routes.rs`
- Modify: `crates/compass-resolve/src/lib.rs`
- Test: `crates/compass-resolve/tests/framework_routes.rs`

**Implementation:**

- [x] Define `RawFrameworkFact::Route(RawRouteFact)` and
  `RawFrameworkFact::Domain(RawDomainFact)`.
- [x] Add `framework_facts: Vec<RawFrameworkFact>` to `Extraction` with an
  empty default so non-framework extractors remain unchanged.
- [x] Define `RawRouteFact` with framework, operation, raw and normalized path,
  declaring scope, route anchor, handler reference, ordered middleware
  references, provenance origin, and framework-specific detail.
- [x] Define resolver output as:

```rust
pub struct ResolvedRoute {
    pub route: RawRouteFact,
    pub state: ResolutionState,
    pub stages: Vec<RouteStage>,
    pub candidates: Vec<ResolutionCandidate>,
}

pub struct RouteStage {
    pub position: u32,
    pub role: RouteStageRole, // Middleware or Handler
    pub target: String,
    pub provenance: Provenance,
}
```

- [x] Resolve exact handler IDs by qualified name, import/export aliases,
  declaring scope, and framework convention. If several candidates remain,
  return at most 20, sorted by stable ID, with reasons and confidence.
- [x] Centralize `FrameworkLimits` with `max_candidates: 20`,
  `max_include_depth: 32`, `max_alias_expansions: 1_000`, and
  `max_facts_per_file: 100_000`. Configuration parsers and resolvers fail with
  a diagnostic when a bound is reached; they do not continue partial
  recursive expansion silently.
- [x] Publish a `route` node and one `routes_to` edge per execution stage.
  Middleware edges carry `stage: "middleware"` and zero-based `position`; the
  final handler carries `stage: "handler"`.
- [x] Use `ast` for explicit bindings, `config` for routing files,
  `convention` for file-based routes, and `heuristic` only at a dynamic
  dispatch boundary with a rule and wiring site.
- [x] Reject URL-looking strings unless a recognized framework shape or
  convention produced them.

**Contract produced:** Framework packs only detect local shapes; the shared
resolver owns exact/ambiguous/unresolved semantics and `routes_to`
publication.

**Verification:**

- Exercise exact, ambiguous, unresolved, duplicate, middleware-order,
  candidate-limit, heuristic-evidence, and URL-like near-match cases.
- Run:

```bash
cargo test -p compass-resolve --test framework_routes --locked
```

**Done when:** A synthetic route from any language produces the same typed
shape and resolution behavior.

**Commit:** `feat(resolve): add framework route substrate`

### Task 8: Add Django, Flask, and FastAPI routing packs

**Objective:** Detect and resolve the full Python routing matrix.

**Context:** Python extraction already identifies functions, classes,
decorators, imports, and calls. The framework pack should reuse those facts
and add only route-specific facts. Cross-file and dotted-path resolution
belongs in `compass-resolve`, not in the AST visitor.

**Files:**

- Create: `crates/compass-languages/src/frameworks/python.rs`
- Create: `crates/compass-resolve/src/frameworks/python.rs`
- Create: `fixtures/code-graph/routes/python/django_urls.py`
- Create: `fixtures/code-graph/routes/python/flask_app.py`
- Create: `fixtures/code-graph/routes/python/fastapi_app.py`
- Create: `fixtures/code-graph/routes/python/near_matches.py`
- Test: `crates/compass-resolve/tests/python_routes.rs`

**Implementation:**

- [x] Detect Django `path`, `re_path`, legacy `url`, `include`, class-based
  `.as_view()`, and dotted handler strings in recognized URL modules.
- [x] Resolve `include()` recursively with cycle detection and a 32-file
  nesting bound. Compose parent and child paths without discarding source
  anchors.
- [x] Detect Flask `@app.route` and blueprint routes, including declared HTTP
  method arrays and blueprint prefixes.
- [x] Detect FastAPI `@app` and `@router` decorators for every standard HTTP
  method, router prefixes, and dependency/middleware execution stages that
  can be proven statically.
- [x] Resolve imported functions, bound methods, class-based views, aliases,
  and dotted paths against existing Python symbol facts.
- [x] Leave dynamic import strings and computed route expressions unresolved
  or heuristic; never report them as exact.

**Contract produced:** Python framework files emit `route` nodes and ordered
`routes_to` edges with exact, convention, or surfaced heuristic evidence.

**Verification:**

- Verify all recognized shapes, blueprint/router prefixes, nested Django
  includes, `.as_view()`, aliases, ambiguous handlers, computed strings, and
  ordinary decorators that happen to accept paths.
- Run:

```bash
cargo test -p compass-resolve --test python_routes --locked
```

**Done when:** Calling the callers query for a Python handler can surface its
binding URL and evidence.

**Commit:** `feat(frameworks): resolve Python routes`

### Task 9: Add TypeScript/JavaScript and file-based routing packs

**Objective:** Cover server routers and front-end/file conventions in the
JavaScript ecosystem.

**Context:** Express and NestJS use explicit calls/decorators, while
SvelteKit, Nuxt, and Astro derive routes from normalized file paths. React
Router and Vue Router sit between those models. The extractor must retain
middleware order and the resolver must distinguish HTTP, GraphQL, messaging,
WebSocket, page component, endpoint, and route-middleware roles.

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

**Implementation:**

- [x] Detect Express `app`/`router` verb calls and preserve every handler in
  the middleware chain in source order.
- [x] Detect NestJS `@Controller` plus HTTP method decorators; GraphQL
  `@Resolver`, `@Query`, and `@Mutation`; `@MessagePattern`,
  `@EventPattern`, and `@SubscribeMessage`. HTTP and GraphQL operations
  publish route nodes and `routes_to` edges with distinct operation types;
  message, event, and WebSocket facts feed Task 12.
- [x] Detect React Router route elements/configuration and Vue Router route
  objects only when path and component/loader/action fields match the
  supported AST shape.
- [x] Derive SvelteKit, Nuxt, and Astro page/endpoint routes from
  repository-relative paths. Support `[param]` and `[...rest]`, preserve the
  original convention path, and use `convention` provenance.
- [x] Emit Nuxt server endpoints and route middleware separately from page
  components. Resolve imported components and handlers through the existing
  TypeScript import/export graph.
- [x] Mark reflective NestJS wiring heuristic only when its registration site
  is known; leave arbitrary computed router calls unresolved.

**Contract produced:** JavaScript ecosystems share route semantics while
retaining framework, operation type, execution stage, and convention detail.

**Verification:**

- Cover middleware chains, controller prefixes, GraphQL/message separation,
  lazy imports, dynamic segments, rest segments, method-suffixed Nuxt APIs,
  Astro endpoints, computed paths, and non-framework method calls.
- Run:

```bash
cargo test -p compass-resolve --test typescript_routes --locked
```

**Done when:** Explicit and file-based routes resolve to their code symbols
without treating arbitrary JSX, decorators, or strings as routes.

**Commit:** `feat(frameworks): resolve TypeScript and file routes`

### Task 10: Add Laravel, Drupal, Rails, Spring, and Play routing packs

**Objective:** Cover PHP, Ruby, Java, and Scala routing configuration and
annotations.

**Context:** These frameworks mix AST syntax with external routing files.
Drupal YAML and Play `conf/routes` need file parsers that emit configuration
anchors; Laravel and Rails require framework-specific call/DSL recognition;
Spring relies on annotations. All handler resolution still terminates in
existing language symbol IDs.

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

**Implementation:**

- [x] Detect Laravel `Route` verbs, `resource`, `Controller@action`, class
  constants, and tuple syntax. Expand resource routes into stable individual
  route nodes with evidence pointing to the resource declaration.
- [x] Parse Drupal `*.routing.yml` for `_controller`, `_form`, and entity
  handlers; connect recognized `hook_*` implementations in supported
  `.module`, `.theme`, `.install`, and `.inc` files.
- [x] Detect Rails verb routes using `to:` and hash-rocket syntax and resolve
  `users#index` to controller class plus action method.
- [x] Detect Spring `@GetMapping`, `@PostMapping`, other composed mappings,
  and method-level `@RequestMapping`; compose class and method prefixes.
- [x] Parse Play verb routes from `conf/routes` and resolve Java or Scala
  controller actions, including static and injected controller forms.
- [x] Give YAML and route-file edges `config` provenance and exact line
  anchors. Dynamic controller names remain bounded candidates or unresolved.

**Contract produced:** PHP/Ruby/JVM handlers have the same canonical
`routes_to` relationship as Python and TypeScript handlers.

**Verification:**

- Cover every listed syntax, route groups/prefixes, resource expansion,
  malformed configuration, handler overloading, controller ambiguity, and
  DSL-like near matches outside framework contexts.
- Run:

```bash
cargo test -p compass-resolve --test php_ruby_jvm_routes --locked
```

**Done when:** Configuration-file and annotation routes navigate to exact
actions with the original wiring anchor visible.

**Commit:** `feat(frameworks): resolve PHP Ruby and JVM routes`

### Task 11: Add Go, Rust, ASP.NET, and Vapor routing packs

**Objective:** Cover native/server frameworks that register handlers through
calls, attributes, or macros.

**Context:** Gin, chi, gorilla/mux, Axum, Actix, Rocket, ASP.NET, and Vapor all
have concise route shapes that can be confused with ordinary method calls.
Detection must therefore require framework imports, receiver types,
attributes, or macros rather than matching method names alone.

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

**Implementation:**

- [x] Detect Gin group/engine verbs, chi methods, gorilla/mux
  `HandleFunc`/method chains, and nested path prefixes.
- [x] Detect Axum and Actix `.route(..., get(handler))` forms and Rocket
  route attributes/macros, preserving macro or call anchors.
- [x] Detect ASP.NET HTTP method attributes on controller actions and compose
  controller-level route attributes with action templates.
- [x] Detect Vapor application and route-group methods, including segmented
  string paths and explicit `use:` handlers.
- [x] Resolve closures as anchored function-like symbols when they exist in
  the structural graph. Do not invent exact handler symbols for opaque
  runtime values.
- [x] Require recognized imports, receiver types, namespaces, attributes, or
  macros before emitting a route fact.

**Contract produced:** All remaining release-one server frameworks publish
canonical routes and typed handler links.

**Verification:**

- Cover group prefixes, middleware, closures, Rust macros, ASP.NET templates,
  Vapor segments, aliases, receiver ambiguity, and unrelated methods named
  `get`, `route`, or `HandleFunc`.
- Run:

```bash
cargo test -p compass-resolve --test native_routes --locked
```

**Done when:** Every framework in the approved routing matrix has positive,
near-match, ambiguous, unresolved, and incremental fixtures.

**Commit:** `feat(frameworks): resolve native server routes`

### Task 12: Emit events, messages, jobs, schedules, and ORM mappings

**Objective:** Complete the first-release enterprise/domain vocabulary beyond
routes and SQL.

**Context:** Some framework constructs recognized in Tasks 8–11 describe
message dispatch, events, scheduled work, or model mappings rather than HTTP
routes. They must reuse the same provenance and resolution discipline but
publish domain-specific nodes and edges. ORM mappings may link to Task 6
database entities only when exact metadata or query evidence exists.

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

**Implementation:**

- [x] Convert recognized NestJS `@MessagePattern`, `@EventPattern`, and
  `@SubscribeMessage` plus Spring message/event annotations into event,
  message, topic, queue, and WebSocket subscription facts with `publishes`,
  `subscribes`, `produces`, `consumes`, `handles`, and `registers` edges as
  appropriate.
- [x] Detect Spring scheduled jobs, ASP.NET hosted/background services, and
  Celery tasks/schedules. Publish job nodes and `schedules`, `triggers`, or
  `handles` edges using exact declaration sites.
- [x] Extract ORM model/table/column metadata for Django, SQLAlchemy, TypeORM,
  JPA, Entity Framework, Active Record, Eloquent, GORM, and Diesel.
- [x] Emit `maps_to` only for explicit table/schema/column mapping or a
  framework convention with unambiguous model identity. Emit `reads` and
  `writes` only for exact query evidence.
- [x] Join ORM targets to Task 6 database entities by normalized qualified
  identity. If the database target is absent, retain the mapping target as an
  unresolved domain reference and diagnostic rather than creating an
  unsupported exact entity.
- [x] Treat runtime registries, computed topic names, and dynamic schedules as
  heuristic only when a wiring site is available; otherwise leave them
  unresolved.

**Contract produced:** All enterprise kinds and their domain relationships
have at least one anchored producer and participate in the same impact and
explore graph as code symbols.

**Verification:**

- Verify exact and dynamic message names, publisher/consumer direction,
  schedule expressions, hosted services, conventional and explicit ORM
  mappings, schema qualification, missing database targets, and false
  near-match suppression.
- Run:

```bash
cargo test -p compass-resolve --test domain_resolution --locked
cargo test -p compass-graph --test domain_normalization --locked
```

**Done when:** The producer coverage report has no zero-producer enterprise
kind or edge.

**Commit:** `feat(domain): resolve enterprise graph facts`

---

## Milestone 3: Build one shared query engine

### Task 13: Define `compass.query/1` and optional Program IR enrichment

**Objective:** Establish the transport-neutral response used by every query
operation and keep structural and Program evidence visibly separate.

**Context:** Compass already has in-memory query, traversal, affected, and
Program analysis paths, but they return unrelated shapes. The structural
graph is authoritative for symbol identity and relationships. Program IR may
add types, effects, signatures, and deeper call evidence, but it must never
overwrite or silently contradict structural facts.

**Files:**

- Create: `crates/compass-model/src/query_contract.rs`
- Modify: `crates/compass-model/src/lib.rs`
- Create: `crates/compass-query/src/program_join.rs`
- Modify: `crates/compass-query/src/lib.rs`
- Modify: `crates/compass-query/Cargo.toml`
- Create: `fixtures/contracts/compass-query-v1.example.json`
- Create: `fixtures/contracts/compass-query-v1.fingerprint`
- Test: `crates/compass-query/tests/query_contract.rs`
- Test: `crates/compass-query/tests/program_join.rs`

**Implementation:**

- [x] Define bounded request types for search, callers/callees, impact,
  explore, and node trail. Require explicit limits for depth, nodes, paths,
  candidates, source bytes, and total response bytes. The public names are
  `SearchRequest`, `CallRequest`, `ImpactRequest`, `ExploreRequest`, and
  `NodeTrailRequest`.
- [x] Define one additive response:

```rust
pub struct CodeQueryResponse {
    pub schema: String, // always "compass.query/1"
    pub operation: CodeQueryOperation,
    pub results: Vec<SearchHit>,
    pub nodes: Vec<QueryNode>,
    pub edges: Vec<QueryEdge>,
    pub files: Vec<QueryFile>,
    pub paths: Vec<QueryPath>,
    pub diagnostics: Vec<QueryDiagnostic>,
    pub limits: CodeQueryLimits,
    pub truncated: bool,
}
```

- [x] Define the response members `SearchHit`, `QueryNode`, `QueryEdge`,
  `QueryFile`, `QueryPath`, `QueryDiagnostic`, and `CodeQueryLimits` in the
  same module. Do not reuse the existing CompassQL `QueryLimits` type.
- [x] Put evidence records on query nodes and edges with a `layer` of
  `structural_graph` or `program_ir`. Preserve origin, confidence, anchor,
  rule, wiring site, and resolution state.
- [x] Add `compass-analysis` and `compass-ir` dependencies to
  `compass-query`. Join Program evidence only through stable
  `graph_node_id`.
- [x] When Program IR has an orphan symbol or contradicts a structural fact,
  retain the structural record and add a typed diagnostic. Do not create a
  new durable node or change `graph.json`.
- [x] Return no match, ambiguous match, unresolved handler, incomplete
  coverage, stale source, and bounded truncation as successful typed responses
  with diagnostics. Return corrupt authoritative artifacts, schema mismatch,
  unsafe paths, and violated graph invariants as `QueryError`.
- [x] Sort every response collection by a documented stable key. Search is
  the only operation that populates `results`; other operations return an
  empty array rather than omitting the field.
- [x] Generate a complete example response and a deterministic fingerprint of
  the Rust field/enum contract under `fixtures/contracts/`. Tasks 16–18 use
  these files for transport and TypeScript parity.

**Contract produced:** All transports can serialize the same
`CodeQueryResponse`; Program enrichment is optional, non-destructive, and
auditable.

**Verification:**

- Cover graph-only queries, graph-plus-Program evidence, orphan Program
  symbols, conflicts, deterministic ordering, truncation, and missing Program
  artifacts.
- Run:

```bash
cargo test -p compass-query --test query_contract --locked
cargo test -p compass-query --test program_join --locked
```

**Done when:** Removing `program.json` reduces enrichment but does not change
structural query identity or relationship results.

**Commit:** `feat(query): define shared code query contract`

### Task 14: Build the disposable SQLite/FTS5 projection and symbol search

**Objective:** Provide fast name search without making SQLite an
authoritative artifact.

**Context:** Current symbol scoring reads the graph into memory and uses
custom token scoring. The product requirement calls for FTS5. The index must
be disposable, content-addressed, bounded, and safe to rebuild after
corruption. `graph.json` and `program.json` remain the sources of truth.

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/compass-query/Cargo.toml`
- Create: `crates/compass-query/src/index.rs`
- Create: `crates/compass-query/src/code_query.rs`
- Modify: `crates/compass-query/src/lib.rs`
- Test: `crates/compass-query/tests/code_search.rs`
- Test: `crates/compass-query/tests/index_recovery.rs`

**Implementation:**

- [ ] Add workspace `rusqlite = { version = "0.31.0", features =
  ["bundled", "modern_sqlite"] }` and use its bundled FTS5-capable SQLite.
- [ ] Key the index directory by SHA-256 of graph digest, optional Program
  digest, graph schema fingerprint, query schema fingerprint, and index
  format version.
- [ ] Create normalized relational tables for nodes, edges, files, evidence,
  aliases, and Program joins plus an FTS5 table over name, qualified name,
  aliases, kind, roles, language, framework, and normalized path.
- [ ] Build into a temporary database, use transactions and prepared
  statements, run integrity checks, fsync, and atomically rename into place.
  A lock coordinates concurrent builders; readers may continue using the
  last complete matching index.
- [ ] Implement:

```rust
pub fn open(graph_path: &Path, program_path: Option<&Path>,
            cache_root: &Path) -> Result<CodeQueryEngine, QueryError>;
pub fn search(&self, request: SearchRequest)
    -> Result<CodeQueryResponse, QueryError>;
```

  Ranking order is exact qualified name, exact name, prefix, FTS rank, then
  stable node ID.
- [ ] Escape FTS syntax through bound parameters and a conservative query
  builder. Enforce term, token, result, and response-size limits before
  executing SQLite work.
- [ ] Delete and rebuild on schema mismatch, digest mismatch, incomplete
  build marker, failed integrity check, or SQLite corruption.

**Contract produced:** `search` returns ranked `SearchHit` records and the
corresponding typed nodes through `compass.query/1`.

**Verification:**

- Cover exact/prefix/fuzzy ranking, aliases, Unicode, punctuation, reserved
  FTS characters, deterministic ties, concurrent open, stale index deletion,
  corruption recovery, and large-input limits.
- Run:

```bash
cargo test -p compass-query --test code_search --locked
cargo test -p compass-query --test index_recovery --locked
```

**Done when:** Deleting the index changes only latency; the next query
rebuilds it and returns the same response.

**Commit:** `feat(query): index graph symbols with FTS5`

### Task 15: Implement callers, callees, impact, explore, and node trail

**Objective:** Put all required graph operations on the shared engine with
consistent evidence, paths, and bounds.

**Context:** Current traversal and affected logic operate on permissive
relation strings and render text directly. The v1 engine must traverse typed
edge families, expose route bindings as callers, distinguish exact from
heuristic impact, and return digest-verified source grouped by file.

**Files:**

- Modify: `crates/compass-query/src/code_query.rs`
- Modify: `crates/compass-query/src/affected.rs`
- Modify: `crates/compass-query/src/traversal.rs`
- Create: `crates/compass-query/src/source.rs`
- Modify: `crates/compass-query/src/lib.rs`
- Test: `crates/compass-query/tests/code_traversal.rs`
- Test: `crates/compass-query/tests/code_impact.rs`
- Test: `crates/compass-query/tests/code_explore.rs`
- Test: `crates/compass-query/tests/node_trail.rs`

**Implementation:**

- [ ] Implement one-hop callers over inbound `calls` and `routes_to`; implement
  one-hop callees over outbound `calls`. Preserve parallel edges and return
  all evidence records.
- [ ] Implement impact as bounded reverse traversal over the approved impact
  family: `calls`, `routes_to`, `imports`, `exports`, `references`,
  `depends_on`, `reads`, `writes`, `publishes`, `subscribes`, `produces`,
  `consumes`, `schedules`, `triggers`, and `maps_to`.
- [ ] Default impact to exact/config/convention evidence. Add an
  explicit request option for heuristic edges and label every affected path
  with its weakest resolution/evidence state.
- [ ] Implement explore by resolving several requested symbols, selecting a
  minimal bounded connecting subgraph, and grouping source slices by
  normalized file path. Include the call/route path among selected symbols.
- [ ] Before returning source, recompute the file digest and compare it to the
  graph’s `FileRecord`. On mismatch, omit source and emit
  `stale_source_digest`; never return unverified current text as indexed
  source.
- [ ] Implement node trail as a bounded best-path search ordered by hop count,
  evidence quality, and stable edge identity. Include route middleware stages
  and heuristic wiring sites inline.
- [ ] Expose the operations with these engine signatures:

```rust
pub fn callers(&self, request: CallRequest)
    -> Result<CodeQueryResponse, QueryError>;
pub fn callees(&self, request: CallRequest)
    -> Result<CodeQueryResponse, QueryError>;
pub fn impact(&self, request: ImpactRequest)
    -> Result<CodeQueryResponse, QueryError>;
pub fn explore(&self, request: ExploreRequest)
    -> Result<CodeQueryResponse, QueryError>;
pub fn node_trail(&self, request: NodeTrailRequest)
    -> Result<CodeQueryResponse, QueryError>;
```

- [ ] Apply `CodeQueryLimits` during candidate resolution, traversal, source
  reading, and serialization. Set `truncated` and diagnostics whenever a
  bound changes the result.

**Contract produced:** Every required query operation returns the same typed
nodes, edges, files, paths, diagnostics, limits, and evidence.

**Verification:**

- Cover route-as-caller behavior, edge direction, parallel calls,
  exact-only/heuristic impact, cycles, path tie-breaking, ambiguous symbols,
  stale source, grouped source, Program enrichment, middleware stages, and
  every truncation bound.
- Run:

```bash
cargo test -p compass-query --test code_traversal --locked
cargo test -p compass-query --test code_impact --locked
cargo test -p compass-query --test code_explore --locked
cargo test -p compass-query --test node_trail --locked
```

**Done when:** Search, callers, callees, impact, explore, and node trail all
serialize `compass.query/1` without transport-specific logic.

**Commit:** `feat(query): add trusted code graph operations`

---

## Milestone 4: Expose the shared contract to agents and developers

### Task 16: Add thin CLI and MCP adapters

**Objective:** Expose query-engine operations without duplicating graph
semantics in command or MCP layers.

**Context:** The CLI currently has separate natural query, CQL, affected, and
call-graph commands. MCP currently implements `query_graph`, `get_node`, and
`get_neighbors` directly in `crates/compass-mcp/src/lib.rs`. Those paths must
remain available where appropriate, but v1 code-graph operations should
delegate to `CodeQueryEngine` and return the shared response unchanged.

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

**Implementation:**

- [ ] Add CLI subcommands `search`, `callers`, `callees`, `impact`, `explore`,
  and `node`, each accepting `--format json|text` plus operation-specific
  limits and evidence options.
- [ ] Serialize `CodeQueryResponse` directly for JSON. Implement text as a
  renderer over the response; never build a second semantic result shape.
- [ ] Route existing overlapping call-graph/affected functionality through
  the new engine where v1 semantics apply, while retaining explicitly
  separate CQL and natural-language commands.
- [ ] Add MCP tools `search_symbols`, `get_callers`, `get_callees`,
  `get_impact`, `explore_code`, and `get_node`. Replace the existing
  `get_node` implementation rather than registering a duplicate name.
- [ ] Use bounded JSON schemas with the same defaults as Rust request types.
  Return structured JSON plus concise text content. Reserve MCP protocol
  errors for invalid requests, corrupt schemas, unsafe paths, or engine
  failures; valid empty results remain successful responses.
- [ ] Ensure pre-contract graphs return the same rebuild diagnostic through
  CLI and MCP.

**Contract produced:** Agents and scripts receive byte-equivalent JSON
semantics for the same request regardless of CLI or MCP transport.

**Verification:**

- Add golden parity cases for every operation, limit, empty result,
  truncation, heuristic option, stale source, and unsupported graph schema.
- Run:

```bash
cargo test -p compass-cli --test code_query_cli --locked
cargo test -p compass-mcp --test code_query_tools --locked
```

**Done when:** Equivalent CLI and MCP requests deserialize to the same Rust
request and return the same `compass.query/1` payload.

**Commit:** `feat: expose shared code graph queries`

### Task 17: Add the shared TypeScript contract and evidence presentation

**Objective:** Give browser-based Compass surfaces a strict mirror of
`compass.query/1` and a reusable way to present trust evidence.

**Context:** The viewer already validates graph and call-graph projections
with Zod, but it has no decoder for the new query envelope. Evidence is also
spread across existing graph components. The viewer package should own the
TypeScript contract and presentation primitives so the standalone viewer and
VS Code webview cannot drift.

**Files:**

- Create: `packages/compass-viewer/src/contracts/codeQuery.ts`
- Create: `packages/compass-viewer/src/contracts/codeQuery.test.ts`
- Modify: `packages/compass-viewer/src/contracts/graph.ts`
- Create: `packages/compass-viewer/src/graph/CodeEvidence.tsx`
- Create: `packages/compass-viewer/src/graph/CodeEvidence.test.tsx`
- Modify: `packages/compass-viewer/src/graph/GraphInspector.tsx`
- Modify: `packages/compass-viewer/src/graph/edgeLabels.ts`
- Modify: `packages/compass-viewer/src/graph/CompassGraph.tsx`
- Modify: `packages/compass-viewer/src/index.ts`

**Implementation:**

- [ ] Mirror every `compass.query/1` enum and record in strict Zod schemas.
  Reject unknown variants and unsafe source anchors. Export inferred
  TypeScript types from the schema module.
- [ ] Decode `fixtures/contracts/compass-query-v1.example.json` and compare
  `fixtures/contracts/compass-query-v1.fingerprint` with the fingerprint
  generated from the TypeScript enum/field manifest so Rust/TypeScript drift
  fails CI.
- [ ] Render exact, config, convention, ambiguous, unresolved, and
  heuristic states with text and icons. For heuristic edges, show rule,
  wiring file/line, extractor, confidence, and candidates inline.
- [ ] Add `routes_to` and enterprise edge labels without collapsing them into
  generic references. Show middleware stage and position on route edges.
- [ ] Integrate `CodeEvidence` into the graph inspector and query-result
  projection. Use text and icons in addition to color, expose diagnostics and
  truncation, and keep source actions callback-based so the package remains
  host-neutral.
- [ ] Export the decoder, types, and evidence component from the viewer
  package entry point.

**Contract produced:** Browser clients share one strict decoder and evidence
component whose vocabulary is fingerprinted against Rust.

**Verification:**

- Cover Zod parity, unknown variants, heuristic evidence, route stage labels,
  ambiguous candidates, diagnostics, truncation, and accessible text/icon
  states.
- Run:

```bash
npm run test -w @compass/viewer
npm run typecheck -w @compass/viewer
npm run build -w @compass/viewer
```

**Done when:** The viewer rejects any Rust/TypeScript contract drift and
displays structural, Program, convention, ambiguity, and heuristic evidence
without host-specific behavior.

**Commit:** `feat(viewer): present code graph evidence`

### Task 18: Integrate code-graph queries into VS Code

**Objective:** Make shared code-graph operations navigable inside the
enterprise extension without adding a second graph implementation.

**Context:** The extension already invokes Compass through its managed process
layer and renders `@compass/viewer` in webviews. It must stay a client: it
should send typed CLI requests, validate responses with Task 17’s decoder,
and delegate all search/traversal semantics to Compass. Source navigation and
rebuild actions remain host responsibilities.

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

**Implementation:**

- [ ] Implement `codeQueryClient` through the existing Compass process
  manager. Pass literal argument arrays, require JSON output, validate it with
  `CodeQueryResponseSchema`, and support cancellation and process cleanup.
- [ ] Register extension actions for symbol search, callers, callees, impact,
  explore selection, and node trail. Add context-menu actions on graph nodes
  and editor symbols.
- [ ] Send validated query responses to the graph webview through strict host
  and webview message unions. The webview consumes Task 17’s components and
  does not read `graph.json`.
- [ ] Open returned source anchors through VS Code APIs only after confirming
  the normalized path remains inside the active repository. For stale source,
  show the diagnostic and offer rebuild rather than opening mismatched text.
- [ ] On an unsupported graph schema, present “Rebuild with Compass” and invoke
  the normal index/update workflow after user action.
- [ ] Keep `routes_to` evidence visible when callers of a handler are shown,
  including middleware order and heuristic wiring-site navigation.
- [ ] Before editing `editors/vscode/package.json`, inspect and preserve the
  existing user-owned modification; stage only intentional command/menu
  additions.

**Contract produced:** VS Code is a validated transport and navigation host
for the same `compass.query/1` results exposed through CLI and MCP.

**Verification:**

- Cover literal CLI arguments, schema rejection, cancellation, message
  validation, source navigation, repository escape rejection, stale source,
  rebuild guidance, route callers, and all command registrations.
- Run:

```bash
npm run test -w crabbuild-compass-vscode
npm run typecheck -w crabbuild-compass-vscode
npm run build:vscode
npm run test:integration -w editors/vscode
```

**Done when:** A developer can select a handler, see the URL route that binds
it, inspect exact or heuristic provenance, follow the node trail, and open
verified source without the extension computing graph results.

**Commit:** `feat(vscode): explore trusted code graph evidence`

---

## Milestone 5: Qualify the release

### Task 19: Add deterministic, cross-platform, and real-repository gates

**Objective:** Turn the v1 trust claims into one repeatable release
qualification command.

**Context:** Fixture tests prove syntax shapes but not real-repository
behavior, deterministic incremental publication, client parity, or platform
path semantics. Compass is the enterprise product, so v1 must not ship with
declared kinds that lack producers or framework resolvers that only work in
toy fixtures.

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

**Implementation:**

- [ ] Implement the coverage checker to fail when a declared node or edge kind
  has zero producers; a framework lacks positive, near-match, exact,
  ambiguous, unresolved, or incremental coverage; a heuristic edge lacks
  wiring evidence; clean and incremental bytes differ; or Rust/CLI/MCP/VS
  Code schema fingerprints differ.
- [ ] Implement `qualify_code_graph_v1.sh --fixtures-only` to run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
npm run test:js
npm run typecheck:js
npm run build
python3 scripts/check_code_graph_v1_coverage.py
```

- [ ] Define the real-repository lock format with `name`, `url`, immutable
  40-hex `commit`, `size_class`, `language_family`, `frameworks`, and at least
  three named route-to-handler flows for every framework. Reject branches,
  tags, shortened hashes, duplicate size/language cells, or insufficient
  framework flows.
- [ ] Make full qualification clone locked repositories into temporary
  directories, build clean and incremental graphs, execute the declared
  queries, and record Compass revision, repository revision, graph digest,
  counts, coverage, unresolved/ambiguous totals, false exact resolutions,
  index time, and query latency.
- [ ] Commit the generated evidence in
  `docs/design/code-graph-v1-qualification.md`. Qualification fails if any
  declared flow does not resolve as expected or if a bounded ambiguous result
  is reported as exact.
- [ ] Add Linux fixture qualification to
  `.github/workflows/compass-ci.yml`. Run platform-sensitive suites on macOS,
  Linux, and Windows for path normalization, atomic publication, SQLite,
  process cancellation, source navigation, and VS Code packaging.
- [ ] Document the hard cutover and rebuild requirement under the Unreleased
  section of `CHANGELOG.md`; do not describe pre-contract graphs as a
  supported old v1.
- [ ] Add `make qualify-code-graph-v1` as the release entry point.

**Contract produced:** Release engineering has a single command and an
immutable evidence report for every vocabulary producer, framework, client,
and platform claim.

**Verification:**

- Run fixture qualification:

```bash
./scripts/qualify_code_graph_v1.sh --fixtures-only
```

- Run locked real-repository qualification:

```bash
./scripts/qualify_code_graph_v1.sh \
  --repositories tests/qualification/code-graph-v1-repositories.toml
```

- Run the outer graph refresh after the final code change:

```bash
cd /Users/haipingfu/graphify
graphify update .
```

**Done when:** Both qualification modes exit 0, the evidence report contains
no unresolved release gate, every declared kind has a producer, every
framework has three real flows, and the cross-platform CI matrix is green.

**Commit:** `test: qualify Compass code graph v1`

---

## Final release review

- [ ] Confirm the durable artifact is a NetworkX-compatible
  `compass.graph/1` envelope and contains no `relation` aliases or flattened
  unknown attributes.
- [ ] Confirm pre-contract artifacts rebuild on update and fail with rebuild
  guidance on read-only paths; no adapter or dual representation exists.
- [ ] Confirm Program IR remains `http://crab.build/compass/v1` and is joined
  only at query time.
- [ ] Confirm all core, enterprise, database, event, job, schema, and resource
  kinds have validated producers.
- [ ] Confirm all approved frameworks publish `route` nodes and canonical
  `routes_to` edges with exact, convention, config, ambiguous, or
  surfaced heuristic evidence.
- [ ] Confirm search uses FTS5 and callers, callees, impact, explore, and node
  trail share `compass.query/1`.
- [ ] Confirm CLI, MCP, viewer, and VS Code schema fingerprints match.
- [ ] Confirm heuristic rule and wiring-site evidence appears inline in
  explore, node trail, the viewer, and VS Code.
- [ ] Confirm clean and incremental graph bytes match across checkout roots.
- [ ] Confirm real-repository qualification and macOS/Linux/Windows gates pass.
- [ ] Inspect `git status --short` and verify unrelated user changes remain
  unmodified and unstaged.
