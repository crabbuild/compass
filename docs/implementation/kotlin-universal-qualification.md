---
meta:
  contentType: Reference
  title: Kotlin universal qualification
  navLabel: Kotlin Qualification
  category: Implementation
  overview: Reproducible baseline and qualifying evidence for the Kotlin hard cut.
  goal: Record what the Kotlin pipeline has proved and which audit gates remain open.
  audience:
    - Compass language contributors
    - release reviewers
  openQuestions: []
---

# Kotlin universal qualification

This record compares the established Kotlin publisher from Compass commit
`2db60035` with the version-1 universal pipeline. It does not promote Kotlin
to `Qualified`.

## Pinned corpus

- Repository: `spring-projects/spring-framework`
- Commit: `da4b31c82b567a0531c6980b5172cba1fc7e6ed5`
- Inventory: 390 `.kt` and `.kts` files; Compass discovers 388 under its normal
  scope policy
- Relative-path/content inventory SHA-256:
  `402a15c4318573cfe87f3e8b0d023c216d8a43295ecadf06f2281550f83453be`

The source checkout is a read-only qualification input. Cold and warm graphs
were built with `--no-cluster --no-viz --inference-level max` from debug
binaries on the same machine.

## Graph comparison

| Relation family | Established | Universal v1 |
| --- | ---: | ---: |
| `calls` | 2,174 | 2,838 |
| `contains` | 2,935 | 5,847 |
| `extends` | 97 | 61 |
| `implements` | 118 | 98 |
| `imports` | 0 | 2,170 |
| `instantiates` | 943 | 1,306 |
| `references` | 1,648 | 4,067 |
| `registers` | 0 | 215 |
| `routes_to` | 11 | 28 |

The universal graph contains 10,649 nodes and 16,632 relationships with graph
SHA-256
`471b99daef7fd69386482637c120ed2ddab667a2a3c56e0a496a388ef409add4`.
Cold and cache-reused publication are byte-identical. Publication reports zero
omitted nodes, zero omitted relationships, and zero identity collisions. Three
source files exercise Tree-sitter recovery and remain explicitly partial;
4,170 external symbols remain unresolved rather than being rebound by terminal
name.

## Performance comparison

| Workload | Established wall / peak RSS | Universal v1 wall / peak RSS |
| --- | ---: | ---: |
| Cold | 12.82 s / 158.3 MB | 28.52 s / 248.1 MB |
| Warm | 0.46 s / 27.5 MB | 0.70 s / 27.8 MB |
| One-file whitespace change | 12.24 s / 144.4 MB | 30.13 s / 229.4 MB |

The candidate expands relation coverage but currently regresses cold, warm,
and incremental latency. These measurements are evidence for optimization
work, not a performance claim.

An incremental-publication follow-up on the same 388-file Spring Kotlin corpus
reduced a one-file trailing-comment restoration to 1.01 seconds with the
optimized binary. The run extracted one file, reused 387, spent zero reported
time in graph assembly, wrote zero new immutable objects, and reused 55 store
objects. The earlier 30.13-second observation exposed two defects now covered
by regression tests: `update` did not enter the fact-neutral path, and Kotlin
file/package envelope anchors made harmless EOF edits look semantic. Peak RSS
was not recaptured in the follow-up, so the original memory evidence remains
the only recorded comparison.

## Completion status

Fixture conformance covers exact UTF-8 anchors, deterministic ordering,
malformed syntax, traversal limits, named/default arguments, extensions,
object and companion projection, and Java-terminal collision rejection. The
Spring Kotlin route fixture qualifies the universal framework pack.

The independent quality audit is still required. In particular, no claim is
made that the minimum 2,000 accepted-relationship pool, precision/recall
thresholds, or zero-tolerance critical judgments have passed. Kotlin must
remain `Qualifying` until that audit and the remaining performance work
complete.
