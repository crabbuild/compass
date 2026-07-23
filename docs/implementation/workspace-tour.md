# Compass workspace tour

This tour maps each crate to its responsibility, main public interfaces, and
verification evidence. Use it to decide where a change belongs before opening
large source files.

> **Who this page is for:** new and returning Compass contributors.
>
> **You will learn:** crate ownership, dependency direction, key modules, and
> the tests that protect each boundary.
>
> **Prerequisites:** [System architecture](../design/architecture.md).
>
> **Reading time:** 15–20 minutes.

## Start at the workspace manifest

`Cargo.toml` defines one Edition 2024 workspace, Rust 1.97, shared dependency
versions, and strict lints:

```text
unsafe_code       forbidden
clippy::all       denied
unwrap_used       denied
expect_used       denied
panic             denied
```

The released binary is:

```text
package: compass-cli
binary:  compass
entry:   crates/compass-cli/src/bin/compass.rs
```

## Core pipeline crates

### `compass-files`

**Purpose:** deterministic filesystem discovery and safe artifacts.

**Key modules:**

```text
atomic       atomic byte/text/JSON writes
build_guard  incomplete-build protection
cache        extraction cache formats
detect       file classification and ignore policy
encoding     source decoding
hash         content/stat/prompt fingerprints
manifest     incremental build state
slice        bounded source slicing
```

**Change here when:** adding detection policy, a cache/manifest contract, or an
atomic filesystem primitive.

**Evidence:** crate tests plus CLI update/extract/watch tests that exercise
incremental behavior and output safety.

### `compass-languages`

**Purpose:** statically linked structural extraction.

**Key concepts:**

- `Registry` and `LanguageSpec`;
- `ExtractorKind`;
- `Extraction` containing nodes, edges, hyperedges, and raw calls;
- stable ID helpers;
- language-specific modules;
- SCIP and project-manifest ingestion.

**Change here when:** adding syntax support, local facts, node/edge attributes,
or a new language registry entry.

**Do not:** resolve arbitrary cross-file calls here when the resolver needs
project-wide facts.

### `compass-resolve`

**Purpose:** deterministic project-wide resolution.

**Public boundary:** merge per-file `Extraction` values and resolve cross-file
imports, calls, members, re-exports, IDs, and stubs.

**Change here when:** the extractor already emitted enough evidence but the
final target requires multiple files/scopes.

**Evidence:** language/member-resolution tests and Python-oracle/parity
fixtures.

### `compass-graph`

**Purpose:** build and analyze the graph.

**Public families:**

- build/deduplication;
- cluster/community scoring and stable remapping;
- god-node and surprising-connection analysis;
- suggested questions;
- import cycles;
- graph diff helpers.

**Change here when:** behavior depends on graph topology rather than source
syntax.

### `compass-core`

**Purpose:** application services.

**Modules:**

```text
pipeline          current graph builds
history           complete historical materialization adapter
cluster_existing  reanalyze a saved graph
diagnostics       graph diagnostics
merge             graph merge service
watch             filesystem watch orchestration
raw_guard         raw-input safety boundary
```

**Primary types:** `BuildOptions`, `BuildPurpose`, `BuildResult`,
`BuildTimings`, `SemanticLayer`, `MaterializeRequest`.

**Change here when:** multiple domain crates must be sequenced into one
transactional workflow.

## Graph model and query crates

### `compass-model`

**Purpose:** typed node-link graph and indexes.

**Core types:**

```text
NodeRecord
EdgeRecord
GraphDocument
Graph
QueryIndex
SchemaFingerprint
GraphError
```

The model retains unknown attributes and preserves directed/multigraph
semantics. `Graph` builds ID, incoming, outgoing, and query indexes.

**Change here when:** altering the graph document contract or core indexing.
Such changes require broad compatibility and history review.

### `compass-cypher`

**Purpose:** CompassQL compiler and logical planner.

**Modules:** lexer, tokens, spans, parser, AST, semantic analysis, values,
logical plan, optimizer, diagnostics, support matrix.

**Versions:** `LANGUAGE_VERSION` and `PLANNER_VERSION` are cache/compatibility
inputs.

**Change here when:** adding documented query syntax, type rules, or a logical
operator.

### `compass-query`

**Purpose:** graph query execution.

**Modules:**

```text
text       normalization and tokens
score      node scoring/selection
traversal  focused query, path, explain
affected   incoming impact traversal
benchmark  query benchmark support
cql        CompassQL execution/cache/profile
```

**Change here when:** implementing execution behavior over the graph. Syntax
acceptance belongs in `compass-cypher`.

## Interface and output crates

### `compass-cli`

**Purpose:** public command surface.

Command-family modules separate history, query, hooks, install, providers,
ingestion, PRs, semantic helpers, results, labels, and integrations.

The crate maps domain results to:

