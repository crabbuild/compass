# Best-Effort Code Graph Heavy-Framework Qualification

Date: 2026-07-30

## Decision

The best-effort publication path is available and queryable on all eight
heavyweight repositories in this qualification. Every completed build emitted
the closed `compass.graph/1` vocabulary, passed the strict reader used by the
query commands, and disclosed quarantined records through
`publication_omission_summary` and `incomplete_coverage`.

One production-scale defect was found and fixed during qualification. Angular
spent more than 12 minutes in generic stub rewiring because the resolver scanned
the complete edge collection inside each stub/candidate comparison. Commit
`0af6326` replaces the repeated scan with a precomputed stub-to-source-file
index. The same full Angular checkout then completed in 49.39 seconds. The
affected resolver stage completed in 6.22 seconds, while preserving file-scoped
import resolution behavior under a regression test.

This result qualifies availability and typed-artifact integrity. It does not
claim complete semantic coverage: Spring's quarantine rate is material and
requires producer improvement, even though the graph remains productive and
truthfully reports the missing coverage.

## Method

The runs used the release binary, the full pinned repository checkout, forced
structural extraction, Program IR analysis, disabled clustering, and disabled
HTML generation:

```text
compass update <repository> --out <output> --no-cluster --no-viz --force --timing
```

`/usr/bin/time -l` measured wall time and peak resident memory. A symbol search
with a dedicated query cache then exercised the normal strict graph reader and
`compass.query/1` response contract. No source subsets, fixture reductions, or
generated miniature applications were used.

Host:

- Apple M2 Max, 12 logical CPUs
- 32 GiB memory
- macOS 26.5.2

The source checkouts and generated evidence are retained under:

```text
<qualification-corpus-root>/compass-heavy-framework-perf-20260729/repos
<qualification-corpus-root>/compass-heavy-framework-best-effort-20260730
```

## Build Results

| Repository | Pinned commit | Tracked files | Wall time | Peak RSS | Graph size | Published nodes | Published edges |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Django | `274a1d494d11` | 7,074 | 17.22 s | 4.98 GiB | 239.4 MB | 52,904 | 206,205 |
| Spring Framework | `317eae88d074` | 11,384 | 139.15 s | 6.41 GiB | 547.6 MB | 152,496 | 305,894 |
| Rails | `0f36bbf72cc8` | 4,973 | 44.64 s | 2.36 GiB | 147.0 MB | 59,504 | 96,958 |
| Laravel | `7e5b3aff7dcc` | 3,333 | 8.16 s | 1.98 GiB | 129.0 MB | 41,623 | 86,169 |
| Bevy | `25368b78ce5e` | 2,969 | 75.76 s | 5.64 GiB | 285.3 MB | 117,061 | 187,221 |
| ASP.NET Core | `2e05e269f599` | 17,281 | 487.67 s | 8.97 GiB | 795.5 MB | 306,282 | 468,897 |
| Angular | `1a2bcb2295c8` | 10,670 | 49.39 s | 7.92 GiB | 362.2 MB | 187,878 | 231,890 |
| Entire | `683a10d3773e` | 2,657 | 5.39 s | 1.32 GiB | 88.6 MB | 22,294 | 72,199 |

ASP.NET Core is the largest published artifact and slowest build in this
corpus. It also verified the production default graph-size cap: the 795.5 MB
artifact opens without a `COMPASS_MAX_GRAPH_BYTES` override after the default
was raised to 1 GiB.

## Query Results

Every graph completed a real symbol search and returned `compass.query/1`.
Every partial graph included an `incomplete_coverage` diagnostic. The observed
search timings include graph loading, strict validation, index preparation or
reuse, execution, and response construction.

| Repository | Observed search wall time | Peak RSS | Result |
| --- | ---: | ---: | --- |
| Django | 24.03 s | 0.65 GiB | Passed |
| Spring Framework | 30.97 s | 4.23 GiB | Passed |
| Rails | 9.78 s | 1.26 GiB | Passed |
| Laravel | 9.20 s | 1.11 GiB | Passed |
| Bevy | 23.47 s | 2.50 GiB | Passed |
| ASP.NET Core | 58.19 s | 6.63 GiB | Passed with the production default size cap |
| Angular | 25.67 s | 3.26 GiB | Passed |
| Entire | 6.01 s | 0.78 GiB | Passed |

These measurements show that the query path is usable but still expensive for
the largest artifacts. They are qualification observations, not a declared
latency service-level objective.

## Framework Topology

