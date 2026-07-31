# Compass performance baseline and hardening results

Date: 2026-07-30

## Scope

The qualification harness lives in `benchmarks/performance/` and defaults to
Compass-only execution. Graphify comparison is explicit. A separate three-run
Graphify cold-build confirmation was performed earlier in the work. Final
Compass verification reused the pinned Graphify 0.9.31 artifacts and did not
rerun Graphify.

The checked-in suite pins Django, Spring Framework, Rails, Laravel, Bevy,
ASP.NET Core, Angular, and Entire to exact remote commits at run time. This
session qualified Entire and Django. It did not promote a suite-wide baseline
because all eight repositories were not measured on this runner.

## Runner and identities

- Runner: `1c73241d0f9762c2`
- Hardware: Apple M2 Max, 12 logical cores, 32 GiB RAM
- OS: Darwin 25.5.0 arm64
- Rust: 1.97.1
- Final measured Compass commit:
  `35d13d4faff9fd8cf14191155edf80ebf4b2bdcb`
- Final Compass release SHA-256:
  `abfdd5a76e7f58c91b9368ee154db2288126f17b7e128b5fc53833ea6f6372ca`
- Retained Graphify baseline version: 0.9.31
- Retained Graphify baseline commit:
  `4fe11092ccbe9f543608f140c790f68d5d83cae4`
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

## Semantic-dominance phase

The implementation-first semantic-dominance phase was verified against the
same pinned Django and Entire revisions. The production binary at
`0bdee95e266c3ab97ba85a879b64e2c2fc403f18` has SHA-256
`22fa9ff03e5f0b78f55a0e2de76610b083ee520f84b8cfc32d629bb66c0ac994`.
The fresh comparison used Graphify 0.9.31 at
`4fe11092ccbe9f543608f140c790f68d5d83cae4`; both tools published graphs
with zero validation errors and stable canonical digests.

The comparator now separates exact facts, uniquely dominated legacy facts,
ambiguous facts, and genuinely missing facts. Dominance requires compatible
language/module identity, a unique anchored definition, exact relationship
occurrence evidence, or a unique same-file ownership path of at most two
hops. It does not accept label equality alone.

| Repository | Graphify fact | Exact | Dominated | Ambiguous | Missing | Exact + dominated |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Django | Nodes | 50,466 | 6 | 25 | 348 | 99.27% |
| Django | Edges | 149,993 | 336 | 26 | 8,355 | 94.72% |
| Entire | Nodes | 19,866 | 663 | 6 | 50 | 99.73% |
| Entire | Edges | 53,593 | 6,104 | 18 | 1,347 | 97.76% |

This is a substantial improvement over the literal baseline (Django: 1,175
missing nodes and 10,600 missing edges; Entire: 720 missing nodes and 7,470
missing edges), but it is not a strict Graphify superset. The remaining
failures are intentionally fail-closed. Django is now dominated by config and
vendored-resource relationships plus unresolved generated/inherited bases.
Entire is dominated by shell entry points, Go embedding occurrences, and a
small number of same-package receiver ambiguities.

The phase also found and fixed two real generated-Go identity defects:
method-before-type traversal could retain a method-site anchor instead of the
later declaration, and grouped generic function types lacked an explicit
`type_alias` kind. On the real Entire corpus this published
`ErrorModelStatusCode` at line 2348 and `optionFunc` at line 17, reduced
ambiguous nodes from 20 to 6, reduced missing nodes from 52 to 50, and reduced
missing or ambiguous edges by 77.

### Fresh build measurements

The complete three-sample comparison run is stored at
`target/performance/runs/semantic-dominance-20a9e96/`. It exposed and led to a
fix for a harness bug that incorrectly rejected independently valid Graphify
samples because a sample-local Compass graph was absent. The raw process
measurements and canonical digests remain valid; the old eligibility labels in
that report do not.

