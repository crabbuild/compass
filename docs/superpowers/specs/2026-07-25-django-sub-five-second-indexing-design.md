# Django Sub-Five-Second Indexing Design

## Context

Compass can now index the Django repository without the duplicate Python
function-symbol failure, but the successful deterministic build is too slow for
an interactive initialization workflow.

The measured release-build baseline on the current machine and Django checkout
is:

| Operation | Wall time | Result |
| --- | ---: | --- |
| Cold `compass init` build | 21.08s | 6,051 files, 52,186 nodes, 191,641 edges |
| Unchanged `compass update` | 5.83s | 6,051 cached files |
| Cold extraction without clustering | 17.41s | 52,190 nodes, 197,340 edges |

The cold no-cluster profile divides into 2.5 seconds of detection, 10.1 seconds
of Program IR plus graph extraction, and 4.5 seconds of serialization and
writes. The unchanged update spends 5.22 seconds validating and deserializing
the existing 272 MB `program.json`, which accounts for almost its entire wall
time.

The completed index occupies about 1 GB. The largest components are:

| Artifact | Size |
| --- | ---: |
| `program.json` | 272 MB |
| Program merge cache | 272 MB |
| Program syntax cache | 213 MB |
| `graph.json` | 104 MB |
| AST cache | 98 MB |

The current pipeline reads and parses supported source files once for Program
IR and again for the graph. It then validates and serializes the complete
Program IR more than once. Those costs were introduced after the earlier cold
graph extraction optimizations and cannot meet the new target through worker
count tuning alone.

## Goal

On the fixed Django checkout and current 12-core Apple Silicon machine, all
three deterministic index workflows must finish in less than five seconds:

1. non-interactive cold `compass init` with no Compass output or cache;
2. a forced full index build through `compass update --force`; and
3. an unchanged `compass update`.

The release binary must preserve the current deterministic graph, Program IR,
community analysis, report, and cache semantics. Correct output may not be
omitted, deferred until after the command exits, or replaced with a warmed-cache
measurement to pass the gate.

Compass must also report the operation's elapsed wall time after `init`,
`extract`, and `update`, with an optional stage breakdown that applies
consistently to all three commands.

## Requirements

The implementation must:

- read each source file at most once during a cold deterministic build;
- parse each supported tree-sitter source at most once during that build;
- preserve byte-stable `program.json` and graph facts for a fixed input;
- preserve incremental Program IR and AST reuse after a source edit;
- avoid fully deserializing `program.json` on an unchanged update;
- detect missing, stale, truncated, or externally modified output artifacts and
  fall back to validation or rebuilding;
- publish graph, Program IR, cache, manifest, and trusted build state as one
  successful build generation;
- report a wall-clock total after successful `init`, `extract`, and `update`;
- expose the existing detailed stages through `--timing` on `update` as well as
  `extract`, while retaining one concise total by default;
- include a reproducible Django qualification command that records every sample,
  output counts, and the median and maximum latency; and
- keep existing focused, crate, and workspace tests green.

No Python process, network service, background daemon, or repository-specific
special case may be added to the production indexing path.

## Selected design

### One source snapshot

Detection will produce the ordered source set and a build-local source snapshot.
Large-corpus word counting and source reads will use the same bounded worker
pool. Each snapshot entry contains:

- canonical repository-relative path;
- language and extractor kind;
- bytes;
- content digest; and
- filesystem stamp used by the manifest.

`compass init` currently detects the repository once to validate and display the
scope, then asks the build pipeline to detect it again. The init path will pass
its validated detection into the build so the command performs one filesystem
walk.

The source snapshot is owned only for the lifetime of the build. Source bytes
are released as soon as both deterministic consumers finish.

### One tree-sitter parse

`compass-languages` will expose a combined deterministic extraction operation.
For languages supported by both graph extraction and Program IR, one
tree-sitter tree feeds both extractors:

```text
source bytes
    |
    +-- one tree-sitter parse
            |
            +-- graph Extraction
            +-- Program EvidenceBatch
```

Languages without Program IR support continue to emit only graph extraction.
Non-tree-sitter and format-specific extractors keep their current behavior.
The existing standalone `TreeSitterSyntaxProvider` remains available for public
provider use, but the core cold-build path uses the combined operation.

