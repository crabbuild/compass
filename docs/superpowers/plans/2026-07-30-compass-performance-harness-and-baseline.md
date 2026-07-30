# Compass performance harness and baseline implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a correctness-gated Compass performance harness, capture a fresh Compass baseline on eight real repositories, and support an explicit Graphify comparison run.

**Architecture:** A Python 3 standard-library harness under `benchmarks/performance/` owns revision resolution, isolated workspaces, process measurement, correctness comparison, statistics, and reports. The default mode runs release Compass and compares it with an approved Compass baseline. An explicit comparison mode also prepares Graphify and runs both tools against the same corpus commit.

**Tech Stack:** Python 3.11 or newer standard library, Rust 1.97 release builds, Git, Compass CLI, optional Graphify CLI, JSON, TOML, SQLite

## Global constraints

- Correctness, determinism, graph completeness, graph precision, provenance, and bounded execution remain release requirements.
- The 5x median target applies independently to every comparable build and natural-language query workload.
- Benchmark Django, Spring Framework, Rails, Laravel, Bevy, ASP.NET Core, Angular, and `entireio/cli`.
- Resolve latest default branches at run start and record exact commit and tree identifiers.
- Build Compass in release mode from a clean branch based on the latest `origin/main`.
- Keep the complete framework under the Compass repository.
- Make Compass-only regression qualification the default mode.
- Prepare and run Graphify only for an explicit comparison command.
- Keep Graphify outside Compass production code, runtime dependencies, release artifacts, and mandatory continuous integration.
- Do not disable clustering, analysis, resolution, provenance, framework extraction, or output validation to improve results.
- Exclude nondeterministic semantic large-language-model enrichment from the suite.
- Do not use a strict test-driven development workflow. Correctness and harness tests remain mandatory before acceptance.
- Use one untimed warmup and three measured samples for expensive builds.
- Use one untimed warmup and at least ten measured batches for queries.
- Never delete a source checkout, repository root, workspace root, home directory, or unresolved path.

## Scope boundary

This plan ends after the framework and fresh baseline expose reproducible stage costs and hot paths. Do not preselect production optimizations. After Task 10, use `superpowers:systematic-debugging` or the project `diagnose` skill to trace the measured bottlenecks, then write a second implementation plan containing only evidence-backed production changes.

## File structure

Create these focused files:

- `benchmarks/performance/harness.py`: command-line entry point and command dispatch
- `benchmarks/performance/repositories.toml`: repository URLs, mutation suffixes, and query oracles
- `benchmarks/performance/README.md`: operator workflow and result interpretation
- `benchmarks/performance/compass_perf/__init__.py`: package version and schema constants
- `benchmarks/performance/compass_perf/model.py`: immutable result and configuration dataclasses
- `benchmarks/performance/compass_perf/config.py`: strict TOML parsing
- `benchmarks/performance/compass_perf/stats.py`: p50, p95, median absolute deviation, and ratios
- `benchmarks/performance/compass_perf/workspace.py`: marker, lock, Git mirrors, checkouts, and guarded cleanup
- `benchmarks/performance/compass_perf/process.py`: per-command wall, CPU, exit, signal, and peak-memory measurement
- `benchmarks/performance/compass_perf/adapters.py`: Compass and Graphify build/query command contracts
- `benchmarks/performance/compass_perf/jsonstream.py`: bounded iteration over top-level JSON arrays
- `benchmarks/performance/compass_perf/correctness.py`: graph normalization, SQLite comparison, invariants, and digests
- `benchmarks/performance/compass_perf/workloads.py`: cold, warm, incremental, natural-query, and CompassQL execution
- `benchmarks/performance/compass_perf/report.py`: immutable JSON, Markdown, baseline compatibility, and gates
- `benchmarks/performance/tests/`: standard-library unit and integration tests
- `PERFORMANCE.md`: public qualification commands and policy

### Task 1: Create an isolated implementation branch and preserve the approved documents

**Files:**
- Preserve: `docs/superpowers/specs/2026-07-30-compass-performance-harness-and-hardening-design.md`
- Preserve: `docs/superpowers/plans/2026-07-30-compass-performance-harness-and-baseline.md`

