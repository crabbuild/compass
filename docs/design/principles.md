# Compass design principles

Compass is designed as a native, inspectable knowledge graph engine rather than
a hidden retrieval service. These principles explain the trade-offs behind its
public behavior and provide a test for future changes.

> **Who this page is for:** contributors, maintainers, evaluators comparing
> architectural approaches, and integrators relying on Compass contracts.
>
> **You will learn:** the principles governing local execution, determinism,
> evidence, bounds, publication, compatibility, and independent evolution.
>
> **Prerequisites:** [How Compass works](../concepts/how-it-works.md).
>
> **Reading time:** 10–12 minutes.

## 1. Local-first structural knowledge

Source-code extraction and graph querying should work without:

- Python at runtime;
- embeddings;
- a vector database;
- runtime grammar downloads;
- separately installed native libraries;
- model credentials.

This keeps the default code path fast, inspectable, and suitable for private
repositories.

“Local-first” does not mean “Compass never supports network features.” Optional
semantic providers, GitHub workflows, remote ingestion, database
introspection, and graph exports cross explicit network boundaries. The design
requirement is that those boundaries are chosen and visible.

## 2. Structure before similarity

Compass records project entities and typed relationships:

```text
handler --CALLS--> service --USES--> repository
```

This is different from returning text chunks that are close in embedding space.
Structural relationships provide:

- direction;
- relation type;
- stable identity;
- source location;
- provenance;
- traversable multi-hop context.

Semantic extraction extends the graph for non-code knowledge; it does not
replace structural facts with similarity scores.

## 3. Evidence stays attached

An edge is more useful when a reader knows how it was established:

```text
EXTRACTED   direct source/input evidence
INFERRED    resolved from multiple structural facts
AMBIGUOUS   multiple viable interpretations remain
```

Compass should preserve uncertainty instead of turning every candidate into an
equally certain edge. Export, merge, history, and query operations must not
silently discard provenance.

## 4. Determinism where the inputs permit it

Structural outputs should be reproducible under equivalent inputs and
meaning-affecting configuration. That requires:

- deterministic discovery and registries;
- stable node identity;
- stable ordering where output contracts require it;
- bounded, explicit resolution;
- canonical historical encoding;
- fixed planner/language versions in cache keys;
- tests against a pinned behavioral oracle where compatibility applies.

Provider-backed semantics may not be perfectly reproducible. Compass isolates
that variability in profiles, caches, validation, and completeness metadata.

Determinism is semantic, not cosmetic: insignificant JSON object ordering is
not graph meaning, while nodes, relationships, attributes, and multiplicity
are.

## 5. Reject unsupported meaning

CompassQL is read-only and deliberately bounded. Unsupported constructs are
rejected rather than approximated.

The same rule applies broadly:

- missing semantic credentials do not silently become a code-only historical
  profile;
- an unreadable preferred realization is not silently replaced;
- unsafe checkout filters are rejected for historical materialization;
- an incomplete semantic build cannot publish as complete;
- a profile mismatch is surfaced before a normal diff.

Explicit failure is easier to debug and safer to automate than plausible but
different behavior.

## 6. Bounded work is part of the interface

Parsers, query engines, services, and network clients process potentially
untrusted or unexpectedly large input.

Compass uses bounds for:

- graph JSON size;
- semantic fragment nodes, edges, hyperedges, and bytes;
- provider response size;
- media and archive expansion;
- CompassQL source, tokens, nesting, paths, rows, expansions, memory, and time;
- subprocess output and timeout;
- URL ingestion and redirects;
- MCP HTTP requests and result sizes.

These are not afterthoughts. A limit failure is a distinct result that a caller
must not interpret as “no matches.”

## 7. Publish coherent artifact sets

Generated artifacts must describe one successful build. The pipeline uses
staging, build guards, validation, and atomic writes so a failure does not
present incomplete data as complete.