| Tool | Repository | Workload | p50 | p95 | Peak RSS |
| --- | --- | --- | ---: | ---: | ---: |
| Compass | Django | Cold, final 3 samples | 12.355 s | 16.587 s | 4.45 GiB |
| Compass | Django | Warm, final 5 samples after warmup | 2.182 s | 2.429 s | 674.58 MiB |
| Graphify 0.9.31 | Django | Cold, 3 samples | 49.985 s | 50.916 s | 1.26 GiB |
| Compass | Entire | Cold, 3-sample matrix | 4.055 s | 4.127 s | 1.42 GiB |
| Compass | Entire | Warm, 3-sample matrix | 0.586 s | 0.599 s | 247.25 MiB |
| Graphify 0.9.31 | Entire | Cold, 3 samples | 17.650 s | 18.214 s | 226.09 MiB |

The final Django cold median remains faster than the earlier qualified Compass
baseline and is 5.05x faster than the pinned Graphify 0.9.30 median. Graphify
0.9.31 itself became materially faster, so the current-head ratios are 4.05x
for Django and 4.35x for Entire rather than 5x. Django warm is 11.96% above the
earlier 1.949-second baseline on this final five-sample check, narrowly outside
the phase's 10% gate. These are recorded as open performance gaps rather than
being hidden by the quality improvements.

### Fresh query checks

These are single fresh-process semantic checks, not qualified latency medians.
Compass passed all four repository oracles. Graphify passed three; its Django
URL-resolution query selected template/i18n/storage results and did not return
the required `URLResolver`.

| Repository / query | Compass | Graphify 0.9.31 |
| --- | ---: | ---: |
| Django URL resolution | 2.783 s, pass | 2.414 s, fail oracle |
| Django model save | 3.303 s, pass | 4.232 s, pass |
| Entire checkpoint creation | 0.655 s, pass | 1.199 s, pass |
| Entire repository state | 1.614 s, pass | 1.923 s, pass |

### Phase-two source-backed qualification

Phase two was implemented before its regression tests, following the execution
rule in the phase plan. The final release binary was rebuilt from the delivery
tree before evidence collection. The production binary is unchanged between
`034fb1e` and the final comparator-only commit `35d13d4`. Fresh output roots
were used for both repositories, and every eligible build produced the same
canonical correctness digest:

- Django graph SHA-256:
  `a55af88fb1a88a58f247270bcc961227f7db89d5fe9dc531d9e38ab272be89ff`
- Entire graph SHA-256:
  `e985a75ac4d59972b39392cfa70efe5596df6587c39664d0f06e436f66d7bec7`

Both final graphs indexed with zero validation errors. The strict comparison
against the same Graphify 0.9.31 artifacts produced:

| Repository | Graphify fact | Exact | Dominated | Rejected | Ambiguous | Missing |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Django | Nodes | 50,457 | 9 | 0 | 33 | 346 |
| Django | Edges | 100,077 | 1,198 | 53,045 | 63 | 4,327 |
| Entire | Nodes | 19,695 | 842 | 0 | 47 | 1 |
| Entire | Edges | 54,309 | 2,993 | 2,889 | 129 | 742 |

Rejected facts are audited baseline conflicts, not Compass matches. After
removing those conflicts from the source-grounded denominator, Compass has
95.84% accepted Django edge coverage and 98.50% accepted Entire edge coverage;
node exact-or-dominated coverage is 99.25% and 99.77%, respectively. Compass
publishes 55,120 nodes and 130,847 distinct edges for Django, and 21,711 nodes
and 72,250 edges for Entire. That is 4,275 more nodes than Graphify on Django,
1,126 more on Entire, and 11,188 more total edges on Entire, while both Compass
graphs retain zero validation errors.