**Interfaces:**
- Consumes: `origin/main` at `f61024eb48826893d97e5745dca1f25f358babb0` or a newer fetched commit
- Produces: clean branch `codex/compass-performance-hardening` in `.worktrees/compass-performance-hardening`

- [ ] **Step 1: Detect the repository and current merge state**

Run from `/Users/haipingfu/graphify/compass`:

```bash
git rev-parse --show-superproject-working-tree
git status --short --branch
git diff --name-only --diff-filter=U
git fetch origin main
```

Expected: the primary checkout remains on `codex/harden-call-graph-resolution`; do not resolve, stage, reset, or remove its six unrelated conflict paths.

- [ ] **Step 2: Verify the project-local worktree directory is ignored**

```bash
git check-ignore -q .worktrees
git worktree list --porcelain
```

Expected: `.worktrees` is ignored and no existing worktree uses the target path or branch.

- [ ] **Step 3: Create the isolated worktree from the fetched main revision**

```bash
git worktree add \
  .worktrees/compass-performance-hardening \
  -b codex/compass-performance-hardening \
  origin/main
```

Expected: the new worktree has no merge state and `git status --short` is empty.

- [ ] **Step 4: Add the approved spec and this plan with `apply_patch`**

Use `apply_patch` in the new worktree to add both approved documents byte-for-byte. Do not copy, stage, or modify any file in the conflicted primary checkout.

- [ ] **Step 5: Commit the approved documents**

```bash
git add docs/superpowers/specs/2026-07-30-compass-performance-harness-and-hardening-design.md
git add docs/superpowers/plans/2026-07-30-compass-performance-harness-and-baseline.md
git commit -m "docs: define Compass performance qualification"
```

Expected: one documentation-only commit on the isolated branch.

### Task 2: Define strict suite, sample, and result models

**Files:**
- Create: `benchmarks/performance/compass_perf/__init__.py`
- Create: `benchmarks/performance/compass_perf/model.py`
- Create: `benchmarks/performance/compass_perf/config.py`
- Create: `benchmarks/performance/compass_perf/stats.py`
- Create: `benchmarks/performance/repositories.toml`
- Create: `benchmarks/performance/tests/test_config.py`
- Create: `benchmarks/performance/tests/test_stats.py`

**Interfaces:**
- Produces: `load_suite(path: Path) -> Suite`
- Produces: `summarize(samples: Sequence[Sample]) -> Aggregate`
- Produces: `speedup(graphify: Aggregate, compass: Aggregate) -> float`
- Produces: JSON-safe dataclasses `RepositorySpec`, `QueryOracle`, `Sample`, `Aggregate`, `WorkloadResult`, and `QualificationRun`

- [ ] **Step 1: Add immutable model types**

Implement these public shapes in `model.py`:

```python
@dataclass(frozen=True)
class QueryOracle:
    question: str
    required: tuple[str, ...]
    forbidden: tuple[str, ...] = ()

@dataclass(frozen=True)
class RepositorySpec:
    name: str
    url: str
    mutation_suffix: str
    queries: tuple[QueryOracle, ...]
```

Add `ToolRevision(name, url, commit, tree, dirty, binary_sha256)`, `EnvironmentIdentity`, `ProcessMetrics`, `Sample`, `Aggregate`, `CorrectnessResult`, `WorkloadResult`, and `QualificationRun`. Use `schema: str` on serialized roots and `Path` only in runtime types, never JSON payloads.

- [ ] **Step 2: Add strict suite parsing**

Implement `load_suite` with `tomllib`. Require top-level schema `compass.performance-suite/1`, exactly eight unique repository names, HTTPS Git URLs, nonempty mutation suffixes beginning with `.`, and at least two query oracles per repository. Reject unknown keys so misspelled gates cannot be ignored.

- [ ] **Step 3: Add the approved repository matrix**

Use these exact URL and mutation suffix pairs:

```toml
schema = "compass.performance-suite/1"

[[repository]]
name = "django"
url = "https://github.com/django/django.git"
mutation_suffix = ".py"

[[repository]]
name = "spring"
url = "https://github.com/spring-projects/spring-framework.git"
mutation_suffix = ".java"
```

Add Rails `.rb`, Laravel `.php`, Bevy `.rs`, ASP.NET Core `.cs`, Angular `.ts`, and Entire `.go` with the approved URLs. Give each repository two domain questions and one required label fragment. Use empty `forbidden` lists initially; Task 7 replaces empty negatives with facts observed and validated during baseline preparation.

