# Achieve Graphify superset parity without slowing Compass

This design corrects Compass extraction gaps found on Podman, makes parity with Graphify measurable, and protects Compass's release-build performance. Compass will preserve its broader language support while matching Graphify's canonical nodes and edges for inputs both tools support.

## Goal and audience

The goal is to make Compass an interpretation-compatible superset of Graphify on shared supported source files. For every Graphify node and edge in that scope, Compass must emit the same canonical identity and topology. Compass may also emit valid facts that Graphify does not, including Perl and extensionless executable scripts.

The audience includes Compass maintainers, contributors who add language extractors, and users evaluating Compass as a faster Graphify replacement on large repositories.

## Requirements

The implementation must:

- Correct C and C++ symbol names so return types, storage classes, and macros do not replace function identifiers
- Preserve positional Markdown sections and rationale entries as distinct graph nodes
- Match every Graphify node and edge for shared supported Podman inputs
- Preserve valid Compass-only nodes and edges
- Invalidate stale extraction cache entries when extractor semantics change
- Keep the Podman release-build median at or below 32 seconds for cold indexing and 10 seconds for warm updates
- Avoid a Graphify, Python, or network dependency in production Compass execution

Correctness has priority over the performance thresholds. The implementation may optimize work, caching, and allocation, but it must not omit correct facts to meet a timing target.

## Parity contract

Parity is set inclusion rather than equal aggregate counts.

Let `G` be Graphify output for source files and languages supported by both tools. Let `C` be Compass output for the same repository. Acceptance requires:

```text
Graphify shared nodes ⊆ Compass nodes
Graphify shared edges ⊆ Compass edges
```

A shared node matches when its canonical ID is equal and its observable extraction fields agree. These fields include label, node type, source path, and source location when Graphify supplies them.

A shared edge matches when its source ID, target ID, and relation are equal. Edge comparison will include deterministic metadata that participates in graph meaning, but it will ignore storage order.

The parity gate will exclude derived clustering and community-analysis output because those results depend on the complete graph. Valid Compass-only facts can change those derived results even when the shared extracted graph is compatible.

The shared-input filter will compare files Graphify successfully supports. It will not treat Perl files or extensionless executable scripts as parity failures when Graphify emits no facts for them.

## Current Podman baseline

The baseline comparison produced these totals:

| Metric | Compass | Graphify |
| --- | ---: | ---: |
| Nodes | 116,336 | 119,191 |
| Edges | 237,600 | 258,417 |
| Code nodes | 110,221 | 111,619 |
| Document nodes | 6,028 | 7,484 |
| Rationale nodes | 49 | 50 |
| Concept nodes | 38 | 38 |

The outputs share 116,132 nodes. Graphify has 3,059 unmatched nodes and Compass has 204 unmatched nodes. The Graphify-only set contains 1,602 code nodes, 1,456 document nodes, and one rationale node.

Two causes explain almost all of the node difference:

1. `vendor/github.com/mattn/go-sqlite3/sqlite3-binding.c` accounts for 1,539 Graphify-only nodes. Compass also emits 62 unmatched SQLite nodes with incorrect generic names such as `char()`, `struct()`, `int()`, and `void()`. The net SQLite gap is 1,477 nodes.
2. Positional document merging accounts for a net gap of 1,456 document nodes. In `RELEASE_NOTES.md`, Compass retains 141 nodes while Graphify retains 533. Repeated headings created with distinct line-based IDs are later merged by generic entity deduplication.

Compass covers all 8,173 source files represented by Graphify plus eight extensionless scripts. It also emits 24 nodes from the Perl `hack/buildah-vendor-treadmill` executable, where Graphify emits none. These additions demonstrate why total-count equality would be the wrong contract.

The observed Compass release timings are 30.11 seconds for the latest cold run, 28.96 seconds for an earlier cold run, and 8.80 seconds for a warm run. The formal acceptance protocol below replaces individual observations with medians.

## C and C++ declarator resolution

Compass currently asks a generic declaration-name helper for a name before it resolves the function declarator. In macro-heavy C, the generic traversal can select a return type, storage class, or macro identifier before it reaches the function identifier.

The extractor will introduce an explicit declarator resolver and call it before any generic fallback. The resolver will:

1. Start from the declaration's `declarator` field.
2. Recursively unwrap pointer, reference, parenthesized, array, function, and attributed declarator nodes.
3. Select C identifiers from the resolved declarator rather than from the declaration prefix.
4. Preserve C++ qualified identifiers, field identifiers, destructors, and operator names.
5. Return no name when the declarator does not contain a supported callable identifier.
6. Use the existing generic fallback only for syntax shapes outside the explicit resolver's contract.

The resolver will not scan arbitrary descendants for the first identifier. This prevents a type or macro in the declaration prefix from becoming the canonical function name.

Focused fixtures will cover:

- Macro-decorated C declarations
- Pointer and function-pointer declarators
- Nested and qualified C++ methods
- Constructors and destructors
- Operator overloads
- Malformed declarations that must fail safely

The SQLite binding file will serve as the large regression corpus. The comparison must confirm canonical names such as `sqlite3CompileOptions()`, `sqlite3StatType()`, and `sqlite3StatusValue()` at the locations where Compass currently emits generic type names.

## Positional document and rationale identity

Markdown headings and rationale entries describe positions in a document, not global entities. Two headings with the same text can represent different sections and must remain distinct.

The extraction stage already creates line-qualified identities for repeated Markdown headings. Generic graph deduplication later merges non-code nodes with the same normalized label and source path. The deduplication policy will distinguish positional content from entity content:

- `document` and `rationale` nodes keep their extractor-assigned positional IDs
- Exact or fuzzy label-based entity merging will not combine positional nodes
- Literal duplicate records with the same canonical ID may still collapse
- Parent-child section edges will continue to reference the preserved positional IDs
- Concept and entity nodes will retain their existing deduplication behavior

