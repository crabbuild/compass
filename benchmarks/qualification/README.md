# Compass qualification corpora

This directory is the versioned, deterministic comparison surface for C-013.
The checked-in files are inputs, schemas, digests, tests, and summarized raw
measurements. Generated graphs, indexes, binaries, and raw logs are disposable
and belong under `target/` or an explicit temporary directory.

`manifest-v1.json` is the closed checksum inventory for the retained benchmark
sources and evidence. The test suite verifies every listed byte digest and
rejects missing or unlisted retained benchmark artifacts.
`raw-traversal-oracle-v1.json` retains the timing-free output of the exact medium
30-task run; tests continuously apply every versioned expectation to that output.
`surreal-dual-engine-decision-v1.json` retains the C-015 medium dual-engine raw
samples, source and graph identities, peak memory, threshold decisions, and the
pre-ratified `REJECT` falsifier outcome. The candidate achieved zero semantic
mismatches but failed the query-regression and native-value gates; omitted
post-falsifier measurements are explicit and must not be interpreted as passes.

## Validate the corpora

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s benchmarks/qualification/tests -v
```

## Generate scale profiles

Plan-only validation is cheap and does not iterate every record:

```bash
python3 benchmarks/qualification/generate_graph.py \
  --profile qualification-large --plan-only
```

Metadata-only validation iterates every logical record and recomputes the
digests in `scale-profile-digests-v1.json` without writing a graph:

```bash
python3 benchmarks/qualification/generate_graph.py \
  --profile qualification-medium --metadata-only
python3 benchmarks/qualification/generate_graph.py \
  --profile qualification-large --metadata-only
```

Materialize the exact medium graph only in a disposable location:

```bash
python3 benchmarks/qualification/generate_graph.py \
  --profile qualification-medium \
  --output target/qualification-c013/medium/graph.json \
  --metadata-output target/qualification-c013/medium/generation.json
```

## Reproduce the current-engine baseline

```bash
cargo build -p compass-cli --release --locked
python3 benchmarks/qualification/run_current_baseline.py \
  --binary target/release/compass \
  --graph target/qualification-c013/medium/graph.json \
  --output target/qualification-c013/current-engine-baseline-v1.json \
  --work-dir target/qualification-c013/run-fixed \
  --samples 5 \
  --timeout-seconds 120
```

The runner measures the bounded 30-task raw denominator itself and emits the
complete retained schema, including source provenance and repository-relative
paths. It computes the source patch SHA-256 from a fixed default path set and
hashes the existing tracked files plus non-ignored untracked files in that
scope; ignored ambient files are excluded. Use repeatable
`--workspace-patch-path` arguments when a later comparison has a different
implementation scope. The legacy `compass path` command has no depth
flag, so its workload uses
fixture endpoints whose deterministic shortest path is exactly three hops; the
finite graph profile and process timeout remain explicit independent bounds.

Reproduction always writes to `target/`. Promoting later evidence is a separate
dated update: review the fresh samples, add a new versioned retained JSON artifact
(for example `current-engine-baseline-v2.json`), preserve every prior baseline,
extend `manifest-v1.json`, and rerun the complete qualification test suite. A
routine reproduction run must not mutate the pinned manifest closure.

The retained baseline is host-specific evidence, not a universal performance
claim. A future engine comparison must run the same generator, commands, sample
count, warmup condition, and p95 method on the same runner. `gzip9Bytes` is tied
to the recorded Python and zlib runtime as well as the input binary; comparisons
must not treat it as runtime-independent.

The current-engine runner intentionally accepts only `qualification-medium`, the
profile ratified for current-engine footprint and query baselines. It fails before
measurement if another pinned or custom profile is supplied.

## Run the bounded raw denominator

```bash
python3 benchmarks/qualification/raw_traversal.py \
  --graph target/qualification-c013/medium/graph.json \
  --tasks benchmarks/qualification/agent-tasks-v1.json \
  --output target/qualification-c013/raw-traversal-v1.json \
  --max-graph-bytes 268435456 --max-nodes 100000 --max-edges 250000 \
  --max-depth 32 --max-results 10000 --timeout-seconds 120
```

Exit status 2 is a limit or input failure; it must never be interpreted as an
empty result. Every task must provide an `expected` evidence object; the runner
applies each contract and fails instead of publishing absent or mismatched oracle
evidence. All operations emit an explicit `complete` or `empty` status. When a
task document declares limits, they must exactly match the effective CLI limits;
the task input is itself capped at 1 MiB and 100 tasks. JSON output files are
published atomically. The portable retained command label `python3` is normalized;
`rawTraversal.commandInterpreter.measuredExecutableName` records the executable
name actually used and `host.python` records its measured version.