- [ ] **Step 4: Implement deterministic statistics**

`summarize` must sort successful sample seconds, reject fewer than three values, compute `statistics.median`, nearest-rank p95 at `ceil(0.95 * n) - 1`, minimum, maximum, and median absolute deviation. `speedup` returns `graphify.p50_seconds / compass.p50_seconds` and rejects nonpositive medians.

- [ ] **Step 5: Add configuration and statistics tests**

Cover duplicate repositories, unknown fields, missing queries, invalid URLs, three-sample p50 and p95, ten-sample query p95, median absolute deviation, failed samples, and division by zero.

Run:

```bash
python3 -m unittest \
  benchmarks.performance.tests.test_config \
  benchmarks.performance.tests.test_stats
```

Expected: all tests pass with no warnings.

- [ ] **Step 6: Commit the model and suite**

```bash
git add benchmarks/performance
git commit -m "test(perf): define qualification suite and result model"
```

### Task 3: Build guarded workspaces and exact revision resolution

**Files:**
- Create: `benchmarks/performance/compass_perf/workspace.py`
- Create: `benchmarks/performance/tests/test_workspace.py`

**Interfaces:**
- Consumes: `RepositorySpec`
- Produces: `QualificationWorkspace.create(path: Path) -> QualificationWorkspace`
- Produces: `QualificationWorkspace.acquire() -> ContextManager[None]`
- Produces: `resolve_remote_head(url: str) -> tuple[str, str]`
- Produces: `prepare_checkout(spec: RepositorySpec, commit: str, destination: Path) -> CheckoutIdentity`
- Produces: `guarded_remove(path: Path) -> None`

- [ ] **Step 1: Implement workspace ownership and destructive-target guards**

Create a marker named `.compass-performance-workspace.json` containing schema `compass.performance-workspace/1` and the resolved workspace path. `guarded_remove` must require the marker, resolve symlinks, require `path.is_relative_to(workspace.root)`, reject the root itself, reject `.git`, and reject any path with fewer than two components below the root.

- [ ] **Step 2: Implement an exclusive run lock**

Acquire `.qualification.lock` with `os.open(..., os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)`. Store PID, hostname, and start time. Remove only the lock created by the current context manager. A present lock fails with an actionable message and is never silently treated as stale.

- [ ] **Step 3: Resolve default branches and exact revisions**

Run `git ls-remote --symref <url> HEAD`, require one `ref: refs/heads/<branch> HEAD` line and one 40-character object identifier, then fetch that commit into a workspace-owned bare mirror. Record `git rev-parse <commit>^{tree}`.

- [ ] **Step 4: Prepare clean detached corpus checkouts**

Clone from the local bare mirror with `--no-checkout`, detach at the recorded commit, run `git status --porcelain=v1 --untracked-files=all`, and reject any output. Never execute cleanup against the original user checkout.

- [ ] **Step 5: Add safety and Git integration tests**

Use temporary repositories to prove marker enforcement, symlink escape rejection, root deletion rejection, lock contention, default-branch parsing, detached checkout identity, and source cleanliness before and after preparation.

Run:

```bash
python3 -m unittest benchmarks.performance.tests.test_workspace
```

Expected: all tests pass and temporary repositories remain intact.

- [ ] **Step 6: Commit workspace isolation**

```bash
git add benchmarks/performance/compass_perf/workspace.py
git add benchmarks/performance/tests/test_workspace.py
git commit -m "test(perf): isolate qualification workspaces"
```

### Task 4: Measure each process without cumulative memory contamination

**Files:**
- Create: `benchmarks/performance/compass_perf/process.py`
- Create: `benchmarks/performance/compass_perf/measure_child.py`
- Create: `benchmarks/performance/tests/helpers/process_fixture.py`
- Create: `benchmarks/performance/tests/test_process.py`

**Interfaces:**
- Produces: `run_measured(spec: ProcessSpec) -> ProcessMetrics`
- `ProcessSpec` contains `command`, `cwd`, `env`, `stdout_path`, `stderr_path`, and `timeout_seconds`

- [ ] **Step 1: Implement one fresh measurement worker per sample**