Positional nodes do not benefit from normalization sketches used for fuzzy entity matching. Skipping that work protects both identity and performance.

Fixtures will include repeated headings at different lines, repeated rationale text, nested heading hierarchies, and literal duplicate records. The tests must prove that positional repeats survive while byte-identical records with one canonical ID do not multiply.

## Cache invalidation

Corrected extraction logic must not reuse graph fragments produced by older semantics. The implementation will advance the deterministic extraction fingerprint or cache namespace used by the manifest and AST cache.

The cache test will:

1. Build an index with the old fixture result represented in cache.
2. Run an update using the new extractor version.
3. Confirm that affected files are re-extracted.
4. Run another update without changes.
5. Confirm that the second update uses the warm path.

The implementation plan will identify the concrete cache-version field after tracing the current manifest and cache-key flow.

## Differential parity gate

A deterministic comparison harness will normalize Compass and Graphify exports into comparable records. Graphify remains a test oracle and benchmark participant only. It will not become a runtime dependency.

The harness will report:

- Missing Graphify node IDs
- Missing Graphify edges grouped by relation
- Field mismatches for shared node IDs
- Counts by source path, language, and node type
- Compass-only nodes and edges as informational additions
- The largest source-level gaps first

Reports must include representative records, not only totals. This keeps failures actionable when aggregate counts happen to balance.

The Podman parity workflow will run in stages:

1. Execute focused unit and integration fixtures.
2. Build fresh Compass and Graphify outputs from the same Podman commit.
3. Apply the shared-input filter.
4. Compare nodes and then edges.
5. Group remaining failures by extractor and source file.
6. Add a focused failing fixture before correcting each new extraction class.
7. Repeat until both Graphify-missing sets are empty.

The checked-in test suite should use compact fixtures and deterministic snapshots. The full Podman and Graphify comparison may remain an explicit release-validation command if its runtime is unsuitable for routine continuous integration.

## Performance design

Performance work will preserve the current incremental architecture and keep expensive compatibility logic outside the production indexing path.

The main safeguards are:

- Resolve declarators with bounded syntax-tree traversal
- Skip entity-normalization and fuzzy-dedup work for positional nodes
- Reuse parsed output for unchanged files after the one-time cache-version change
- Stream or index parity records in the external comparison harness instead of adding runtime graph passes
- Measure release binaries only

The benchmark procedure will use one fixed Compass commit, one fixed Graphify commit, and one fixed Podman commit. It will record tool versions, commands, machine details, output counts, and elapsed wall time.

For each tool:

1. Remove only that tool's output directory.
2. Run three cold builds.
3. Recreate the same warm starting state for each warm sample.
4. Run three no-source-change updates.
5. Record every sample and compare medians.

Compass passes when its median cold time is at most 32 seconds and its median warm time is at most 10 seconds. Query benchmarks will use the same completed indexes and identical semantic queries. Query results must first satisfy correctness checks, then report median latency over repeated runs.

Graphify timing provides the comparison baseline. It does not relax the absolute Compass thresholds.

## Error handling and diagnostics

Unsupported or malformed files must not abort a repository-wide update. The extractor will attach the source path and parser context to recoverable diagnostics, skip only the unsupported construct, and continue processing the file when safe.

The parity harness will exit unsuccessfully when Graphify nodes, Graphify edges, or shared-node fields are missing. Compass-only additions will not fail the run. Performance threshold failures will be reported separately from parity failures so a speed regression cannot hide a correctness result.

## Test strategy

Implementation will follow a red-green-refactor sequence:

1. Add a focused failing test for one diagnosed behavior.
2. Confirm that the failure reflects the intended contract.
3. Make the smallest production change that passes it.
4. Run the relevant crate tests.
5. Refactor while keeping the focused test green.
6. Run workspace checks and the Podman differential gate.

Required verification includes:

- C and C++ declarator unit tests
- Markdown and rationale deduplication tests
- Cache invalidation and warm-update tests
- Deterministic parity-harness tests with known missing and extra records
- Existing language extraction and graph tests
- Workspace formatting, linting, and test commands
- Fresh Podman Compass and Graphify output comparison
- Three-sample cold, warm, and query benchmarks

After code changes, `graphify update .` will refresh the repository knowledge graph as required by the project instructions.

## Acceptance criteria

The work is complete when all of these conditions hold:

1. No Graphify node in the shared Podman input set is missing from Compass.
2. No Graphify edge in the shared Podman input set is missing from Compass.
3. Shared node IDs, labels, types, source paths, and available locations agree.
4. Valid Compass-only Perl and extensionless-script facts remain present.
5. SQLite C functions use callable identifiers rather than return types or macros.
6. Repeated positional Markdown sections and rationale entries remain distinct.
7. A semantic extractor change invalidates stale cache entries once, then returns to the warm path.
8. The Compass release median is at most 32 seconds cold and 10 seconds warm across three runs each.
9. Query comparisons return equivalent shared facts before latency is evaluated.
10. Focused tests, workspace verification, and the refreshed Graphify knowledge graph complete successfully.

## Out of scope

This work will not force equal total node or edge counts, remove valid Compass-only language support, match derived community assignments, add Graphify to Compass runtime dependencies, or trade away correct extraction to reach a benchmark threshold.

## Documentation plan

The implementation will update contributor-facing extraction documentation with the parity contract, the local differential command, and the benchmark protocol. Release notes will describe corrected C and C++ names, preserved positional document sections, and any cache rebuild users should expect after upgrading.

There are no unresolved product questions. Concrete module boundaries and command names will be finalized in the implementation plan after the written design is approved.
