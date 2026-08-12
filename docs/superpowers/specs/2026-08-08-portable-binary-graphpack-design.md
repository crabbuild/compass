# Portable Binary GraphPack

**Date:** 2026-08-08

**Status:** Proposed

**Audience:** Compass maintainers implementing graph publication, storage,
query, export, history, and compatibility changes

## Summary

Compass currently publishes a canonical, typed `graph.json` artifact and can
also build an indexed immutable graph snapshot in SQLite. JSON is portable and
stable, but its repeated field names and strings, graph-sized parsing, hashing,
allocation, and index reconstruction consume a material fraction of build and
query startup time.

This design introduces `graph.cgb`, a deterministic, portable, bounded,
indexed binary realization called GraphPack. GraphPack preserves the complete
logical `compass.graph/1` document: every node, edge, source anchor, evidence
record, ambiguity candidate, diagnostic, coverage record, ordering rule,
direction, and parallel relationship remains available. A deterministic JSON
export adapter reproduces the existing canonical `graph.json` bytes.

GraphPack is independently versioned as `compass.graphpack/1`. Binary versus
JSON is a physical representation choice, not a reason to change logical graph
meaning. The current release line continues to publish `graph.json`; a
binary-first default is a later compatibility change with its own migration and
release decision.

The execution plan is phased. Every phase below contains enough context,
dependencies, scope, verification, and acceptance criteria to be assigned as a
self-contained implementation task.

## Background

### Current graph publication

The structural pipeline produces a normalized typed
`compass_model::code_graph::GraphDocument`. The publication path:

1. validates and canonically orders the logical graph;
2. streams canonical JSON into `graph.json`;
3. optionally builds immutable node, edge, adjacency, file, name, term,
   community, and diagnostic indexes in `store.sqlite3`;
4. seals the selected artifacts;
5. commits one immutable snapshot; and
6. atomically projects conventional root artifacts.

The JSON writer already avoids one giant output buffer and serializes bounded
record chunks in parallel. The SQLite snapshot builder already exposes bounded
point, prefix, and adjacency reads. GraphPack must reuse those semantics rather
than create a third graph model or a conflicting query vocabulary.

### Current compatibility contract

`graph.json` with logical schema `compass.graph/1` is the permanent
compatible engine in the current release line. It supports publication,
interchange, inspection, recovery, deterministic export, and direct input.
SQLite is an optional, rebuildable implementation detail selected by a typed
`store.ref`.

Therefore:

- adding `graph.cgb` beside `graph.json` is compatible;
- preferring a validated, digest-bound `graph.cgb` inside Compass is
  compatible when JSON remains available;
- omitting `graph.json` by explicit opt-in requires a documented new mode;
- making binary-only publication the default is an incompatible product
  change.

### Measured motivation

Representative release-mode TypeScript/JavaScript qualification on the same
machine produced:

| Corpus | Nodes | Edges | Canonical JSON | Graph publication | Total |
| --- | ---: | ---: | ---: | ---: | ---: |
| Axios | 19,170 | 39,583 | 51 MiB | about 0.80 s | about 3.2 s |
| date-fns | 57,598 | 108,379 | 144 MiB | about 2.78 s | about 8.0 s |

These observations identify graph representation as meaningful but not
sufficient for a two-times cold-build improvement. Even removing the measured
publication duration entirely yields theoretical cold-build ceilings of about
1.33 times for Axios and 1.53 times for date-fns. Extraction and resolution
remain independent optimization areas.

GraphPack can have larger leverage on graph reopen and query startup because it
can avoid:

- JSON tokenization;
- repeated UTF-8 string allocation;
- full `GraphDocument` materialization;
- graph-sized ID maps;
- adjacency reconstruction;
- disposable index construction; and
- a second artifact hash pass.

## Goals

1. Preserve the complete logical `compass.graph/1` graph without quality loss.
2. Publish a smaller and faster portable binary realization.
3. Support bounded point, range, search, incoming, and outgoing reads without
   materializing the complete graph.
4. Export canonical JSON deterministically and byte-identically.
5. Share query ordering, tokenization, truncation, and validation semantics
   with the existing JSON and store adapters.
6. Preserve atomic snapshot publication and fail-closed corruption handling.
7. Keep physical format versions independent from logical graph schema and
   publication semantics versions.
8. Make format and query work measurable through release-mode qualification.
9. Provide a safe migration path from dual publication to an optional
   binary-authority mode.

## Non-goals

- Improve node or edge recall, precision, or provenance by changing the
  representation.
- Remove, aggregate, sample, or summarize evidence to reduce size.
- Change stable node or edge identities.
- Change relationship direction, multiplicity, occurrence identity, source
  anchors, ambiguity, or deterministic ordering.
- Replace Program IR, semantic sidecars, history realizations, or report
  formats.
- Make SQLite's private physical schema public.
- Add runtime schema downloads, model calls, Graphify, Python, or network
  dependencies.
- Claim a two-times end-to-end build improvement from serialization alone.
- Use memory mapping that requires unsafe code.
- Silently recover from a present but corrupt authoritative binary artifact.

## Design decisions proposed for approval

### Logical and physical versions are separate

The versions are:

| Meaning | Identifier |
| --- | --- |
| Logical graph | `compass.graph/1` |
| Binary container | `compass.graphpack/1` |
| Logical digest encoding | `compass.graph.logical-digest/1` |
| Canonical JSON | existing `compass.graph/1` canonical writer |
| Publication semantics | existing independently versioned publication value |

A future `compass.graph/2` is reserved for an actual logical contract change.
Changing compression, section layout, indexes, or integer encodings advances
GraphPack's physical version instead.

### GraphPack is a deep graph-publication module

The implementation belongs in `compass-graph`, with responsibilities split by
existing crate ownership:

| Crate | Ownership |
| --- | --- |
| `compass-model` | logical records and validation |
| `compass-graph` | GraphPack format, canonical ordering, indexes, reader, writer |
| `compass-files` | atomic writes, durability, safe paths |
| `compass-query` | backend-neutral query adapter and execution |
| `compass-output` | public JSON/export command presentation |
| `compass-core` | staged build and coherent publication orchestration |
| `compass-cli` | options, help, streams, exits, and migration diagnostics |
| `compass-history` | authoritative realization classification and reconstruction |

The GraphPack module hides string interning, table layout, section compression,
checksums, indexes, and decoding behind one small interface.