`run_measured` starts `measure_child.py` as a new Python process. The worker starts the target command, waits for it, reads `resource.getrusage(resource.RUSAGE_CHILDREN)`, normalizes Darwin bytes versus Linux KiB, and writes one JSON object through a dedicated pipe. This prevents peak resident memory from accumulating across samples.

- [ ] **Step 2: Capture complete process evidence**

Record monotonic wall time, user CPU, system CPU, peak RSS KiB, return code, terminating signal, timeout status, command, working directory, stdout path, stderr path, and SHA-256 digests of both streams. Use explicit file handles and close them before returning.

- [ ] **Step 3: Enforce timeout and process-group cleanup**

Start a new session on POSIX. On timeout, send `SIGTERM` to the process group, wait 5s, then send `SIGKILL`. Mark the sample ineligible and retain both streams.

- [ ] **Step 4: Test success, failure, memory, and timeout**

The fixture accepts `--exit`, `--allocate-mib`, `--sleep`, and `--stdout`. Assert distinct samples do not inherit earlier peak RSS, nonzero exits remain recorded, stdout digests are stable, and timed-out grandchildren do not survive.

Run:

```bash
python3 -m unittest benchmarks.performance.tests.test_process
```

Expected: all tests pass on macOS and Linux. Unsupported platforms fail `doctor` instead of reporting zero memory.

- [ ] **Step 5: Commit process measurement**

```bash
git add benchmarks/performance/compass_perf/process.py
git add benchmarks/performance/compass_perf/measure_child.py
git add benchmarks/performance/tests
git commit -m "test(perf): measure isolated process resources"
```

### Task 5: Implement release Compass and optional Graphify adapters

**Files:**
- Create: `benchmarks/performance/compass_perf/adapters.py`
- Create: `benchmarks/performance/tests/test_adapters.py`

**Interfaces:**
- Produces: abstract `ToolAdapter`
- Produces: `CompassAdapter`
- Produces: `GraphifyAdapter`
- Each adapter implements `prepare`, `build_command`, `query_command`, `graph_path`, `parse_build_evidence`, and `revision`

- [ ] **Step 1: Define a feature-equivalent build profile**

Use structural extraction with clustering and graph analysis enabled. Do not pass `--no-cluster`. Do not invoke semantic providers. Run Compass as:

```text
target/release/compass extract <checkout> --code-only --timing --out <output>
```

Run Graphify from its isolated virtual environment as:

```text
<venv-python> -m graphify extract <checkout> --code-only --out <output>
```

If the latest Graphify command rejects `--out`, fail adapter preparation and record the exact help output. Do not silently change the corpus working tree or use a different output contract.

- [ ] **Step 2: Prepare the exact Compass binary**

Require a clean isolated performance branch, run `cargo build --release --locked -p compass-cli --bin compass`, record the commit, tree, dirty state, Rust version, command, and SHA-256 of `target/release/compass`.

- [ ] **Step 3: Prepare latest Graphify only in comparison mode**

Resolve Graphify's `origin/main` or remote default branch at run start, create a workspace-owned detached checkout and virtual environment, then install with:

```text
<venv-python> -m pip install --disable-pip-version-check <graphify-checkout>
```

Record the checkout commit, tree, dirty state, Python version, installed package metadata, and interpreter path.

- [ ] **Step 4: Locate authoritative graph artifacts**

Compass reads `.compass-active-generation`, validates a single `generation-*` component, rejects incomplete generation markers, and returns the generation's `graph.json`. Graphify requires `<output>/graph.json`. Both paths must be regular files below their harness-owned output directory.

- [ ] **Step 5: Add fake-adapter and command-contract tests**

Assert exact flags, output confinement, release-binary requirement, Graphify virtual-environment isolation, active-generation validation, incomplete-output rejection, and tool revision recording.

Run:

```bash
python3 -m unittest benchmarks.performance.tests.test_adapters
```

Expected: all tests pass without building either real tool.

- [ ] **Step 6: Commit tool adapters**

```bash
git add benchmarks/performance/compass_perf/adapters.py
git add benchmarks/performance/tests/test_adapters.py
git commit -m "test(perf): add Compass and Graphify adapters"
```

### Task 6: Stream and normalize large graph outputs