- stdout/stderr;
- human and JSON formats;
- usage help;
- exact exit codes;
- filesystem side effects.

**Change here when:** adding or modifying a public command. Keep reusable
domain logic in the owning lower crate.

**Evidence:** many subprocess-style integration tests under
`crates/compass-cli/tests/`.

### `compass-output`

**Purpose:** graph renderers and export documents.

Formats include Markdown report, HTML, JSON, SVG, GraphML, Cypher, tree,
call-flow, Obsidian, wiki, and canvas outputs.

**Change here when:** the graph meaning is already complete and only its
representation changes.

### `compass-mcp`

**Purpose:** MCP server over graph and PR services.

Owns MCP schema, resources/tools, stdio/HTTP transport, authentication, and
request limits.

**Change here when:** adding a service tool or transport behavior. Reuse
`compass-query` for graph logic.

## History crate

### `compass-history`

**Purpose:** immutable versioned graph storage.

**Module map:**

| Module | Responsibility |
| --- | --- |
| `model` | commits, realizations, stored trees, publication model |
| `fingerprint` | profiles and meaning-affecting identity |
| `canonical` | canonical encoding |
| `keys` | typed node/edge/hyperedge keys |
| `artifacts` | graph partitioning and artifact sets |
| `store` | SQLite/Prolly read and publication |
| `validate` | limits and integrity reports |
| `diff` | typed record streaming |
| `git` | repository and protected worktree |
| `jobs` | durable FIFO requests and state |
| `leases` | claim heartbeat/expiry |
| `lock` | activity/maintenance coordination |
| `gc` | reachability and pruning plans |
| `config` | repository profile and enablement |
| `durable` | durable file operations |

**Change here when:** altering historical identity, durability, publication,
validation, diff, or maintenance behavior.

**Evidence:** dedicated tests for canonical encoding, diffs, Git isolation,
jobs, maintenance, publication, round trips, performance, and SQLite contracts.

## Semantic and media crates

### `compass-semantic`

**Purpose:** validate untrusted semantic fragments and orchestrate providers.

The crate owns hard caps for fragment bytes and record counts, prompt
construction, response normalization, endpoint checks, adaptive retry, partial
tracking, evidence binding, and community labels.

**Change here when:** adding a backend, prompt contract, semantic validator, or
provider safety rule.

### `compass-media`

**Purpose:** bounded text extraction from local documents.

It handles PDF and Office formats with raw, expanded, member, and compression
ratio limits.

### `compass-transcribe`

**Purpose:** bounded transcription orchestration and download/backend traits.

### `compass-whisper`

**Purpose:** Compass-owned native Whisper inference internals, currently
portable CPU behavior.

## Integration crates

### `compass-cargo`

Parses Cargo workspaces and dependencies deterministically into graph
fragments.

### `compass-global`

Maintains a persistent cross-project graph registry and enforces graph/manifest
size limits.

### `compass-google-workspace`

Exports `.gdoc`, `.gsheet`, and `.gslides` shortcuts through bounded `gws`
subprocess calls.

### `compass-graphdb`

Implements native Neo4j Bolt and FalkorDB RESP clients and graph-to-operation
mapping.

### `compass-ingest`

Fetches bounded public URL content with SSRF defenses and writes corpus files
atomically.

### `compass-postgres`

Performs read-only PostgreSQL catalog introspection for tables, views,
routines, and constraints.

### `compass-prs`

Uses bounded Git/GitHub subprocesses to build a PR dashboard and connect changed
files with graph impact.

### `compass-reflect`

Aggregates session memory deterministically and can write a learning overlay
sidecar.

## Verification-only crate

### `compass-parity`

Development-only differential tests compare selected native behavior with the
pinned Graphify baseline. It is broad evidence, not a runtime dependency.

## Vendored parser package

`vendor/compass-tree-sitter-language-pack` is a pinned Compass-specific parser
package with registry, queries, language definitions, download/build policy,
and extraction helpers.

Changes can affect many languages and release artifacts. Run its registry and
multilingual qualification, not one language test alone.

## Where should my change go?

```text
new file extension / ignore behavior?       compass-files
new syntax entity or local edge?            compass-languages
cross-file target selection?                compass-resolve
topology algorithm / communities?           compass-graph
graph JSON/index behavior?                   compass-model
query syntax or plan?                        compass-cypher
query execution?                             compass-query
pipeline sequencing?                        compass-core
command/flags/exits?                         compass-cli
render/export format?                        compass-output
semantic backend/validation?                 compass-semantic
immutable revision storage?                  compass-history
external protocol?                           corresponding integration crate
```

## Related pages

- [System architecture](../design/architecture.md)
- [Extraction pipeline](extraction-pipeline.md)
- [Query engine](query-engine.md)
- [Extending Compass](extending-compass.md)

**Next step:** locate one feature through this map, then read its crate
`src/lib.rs`, nearest integration test, and CLI call site in that order.