### Existing immutable snapshot semantics are reused

GraphPack uses the same logical index families as the current immutable graph
snapshot:

- metadata;
- nodes;
- edges;
- outgoing adjacency;
- incoming adjacency;
- files;
- normalized names;
- terms;
- communities; and
- diagnostics.

The binary format may encode these indexes more compactly, but their ordering,
tokenization, candidate selection, truncation, and corruption behavior remain
backend-neutral.

### Canonical JSON remains a first-class adapter

GraphPack is not useful if JSON export is lossy, unstable, or requires a second
graph build. The reader provides a streaming canonical-record cursor. The JSON
adapter writes the current canonical document directly from that cursor.

For every accepted graph:

```text
GraphDocument -> canonical graph.json
    equals byte-for-byte
GraphDocument -> graph.cgb -> canonical graph.json
```

## Proposed public and internal interfaces

### GraphPack write interface

```rust
pub const GRAPHPACK_SCHEMA_V1: &str = "compass.graphpack/1";

pub struct GraphPackWriteOptions {
    pub compression: GraphPackCompression,
    pub max_block_bytes: usize,
    pub include_query_indexes: bool,
}

pub struct GraphPackReceipt {
    pub artifact_bytes: u64,
    pub artifact_sha256: String,
    pub logical_digest: String,
    pub node_count: u64,
    pub edge_count: u64,
    pub section_count: u32,
}

pub fn write_graphpack_atomic(
    graph: &GraphDocument,
    destination: &Path,
    options: GraphPackWriteOptions,
) -> Result<GraphPackReceipt, GraphPackError>;
```

The writer accepts only a validated typed graph. It canonicalizes ordering
itself so arbitrary library callers cannot create different bytes for
equivalent logical input.

### GraphPack read interface

```rust
pub struct GraphReadLimits {
    pub max_artifact_bytes: u64,
    pub max_items: usize,
    pub max_decoded_bytes: usize,
    pub max_blocks: usize,
    pub max_string_bytes: usize,
}

pub struct GraphPackReader {
    // Private validated file, manifest, and section directory.
}

impl GraphPackReader {
    pub fn open(
        path: &Path,
        limits: GraphReadLimits,
    ) -> Result<Self, GraphPackError>;

    pub fn metadata(&self) -> Result<GraphSnapshotMetadata, GraphPackError>;

    pub fn get_node(&self, id: &str)
        -> Result<Option<NodeRecord>, GraphPackError>;

    pub fn get_edge(&self, id: &str)
        -> Result<Option<EdgeRecord>, GraphPackError>;

    pub fn incoming(
        &self,
        node_id: &str,
        kinds: &[EdgeKind],
        limits: GraphReadLimits,
    ) -> Result<BoundedRecords<EdgeRecord>, GraphPackError>;

    pub fn outgoing(
        &self,
        node_id: &str,
        kinds: &[EdgeKind],
        limits: GraphReadLimits,
    ) -> Result<BoundedRecords<EdgeRecord>, GraphPackError>;

    pub fn nodes_for_terms(
        &self,
        terms: &[String],
        limits: GraphReadLimits,
    ) -> Result<BoundedRecords<NodeRecord>, GraphPackError>;

    pub fn scan_nodes(
        &self,
        limits: GraphReadLimits,
    ) -> Result<GraphRecordCursor<NodeRecord>, GraphPackError>;

    pub fn scan_edges(
        &self,
        limits: GraphReadLimits,
    ) -> Result<GraphRecordCursor<EdgeRecord>, GraphPackError>;

    pub fn materialize(
        &self,
        limits: GraphReadLimits,
    ) -> Result<GraphDocument, GraphPackError>;

    pub fn export_canonical_json(
        &self,
        writer: &mut dyn Write,
    ) -> Result<ArtifactSeal, GraphPackError>;
}
```

Every result carries an explicit truncation indicator when public query
semantics allow truncation. Structural decoding and validation never turn a
limit into an empty result.

### Backend-neutral query interface

The current `GraphEngine::graph() -> &GraphDocument` interface forces JSON and
generic store adapters through complete materialization. Introduce a deeper
query-facing interface:

```rust
pub trait GraphRead: Send + Sync {
    fn kind(&self) -> QueryEngineKind;
    fn logical_identity(&self) -> &str;
    fn metadata(&self) -> Result<GraphSnapshotMetadata, QueryError>;
    fn get_node(&self, id: &str) -> Result<Option<NodeRecord>, QueryError>;
    fn get_edge(&self, id: &str) -> Result<Option<EdgeRecord>, QueryError>;
    fn incoming(
        &self,
        id: &str,
        kinds: &[EdgeKind],
        limits: QueryLimits,
    ) -> Result<BoundedRecords<EdgeRecord>, QueryError>;
    fn outgoing(
        &self,
        id: &str,
        kinds: &[EdgeKind],
        limits: QueryLimits,
    ) -> Result<BoundedRecords<EdgeRecord>, QueryError>;
    fn nodes_for_terms(
        &self,
        terms: &[String],
        limits: QueryLimits,
    ) -> Result<BoundedRecords<NodeRecord>, QueryError>;
    fn materialize(&self, limits: QueryLimits)
        -> Result<GraphDocument, QueryError>;
}
```

JSON, GraphPack, and store-backed adapters satisfy the same interface.
Whole-graph renderers may request bounded materialization; typed search,
callers, callees, impact, explore, and node commands do not.

## Binary container

### File name and media type

- conventional artifact: `graph.cgb`;
- proposed media type: `application/vnd.compass.graphpack`;
- magic: eight ASCII bytes `CGRPAK01`;
- byte order: little-endian;
- whole-file cap: independently bounded and no larger than the supported local
  graph snapshot cap.

The extension is a convenience. Readers trust the magic, manifest, version,
length, section directory, checksums, and logical validation.

### Fixed header

The fixed header contains:

```text
magic                       [u8; 8]
physical_major              u16
physical_minor              u16
feature_flags               u32
declared_file_length        u64
section_directory_offset    u64
section_directory_length    u32
section_count               u32
manifest_sha256             [u8; 32]
reserved                    zero-filled
```

The header contains no unbounded string or collection. Reserved bytes must be
zero in major version 1. A nonzero unknown required flag fails explicitly.

### Manifest

The manifest is a small typed MessagePack record containing:

```text
schema
logical_schema
logical_digest_schema
logical_digest
publication_semantics
builder_version
node_count
edge_count
file_count
evidence_count
diagnostic_count
required_features
optional_features
canonical_json_sha256?
canonical_json_bytes?
```

During dual publication, the optional JSON fields bind `graph.cgb` to the
exact co-published `graph.json`. Binary-only mode omits those fields.

### Section directory

The section directory is sorted by numeric section identifier. Every entry
contains:

```text
section_id                  u16
codec                       u8
flags                       u8
alignment                   u32
offset                      u64
stored_length               u64
uncompressed_length         u64
item_count                  u64
uncompressed_sha256         [u8; 32]
```

Open-time validation rejects:

- duplicate required sections;
- missing required sections;
- unknown required sections;
- overlapping ranges;
- ranges before the header or beyond the declared file length;
- invalid alignment;
- stored or decoded lengths above limits;
- counts above contract limits;
- digest mismatches; and
- a directory that overlaps a payload.

Unknown optional sections are skipped. A read-modify-write operation preserves
their raw bytes and directory metadata when their feature flag declares them
forward-preservable.

### Required sections

| ID | Section | Purpose |
| ---: | --- | --- |
| 1 | manifest | physical and logical contract |
| 2 | strings | shared UTF-8 dictionary |
| 3 | metadata | build metadata, files, graph coverage, graph diagnostics |
| 4 | anchors | repository-relative half-open source ranges |
| 5 | candidates | ambiguous and unresolved resolution candidates |
| 6 | provenance | complete node and edge evidence |
| 7 | coverage | node, file, and graph coverage records |
| 8 | diagnostics | node, edge, file, and graph diagnostics |
| 9 | node-details | tagged category-specific node payloads |
| 10 | edge-details | tagged category-specific edge payloads |
| 11 | nodes | fixed node columns and variable-range references |
| 12 | edges | fixed edge columns and variable-range references |
| 13 | node-id-index | ID dictionary ordinal to node ordinal |
| 14 | edge-id-index | ID dictionary ordinal to edge ordinal |
| 15 | outgoing | outgoing compressed-sparse-row adjacency |
| 16 | incoming | incoming compressed-sparse-row adjacency |

### Optional indexed sections

| ID | Section | Purpose |
| ---: | --- | --- |
| 32 | files | normalized path and digest lookup |
| 33 | names | normalized exact-name lookup |
| 34 | terms | backend-neutral search postings |
| 35 | communities | community membership lookup |
| 36 | diagnostic-codes | graph diagnostic point lookup |
| 63 | extensions | forward-preserved optional logical fields |

A minimal portable GraphPack may omit optional query indexes. A Compass build
intended for normal querying includes them. The manifest records which are
present so query planning never guesses.

## Deterministic record encoding

### String dictionary

All repeated UTF-8 strings are interned:

- node and edge identities;
- names and qualified names;
- source paths;
- languages and frameworks;
- extractor names;
- evidence rules;
- candidate reasons;
- diagnostic codes and messages;
- coverage producers and capabilities;
- detail payload strings; and
- build metadata strings.

The writer:

1. collects strings in bounded worker-local sets;
2. merges the sets;
3. validates aggregate count and bytes;
4. sorts by raw UTF-8 bytes;
5. deduplicates;
6. assigns stable `u32` ordinals; and
7. emits one offsets array and one concatenated byte region.

Empty string has a reserved ordinal. Optional strings use a presence bitmap
rather than a sentinel that can collide with valid data.

### Source anchors

Anchors use:

```text
file_string_id    u32
start_byte        u64
end_byte          u64
start_line        u32
start_column      u32
end_line          u32
end_column        u32
```

Equivalent anchors are deduplicated after canonical sorting. All anchor
validation rules from `compass-model` run during write and read.

### Nodes

The fixed node table stores one row per canonical node:

```text
id_string_id                 u32
kind                         u16
role_bits                    u64
name_string_id               u32
qualified_name_string_id     u32
language_string_id           optional u32
framework_string_id          optional u32
source_anchor_id             optional u32
details_tag                  u16
details_offset               u32
evidence_start/count         u32/u32
coverage_start/count         u32/u32
diagnostic_start/count       u32/u32
community_offset             optional u32
```

Closed enums use explicit wire numbers defined in one table. Rust enum
declaration order is not the wire contract.

### Edges

The fixed edge table stores:

```text
id_string_id                 u32
key_string_id                u32
source_node_ordinal          u32
target_node_ordinal          u32
kind                         u16
occurrence_rule_string_id    optional u32
relationship_anchor_id       optional u32
details_tag                  u16
details_offset               u32
evidence_start/count         u32/u32
weight_bits                  optional u64
context_string_id            optional u32
deferred                     bit
diagnostic_start/count       u32/u32
```

Endpoints use node ordinals, removing repeated opaque IDs while preserving the
original identities in the node table. The writer rejects missing endpoints.

Floating-point values use exact IEEE-754 bits. Non-finite values remain
invalid. Negative zero remains distinguishable where the logical contract
permits it.

### Evidence and candidates

Provenance remains complete:

```text
origin
extractor_string_id
confidence
rule_string_id?
anchor_start/count
wiring_anchor_id?
score_bits?
candidate_start/count
```

Resolution candidates retain:

```text
node_id_string_id
reason_string_id
confidence
score_bits?
anchor_id?
```

No evidence arrays are reordered unless the logical contract already declares
set semantics. Repeated evidence records remain repeated when multiplicity is
observable.

### Details and extensions

Node and edge details use an explicit tag plus a versioned typed payload.
Unknown tags in a required logical major fail. Additive optional fields use the
extensions section:

```text
owner_kind
owner_ordinal
key_string_id
canonical_json_length
canonical_json_bytes
```

This section lets future same-major readers preserve optional values they do
not interpret. Unknown values never participate in stable identity unless the
logical contract explicitly says they do.

## Index layout

### ID lookup

Node and edge ID indexes are sorted by the UTF-8 bytes of the referenced ID,
not by dictionary ordinal. Each entry maps:

```text
string_id -> record_ordinal
```

Readers perform binary search without allocating a `HashMap<String, usize>`.

### Adjacency

Incoming and outgoing indexes use compressed sparse row representation:

```text
offsets[node_count + 1]      u64
edge_ordinals[edge_count]    u32
```

Each node bucket is sorted by:

```text
(edge kind wire number, edge ID UTF-8 bytes, source ordinal, target ordinal)
```