The raw Django edge result is intentionally not presented as a literal recall
win.
The implementation removed the all-imports × all-classes resolver rule. In the
pinned corpus, 53,005 Graphify `references` edges project an import occurrence
onto symbols without a symbol-use occurrence; these moved to the comparator's
explicit `rejected` category. Rejection requires the reference target and
occurrence to coincide with a real import fact, so it does not turn arbitrary
missing facts into passes. In exchange, 3,102
previously missing decorator references became exact at their real decorator
lines. The phase also moved 46 inherited-type edges from dominated to exact,
added 32 exact calls, and restored two shell containment edges. A unique
same-target relationship at the same exact occurrence may now dominate a
Graphify relationship whose owner is only a broader placeholder; this accounts
for the increase to 1,198 dominated Django edges.

Entire gained first-class `embeds` and shell entrypoint facts and reduced its
genuine missing edge set to 742. Another 2,889 Graphify edges were rejected
only where a qualified external Compass target proves Graphify rebound the
same occurrence to an unrelated local same-name type. Examples include
`context.Context` rebound to the repository's `contexts.Context` and
`io.Writer` rebound to a project-local `Writer`. Exact matches take precedence
over this rejection, and qualified-label compatibility plus the exact
relationship occurrence are both required. The remaining misses contain
genuine unresolved call/reference targets and remain visible rather than being
accepted through label-only matching.

The final five-sample release measurements used the same `compass extract
--code-only --timing --out` contract as the harness:

| Repository | Compass cold p50 | Cold p95 | Warm p50 | Incremental p50 | Peak cold RSS | Graphify 0.9.31 cold p50 | Cold speedup |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Django | 9.3868 s | 10.7207 s | 1.6324 s | 15.6107 s | 4,209.44 MiB | 49.985 s | 5.33x |
| Entire | 3.5001 s | 4.7039 s | 0.5391 s | 5.9863 s | 1,492.73 MiB | 17.650 s | 5.04x |

All five cold samples for each repository were eligible and produced identical
correctness digests. Both retained-Graphify cold comparisons clear 5x, although
the Entire margin is narrow and is not described as comfortable. No Graphify
warm, incremental, or query ratio is claimed because final verification did
not rerun Graphify. Publication now uses indexed inventory/file-coverage and
hash lookups, a trivial-evidence fast path, a single proven-clean validation
path, and deterministic parallel preparation for v1 edges, files, and
independent publication metadata.

All four semantic query oracles passed across ten measured fresh processes per
query:

| Repository / query | p50 | p95 | Result |
| --- | ---: | ---: | --- |
| Django URL resolution | 1.4484 s | 1.4638 s | 10/10 pass |
| Django model save | 1.4598 s | 1.5053 s | 10/10 pass |
| Entire checkpoint creation | 0.7148 s | 0.7494 s | 10/10 pass |
| Entire repository state | 0.7060 s | 0.7847 s | 10/10 pass |

Strict literal Graphify-superset quality is therefore still not achieved.
Phase two does improve source-backed quality over the retained Graphify
baseline: it preserves qualified identity, real occurrences, decorators,
embeddings, and shell entrypoints while explicitly rejecting two demonstrated
false-positive families. The comparator remains fail-closed and exposes 346
Django and one Entire node misses plus 4,327 Django and 742 Entire edge misses.
Those residuals, rather than restoration of audited false positives, are the
next graph-quality target.

### Universal semantic-evidence hard cutover

The universal semantic-evidence increment hard-cuts Python and Go extraction
and resolution to one typed, bounded evidence contract. It does not translate
or shadow the removed language-specific algorithms. Other languages continue
their existing implementations until separate atomic cutovers. Framework
packs now have a universal registration contract, but no existing framework
detector is maintained through a compatibility projection.

No extraction, cache, producer, graph, adapter, framework, or package version
was changed. `graphify update .` was not run.

The final release binary SHA-256 is
`8d692043471f4bdfdffa314a914bffcd55dff5a3f0ceebb29a73c5b15873c3e0`.
Measurements use the same pinned revisions:

- Django `50d706d0aebcc2d073c8d034b6e22fc98fad49f2`;
- Entire `279b988597f1037c14cdd4c46765a5552e067d17`; and
- retained Graphify 0.9.31 at
  `4fe11092ccbe9f543608f140c790f68d5d83cae4`.