Each worker returns graph facts, optional program evidence, source identity, and
cache-ready values. Per-file validation remains at the language boundary.

### Parallel graph and Program assembly

Once all per-file results exist, the graph and Program branches no longer need
to run serially:

```text
combined per-file results
    |
    +-- graph resolve -> build -> cluster -> analyze
    |
    +-- merge evidence -> analyze Program IR
```

The two branches execute concurrently in a scoped build-local pool. Their
outputs join only for atomic publication and the final CLI summary. This keeps
determinism because each branch retains its existing canonical ordering before
serialization.

Graph export, Program export, cache publication, and small report artifacts are
also serialized concurrently after their immutable models are complete.
Successful command completion waits for every artifact.

### Binary deterministic caches

Human-facing outputs remain JSON. Internal AST and Program syntax caches move
to versioned MessagePack records using the existing `rmp-serde` workspace
dependency.

The binary cache contract includes:

- an explicit schema and producer version in the cache directory;
- repository-relative source identities;
- atomic per-entry writes;
- validation after decode;
- JSON cache fallback only for the one compatibility transition when safe; and
- pruning based on the same live logical keys used today.

This removes repeated JSON value construction and reduces the 6,000-file cold
cache publication cost. A cache format/version change invalidates only the
affected deterministic cache namespace.

The complete Program merge cache will be removed. `program.json` is already the
canonical analyzed Program generation. An unchanged build trusts it through the
sealed build state described below; a changed provider set performs a new merge.
This avoids storing and serializing a second 272 MB analysis bundle.

### Sealed build state

Compass will replace size-only output trust with a versioned
`.compass_build_state.json`. It is the last file published by a successful
build and contains:

- schema and producer versions;
- normalized build options that affect output;
- manifest identity;
- graph byte length, digest, nodes, edges, and communities;
- Program byte length, digest, modules, summaries, provider count, and conflict
  count;
- required side-artifact identities; and
- completion generation.

Artifact digests are computed while artifacts are serialized, not by a second
canonicalization pass. On an unchanged update, Compass:

1. performs detection and the existing manifest change check;
2. validates the build-state schema and build options;
3. verifies required artifact metadata and streamed digests without parsing
   their JSON;
4. returns the saved counts and Program statistics.

If the state is absent or any seal fails, Compass falls back to the existing
safe validation/rebuild path. A failed or interrupted build never publishes the
new state file, so partial artifacts cannot become trusted.

The seal does not weaken source-change detection. The existing manifest remains
authoritative for the repository corpus; the new state proves only that the
published outputs still correspond to the completed manifest generation.

### Single-pass canonical Program serialization

`AnalysisBundle::canonical_bytes()` currently canonicalizes, regenerates
summaries for validation, and materializes the complete JSON byte vector.
Callers then repeat related work for caching and artifact validation.

The build path will:

1. validate the newly assembled analysis once;
2. canonicalize it once;
3. stream compact canonical JSON through a buffered atomic writer;
4. compute its digest and byte count through the same writer; and
5. publish those values in the sealed build state.

Offline `compass program` commands retain strict full validation when opening an
untrusted artifact directly. The optimization applies only to an analysis just
constructed by the current process or to a sealed artifact generation.

### Timing output

Every successful index operation prints one final line:

```text
Compass init completed in 4.82s.
Compass extract completed in 4.37s.
Compass update completed in 0.91s.
```

For interactive init, the duration begins after the user confirms the build so
human response time is not reported as indexing latency. Non-interactive init
uses the same boundary.

`--timing` is accepted by `init`, `extract`, and `update`. It adds a stable stage
breakdown on stderr:

```text
[compass timing] detect: 0.4s
[compass timing] deterministic extract: 1.8s
[compass timing] graph assembly: 1.1s
[compass timing] program analysis: 0.9s
[compass timing] publish: 0.8s
[compass timing] total: 4.5s
```

Concurrent stage durations may overlap and therefore are not presented as
additive. The total is measured independently with `Instant`.

Failures report the same operation name and elapsed duration without claiming
completion.

## Performance budget

The design uses the following engineering budget for the cold Django gate:

| Work | Target |
| --- | ---: |
| Scope detection and source snapshot | 0.7s |
| Combined deterministic extraction | 1.8s |
| Concurrent graph and Program assembly | 1.2s |
| Concurrent cache and artifact publication | 1.0s |
| CLI and synchronization overhead | 0.3s |
| Total | <5.0s |