Kind-filtered traversal binary-searches the bucket's kind ranges, then decodes
only selected edges.

### Search indexes

Name and term indexes reuse the exact normalization and tokenizer from the
current query/store implementation. Posting lists are canonical node ordinals,
delta encoded in bounded blocks. Candidate truncation still selects canonical
node IDs before ranking.

Changing tokenization advances the query-index semantics version and invalidates
that optional section. It does not change the core GraphPack physical major.

## Compression and block layout

The entire file is never one compressed stream. Required random-access sections
use independently readable blocks.

Initial defaults:

- maximum uncompressed block: 256 KiB;
- no compression for fixed-width node, edge, ID, and adjacency tables;
- benchmark-selected Zstandard level 1 for strings, provenance, diagnostics,
  details, and term postings;
- raw storage when compressed bytes plus block header are not smaller;
- stored length, decoded length, item count, and digest per block.

Readers validate the declared decoded size before allocating. They read at most
the requested blocks and enforce cumulative decoded-byte and block-count
limits. Compression failure is corruption, not an empty result.

The implementation uses safe positioned reads. Platform-specific standard
library file extensions may provide read-at behavior; a safe seek/read fallback
may use a synchronized file handle. Memory mapping is excluded.

## Logical identity and digests

GraphPack defines:

```text
logical_digest =
  SHA-256(
    "compass.graph.logical-digest/1"
    + length-prefixed logical schema
    + length-prefixed publication semantics
    + canonical metadata records
    + canonical node records
    + canonical edge records
  )
```

The digest uses typed, length-prefixed values and explicit enum wire numbers.
It excludes physical offsets, block boundaries, compression, section order,
artifact paths, and temporary generation paths.

During dual publication:

- `logical_digest` identifies equivalent graph meaning across adapters;
- `artifact_sha256` seals the exact `graph.cgb` bytes;
- `canonical_json_sha256` seals the exact `graph.json` bytes; and
- build state binds both artifact seals to one successful snapshot.

Existing store and JSON digest contracts remain unchanged during the
compatibility phase. New query caches use the logical digest plus query schema
and tokenizer versions.

## Write algorithm

1. Validate the typed `GraphDocument`.
2. Canonically order graph metadata, nodes, edges, and declared set-valued
   collections.
3. Collect and deterministically assign string ordinals.
4. Collect and deterministically assign anchor ordinals.
5. Build node ID to ordinal mapping.
6. Encode metadata, evidence, candidates, details, nodes, and edges.
7. Build ID and adjacency indexes.
8. Build optional file, name, term, community, and diagnostic indexes.
9. Write bounded sections to one staging file while hashing each section.
10. Write the sorted section directory.
11. Write and seal the manifest and fixed header.
12. Flush and synchronize the file.
13. Reopen through `GraphPackReader`.
14. Validate header, directory, section hashes, counts, references, and logical
    graph invariants.
15. Return an `ArtifactSeal` only after successful reopen.
16. Let `BuildGuard` publish the complete snapshot atomically.

Encoding may parallelize independent section preparation, but final ordinal
assignment and section order are deterministic. In-flight encoded bytes remain
bounded.

## Read and validation algorithm

Open performs only bounded structural work:

1. stat the file and enforce the artifact cap;
2. read the fixed header;
3. verify magic, version, required flags, and declared length;
4. read and validate the bounded section directory;
5. read and verify the manifest;
6. validate counts and feature declarations;
7. retain validated offsets and digests; and
8. defer large section reads until requested.

Point and traversal operations verify every block they read. A full explicit
`validate` operation scans all blocks, decodes all logical records, checks
cross-references, recomputes the logical digest, and optionally compares the
canonical JSON digest.

## Canonical JSON export

The canonical JSON adapter writes:

1. directed and multigraph values;
2. typed graph metadata;
3. nodes in canonical ID order; and
4. links in canonical relationship order.

It reads records through bounded cursors and writes directly to the existing
canonical JSON writer. It does not build a complete generic JSON tree or a
second complete `GraphDocument`.

Dual-publication qualification requires exact byte equality between direct JSON
publication and GraphPack export for:

- all Code Graph v1 fixtures;
- generated scale fixtures;
- Axios;
- date-fns; and
- at least one repository for every supported language family before a
  binary-first release.

## Query integration

Typed queries use the backend-neutral `GraphRead` interface. Query algorithms
must not distinguish JSON, store, and GraphPack results.

Selection rules during dual publication:

1. an explicit engine selection wins;
2. default selection uses a valid digest-bound GraphPack when present;
3. otherwise a valid selected store snapshot may be used according to current
   policy;
4. otherwise JSON is used;
5. a present selected artifact that is corrupt or mismatched fails closed; and
6. no reader silently switches realization after detecting corruption.

Whole-graph CompassQL operators may initially request bounded materialization.
Later operator-specific streaming is a query optimization, not a prerequisite
for GraphPack publication.

## History and immutable realizations

Historical realizations are immutable and already partition graph records into
content-addressed roots. GraphPack is initially a derived export, not a new
history authority.

History integration rules:

- realization identity remains unchanged;
- historical GraphPack export scans the existing graph roots;
- the export may be cached by realization ID and GraphPack physical version;
- a malformed cached GraphPack is a cache miss;
- canonical JSON reconstruction remains available;
- no historical realization is rewritten in place; and
- promoting GraphPack to authoritative history content requires a separate
  history-schema design.

## Atomic publication and recovery

During dual publication, one snapshot is complete only when required
`graph.json`, `graph.cgb`, manifest, build state, and selected store reference
artifacts are sealed coherently.

Publication order:

```text
staging graph.json + graph.cgb
  -> validate both
  -> write build-state seals
  -> commit immutable snapshot
  -> publish current-snapshot pointer
  -> atomically project root artifacts
  -> publish root-artifacts-complete
```

An interruption before snapshot commit leaves the previous complete snapshot
active. Root projection repairs itself from the selected immutable snapshot.

Binary-only opt-in uses the same sequence without requiring `graph.json`.

## Error and security model

GraphPack input is untrusted. Typed errors distinguish:

- unsupported physical major;
- unsupported required feature;
- artifact too large;
- malformed header;
- malformed or overlapping section directory;
- section too large;
- digest mismatch;
- decompression failure;
- invalid UTF-8;
- invalid enum wire value;
- invalid optional bitmap;
- invalid range or ordinal;
- duplicate identity;
- missing edge endpoint;
- invalid source anchor;
- logical validation failure;
- canonical JSON mismatch; and
- resource limit exceeded.