#### What materially improved

- Python imports, relative modules, re-exports, bases, decorators, annotations,
  calls, and imported members now use exact AST ranges and typed bindings.
  Dynamic bases and unbound qualified receivers fail closed.
- Go package imports, receivers, typed parameters, embeddings, calls, and type
  references preserve package and declared-type identity. Uppercase Go calls
  are no longer treated as Python-style constructors.
- Exact external bindings remain external rather than being rebound to an
  unrelated local terminal-name match.
- Repeated external type uses share one exact import-bound target while every
  use retains its own relationship occurrence. The real Django build found
  and fixed this identity defect: the pre-fix graph quarantined 339 colliding
  nodes and 408 incident edges; the final graph has no publication collision
  or omission diagnostic.
- Candidate lookup is indexed and bounded instead of scanning every import
  against every declaration. Unchanged generation seals are verified in
  parallel after the manifest trust boundary.
- Natural query ties prefer callable/type semantics, production source, and
  graph connectivity. The Django model-save query now starts from `.save()`
  and the production `django/db/models/base.py::Model`.

#### Final graph size and determinism

| Repository | Compass nodes | Compass raw edges | Compass canonical edges | Graphify nodes | Graphify edges | Cold/warm graph SHA-256 |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Django | 63,796 | 157,560 | 147,223 | 50,842 | 158,704 | `3e2e913f38b2059b5b23ab6e7bbc54ace1c73b8afef540fad47540f2c8781816` |
| Entire | 58,391 | 152,151 | 151,257 | 20,585 | 61,062 | `96d51d3e917182915404482c59ecf87d4c229f8c9cd452256413d66a911318ba` |

Django has 12,954 more nodes than Graphify but 1,144 fewer raw edges. That
edge reduction is only 0.72%, and it is not itself called an improvement.
Entire has 37,806 more nodes and 91,089 more raw edges. These totals show that
the hard cutover is not an indiscriminate edge-removal strategy.

Both repositories produced byte-identical cold and warm `graph.json`.
Django has no publication omission or identity-collision diagnostic. Entire
has one fail-closed ambiguous normalized endpoint diagnostic and no partial
publication summary.

#### Source-grounded Graphify comparison

The comparator accepts an exact fact or a unique source-grounded dominance
mapping, explicitly rejects a demonstrated Graphify conflict, and otherwise
leaves the fact ambiguous or missing. Rejected facts are not counted as
Compass coverage.

| Repository | Graphify fact | Exact | Dominated | Rejected | Ambiguous | Missing |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Django | Nodes | 50,189 | 131 | 0 | 61 | 461 |
| Django | Edges | 51,530 | 44,065 | 55,437 | 257 | 7,415 |
| Entire | Nodes | 18,184 | 898 | 0 | 233 | 1,270 |
| Entire | Edges | 27,489 | 24,238 | 6,127 | 397 | 2,811 |

After excluding only explicit rejected conflicts, exact-or-dominated edge
coverage is 92.57% for Django and 94.16% for Entire. Node coverage is 98.97%
and 92.70%, respectively. Strict Graphify-superset quality is therefore not
achieved, and the universal cutover has lower measured Graphify recall than
phase two.

The major Django rejection is 53,006 Graphify module-import relationships
projected onto symbols without a later symbol-use occurrence. Graphify also
rebounds `import inspect` in `django/apps/config.py` to the local
`django/utils/inspect.py`. Compass retains the exact external `inspect`
binding. In `django/contrib/admin/apps.py`, Graphify treats
`check_dependencies` and `check_admin_app` arguments to `checks.register(...)`
as calls; Compass records the actual call to `register` and does not fabricate
calls to the arguments.

Entire rejects 6,127 relationships only when a qualified external target at
the same occurrence proves a local same-name rebound, such as
`context.Context` to a project-local `Context`. Its 1,270 node misses are
mainly Graphify sourceless/generated placeholders that do not yet map to a
unique source-backed Compass declaration. Its 2,811 edge misses include real
unresolved owner and call/reference relationships. Those are regressions or
open recall gaps, not improvements.