```text
build temporary facts
      |
      v
validate completeness
      |
      +-- fail --> retain prior coherent output / report failure
      |
      `-- pass --> atomically publish the new set
```

Historical publication is stronger: realizations are immutable and preferred
pointers change only through validated, explicit operations.

## 8. Separate models from orchestration

Crate boundaries keep responsibilities reviewable:

- `compass-model` owns graph data and indexing;
- `compass-files` owns discovery, fingerprints, caches, and atomic filesystem
  primitives;
- `compass-languages` owns per-language extraction;
- `compass-resolve` owns cross-file resolution;
- `compass-graph` owns graph construction and algorithms;
- `compass-query` and `compass-cypher` own query behavior;
- `compass-core` orchestrates application workflows;
- `compass-cli` owns command parsing and user-facing outcomes.

Optional integrations have their own crates. A language extractor should not
need to know how MCP authentication works; a graph renderer should not own Git
history leases.

## 9. Keep machine contracts explicit

Human-readable output can improve. Machine consumers need version tags,
documented schemas, stable diagnostics, and exact exit categories.

Examples:

- `compass.cql.result/1`;
- `compass.cql.jsonl/1`;
- canonical history encoding and schema versions;
- typed diagnostics and error families;
- opaque stable string IDs.

An integration should reject an unknown major version. Compass should not
encourage parsing prose when a structured format exists.

## 10. Make current and historical truth distinct

The working tree and an exact commit are different sources:

```text
compass update .            may include uncommitted working-tree state
compass ... --at REV        exact commit + exact extraction fingerprint
```

Historical materialization is offline and isolated so local excludes,
uncommitted files, network fetches, hooks, and checkout filter execution do not
change the requested revision.

The same commit can have several valid realizations. Compass makes the selected
profile and preference visible.

## 11. Compatibility is evidence, not identity

Compass was inspired by Graphify and uses a frozen Graphify version as a
development oracle for certified command families. This provides valuable
behavioral evidence.

Compass is not permanently constrained to Graphify's surface:

- the shipped executable is `compass`;
- CompassQL is Compass-native;
- versioned graph history is Compass-native;
- native architecture, performance, safety, and product needs can justify
  divergence;
- intentional incompatibility requires documentation and migration guidance.

The compatibility ledger defines the current boundary. Marketing language does
not.

## 12. Developer experience follows correctness

Fast cold builds, very fast unchanged updates, helpful reports, native
installers, and coding-assistant integration matter because they make correct
use practical.

Performance changes still must preserve:

- graph parity/equivalence;
- deterministic ordering contracts;
- completeness;
- resource bounds;
- clear failure.

The [performance qualification](../../PERFORMANCE.md) requires graph
correctness before speed results are accepted.

## 13. Generated output remains inspectable

Compass produces ordinary files and documented structures:

- JSON graphs;
- Markdown reports;
- optional HTML/SVG/export formats;
- SQLite-backed history with explicit commands;
- plain-text and structured query output.

Users can inspect, diff, validate, export, and archive the result. Compass does
not require all value to remain behind a proprietary hosted API.

## Decision checklist for a new feature

Before adding a feature, ask:

```text
Does structural/local behavior still work without it?
Where is the network, credential, or execution boundary?
What evidence and provenance will results carry?
What are the resource and input bounds?
How is incomplete work prevented from publishing?
Which crate owns the responsibility?
What is the exact machine contract?
Does it affect historical fingerprints?
Is it compatible, intentionally divergent, or Compass-native?
What fixture proves direction, multiplicity, attributes, and failure?
```

If those questions do not have clear answers, the design is not ready.

## Related pages

- [System architecture](architecture.md)
- [Security and privacy](security-and-privacy.md)
- [Compatibility reference](../reference/compatibility.md)
- [Performance qualification](../../PERFORMANCE.md)

**Next step:** read [System architecture](architecture.md) to see how these
principles map onto crates and data flows.
