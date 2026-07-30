# Make Compass at least 5x faster without weakening its code graph

This design defines a reproducible Compass performance harness, a fresh Compass baseline, an explicit Graphify comparison mode, and an evidence-driven optimization loop for build and query performance. Correctness, determinism, graph completeness, graph precision, provenance, and bounded execution remain release requirements.

## Goal and audience

The primary goal is to measure and prevent release-mode Compass regressions across eight real repositories. An explicit comparison run must also prove that Compass is at least 5x faster than the latest Graphify revision on every comparable primary workload.

The audience includes Compass maintainers, contributors changing build or query hot paths, and evaluators comparing Compass with Graphify.

## Approved decisions

The user approved these decisions:

- Benchmark cold builds, unchanged warm builds, one-file incremental builds, natural-language queries, and CompassQL queries
- Apply the 5x median target independently to every comparable build and natural-language query workload
- Use the latest default-branch versions of Compass, Graphify, and each corpus, then record exact commit identifiers in every result
- Build Compass in release mode after incorporating the latest `origin/main`
- Use Django, Spring Framework, Rails, Laravel, Bevy, ASP.NET Core, Angular, and `entireio/cli` as the primary real-repository matrix
- Keep deterministic generated fixtures for harness tests and scaling diagnosis, but never use them to satisfy the real-repository 5x claim
- Keep the complete harness under the Compass repository and make Compass-only verification its default mode
- Run Graphify only when an operator explicitly requests a comparative baseline
- Do not require a strict test-driven development workflow

CompassQL has no Graphify equivalent. CompassQL receives Compass-owned correctness, latency, memory, and regression gates instead of a fabricated cross-tool speed ratio.

## Repository matrix

The harness resolves the latest default-branch commit for each repository at the start of a qualification run:

| Name | Repository |
| --- | --- |
| Django | `https://github.com/django/django.git` |
| Spring | `https://github.com/spring-projects/spring-framework.git` |
| Rails | `https://github.com/rails/rails.git` |
| Laravel | `https://github.com/laravel/framework.git` |
| Bevy | `https://github.com/bevyengine/bevy.git` |
| ASP.NET Core | `https://github.com/dotnet/aspnetcore.git` |
| Angular | `https://github.com/angular/angular.git` |
| Entire | `https://github.com/entireio/cli.git` |

Every result records the repository URL, resolved default branch, commit identifier, tree identifier, checkout size, file inventory, and corpus fingerprint. The harness never compares results from different corpus commits as if they came from the same input.

## Historical reference

The initial Graphify observations supplied by the user are historical references, not the new acceptance baseline:

| Corpus | Wall time | Peak memory | Nodes | Edges |
| --- | ---: | ---: | ---: | ---: |
| Django | 17.22s | 4.98 GiB | 52,904 | 206,205 |
| Spring | 139.15s | 6.41 GiB | 152,496 | 305,894 |
| Rails | 44.64s | 2.36 GiB | 59,504 | 96,958 |
| Laravel | 8.16s | 1.98 GiB | 41,623 | 86,169 |
| Bevy | 75.76s | 5.64 GiB | 117,061 | 187,221 |
| ASP.NET Core | 487.67s | 8.97 GiB | 306,282 | 468,897 |
| Angular | 49.39s | 7.92 GiB | 187,878 | 231,890 |
| Entire | 5.39s | 1.32 GiB | 22,294 | User supplied a truncated edge count |

The harness creates a new baseline from exact revisions and complete outputs before any optimization.

## Qualification architecture

The framework lives under `benchmarks/performance/` in the Compass repository. It uses Python 3 standard-library modules for orchestration, process measurement, result validation, statistics, and reporting. Rust microbenchmarks may be added only after the end-to-end baseline identifies a specific hot path.

The framework has six bounded responsibilities:

1. Resolve and prepare exact tool and corpus revisions in isolated directories
2. Build the release Compass binary and, in comparison mode, an isolated Graphify environment
3. Execute matched build and query workloads with controlled cache state
4. Validate graph and query correctness before accepting timing samples
5. Store raw samples and aggregate p50, p95, peak resident memory, throughput, counts, and digests
6. Compare Compass with an approved same-runner baseline and, in comparison mode, with Graphify

Compass-only qualification is the default maintainer command. Cross-tool qualification requires an explicit comparison command. It does not add Graphify to Compass production code, runtime dependencies, release artifacts, or mandatory continuous integration. Compass-only harness tests and regression checks may run in continuous integration.

## Workload definitions

Each build workload includes all default deterministic code-graph stages that affect the shipped artifact. Neither tool may disable clustering, analysis, resolution, provenance, framework extraction, or output validation to improve its result. Semantic large-language-model enrichment is excluded because provider latency, network state, and model output are not deterministic.

### Cold build

The harness removes only the selected tool's output and disposable cache directories inside an isolated corpus copy. It then runs the shipped release command to completion and validates every authoritative artifact.

### Unchanged warm build

The harness starts from a validated cold build and runs the same update command without changing source files. Each measured sample begins from an equivalent warm state.

### One-file incremental build

The harness selects one supported source file through a recorded deterministic rule and changes only trailing whitespace that the language parser ignores. This forces content invalidation without changing graph meaning. The incremental graph must equal a clean graph for the changed corpus, and the restored graph must equal the original clean graph.

### Natural-language query

The query suite uses repository-specific questions with required entity and relationship identities. Both tools must satisfy the same semantic oracle before their timings can be compared. Valid Compass additions are allowed, but a required shared fact may not disappear and a known negative may not become an exact relationship.

Each timing sample executes a batch of identical queries so process startup and timer resolution cannot dominate the ratio. The result records per-query latency and a canonical semantic digest.

### CompassQL query

CompassQL covers anchored matches, scans, one-hop traversal, bounded paths, aggregation, optional matching, policy-shaped traversal, plan-cache reuse, cancellation, and resource limits. Every query has an expected canonical result digest or an explicit invariant. CompassQL medians may not regress more than 10% from an approved same-runner baseline without review.

## Sampling and environment control

Expensive build workloads use one untimed warmup where applicable and three measured samples. Query batches use one untimed warmup and at least ten measured batches. The report includes every observation, the median, the nearest-rank p95, minimum, maximum, and median absolute deviation.

The harness records:

- Operating system, architecture, CPU model, physical and logical cores, total memory, and power mode when available
- Rust, Cargo, Python, and package-manager versions
- Tool commit identifiers, dirty state, build flags, binary digest, and command line
- Corpus commit and tree identifiers
- Wall time, user time, system time, exit status, signal, and peak resident memory
- Detected files, indexed files, cache hits, nodes, edges, graph bytes, and stage timings when the tool exposes them
- Canonical graph, query-result, and correctness-report digests

The harness rejects concurrent qualification runs, insufficient disk space, missing required tools, unresolved Git state in a source checkout, mismatched corpus revisions, incomplete artifacts, and nonzero command exits. Interrupted runs retain diagnostic samples but cannot become a baseline.

## Correctness and graph-quality gate

Timing results are ineligible until the corresponding output passes every applicable correctness gate:

- The shipped Code Graph v1 fixture qualification remains green
- Clean, warm, forced, changed, and restored builds produce the required deterministic equivalence
- Every node has a stable identity and valid typed payload
- Every edge has valid endpoints, direction, kind, evidence, source bounds, and provenance
- Unknown or unresolved references remain bounded and do not become global hubs
- Near-match framework fixtures do not produce false exact routes or relationships
- Every Graphify fact in the shared supported input scope exists in Compass with compatible observable fields
- Compass-only facts remain present and pass the Compass producer, identity, endpoint, provenance, and diagnostic contracts
- Query outputs satisfy repository-specific positive and negative semantic assertions
- Cache reuse never changes graph meaning

Aggregate node and edge counts are diagnostic evidence, not a correctness proof. Compass may emit more valid facts than Graphify, but missing shared facts, invalid additions, false exact relationships, or weaker provenance fail qualification.

