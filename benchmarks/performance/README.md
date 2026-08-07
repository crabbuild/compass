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

Both tools use the same corpus commits. Build comparisons use the same
structural profile: Compass `--code-only --no-cluster --no-viz --store json`
and Graphify's native `--code-only` profile. Every cold, warm, incremental, and
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
For Rust return types, Graphify's declaration-line projection is dominated by
Compass's exact `returns` occurrence only when source and target identities
match, the context is type-related, the projected site is the callable's own
declaration, and one return fact proves the pair. This bounded compatibility
rule does not relax body references or endpoint conflicts.

## Source-grounded quality audit

TypeScript and JavaScript source recall uses the independent compiler oracle in
`benchmarks/performance/oracles/typescript-source-oracle.mjs`. It is a pinned
developer-side tool, not a Compass runtime dependency. Run `npm ci` before the
first TypeScript/JavaScript audit so the lockfile's TypeScript 5.9.3 package is
available. The provider records exact UTF-8 byte ranges and fails closed when a
source file cannot be parsed or bounded.

Module/import target qualification uses the companion
`benchmarks/performance/oracles/typescript-resolution-oracle.mjs`. It resolves
imports, reexports, dynamic imports, `import type`, `import =`, and literal
`require()` sites under each discovered project configuration. Its output
records the selected module mode, exact source range, target or explicit
external/unresolved/ambiguous outcome, and configuration/source digests. The
resolver fixture suite cross-checks representative decisions against
`tsc --traceResolution`; neither oracle is used by normal Compass builds.

The Rust-side candidate seam has one opt-in compiler differential fixture. Run
it from a checkout with the pinned Node dependencies installed:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 \
  cargo test -p compass-languages --test typescript_oracle_differential \
  --locked -- --ignored
```

The test compares exact source-byte coverage for a mixed TSX fixture. It is
ignored by ordinary workspace tests so native Compass remains Node-free.

For developer-only same-file target adjudication on a pinned real corpus, use
the separate checker oracle and keep the candidate adapter out of production:

```bash
RUST_MIN_STACK=33554432 \
COMPASS_TS_QUALIFICATION_ROOT=/Volumes/Workspace/Github/<owner>/<pinned-corpus> \
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-<checkout> \
  cargo test -p compass-languages --test typescript_target_differential \
  checker_oracle_adjudicates_local_candidate_targets --locked -- --ignored --nocapture
```

The report separates exact local targets, missing targets, wrong targets,
external positives, and unresolved/ambiguous outcomes by capability. It is an
adjudication instrument rather than a release claim: accepted labels, Wilson
intervals, cross-file/project strata, framework tiers, and an equivalent
Graphify/SCIP comparison must be frozen before an adapter can be registered.

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

The export requires the comparison database to contain the exact raw SHA-256
of the supplied Compass graph; databases created before exact artifact identity
was recorded fail closed and must be reindexed. It produces three explicitly
separated populations:

- every eligible Compass-published relationship with an exact in-repository
  byte range (`compass_graph`, suggested `accepted` pool);
- independently parsed Python call and import constructs
  (`independent_source`, suggested `source_oracle` pool); and
- Graphify comparison records (`graphify_comparison`, always a
  `graphify_hypothesis`, even when Compass has a matching fact).

All records pin the corpus commit and exact graph digest and leave `judgment`
and adjudicator `reason` empty. Compass graph candidates already carry the
exact published range and set `requiresExactGraphRange` to false. Independent
source and Graphify candidates are discovery evidence; their owner, target,
relation, and exact Compass representation must be adjudicated before they can
enter a qualification manifest. Generated hypotheses can therefore never
silently become positive precision evidence. Unsupported independent-source
providers report an explicit zero population. Supported providers report every
scanned, parsed, and rejected file and set `sourceOracleCoverage.complete` only
when no file was skipped. Qualification still fails until the required complete
source-oracle strata exist.

After explicit adjudication, run the existing audit gate:

```bash
python3 benchmarks/performance/harness.py audit \
  --manifest /path/to/qualification.json \
  --graph /path/to/pinned/compass/graph.json \
  --corpus /path/to/pinned/corpus
```

The qualification manifest must copy the candidate export's source-oracle
provider, scanned/parsed counts, and inventory SHA-256 into its
`sourceOracles` collection. The audit command independently regenerates that
inventory from the pinned corpus; copying only selected records cannot conceal
an unsupported, skipped, or stale source population.

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