The reader never:

- trusts a count before checking its byte representation;
- multiplies sizes without checked arithmetic;
- allocates the declared uncompressed size before applying limits;
- follows paths stored in the graph;
- executes embedded data;
- accepts absolute source paths;
- treats corruption as an empty graph; or
- falls back after selecting a present authoritative artifact.

## Observability

Build timings add:

- GraphPack string collection;
- GraphPack record encoding;
- GraphPack index construction;
- GraphPack compression;
- GraphPack write and sync;
- GraphPack reopen validation;
- canonical JSON export from GraphPack; and
- total graph artifact publication.

Output stats add:

- GraphPack bytes;
- JSON-to-GraphPack size ratio;
- compressed and raw section bytes;
- string dictionary count and bytes;
- evidence bytes;
- index bytes;
- blocks written;
- blocks reused if later supported; and
- GraphPack physical version.

Query profiles add:

- selected graph adapter;
- open duration;
- blocks read;
- compressed and decoded bytes;
- records decoded;
- index lookups; and
- whether full materialization occurred.

## Performance qualification

Microbenchmarks:

- direct JSON encoding;
- named MessagePack encoding as a prototype baseline;
- GraphPack encoding;
- GraphPack reopen;
- node and edge point lookup;
- incoming and outgoing traversal;
- term lookup;
- full materialization;
- canonical JSON export;
- corruption rejection; and
- peak resident memory.

End-to-end qualification covers:

- cold build;
- unchanged build;
- one-file fact-neutral update;
- one-file topology-changing update;
- first query after build;
- repeated query;
- JSON export; and
- history GraphPack export.

Performance targets are acceptance gates, not claims:

| Metric | Target |
| --- | ---: |
| GraphPack size | no more than 55% of canonical JSON |
| GraphPack encode + sync | at least 2x faster than JSON encode + sync |
| Validated GraphPack reopen | at least 3x faster than cold JSON load |
| Point node lookup | no full graph materialization |
| Bounded traversal | memory proportional to requested blocks and records |
| Canonical JSON export | no more than 20% slower than direct JSON publication |
| Dual-publication cold build | no more than 10% slower than JSON-only |
| Binary-only cold build | at least 15% faster where JSON publication is at least 20% of baseline |

Missing a target is reported as a performance gap. Correctness and determinism
are never weakened to satisfy timing.

---

# Phased execution plan

## Phase 0 — Freeze contracts and establish the representation baseline

### Objective

Create reproducible evidence for deciding whether GraphPack is worth shipping
and freeze the logical equivalence contract before any persistent binary format
exists.

### Context and background

Compass already has a bounded canonical JSON writer, a typed
`compass.graph/1` model, disposable MessagePack caches, and indexed immutable
store snapshots. A new format should not be selected from serialization
intuition alone. This phase measures the upper bound and establishes exact
fixtures that every later phase must preserve.

### Starting state and dependencies

- No GraphPack source code exists.
- `graph.json` remains the only portable authority.
- Existing Axios and date-fns qualification artifacts are available as useful
  observations but are not a durable format benchmark.
- This phase has no dependency on another GraphPack phase.

### Scope

1. Add a benchmark harness for encoding, reopening, lookups, traversal, size,
   and memory.
2. Record exact tool, corpus, command, hardware, and graph digests.
3. Add a prototype named-MessagePack encoder only inside benchmark code.
4. Define canonical logical-equivalence fixtures covering every node detail,
   edge detail, evidence origin, confidence, ambiguity state, diagnostic,
   coverage status, optional field, Unicode case, and floating-point edge case.
5. Record current JSON and optional store publication timings separately.
6. Write an ADR or approved design decision confirming separate logical and
   physical versions before persistent format code lands.

### Expected files

- `benchmarks/graphpack/`
- `crates/compass-model/tests/fixtures/` or the nearest existing fixture owner
- `docs/superpowers/reviews/<date>-graphpack-baseline.md`
- optional ADR location selected by repository convention

### Verification

- Harness unit tests for statistics, digest comparison, malformed samples, and
  peak-memory parsing.
- Release-mode benchmark on deterministic fixtures.
- Release-mode benchmark on Axios and date-fns.
- Existing Code Graph v1 qualification.

### Acceptance criteria

1. The harness measures JSON encode/sync, JSON cold load, MessagePack prototype
   encode/load, point lookup after load, traversal after load, bytes, and peak
   RSS.
2. Every sample is correctness-eligible only after logical graph validation and
   canonical digest verification.
3. The fixtures exercise every current logical record variant.
4. The baseline report identifies graph-sized costs and states the maximum
   possible end-to-end speedup.
5. Maintainers approve independent logical and physical versioning.
6. No production reader, writer, CLI option, or persistent artifact is added.

### Non-goals and rollback

This phase does not select final compression or claim performance success. Its
changes are benchmark and fixture additions; they can be removed without
affecting product behavior.

## Phase 1 — Implement the bounded GraphPack envelope

### Objective

Implement a deterministic, safely reopenable GraphPack container with a fixed
header, manifest, section directory, block framing, checksums, limits, and
atomic file publication. Payload sections remain synthetic in this phase.

### Context and background

The highest-risk part of a persistent binary format is not serializing Rust
types; it is validating offsets, sizes, versions, blocks, compression, and
interruption without unbounded allocation or partial success. This phase
establishes that physical trust contract before adding graph meaning.

### Starting state and dependencies

- Phase 0 has frozen physical/logical version separation and supplied benchmark
  infrastructure.
- No GraphPack artifact is consumed by product commands.
- The logical graph remains JSON-only.

### Scope

1. Add `compass-graph::graphpack` with header, manifest, section, codec, limits,
   receipt, and typed error definitions.
2. Define explicit numeric section and enum wire values.
3. Implement safe checked integer arithmetic for every offset and length.
4. Implement raw blocks and optional Zstandard blocks.
5. Implement per-section SHA-256 verification.
6. Implement fixed-header and directory golden bytes.
7. Implement atomic write, sync, reopen, and cleanup through
   `compass-files`.
8. Add a format inspection helper for tests and future diagnostics.

### Expected files