## Performance gates

A comparable workload passes only when:

```text
graphify_median_seconds / compass_median_seconds >= 5.0
```

The ratio applies separately to every cold build, unchanged warm build, one-file incremental build, and natural-language query workload for every repository. A fast workload is batched until its Graphify median is long enough for stable measurement.

Compass peak resident memory must not exceed Graphify peak resident memory for the matched build. Compass p95 and peak resident memory may not regress more than 10% from the approved Compass baseline on the same runner and corpus without explicit review.

The report lists failures without hiding them behind a geometric mean or suite-wide average. Summary averages may supplement the per-workload table, but they never satisfy the acceptance gate.

## Baseline lifecycle

The first full run creates an immutable raw-result directory and a compact candidate baseline. A candidate becomes approved only when all correctness checks pass and the report identifies the exact runner and revisions.

Approved Compass baselines are runner-specific. A later run compares only compatible machine, toolchain, workload-schema, and corpus identities. A new corpus revision creates a new baseline lineage instead of overwriting the old result.

Raw outputs live under `target/performance/`. Compact approved baseline records and their schema may be checked in under `benchmarks/performance/baselines/` when they contain no repository source or secrets.

## Optimization loop

Optimization begins only after the baseline identifies stage costs and reproducible hot paths. Each change follows this loop:

1. State one performance hypothesis and the evidence supporting it
2. Add or select a correctness test that would catch semantic drift
3. Change one hot path
4. Run focused correctness and microbenchmark checks
5. Run the affected end-to-end corpus workloads
6. Accept the change only when correctness remains green and measured performance improves
7. Revert or revise changes that move cost without improving the required workload

Likely investigation areas include repeated graph indexing, redundant serialization, cache decoding, source rereads, allocation-heavy resolution, global sorting, clustering passes, query index construction, and per-process query startup. These are hypotheses, not preselected fixes.

## Error handling

The harness distinguishes setup failure, tool failure, correctness failure, performance failure, resource exhaustion, and interrupted execution. It writes partial diagnostics atomically and never promotes an incomplete run.

Repository cleanup operates only inside harness-owned directories. The harness validates each destructive target before deleting output or cache content. It never deletes a source checkout, workspace root, home directory, or unresolved path.

## Verification

Framework verification includes deterministic synthetic fixtures, fake tool adapters, malformed result files, interrupted commands, timeout handling, peak-memory measurement, cache-state transitions, statistics, baseline compatibility, report generation, and destructive-target guards.

Release verification includes:

- Harness unit and integration tests
- Compass formatting, linting, workspace tests, and release build
- Code Graph v1 fixture qualification
- The full eight-repository correctness matrix
- The full eight-repository build and query matrix

The user explicitly waived strict test-driven development. Tests and equivalence checks remain mandatory before a result can be accepted.

## Acceptance criteria

The work is complete when:

1. One command can resolve exact revisions, prepare corpora, run selected workloads, resume safely, and emit JSON plus Markdown reports
2. The harness records a fresh Compass baseline for all eight repositories
3. Every timed Compass result has a passing correctness record and canonical output digest
4. Compass is at least 5x faster on every comparable primary workload for every repository
5. Compass peak memory does not exceed Graphify peak memory on matched builds
6. CompassQL correctness passes and its median, p95, and memory remain within approved budgets
7. Code Graph v1 qualification and the full relevant Compass test suite pass
8. The final report discloses every failure, excluded sample, revision, command, and environment detail
9. An explicit comparison run records the latest Graphify baseline and proves every 5x and matched-memory gate
10. Compass production code and release artifacts have no Graphify runtime dependency

## Out of scope

This work does not benchmark semantic provider latency, weaken graph semantics, remove supported facts, reduce provenance, disable default deterministic graph stages, compare different corpus commits, add Graphify to Compass production dependencies, or claim a CompassQL cross-tool ratio without an equivalent Graphify workload.