**Files:**
- Create: `benchmarks/performance/compass_perf/jsonstream.py`
- Create: `benchmarks/performance/compass_perf/correctness.py`
- Create: `benchmarks/performance/tests/fixtures/compass_graph.json`
- Create: `benchmarks/performance/tests/fixtures/graphify_graph.json`
- Create: `benchmarks/performance/tests/test_jsonstream.py`
- Create: `benchmarks/performance/tests/test_correctness.py`

**Interfaces:**
- Produces: `iter_top_level_array(path: Path, key: str) -> Iterator[dict[str, object]]`
- Produces: `index_graph(tool: str, graph_path: Path, database: sqlite3.Connection) -> GraphSummary`
- Produces: `compare_graphs(database: sqlite3.Connection) -> CorrectnessResult`
- Produces: `canonical_graph_digest(database: sqlite3.Connection, tool: str) -> str`

- [ ] **Step 1: Implement bounded top-level JSON array iteration**

Use `json.JSONDecoder.raw_decode` with a rolling byte-to-text buffer. Scan the top-level object while respecting strings and escapes, enter only the requested `nodes`, `links`, or `edges` array, yield one object at a time, and discard consumed prefixes. Cap an individual record at 16 MiB and the rolling buffer at 32 MiB.

- [ ] **Step 2: Prove the stream parser across chunk boundaries**

Test one-byte chunks, escaped quotes, braces inside strings, Unicode, empty arrays, `links` versus `edges`, malformed separators, truncated input, oversized records, duplicate top-level keys, and a requested key whose value is not an array.

- [ ] **Step 3: Normalize graphs into SQLite**

Create `nodes(tool,id,label,kind,source_file,source_location,payload_sha256)` and `edges(tool,source,target,relation,payload_sha256)` with primary keys and indexes. Normalize relation case, POSIX source paths, Graphify `type` versus Compass `kind`, and `links` versus `edges`. Preserve IDs and observable payload hashes; do not rewrite identities to manufacture parity.

- [ ] **Step 4: Enforce graph invariants**

Reject duplicate IDs with different payloads, dangling endpoints, missing IDs, missing relationship kinds, invalid source bounds, non-finite confidence, malformed typed details, incomplete Compass publication, and Graphify parse errors. Count diagnostics by severity and fail any Compass validation error.

- [ ] **Step 5: Compare shared facts and Compass additions**

Use SQL `EXCEPT` queries to report Graphify node IDs missing from Compass, shared-node field mismatches, and Graphify `(source,target,relation)` edges missing from Compass. List at most 100 examples per category while retaining total counts. Validate Compass-only facts through endpoint, provenance, identity, and diagnostic rules rather than treating a higher count as proof.

- [ ] **Step 6: Compute canonical digests**

Hash schema-tagged, length-prefixed rows ordered by node ID and edge tuple. Include all normalized graph-meaning fields. Exclude timestamps, absolute output paths, and storage order.

- [ ] **Step 7: Add correctness tests**

Cover exact parity, valid Compass superset, missing shared node, missing edge, field mismatch, dangling endpoint, false duplicate, storage reordering, volatile metadata, and deterministic digest behavior.

Run:

```bash
python3 -m unittest \
  benchmarks.performance.tests.test_jsonstream \
  benchmarks.performance.tests.test_correctness
```

Expected: all tests pass while the parser's maximum buffer remains within its declared bound.

- [ ] **Step 8: Commit graph correctness**

```bash
git add benchmarks/performance/compass_perf/jsonstream.py
git add benchmarks/performance/compass_perf/correctness.py
git add benchmarks/performance/tests
git commit -m "test(perf): gate timings on graph correctness"
```

### Task 7: Execute cold, warm, incremental, and query workloads

**Files:**
- Create: `benchmarks/performance/compass_perf/workloads.py`
- Create: `benchmarks/performance/tests/test_workloads.py`
- Modify: `benchmarks/performance/repositories.toml`

**Interfaces:**
- Consumes: `ToolAdapter`, `QualificationWorkspace`, `RepositorySpec`, and `run_measured`
- Produces: `run_build_matrix(...) -> tuple[WorkloadResult, ...]`
- Produces: `run_query_matrix(...) -> tuple[WorkloadResult, ...]`
- Produces: `select_mutation_file(checkout: Path, suffix: str) -> Path`
- Produces: `validate_query_output(text: str, oracle: QueryOracle) -> CorrectnessResult`