- `crates/compass-graph/src/graphpack/mod.rs`
- `crates/compass-graph/src/graphpack/format.rs`
- `crates/compass-graph/src/graphpack/read.rs`
- `crates/compass-graph/src/graphpack/write.rs`
- `crates/compass-graph/tests/graphpack_format.rs`
- `crates/compass-files` only if a missing atomic primitive is required

### Verification

- Golden-byte encode/decode tests.
- Truncated header and directory tests.
- Unknown major and required-feature tests.
- Overlap, overflow, out-of-file range, duplicate section, missing section, and
  invalid alignment tests.
- Raw and compressed block round trips.
- Compressed-size and decoded-size bomb tests.
- Digest mismatch tests.
- Interrupted temporary-file publication tests.
- Cross-platform formatting, Clippy, and tests for affected crates.

### Acceptance criteria

1. Equivalent synthetic sections produce byte-identical files across repeated
   runs.
2. Unknown physical majors and required features fail explicitly.
3. Every malformed offset, length, count, overlap, and checksum fixture fails
   before graph-sized allocation.
4. Compressed blocks cannot decode beyond configured cumulative limits.
5. Atomic publication never exposes a partial destination.
6. Reopen validates the complete physical envelope.
7. No product command reads or publishes GraphPack yet.

### Non-goals and rollback

This phase does not encode logical nodes or edges. Removing the module and its
tests removes the feature without changing any published Compass artifact.

## Phase 2 — Encode the complete logical graph and prove JSON equivalence

### Objective

Encode and decode every `compass.graph/1` value losslessly, compute a
representation-independent logical digest, and export canonical JSON
byte-identically.

### Context and background

Graph quality depends on more than node and edge endpoints. Evidence arrays,
ambiguity candidates, occurrence rules, source anchors, diagnostics, coverage,
details, direction, and multiplicity are part of the product contract. A
compact format that drops any of them is unacceptable even if aggregate counts
match.

### Starting state and dependencies

- Phase 1 provides a validated physical container.
- Phase 0 provides exhaustive logical fixtures and canonical JSON baselines.
- Product commands still use JSON.

### Scope

1. Implement deterministic string collection and dictionary encoding.
2. Implement anchor deduplication and validation.
3. Implement every node, edge, metadata, details, evidence, candidate,
   diagnostic, and coverage wire record.
4. Implement stable enum wire-number tables.
5. Implement optional-field presence bitmaps and exact floating-point encoding.
6. Implement logical digest encoding.
7. Implement complete bounded materialization into `GraphDocument`.
8. Implement streaming canonical JSON export from GraphPack record cursors.
9. Implement extension records for same-major optional value preservation.

### Expected files

- `crates/compass-graph/src/graphpack/dictionary.rs`
- `crates/compass-graph/src/graphpack/records.rs`
- `crates/compass-graph/src/graphpack/digest.rs`
- `crates/compass-graph/src/graphpack/export.rs`
- `crates/compass-graph/tests/graphpack_roundtrip.rs`
- `crates/compass-output` only for presentation-facing JSON export plumbing

### Verification

- Round trip every Phase 0 logical fixture.
- Property tests over bounded generated valid graphs.
- Negative tests for every invalid ordinal, range, enum, bitmap, UTF-8 value,
  endpoint, anchor, score, and count.
- Direct canonical JSON versus GraphPack-exported JSON byte comparison.
- Logical digest equality across direct and GraphPack paths.
- Determinism tests with shuffled input records and parallel worker counts.
- Real-repository Axios and date-fns encode, reopen, materialize, and JSON
  comparison.

### Acceptance criteria

1. Every current logical field survives round trip with typed equality.
2. Canonical JSON exported from GraphPack is byte-identical to direct canonical
   JSON for all fixtures, Axios, and date-fns.
3. Nodes, edges, evidence entries, candidates, diagnostics, coverage,
   direction, and multiplicity have exact matching counts and values.
4. Equivalent logical graphs produce one logical digest independent of
   compression and block boundaries.
5. Shuffled equivalent input produces byte-identical GraphPack.
6. Invalid logical records fail with typed errors.
7. GraphPack meets the Phase 0 size and encode measurements or records an
   explicit performance gap before proceeding.

### Non-goals and rollback

This phase may materialize a complete graph for validation. Indexed point and
adjacency reads belong to Phase 3. Product publication remains unchanged, so
the module can still be removed without a migration.

## Phase 3 — Add indexed, bounded GraphPack reads

### Objective

Provide point, scan, adjacency, file, name, term, community, and diagnostic
operations without full graph materialization.

### Context and background

A monolithic binary serialization reduces bytes but leaves callers paying for
complete decode and index reconstruction. GraphPack earns its architectural
cost only when query work can decode records proportional to the request. The
existing immutable snapshot defines the backend-neutral semantics to match.

### Starting state and dependencies

- Phase 2 provides complete lossless record encoding and bounded cursors.
- Current store indexes provide differential behavior for lookup, ordering,
  tokenization, and truncation.
- Product query selection is unchanged.

### Scope

1. Implement node and edge ID indexes.
2. Implement incoming and outgoing compressed-sparse-row adjacency.
3. Implement file, normalized-name, term, community, and diagnostic indexes.
4. Implement safe positioned block reads and a bounded decoded-block cache.
5. Implement kind-filtered adjacency without decoding unrelated edges.
6. Implement exact term candidate semantics and truncation.
7. Add explicit operation counters for blocks, bytes, and records.
8. Keep full materialization available for whole-graph consumers.

### Expected files

- `crates/compass-graph/src/graphpack/index.rs`
- `crates/compass-graph/src/graphpack/adjacency.rs`
- `crates/compass-graph/src/graphpack/cache.rs`
- `crates/compass-graph/tests/graphpack_index.rs`
- `crates/compass-graph/tests/graphpack_limits.rs`

### Verification

- Point lookup for first, middle, last, missing, and Unicode IDs.
- Incoming/outgoing self-loops and parallel edges.
- Every edge kind and relation filter.
- High-degree node truncation.
- Vocabulary-heavy term prefixes exceeding candidate limits.
- File suffix and exact path behavior.
- Corrupt index references and mismatched section counts.
- Block-cache byte and entry limits.
- Differential operations against current store snapshot readers.
- Peak-memory and bytes-read microbenchmarks.

### Acceptance criteria

1. Point lookup reads no complete node or edge table.
2. One-hop traversal decodes only index blocks and selected edge/node records.
3. Query ordering and truncation match the store adapter exactly.
4. Parallel edges and self-loops are preserved.
5. Limits produce explicit truncation or limit errors according to the public
   operation contract.
