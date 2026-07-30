# Compass performance baseline and hardening results

Date: 2026-07-30

## Scope

The qualification harness lives in `benchmarks/performance/` and defaults to
Compass-only execution. Graphify comparison is explicit. A separate three-run
Graphify cold-build confirmation was performed after the final Compass
verification.

The checked-in suite pins Django, Spring Framework, Rails, Laravel, Bevy,
ASP.NET Core, Angular, and Entire to exact remote commits at run time. This
session qualified Entire and Django. It did not promote a suite-wide baseline
because all eight repositories were not measured on this runner.

## Runner and identities

- Runner: `1c73241d0f9762c2`
- Hardware: Apple M2 Max, 12 logical cores, 32 GiB RAM
- OS: Darwin 25.5.0 arm64
- Rust: 1.97.1
- Final Compass commit: `b86dc228037b6410e006078ec85e76bf44ef7c9d`
- Final Compass release SHA-256:
  `9c27ea72451d1bde002753b744a18698f5ff5e9ed742977df23d96ce6e9bcb9c`
- Graphify version: 0.9.30
- Graphify commit: `ecfcd160d56b420eb8241430fa7b5b1951c7829f`
- Django commit: `50d706d0aebcc2d073c8d034b6e22fc98fad49f2`
- Entire commit: `279b988597f1037c14cdd4c46765a5552e067d17`

## Qualified results

### Django build hardening

All values are fresh-process measurements. Build workloads use three eligible
samples and report median wall time, nearest-rank p95, and maximum peak RSS.

| Workload | Before p50 | Final p50 | Change | Final p95 | Final peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Cold | 20.240 s | 12.402 s | -38.7% | 12.415 s | 4.50 GiB |
| Warm | 1.963 s | 1.949 s | -0.7% | 1.977 s | 568.55 MiB |
| Incremental | 23.815 s | 23.361 s | -1.9% | 24.129 s | 5.56 GiB |

Every eligible Django sample produced the same restored canonical digest,
`f14002cacff585fef853870bf75ed50b1cf9fb11e316ffc4458284be7873a81b`.
Cold and warm samples also produced byte-identical graph JSON:
`febd3bb205ae91b13aefa389012cbed9ef50ef017c425e3eab32309c4f41b400`.

The graph contains 52,904 nodes and 206,205 emitted links. The correctness
index contains 193,542 unique semantic edges after duplicate normalization and
reports zero validation errors.

### Django query baseline

The query qualification used ten eligible fresh-process samples per workload.
The routing and model-save natural-language queries passed their semantic
oracles.

| Workload | p50 | p95 | Peak RSS |
| --- | ---: | ---: | ---: |
| Natural query: routing | 1.867 s | 1.892 s | 1.54 GiB |
| Natural query: model save | 1.898 s | 1.939 s | 1.54 GiB |
| CompassQL scan | 1.797 s | 1.825 s | 1.54 GiB |
| CompassQL anchored | 1.742 s | 1.796 s | 1.54 GiB |
| CompassQL one-hop | 2.185 s | 2.500 s | 1.96 GiB |
| CompassQL bounded path | 1.894 s | 1.922 s | 1.56 GiB |
| CompassQL aggregate | 1.760 s | 1.792 s | 1.54 GiB |
| CompassQL optional | 1.839 s | 1.865 s | 1.57 GiB |
| CompassQL policy-shaped | 1.807 s | 1.826 s | 1.54 GiB |

### Entire baseline

The corrected pre-hardening Entire run passed all correctness gates:

| Workload | p50 | p95 | Peak RSS |
| --- | ---: | ---: | ---: |
| Cold | 6.964 s | 7.424 s | 1.68 GiB |
| Warm | 8.142 s | 8.474 s | 1.90 GiB |
| Incremental | 8.075 s | 8.671 s | 1.95 GiB |

The subsequent unchanged-extract fast path targets the anomalous warm result.
It is covered by byte-identity and sealed-generation tests, but Entire was not
rerun after that change in this session.

## Graphify reference

The supplied 17.22-second historical value was not reproducible as a cold
Graphify build. Graphify 0.9.30 was run from an isolated worktree at the exact
remote-default-branch commit above. Each sample used a fresh process and empty
output root; Graphify reported all 3,096 code files as uncached.

| Sample | Wall time | Peak RSS | Nodes | Edges |
| --- | ---: | ---: | ---: | ---: |
| 1 | 65.669 s | 1.26 GiB | 50,842 | 158,704 |
| 2 | 62.346 s | 1.25 GiB | 50,842 | 158,704 |
| 3 | 59.644 s | 1.25 GiB | 50,842 | 158,704 |
| p50 / p95 / max | 62.346 s / 65.669 s | 1.26 GiB | 50,842 | 158,704 |

Against the qualified Compass cold result, Compass is 5.03x faster at p50 and
5.29x faster at p95. This meets the 5x median target narrowly, not comfortably.
The tradeoff remains material: Compass peak RSS is 3.58x Graphify's.

The schema-aware comparison found zero validation errors and no mismatches
among aligned shared nodes, but Compass is not yet a strict semantic superset:
it retained 49,667 of 50,842 Graphify node facts (97.69%) and 148,104 of
158,704 Graphify edge facts (93.32%). Compass emits a larger graph overall
(52,904 nodes and 206,205 links), but size alone is not a quality claim.

Graphify's node and edge counts were stable across the three samples, while
the raw graph SHA-256 and community count differed in every sample. There is no
qualified Graphify query baseline in this report, so no query speedup ratio is
claimed here.

## Changes validated

- Exact path-aware AST cache keys prevent byte-identical symlinks from losing
  logical graph identity.
- Empty semantic layers now retain the complete AST manifest.
- Verified unchanged extracts reuse the sealed published generation.
- Cold builds skip a publication preflight when no prior artifact can possibly
  be reused.
- Publication consumes untrusted node facts instead of cloning every node.
- Query tokenization splits identifiers such as `URLResolver` and normalizes
  resolver/resolution terms.
- Correctness indexing compares cross-schema semantic facts and streams large
  graph metadata without weakening per-record limits.

## Remaining bottlenecks

The final Django cold profile spends approximately 5.7 seconds in the required
v1 publication normalization, 4.4 seconds in extraction/resolution, and 2.5-2.7
seconds in Program analysis. Fresh-process queries are dominated by loading and
indexing the 242 MiB JSON graph. These are the next optimization targets; none
can be bypassed at the cost of graph validation or semantic quality.

## Reports

- `target/performance/runs/django-build-b86dc22/summary.md`
- `target/performance/runs/django-build-b86dc22/run.json`
- `target/performance/runs/django-optimized-fixed/summary.md`
- `target/performance/runs/django-optimized-fixed/run.json`
- `target/performance/runs/entire-baseline-fixed/summary.md`
- `target/performance/runs/entire-baseline-fixed/run.json`