These are diagnostic budgets, not separate acceptance gates. Overlapping work
can make individual stage durations sum to more than the wall-clock total.

## Alternatives considered

### Optimize only unchanged updates

A compact Program statistics file would reduce the warm path below five
seconds, but the cold build would remain near twenty seconds. This fails the
explicit init/build requirement.

### Disable or defer Program IR

Skipping Program IR removes much of the regression, but changes Compass output
and makes command completion dishonest because required indexing continues
later. This is rejected.

### Increase the Rayon worker count

The current cold build already uses the host CPU count and averages far below
full utilization because major branches are serial and perform duplicate work.
Oversubscription cannot remove duplicate parsing, validation, or serialization.

### Replace public JSON with a binary format

This would reduce output cost but break documented artifacts and offline
consumers. The selected design changes only internal caches and keeps public
JSON stable.

## Correctness and recovery

- Combined extraction is covered by differential tests against the existing
  standalone graph and Program extractors for Python, Rust, TypeScript, TSX, and
  JavaScript fixtures.
- Output ordering remains canonical before concurrent serialization.
- Cache decode or validation failure is a cache miss, never a repository-wide
  failure.
- Build-state schema, option, manifest, artifact length, or digest mismatch
  forces safe fallback.
- The state file is published last under the existing output build guard.
- A process crash before state publication leaves the previous generation
  trusted or the output marked incomplete.
- Explicit `--force` never takes the unchanged fast path.
- Existing semantic and supplemental layers retain their refresh and shrink
  guards.

## Qualification

A guarded `scripts/qualify_django_performance.sh` command will:

1. validate the Django Git root and reject broad or unsafe paths;
2. build the release Compass binary once outside measured samples;
3. create an isolated detached Django worktree;
4. run three cold non-interactive init samples with only the exact benchmark
   output and generated config removed between samples;
5. run three forced full update samples;
6. run three unchanged update samples;
7. capture Compass's built-in total and independent monotonic wall time;
8. record commit IDs, machine details, command, sample, duration, file count,
   nodes, edges, communities, Program modules, summaries, and conflicts;
9. verify output counts and canonical artifact digests across samples; and
10. fail unless every measured sample is below five seconds.

The script prints a TSV artifact and a concise summary containing every sample,
the median, and the maximum. Worktree setup, release compilation, and deliberate
human prompt time are outside the measured index operations.

The default Django root is `/Users/haipingfu/Github/django`, and callers can
override it with `DJANGO_ROOT`. Destructive cleanup is limited to paths created
inside the script's validated temporary directory.

## Verification strategy

Each coherent implementation task is followed by focused regression coverage
and verification:

1. Verify combined extraction is equivalent to the standalone graph and
   Program extractors.
2. Add source-read and parse-count instrumentation proving one cold pass.
3. Cover binary cache round trips, corruption, versioning, and migration.
4. Cover the sealed-state fast path for valid, missing, truncated, modified,
   option-mismatched, and interrupted generations.
5. Cover default totals, `--timing` on all build commands, and failure
   durations.
6. Run focused language, files, core, output, analysis, and CLI tests after
   their corresponding implementation changes.
7. Run workspace formatting, linting, and tests.
8. Run the Django qualification script against the release binary.

`graphify update .` is explicitly outside this work. Runtime verification uses
the actual Compass init and update commands on the Django checkout.

Microbenchmarks guide optimization but cannot satisfy the acceptance gate. Only
the complete release commands on the Django repository prove the target.

## Acceptance criteria

The work is complete when:

1. all three cold init samples finish in less than 5.0 seconds;
2. all three forced full update samples finish in less than 5.0 seconds;
3. all three unchanged update samples finish in less than 5.0 seconds;
4. Compass prints its own elapsed total for init, extract, and update;
5. `--timing` reports the new stable stages for all three commands;
6. Django file, graph, community, Program module, summary, and conflict counts
   match the approved baseline or an explained correctness fix;
7. repeated builds produce canonical-equivalent public artifacts;
8. corruption and interrupted-publication tests prove the fast path fails safe;
9. focused, workspace, and Django Compass init/update verification pass.