6. Corrupt index references fail closed.
7. Reader memory remains bounded by configured cache and result limits.
8. Validated reopen and point lookup satisfy the approved performance targets
   or produce a documented gap.

### Non-goals and rollback

No public query command selects GraphPack yet. Index sections are internal and
can be redesigned while the physical format remains unreleased.

## Phase 4 — Deepen the query seam and add a GraphPack adapter

### Objective

Make typed code queries execute over JSON, store, and GraphPack through one
backend-neutral interface without forcing complete graph materialization.

### Context and background

The current materialized `GraphEngine` interface exposes a complete
`GraphDocument`, while the optimized local-store path bypasses it with a
separate backend. Adding GraphPack by cloning another full document would
preserve this architectural friction and lose most binary-read leverage.

### Starting state and dependencies

- Phase 3 supplies bounded GraphPack operations.
- JSON and store query behavior is covered by differential tests.
- GraphPack is still not part of default publication.

### Scope

1. Introduce the backend-neutral `GraphRead` interface.
2. Implement JSON, store, and GraphPack adapters.
3. Migrate typed search, callers, callees, impact, explore, and node commands.
4. Preserve common Rust ranking, direction, multiplicity, source rendering,
   candidate bounds, and truncation.
5. Keep bounded materialization for CompassQL and whole-graph consumers that
   have not yet moved.
6. Add explicit `binary` engine selection for development qualification only.
7. Expose query-profile adapter and I/O counters.

### Expected files

- `crates/compass-query/src/graph_engine.rs`
- `crates/compass-query/src/index.rs`
- `crates/compass-query/src/code_query.rs`
- `crates/compass-query/tests/graphpack_engine.rs`
- `crates/compass-query/tests/store_engine.rs`
- `crates/compass-cli/src/code_query_commands.rs` for hidden or experimental selection

### Verification

- JSON/store/GraphPack differential tests for every typed query.
- Candidate sets larger than public limits.
- Parallel relationship and direction-sensitive traversal.
- Partial-publication diagnostics.
- Query result schema and exact ordering.
- Cancellation and resource limits.
- First-query and repeated-query release benchmarks.
- Tests proving typed GraphPack queries do not call full materialization.

### Acceptance criteria

1. All typed query results are identical across the three adapters.
2. GraphPack typed queries do not allocate a complete `GraphDocument`.
3. Search tokenization, ranking, tie-breaking, and truncation remain
   backend-neutral.
4. A selected corrupt GraphPack fails closed and never falls through to JSON.
5. Whole-graph callers remain functional through bounded materialization.
6. Query startup and point/traversal performance meet approved targets or
   report a documented gap.
7. The experimental selector is not advertised as a stable release contract.

### Non-goals and rollback

Default reader selection and build publication remain unchanged. The new
adapter can be disabled without invalidating existing artifacts.

## Phase 5 — Publish GraphPack beside JSON

### Objective

Co-publish `graph.cgb` and `graph.json` as one coherent immutable snapshot,
validate their exact equivalence, and let Compass prefer GraphPack by default
when its build-state binding is valid.

### Context and background

This is the first phase that changes normal output. The current release
contract still requires `graph.json`, so GraphPack is an additional
realization. Dual publication proves production durability and reader behavior
without removing the recovery and interchange artifact.

### Starting state and dependencies

- Phases 1–3 provide a stable GraphPack format.
- Phase 4 provides a differential query adapter.
- Canonical JSON export is byte-identical for qualified inputs.
- Compatibility documentation still names JSON as permanent.

### Scope

1. Add `graph.cgb` to staged and root artifact sets.
2. Encode GraphPack and canonical JSON concurrently after graph normalization.
3. Seal both exact artifacts and one shared logical digest in build state.
4. Reopen GraphPack before snapshot commit.
5. Compare the GraphPack-exported JSON digest with the co-published JSON seal.
6. Add root projection repair for `graph.cgb`.
7. Prefer GraphPack for default Compass reads when the selected immutable
   snapshot binds both artifacts.
8. Use JSON only when GraphPack is absent, not when selected GraphPack is
   present but corrupt.
9. Include GraphPack in backup/export classification as a derived,
   reproducible realization unless later compatibility policy promotes it.
10. Update current behavior documentation.

### Expected files

- `crates/compass-core/src/pipeline.rs`
- `crates/compass-core/src/build_state.rs`
- `crates/compass-files/src/build_guard.rs`
- `crates/compass-query/src/index.rs`
- `crates/compass-output/src/backup.rs`
- `docs/reference/outputs.md`
- `COMPATIBILITY.md`
- `CHANGELOG.md`

### Verification

- Cold, forced, unchanged, one-file fact-neutral, topology-changing, deletion,
  partial, and empty-build publication tests.
- Interruption before either artifact, before build state, before snapshot
  pointer, and during root projection.
- Root repair tests.
- Corrupt, truncated, stale, and mismatched GraphPack tests.
- JSON/store/GraphPack query differential suite.
- Snapshot retention and garbage collection.
- Backup and restore classification.
- Code Graph v1 fixture qualification.
- Axios and date-fns release qualification.
- Full applicable repository baseline and product-boundary checks.

### Acceptance criteria

1. A successful normal build publishes sealed, equivalent `graph.json` and
   `graph.cgb`.
2. A failed or interrupted build leaves the prior complete snapshot active.
3. Build state binds exact JSON and GraphPack artifacts to one logical digest.
4. GraphPack export reproduces the exact co-published JSON bytes.
5. Default Compass typed queries select GraphPack when valid.
6. Missing GraphPack uses JSON; corrupt or mismatched selected GraphPack fails
   explicitly.
7. Existing external consumers can continue using literal `graph.json`.
8. Dual publication adds no more than 10% to qualified cold-build median and
   does not increase peak RSS beyond the approved bound.
9. No current public JSON command, schema, or path is removed.

### Non-goals and rollback

Binary-only output is not available. Rollback removes GraphPack from required
artifacts and default selection while leaving `graph.json` fully usable.

## Phase 6 — Add explicit binary-authority mode

### Objective

Allow an operator to skip default JSON publication explicitly while retaining
deterministic JSON export and all Compass query behavior.

### Context and background