- [ ] **Step 1: Select one deterministic source mutation**

Read `git ls-files -z`, keep regular files with the configured suffix and size from 1 KiB through 256 KiB, exclude paths containing `vendor`, `third_party`, `node_modules`, `generated`, `fixtures`, or `test`, and select the lexicographically first path. Record the relative path and original SHA-256.

- [ ] **Step 2: Implement graph-neutral mutation and restoration**

Append one newline byte, require the file hash to change, and require `git diff --numstat` to report exactly one changed file. Restore with the original bytes held in a bounded temporary file, verify the original SHA-256, and require a clean Git status. Never use `git reset`, `git checkout --`, or a broad cleanup command.

- [ ] **Step 3: Run balanced build samples**

For the default mode, prepare a Compass checkout and capture cold, one warmup, three unchanged-warm samples, and three independently restored incremental samples. In comparison mode, also prepare an independent Graphify checkout at the same commit and alternate tool order across three rounds as `Compass, Graphify`, `Graphify, Compass`, `Compass, Graphify`.

- [ ] **Step 4: Gate every build sample on correctness**

After each build, index and validate the graph. Compare the first valid Compass and Graphify cold graphs. Require Compass clean/warm equivalence, incremental versus clean equivalence for the graph-neutral mutation, restored versus original equivalence, and Graphify shared-fact inclusion before making samples eligible.

- [ ] **Step 5: Implement natural-language query batches**

Use each adapter's public `query` command with the built graph and the exact question. Validate one captured result per oracle, then execute at least ten timed batches. Each batch runs every oracle once in a rotating order and records total time divided by query count.

- [ ] **Step 6: Add repository-specific positive and negative assertions**

During an untimed preparation pass, resolve each required fragment to a stable node identity in both graphs. Add at least one forbidden near-match or unrelated identity per repository. Store exact required and forbidden IDs in the run manifest so later query validation does not depend only on substring matching.

- [ ] **Step 7: Run CompassQL workloads**

Run anchored, scan, one-hop, bounded-path, aggregate, optional, and policy-shaped queries. Capture JSON, canonicalize rows, enforce resource-limit and cancellation outcomes, and compare median latency with compatible Compass baselines only.

- [ ] **Step 8: Add fake-tool workload tests**

Use fake adapters to verify alternating order, cold output isolation, warm reuse, mutation restoration after success and failure, ineligible incorrect samples, query batching, forbidden-result rejection, CompassQL row digests, and resumable per-sample records.

Run:

```bash
python3 -m unittest benchmarks.performance.tests.test_workloads
```

Expected: all tests pass and every temporary source checkout ends clean.

- [ ] **Step 9: Commit workload execution**

```bash
git add benchmarks/performance/compass_perf/workloads.py
git add benchmarks/performance/repositories.toml
git add benchmarks/performance/tests/test_workloads.py
git commit -m "test(perf): execute correctness-gated workloads"
```

### Task 8: Generate immutable reports and enforce honest gates

**Files:**
- Create: `benchmarks/performance/compass_perf/report.py`
- Create: `benchmarks/performance/tests/test_report.py`

**Interfaces:**
- Produces: `write_run(run: QualificationRun, output: Path) -> tuple[Path, Path]`
- Produces: `compare_tools(results: Sequence[WorkloadResult]) -> GateReport`
- Produces: `compare_baseline(run: QualificationRun, baseline: QualificationRun) -> GateReport`
- Produces: `promote_baseline(run_path: Path, destination: Path) -> Path`

- [ ] **Step 1: Write atomic raw and summary results**

Write canonical `run.json` through a same-directory temporary file, `flush`, `os.fsync`, and `os.replace`. Write `summary.md` from the same in-memory result. Include environment, commands, revisions, samples, exclusions, correctness failures, p50, p95, median absolute deviation, RSS, throughput, graph counts, digests, and ratios.

- [ ] **Step 2: Enforce per-workload 5x gates**

In comparison mode, require `graphify_p50 / compass_p50 >= 5.0` for every eligible cold, unchanged-warm, incremental, and natural-query result. Never replace a failed row with an average. Mark missing or correctness-ineligible comparisons as failures, not skips. Default Compass-only runs do not require or start Graphify.

