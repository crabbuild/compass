# Compass performance qualification

This harness measures release Compass against an approved Compass baseline.
Correctness is evaluated before a sample becomes timing-eligible. Graphify is
never installed or run by the default workflow; it is available only through
the explicit `compare` command.

## Requirements

- Python 3.11 or newer, Git, Cargo, and the repository's Rust 1.97 toolchain
- macOS or Linux for child-process peak-RSS measurement
- A clean Compass source checkout
- About 100 GiB free for the complete eight-repository suite

Run the preflight and prepare exact latest corpus revisions:

```bash
python3 benchmarks/performance/harness.py doctor
python3 benchmarks/performance/harness.py prepare
```

For a constrained machine, select one or more repositories. A selected-repository
doctor check requires 5 GiB free:

```bash
python3 benchmarks/performance/harness.py doctor --repository entire
python3 benchmarks/performance/harness.py prepare --repository entire
```

## Compass baseline

The default command builds release Compass and runs cold, unchanged-warm,
graph-neutral incremental, natural-language query, and CompassQL workloads:

```bash
python3 benchmarks/performance/harness.py run \
  --workspace target/performance/workspace \
  --output target/performance/runs/baseline \
  --build-repeats 3 \
  --query-batches 10
```

Use `--repository NAME` repeatedly to select repositories and `--workload`
to select `build`, `query`, or `compassql`. Query selections still perform the
build prerequisite. Raw graphs and process logs remain under the owned
workspace; `run.json` and `summary.md` are written under the output directory.
Every process is fresh, expensive build workloads have three samples, query
workloads have one untimed warmup and ten measured samples, and reports retain
excluded observations.

Promotion is allowed only for a complete, clean, passing eight-repository run:

```bash
python3 benchmarks/performance/harness.py promote \
  target/performance/runs/baseline/run.json
```

Approved baselines are runner-, environment-, suite-, and corpus-specific.
Compass p50, p95, and peak RSS may not regress more than 10%.

## Explicit Graphify comparison

Only this command resolves and installs Graphify:

```bash
python3 benchmarks/performance/harness.py compare \
  --output target/performance/runs/comparison
```

Both tools use the same corpus commits. Every cold, warm, incremental, and
natural-language query row must independently reach
`graphify p50 / compass p50 >= 5.00`; averages cannot hide a failed row.
Compass build peak RSS must not exceed Graphify, and Graphify's shared graph
facts must remain present and compatible in Compass. CompassQL is excluded from
the cross-tool ratio because Graphify has no equivalent workload.

The comparison environment is isolated under `target/performance/` and is not a
Compass runtime or development dependency.

## Analyze external comparison runs

`analyze.py` profiles Compass and Graphify graphs that were built outside the
qualification harness. It expects this workspace layout:

```text
run/
├── logs/{compass,graphify}-CORPUS.log
└── outputs/
    ├── compass/CORPUS/compass-out/
    └── graphify/CORPUS/graphify-out/
```

Describe the matching source checkouts in a JSON manifest. Relative `source`
paths are resolved from the manifest directory, so manifests remain portable
when the corpus bundle is moved:

```json
{
  "corpora": [
    {
      "name": "django",
      "language": "Python",
      "framework": "Django",
      "source": "sources/django"
    }
  ]
}
```

Run the analyzer with the exact binaries used to build the graphs:

```bash
python3 benchmarks/performance/analyze.py \
  --workspace /path/to/run \
  --corpora /path/to/corpora.json \
  --compass-binary /path/to/compass \
  --compass-source /path/to/compass-checkout \
  --graphify-binary /path/to/graphify
```

The command writes `metrics/results.json`, one comparator SQLite database per
corpus, and `REPORT.md` under the run workspace. This is diagnostic evidence,
not a promotable Compass performance baseline; use `harness.py compare` for
qualification.

The comparator uses each manifest's pinned `source` checkout to prove
statement-level occurrence equivalence. For example, the first line of a
multiline Python import and the exact imported-item line are equivalent only
when both fall inside the same parsed import statement. Missing, unreadable,
escaped, unsupported, or malformed source fails closed to exact-line matching.

## Source-grounded quality audit

Create an unjudged candidate set from a comparator database without rerunning
either graph producer:

```bash
python3 benchmarks/performance/harness.py audit-candidates \
  --database target/performance/manual/django-comparison.sqlite \
  --graph /path/to/pinned/compass/graph.json \
  --corpus /path/to/pinned/django \
  --name django \
  --adapter python \
  --output target/performance/audits/django-candidates.json
```

The export pins the corpus commit and graph digest, bounds every candidate to
repository source, records the comparator classification, and leaves
`judgment` and adjudicator `reason` empty. Candidate line ranges are discovery
evidence, not accepted graph ranges: `requiresExactGraphRange` must be resolved
to the graph's exact occurrence before a record enters a qualification
manifest. This prevents generated hypotheses from silently becoming positive
precision evidence.

After explicit adjudication, run the existing audit gate:

```bash
python3 benchmarks/performance/harness.py audit \
  --manifest /path/to/qualification.json \
  --graph /path/to/pinned/compass/graph.json \
  --corpus /path/to/pinned/corpus
```

The checked-in `audits/universal-core.json` is only a small conformance fixture.
Production qualification still requires the fixed sample, precision, Wilson
lower-bound, capability, corpus, relation, diversity, and recall thresholds
enforced by `compass/audit.py`.

## Output and interruption policy

`run.json` contains exact tool, environment, corpus, command, timing, memory,
digest, eligibility, and gate evidence. `summary.md` is generated from the same
in-memory result. Interrupted or failed runs are diagnostic evidence only and
cannot be promoted. Never delete a performance workspace manually to recover
from a live lock; first verify that its recorded process is no longer running.