Dual publication improves Compass query startup but cannot remove JSON write
cost from build completion. An explicit binary-authority mode is required to
measure and deliver publication savings without breaking existing default
consumers. This is a new user-visible contract and requires documentation,
migration notes, and CLI regression coverage.

### Starting state and dependencies

- Phase 5 has qualified dual publication in production.
- GraphPack is stable and reopenable across supported platforms.
- Canonical JSON export is exact.
- Default output still publishes both artifacts.

### Scope

1. Add a proposed `--graph-format json|dual|binary` selection with an
   explicitly chosen default.
2. Persist the selected format in build profile and build state.
3. In binary mode, require `graph.cgb` and omit normal `graph.json`
   publication.
4. Add deterministic JSON export from `graph.cgb`.
5. Make commands resolve an output directory containing only GraphPack.
6. Produce actionable diagnostics for external workflows requiring JSON.
7. Update backup, restore, history export, viewer generation, and integration
   commands.
8. Add migration and release notes.

### Expected files

- `crates/compass-core/src/pipeline.rs`
- `crates/compass-cli/src/lib.rs`
- `crates/compass-cli/src/help.rs`
- `crates/compass-cli/tests/`
- `crates/compass-output/src/json.rs`
- `docs/reference/commands.md`
- `docs/reference/outputs.md`
- `MIGRATION.md`
- `CHANGELOG.md`
- `COMPATIBILITY.md`

### Verification

- CLI parsing, help, conflicts, defaults, and exit codes.
- Cold, warm, update, watch, extract, backup, restore, query, viewer, and
  history flows for JSON, dual, and binary modes.
- Binary-only build followed by JSON export and exact digest comparison.
- Switching modes without stale-artifact confusion.
- Missing JSON diagnostics for literal external paths.
- Release packaging on Linux, macOS, and Windows.
- Full end-to-end performance and memory matrix.

### Acceptance criteria

1. Binary mode completes successfully without creating `graph.json`.
2. Export from binary mode produces the exact JSON bytes a direct build would
   have published for the same logical graph.
3. Every Compass-owned query and renderer works from binary-only output.
4. Mode changes invalidate or refresh build state correctly.
5. Help, reference, migration, and release documentation clearly state which
   integrations require export.
6. Unknown or corrupt binary artifacts fail explicitly.
7. Qualified binary-only builds meet the approved performance target.
8. The default remains compatible unless a separate release decision approves
   a hard cutover.

### Non-goals and rollback

This phase does not make binary the default. The opt-in can be removed in a
pre-release if qualification fails; dual and JSON output remain complete.

## Phase 7 — Decide and execute a next-major binary-first cutover

### Objective

Make GraphPack the default portable authority only if field evidence proves the
performance, portability, recovery, and integration case outweighs the
compatibility cost.

### Context and background

The current release contract explicitly makes `graph.json` permanent and
SQLite optional. A binary-first default changes literal output paths and the
recovery story for scripts and offline consumers. It must be a conscious
next-major product decision, not an optimization hidden in a patch release.

### Starting state and dependencies

- Phase 6 binary mode has shipped or completed release-candidate qualification.
- At least one release cycle has collected binary/dual mode telemetry or
  operator reports without transmitting repository data.
- All supported platforms pass corruption, recovery, export, and performance
  qualification.
- Maintainers have approved the compatibility change.

### Scope

1. Choose the new default and supported compatibility window.
2. Update the output contract and stable path documentation.
3. Decide whether default builds publish only `graph.cgb` or retain an
   optional eager JSON export.
4. Provide explicit JSON export guidance for integrations.
5. Update init templates, CLI help, examples, release packaging, backup,
   restore, and support procedures.
6. Reject unsupported GraphPack majors explicitly.
7. Remove transitional dual-write behavior only when the approved migration
   allows it.
8. Preserve historical realizations; never rewrite them in place.

### Expected files

- `COMPATIBILITY.md`
- `MIGRATION.md`
- `CHANGELOG.md`
- `docs/reference/outputs.md`
- `docs/reference/commands.md`
- `docs/guides/integrating-compass.md`
- CLI, core, output, history, backup, and release tests

### Verification

- Complete native baseline and applicable product gates.
- Cross-platform release packaging.
- Upgrade tests from the last JSON-default release.
- Downgrade and unsupported-major diagnostics.
- Binary-only recovery and JSON export.
- External integration fixtures consuming exported JSON.
- Full real-repository build and query performance matrix.
- Documentation link and example checks.

### Acceptance criteria

1. The compatibility ledger explicitly identifies GraphPack as the default
   portable authority and JSON as deterministic export.
2. Migration instructions cover literal `graph.json` consumers.
3. Upgrade behavior never silently mistakes an old snapshot for a new format.
4. Historical realizations remain immutable and readable through their
   supported contracts.
5. All Compass-owned commands work without eager JSON.
6. JSON export remains exact and bounded.
7. Release qualification demonstrates material end-to-end and query-startup
   improvement on the approved repository matrix.
8. The cutover is rejected if performance gains are marginal, portability is
   incomplete, or major integrations cannot migrate safely.

### Non-goals and rollback

This phase is a product release decision, not an automatic consequence of
earlier implementation. If the cutover is rejected, dual publication or
explicit binary mode remains the supported endpoint.

## Cross-phase completion checklist

Every implementation phase must:

1. inspect `git status --short` and preserve unrelated changes;
2. keep all Cargo artifacts on the mounted workspace target directory;
3. add regression tests at the lowest owning crate;
4. retain bounded reads, writes, subprocess output, and allocations;
5. preserve deterministic ordering and exact logical equivalence;
6. run formatting and focused Clippy/tests;
7. run Code Graph v1 fixture qualification when graph publication or query
   behavior changes;
8. run product-boundary checks;
9. inspect final diff and status;
10. report checks not run and why; and
11. update compatibility, migration, release, security, performance, and
    reference documents when that phase changes their contracts.

## Final recommendation

Proceed through Phases 0–4 before committing to normal dual publication.
Those phases establish whether indexed binary reads deliver enough leverage
without changing product output. Phase 5 is the first compatible production
publication. Phase 6 is an explicit performance mode. Phase 7 remains a
separate next-major decision.

This ordering avoids two failure modes:

- shipping a monolithic binary cache that is smaller but still requires full
  materialization; and
- breaking the permanent JSON contract before GraphPack has proved exact
  export, bounded recovery, cross-platform durability, and material
  end-to-end performance.