- [ ] **Step 3: Enforce memory and Compass regression gates**

Require Compass matched-build peak RSS at or below Graphify. For compatible Compass baselines, fail p50, p95, or peak RSS regressions above 10%. Compatibility requires exact environment identity, workload schema, tool build profile, corpus commit, and query manifest digest.

- [ ] **Step 4: Promote only complete compatible runs**

`promote_baseline` rejects interrupted runs, missing repositories, correctness failures, performance failures, dirty tool revisions, and mutable output paths. Store only compact samples, identities, gates, and digests under `benchmarks/performance/baselines/<runner-id>/`.

- [ ] **Step 5: Add reporting tests**

Cover a 5.00x pass, 4.99x failure, per-row failure hidden by a passing mean, memory failure, baseline incompatibility, 10% boundary, atomic replacement, interrupted run rejection, and Markdown disclosure of excluded samples.

Run:

```bash
python3 -m unittest benchmarks.performance.tests.test_report
```

Expected: all tests pass.

- [ ] **Step 6: Commit reporting and gates**

```bash
git add benchmarks/performance/compass_perf/report.py
git add benchmarks/performance/tests/test_report.py
git commit -m "test(perf): enforce per-workload qualification gates"
```

### Task 9: Add the operator command, doctor checks, resume, and documentation

**Files:**
- Create: `benchmarks/performance/harness.py`
- Create: `benchmarks/performance/README.md`
- Create: `benchmarks/performance/tests/test_harness.py`
- Modify: `PERFORMANCE.md`

**Interfaces:**
- Produces commands `doctor`, `prepare`, `run`, `compare`, `report`, and `promote`
- `run` supports `--suite`, `--repository`, `--workload`, `--workspace`, `--output`, `--build-repeats`, `--query-batches`, and `--resume`

- [ ] **Step 1: Implement strict command parsing**

Use `argparse` subcommands. Defaults are `build_repeats=3`, `query_batches=10`, suite `repositories.toml`, workspace `target/performance/workspace`, and timestamped output under `target/performance/runs/`. Reject values below the approved minimums for a qualification run.

- [ ] **Step 2: Implement `doctor`**

Check Python 3.11, Git, Rust 1.97, Cargo, available disk space of at least 100 GiB for the full suite, supported peak-RSS measurement, network access to all nine remotes, workspace marker validity, absence of a run lock, and a clean Compass worktree. Emit JSON and return nonzero on any failed requirement.

- [ ] **Step 3: Implement preparation and resume**

`prepare` resolves Compass and corpus revisions without timing them. `compare` additionally resolves Graphify. `run --resume` reads immutable per-sample JSON, verifies command, corpus, tool, and build-profile digests, and reruns only absent or interrupted samples. It never reuses a sample from a different identity.

- [ ] **Step 4: Implement one-command qualification**

This command must perform the approved full workflow:

```bash
python3 benchmarks/performance/harness.py run \
  --suite benchmarks/performance/repositories.toml \
  --workspace target/performance/workspace \
  --output target/performance/runs/latest \
  --build-repeats 3 \
  --query-batches 10
```

The default `run` command exits zero only when setup, correctness, memory, and Compass regression gates pass. The explicit `compare` command also requires every 5x and matched-memory gate.

- [ ] **Step 5: Add command integration tests**

Use fake Git remotes and fake tool adapters to cover `doctor`, selection filters, full orchestration, resume identity, nonzero exits, report regeneration, promotion, lock contention, and help output.

Run:

```bash
python3 -m unittest discover \
  -s benchmarks/performance/tests \
  -p 'test_*.py'
```

Expected: all harness tests pass.

- [ ] **Step 6: Document operation and policy**

Document disk and time expectations, exact commands, latest-revision resolution, raw output layout, correctness-first eligibility, 5x interpretation, memory gates, resume behavior, baseline promotion, and the absence of a Graphify production dependency. Update `PERFORMANCE.md` to link the harness without adding Graphify execution to `scripts/`, `Makefile`, or mandatory CI.

- [ ] **Step 7: Commit the operator surface**