Framework-specific entities use the same closed graph contract as ordinary
symbols. Route declarations become `route` nodes and resolved handlers are
connected with explicit `routes_to` edges. Framework detectors contribute
facts; the central resolver performs bounded target resolution; the v1
normalizer validates both endpoints before publication.

| Repository | `route` nodes | `routes_to` edges |
| --- | ---: | ---: |
| Django | 578 | 352 |
| Spring Framework | 645 | 300 |
| Rails | 124 | 45 |
| Laravel | 27 | 34 |
| Bevy | 0 | 0 |
| ASP.NET Core | 691 | 623 |
| Angular | 7 | 14 |

Zero routes in a framework source tree is not itself an extraction failure:
Bevy is not a web-routing framework, and this qualification does not infer
routes where the repository contains no supported route declaration shape.
The Angular checkout is the framework implementation rather than a large
Angular application, so it is primarily a TypeScript-scale and resolver test.

## Quarantine and Coverage

Rates below use `omitted / (published + omitted)`. Identity collisions are
reported separately.

| Repository | Omitted nodes | Node rate | Omitted edges | Edge rate | Identity collisions |
| --- | ---: | ---: | ---: | ---: | ---: |
| Django | 316 | 0.59% | 348 | 0.17% | 0 |
| Spring Framework | 32,064 | 17.37% | 87,078 | 22.16% | 0 |
| Rails | 1 | <0.01% | 1,457 | 1.48% | 0 |
| Laravel | 1,191 | 2.78% | 1,443 | 1.65% | 0 |
| Bevy | 1,064 | 0.90% | 356 | 0.19% | 10 |
| ASP.NET Core | 1,828 | 0.59% | 5,048 | 1.07% | 6 |
| Angular | 2,580 | 1.35% | 8,233 | 3.43% | 0 |
| Entire | 59 | 0.26% | 182 | 0.25% | 0 |

Sampled diagnostics show four recurring producer-quality categories:

- unresolved placeholders without an exact node kind or wiring site;
- incident edges removed after their endpoint was quarantined;
- invalid inheritance endpoints, especially Rails class-to-module relations;
- stable same-identity conflicts in Bevy modules and ASP.NET method/namespace
  symbols.

Spring is the only corpus member where quarantine removes roughly one fifth of
the candidate topology. The artifact is strict-valid and queryable, but agents
must treat its `incomplete_coverage` diagnostic as material. The next
producer-quality priority is to infer or resolve Spring placeholders before
publication rather than broadening the validator.

## Angular Performance Defect

Before the fix, all early stages completed quickly:

- combined tree-sitter extraction: 3.45 seconds;
- declaration merge: 0.14 seconds;
- Program syntax cache publication: 3.94 seconds;
- family stub rewiring: 0.18 seconds.

The next stage, unique stub rewiring, had not completed after more than 12
minutes. A 10-second process sample showed the main thread continuously sorting
and comparing while worker threads were idle. Inspection identified this
effective shape:

```text
for each stub
  for each same-name candidate
    scan every edge to recover incident source files
```

The resolver already performs one complete edge pass to collect stub families,
scopes, consumers, and imports. The fix records incident source files during
that pass and performs indexed file/import lookups in the candidate loop. The
resolution conditions and endpoint rewrite provenance remain unchanged.

After the fix:

- unique stub rewiring: 6.22 seconds;
- complete cross-file resolution: 11.03 seconds;
- Compass-reported update time: 43.77 seconds;
- externally measured wall time: 49.39 seconds;
- published topology: 187,878 nodes and 231,890 edges;
- strict query: passed with 500 returned symbols and explicit incomplete
  coverage.

## Release Assessment

The branch now meets the intended best-effort contract on real heavyweight
repositories:

- invalid records are quarantined rather than weakening v1 invariants;
- a valid non-empty generation is atomically published;
- omitted counts and bounded examples remain visible to CLI and query clients;
- graphs larger than 512 MiB remain readable under the production default;
- all eight full repositories build and answer a typed search;
- the Angular superlinear resolver defect is covered and removed.

Remaining production work is quality and efficiency hardening rather than an
artifact-integrity failure:

1. Reduce Spring placeholder and incident-edge quarantine.
2. Reduce peak build memory for ASP.NET Core and Angular.
3. Avoid loading and validating the complete graph for each fresh query.
4. Add this pinned corpus, or an equivalently sized internal mirror, to
   scheduled release qualification with wall-time, memory, validity, and
   omission-rate regression thresholds.