#### Performance and query checks

These are final single cold runs plus a stable second warm run, not replacement
p50/p95 qualification samples.

| Repository | Cold | Stable warm | Peak cold RSS | Retained Graphify cold p50 | Observed cold ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| Django | 15.14 s | 1.66 s | 4.57 GiB | 49.985 s | 3.30x |
| Entire | 7.20 s | 0.90 s | 2.63 GiB | 17.650 s | 2.45x |

Django stable warm is within 2% of the prior 1.6324-second baseline. Entire
warm is 67% above its prior 0.5391-second baseline. Cold is 61% above the prior
Django median and 106% above the prior Entire median. The 10% cold/warm preservation
gate and the previous 5x Graphify cold ratio are not met. Higher-recall graph
materialization and publication size remain material performance costs.

All four final fresh-process query oracles passed:

| Repository / query | Wall time | Semantic seed |
| --- | ---: | --- |
| Django URL resolution | 1.883 s | `URLResolver`, `.resolve()` |
| Django model save | 1.745 s | `.save()`, production `Model` |
| Entire checkpoint creation | 1.630 s | `checkpointCreatedAt()`, `Checkpoint` |
| Entire repository state | 1.440 s | `.Repository()`, `Repository`, `State` |

#### Qualification boundary

The deterministic audit harness and its negative conformance fixtures pass
their unit contracts. The checked-in conformance manifest intentionally
contains one example of each forbidden critical violation and correctly exits
nonzero; it is ineligible for a production claim.

No independently labeled 2,000-record Python/Go qualification manifest was
produced in this increment. Consequently, the requested observed precision of
at least 99%, its Wilson lower bound, capability recall, and zero production
critical-violation claim are **not proven**. The implementation is
source-grounded and substantially more conservative than Graphify in the
demonstrated false-positive families, but “flawless,” “99% precision,” and
“better than Graphify overall” would overstate the evidence.

The next quality increment should prioritize local closure/receiver inference,
generated declaration identity, and unresolved call/reference owners, then run
the real stratified audit. The next performance increment should reduce
high-recall graph materialization and loading cost without reintroducing
terminal-label or all-pairs resolution.

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
- Python imports retain qualified type provenance across re-export chains.
- Safely qualified external Python member calls retain exact occurrences and
  source-scoped deferred endpoints.
- Empty generic source files publish deterministic inventory nodes.
- JavaScript `prototype` and `.fn` assignments publish bounded owned methods.
- Generated Go declarations are indexed before receiver methods and retain
  explicit canonical type kinds.
- Graph publication bounds anchor parsing, preserves canonical order, and
  transfers owned facts without redundant cloning.

## Remaining bottlenecks

The final profile reduced v1 edge normalization from approximately 0.252
seconds to 0.099 seconds and the full v1 boundary to approximately
0.63-0.67 seconds on the measured production graph. Fresh-process queries are
still dominated by loading and indexing the JSON graph. Graph loading,
incremental publication, and the residual genuine semantic misses are the next
optimization targets; none can be bypassed at the cost of validation or
source-backed identity.

## Reports

- `target/performance/runs/django-build-b86dc22/summary.md`
- `target/performance/runs/django-build-b86dc22/run.json`
- `target/performance/runs/django-optimized-fixed/summary.md`
- `target/performance/runs/django-optimized-fixed/run.json`
- `target/performance/runs/entire-baseline-fixed/summary.md`
- `target/performance/runs/entire-baseline-fixed/run.json`
- `target/performance/runs/django-final-35d13d4/summary.md`
- `target/performance/runs/django-final-35d13d4/run.json`
- `target/performance/runs/entire-final-034fb1e/summary.md`
- `target/performance/runs/entire-final-034fb1e/run.json`
- `target/performance/runs/query-final-35d13d4/summary.md`
- `target/performance/runs/query-final-35d13d4/run.json`