```bash
git add benchmarks/performance/harness.py
git add benchmarks/performance/README.md
git add benchmarks/performance/tests/test_harness.py
git add PERFORMANCE.md
git commit -m "feat(perf): add end-to-end qualification harness"
```

### Task 10: Capture the fresh baseline and produce the optimization brief

**Files:**
- Create: `target/performance/runs/<run-id>/run.json`
- Create: `target/performance/runs/<run-id>/summary.md`
- Create: `docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md`
- Modify only after a fully passing run: `benchmarks/performance/baselines/<runner-id>/baseline.json`

**Interfaces:**
- Consumes: the complete harness from Tasks 2 through 9
- Produces: exact Compass, Graphify, corpus, correctness, time, memory, throughput, and stage evidence
- Produces: ranked evidence for the follow-up optimization plan

- [ ] **Step 1: Run all local framework and product-boundary tests**

```bash
python3 -m unittest discover \
  -s benchmarks/performance/tests \
  -p 'test_*.py'
bash scripts/check_product_boundary.sh
```

Expected: both commands exit zero.

- [ ] **Step 2: Build and qualify release Compass**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo build --release --locked -p compass-cli --bin compass
./scripts/qualify_code_graph_v1.sh --fixtures-only
```

Expected: all commands exit zero. If the clean `origin/main` baseline fails before harness changes, record the exact pre-existing failure and stop for user direction.

- [ ] **Step 3: Run `doctor` and prepare exact revisions**

```bash
python3 benchmarks/performance/harness.py doctor
python3 benchmarks/performance/harness.py prepare \
  --suite benchmarks/performance/repositories.toml
```

Expected: the preparation manifest contains one exact Compass revision and eight exact corpus revisions. The comparison preparation also contains one exact Graphify revision.

- [ ] **Step 4: Run the complete baseline with resumable output**

```bash
python3 benchmarks/performance/harness.py run \
  --suite benchmarks/performance/repositories.toml \
  --workspace target/performance/workspace \
  --output target/performance/runs/baseline \
  --build-repeats 3 \
  --query-batches 10 \
  --resume
```

Expected: the command may report honest correctness or performance failures, but it must complete every runnable row and preserve every raw sample. Do not weaken a gate to make the command exit zero.

- [ ] **Step 5: Audit the baseline report**

Verify all eight repository identities, all workload rows, sample counts, command lines, correctness eligibility, graph counts, digests, p50, p95, RSS, throughput, and exclusions. For an explicit comparison run, also verify ratios and compare the new Graphify observations with the user's historical figures without treating different commits as regressions.

- [ ] **Step 6: Write the evidence-backed optimization brief**

Create `docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md` with:

- Exact machine, tool, and corpus identities
- Every per-workload result and failure
- Build stage shares from Compass `--timing`
- Query startup versus graph-load versus execution costs
- Peak-memory deltas
- The three largest reproducible Compass costs by absolute seconds
- One root-cause investigation entry point for each cost
- Explicit correctness gaps that must be fixed before timing eligibility

Do not propose a production fix unless the evidence identifies the responsible stage and data flow.

- [ ] **Step 7: Promote only a passing Compass baseline**

Run `promote` after the Compass-only run only if every correctness, completeness, Compass regression, and memory gate passes. A comparison result may be attached to the review, but it does not change the promoted Compass baseline identity.

- [ ] **Step 8: Run final verification**

```bash
python3 -m unittest discover \
  -s benchmarks/performance/tests \
  -p 'test_*.py'
bash scripts/check_product_boundary.sh
cargo fmt --all --check
git status --short
```

Expected: tests and checks pass. Status contains only intended harness, documentation, baseline summary, and generated graph metadata selected for commit.

- [ ] **Step 9: Commit baseline evidence**

```bash
git add benchmarks/performance PERFORMANCE.md
git add docs/superpowers/reviews/2026-07-30-compass-performance-baseline.md
git commit -m "perf: establish Compass qualification baseline"
```

Do not add corpus clones, virtual environments, raw repository source, command stdout, command stderr, or `target/performance/` artifacts.

## Completion handoff

After Task 10, inspect the baseline review and create a separate performance-hardening plan. That plan must name the exact hot functions, correctness tests, focused benchmarks, expected mechanism, and end-to-end workloads for each optimization. The final product claim remains blocked until the full matrix proves every 5x and memory gate.
