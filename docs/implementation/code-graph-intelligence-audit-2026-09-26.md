# Code graph intelligence audit: 2026-09-26

## Status and acceptance criteria

Broad superiority over Graphify is **unproven**. The objective covers hub
analysis, code graph correctness, queries, explanations, and navigation/path
finding. A focused text-recall score cannot establish all of those properties.

| Requirement | Evidence needed | Current evidence |
| --- | --- | --- |
| Reliable hub analysis | Declaration-aware candidates, stable rankings, source-reviewed false positives and negatives | Five hub defects fixed; no reviewed god-object corpus yet |
| Accurate code graph | Reviewed declaration and relationship precision/recall, direction, occurrences, unresolved/ambiguous cases | Anchor scorer repaired; relationship accuracy not measured by that scorer |
| Better query answers | Held-out equivalent questions, independent source judgments, precision and recall | Existing five-repository suites are development samples with text oracles |
| Better explanations | Correct target, source provenance, callers/callees and explicit uncertainty | Prior query changes exist; fresh paired evidence still needed |
| Better navigation and walks | Valid ordered edges, direction, hop bounds, alternatives, ambiguity and negative cases | Existing path tests/suites are useful but do not prove real-repository path precision |
| Fair efficiency comparison | Same successful questions, repeated timings, token methodology and complete environment provenance | Paired token aggregation exists; bytes/4 remains an estimate |

“God mode” is interpreted here as the existing `god_nodes` hub analysis.
It orders connected candidates by degree; it does not measure responsibility,
cohesion, or whether a high-degree declaration needs refactoring.

## Reproduced production defects

`crates/compass-graph/src/analyze.rs` previously discarded names such as
`Path` and `Counter` even when they were project declarations with source
locations. It also used graph input order to break degree ties, and returned
isolated declarations when enough results were requested.

The fix retains those source-located declarations, breaks ties by stable node
ID, and omits degree-zero candidates. Two regression tests failed before the
fix; all ten tests in `analyze_coverage` passed afterward. An MCP regression
checks the rendered ordering and top-N behavior. Public fields and degree
semantics are unchanged; candidate lists can change.

A fourth defect treated every `.method()` label as a file even when the node
had canonical kind `method`, and allowed explicitly typed files with descriptive
labels into the hub list. The regression returned `[caller, file]` where the
source-located candidates were `[method, caller, function]`. Recognized canonical
kinds now take precedence; the label heuristic remains for legacy unknown kinds.
The updated graph-analysis integration suite passes all eleven tests.

A fifth defect excluded typed functions whose source filename has no extension.
An MCP diagnostic using the release binary omitted `prepare()` in `bin/launch`,
but included the identical node when its source was `bin/launch.sh`. A native
regression reproduced the failure. Canonical structural kinds now take
precedence over the concept filename heuristic when the source path is nonempty.
Nodes without a source remain excluded, and unknown legacy kinds retain the
existing fallback. The graph-analysis integration suite passes all twelve tests.

Remaining limitation: legacy file/concept/JSON-noise eligibility still uses heuristics.
Degree combines relationship kinds and counts directed endpoint pairs, not
responsibilities or call-site occurrences. A popular infrastructure type may
be a legitimate hub. A separate design diagnosis requires reviewed evidence
and an explicit metric contract before it can be claimed.

## Reproduced navigation defects

Two adversarial native regressions exposed problems in the typed `node` search:

1. Keeping only the cheapest arrival at each node loses feasible paths under a
   hop limit. For `s -> a -> b -> t` (cheap calls) and `s -> b` (costlier
   reference), a two-hop request incorrectly returned no path. The search now
   retains nondominated cost/depth states and reconstructs the exact state path.
2. A node rejected by the node budget was inserted into the admitted set before
   the budget check. A second visit could admit it without paying, leaking that
   rejected node into a truncated response. Admission now happens only after
   successful budget consumption.

Both tests failed before the changes and passed afterward. The full traversal
integration suite passes 11/11, including JSON and SQLite checks, record-order
permutations, two/three-hop expectations and a cycle under a five-edge work
budget. Positive-cost cycles are dominated rather than repeatedly expanded.
All search labels and predecessor records remain bounded by examined edges.
The CLI contract regression also passed, within the 34-test CLI query suite.

A third navigation defect was then reproduced in the separate undirected
`path` implementation: it retained only one arrival per node and made the same
incorrect no-path claim under a two-hop bound. The release CLI and a native
regression both reproduced it. That implementation now also keeps cost/depth
states and reconstructs their exact predecessors. Its two ranking passes each
cap adjacency work at 1,000,000 entries and cumulative path keys at 16 MiB;
exhaustion is a command error, not a no-path claim.

An independent oracle enumerates all simple paths through 729 four-node graphs,
where each of six pairs is absent, a cost-1 call, or a cost-4 reference. Both
engines are checked at depths one through three (4,374 queries). The directed
engine sees DAGs; the undirected engine also sees cycles. The oracle compares
reachability, minimum cost and hop count, and validates rendered/native edge
chains and directions. It reproduced the legacy defect at graph 113, depth 2.
This is exhaustive coverage of that small family, not a general graph proof or
independent real-source extraction evaluation.

## Evaluation corrections

The v1 graph-anchor scorer ignored the requested symbol. For Compass, any
node span covering a reviewed line could earn credit, including a whole
module. For Graphify, any name at that line could earn credit. Fourteen
negative subcases reproduced false positives in the old scorer.

Run schema v2 records `exact-file-start-terminal-symbol/1`: exact file,
exact declaration start, and case-sensitive terminal name on both tools.
Qualification/signature text is stripped symmetrically; this does not verify
owner identity or parameter types. Missing anchors are listed for review.
Source-located ratios are metadata counts, not verified source correctness.

The runner also previously reused graph files by directory existence and
reported the current executable identity. A changed tool could be credited
with an old graph. Runs now build fresh artifacts beneath their own run
directory and refuse existing run IDs/artifact directories. They check clean
pinned Git state before extraction and after querying, retain graph digests
and build logs, and compare executable identities before/after the run.
Executable-file hashes do not cover Python imports or the full environment.

A response containing the requested strings could pass even if its process
failed or timed out. Such executions now fail independently of text matching.
Capture now enforces a 16 MiB per-stream disk cap during execution; limit
failures cannot pass. An invalid snapshot pointer cannot silently select an
unpublished graph. Unpaired token medians no longer produce a cost-winner claim.
The existing text oracle remains a recall proxy: mentioning both endpoints
does not prove a valid path, and mentioning a caller does not prove its edge.

The historical report is annotated and its unsupported graph-quality conclusion
withdrawn. Its original raw artifact directories are unavailable on this host,
so the historical graph scores have not been recalculated. Do not substitute
newer tool outputs for that missing historical evidence.

## Available comparison inputs

Cobra, Flask, Gson and Zod have clean working checkouts at the suite commits.
The old `doctor` accepted Axum's bare Git repository merely because HEAD matched;
that directory has no source tree to extract. The runner and doctor now reject
bare repositories. A separate Axum working checkout was created and verified at the suite commit.
Existing checkouts remain read-only.
The installed Compass reports 0.3.29 and cannot represent this working branch.
Installed Graphify reports 0.9.67; the old report used 0.9.36. The separately
available Graphify source checkout is at `26b02b5e3430e4ab85dd7e72c7b98836d8e65c48`
(version 0.9.63), so its implementation is not assumed identical to 0.9.67.

## Verification ledger

- Graph analysis integration suite: 12/12 passed after all five hub fixes.
- Benchmark Python unit suite: 38/38 passed, including ten path-auditor tests.
- CLI query contract suite: 35/35 passed; product suite: 9/9 passed.
- After the separate legacy `path` fix: query library 178/178, exhaustive oracle
  1/1 (4,374 queries), traversal integration 11/11, legacy query coverage 6/6,
  CLI query suite 35/35 and product suite 9/9 passed. This includes an actual
  work-limit CLI failure with empty stdout. These graph/path regressions and
  the competitor-free Python scorer tests are now explicitly wired into CI.
- Query relevance qualification: 5/5 passed, including the 500 synthetic cases.
- Workspace Clippy (`--workspace --lib --bins --locked -- -D warnings`): passed
  after all eight production corrections. The new query integration tests also
  pass a dedicated Clippy run with warnings denied.
- Rust formatting check: passed.
- Product boundary script: passed; competitor tooling stays outside production.
- Workspace native tests (`--workspace --lib --bins --locked`): 1,083 passed,
  zero failed, two ignored after all eight production corrections, including
  the three new MCP regressions.
- Code-graph fixture qualification: initial native stages passed; the React
  oracle then failed because locked TypeScript dependencies were absent.
  After `npm ci --ignore-scripts`, the complete final gate passed (exit 0),
  including deterministic production updates, semantic/topology assertions,
  Markdown quality, and independent React source-anchor checks at the six-defect
  checkpoint. The complete gate also passed at the seven-defect checkpoint
  after the extensionless-source fix. The built release binary still reproduces
  the separate legacy `path` defect; that run is not evidence for the eighth fix.
- First v2 replay: complete but invalidated for comparative scoring (see below).
- Corrected v2 replay `v2-corrected-02`: complete, after all three evaluation
  corrections. It uses the debug binary and recorded source patch from before
  the explicit-kind hub and typed-trail fixes, with Graphify 0.9.67.
- Release replay `v2-release-03`: complete at source commit `8a13928e`, including
  the six original production fixes, before the extensionless-source correction.
  Scores and paired token estimates match the corrected debug replay. Its
  source-grounded path audit passes 5/5 per tool. All 92 previously recorded
  Graphify source/data file hashes remain unchanged after the run; this does not
  pin every transitive dependency.
- Explicit fixed-graph regression replay `query-only-04`: the current debug
  query binary passes all 50 text oracles on the retained `v2-release-03`
  Compass graphs and all five source-grounded path witnesses. Source state and
  graph hashes were verified before/after querying. This checks query
  compatibility; it is not a new extraction or comparative performance run.

## Findings from the first fresh replay

The raw evidence is retained under the `code-graph-audit-20260926` evaluation
workspace, run `v2-fresh-01`, with a separate invalidation record. Before scoring
it as a comparison, three issues required correction (now implemented):

- Graphify 0.9.67 deliberately exits 1 when `explain` returns an ambiguity list.
  Its installed `cli.py` and the captured Cobra, Flask and Gson responses confirm
  this. The new blanket nonzero-exit rejection wrongly penalizes a correct
  pick-list outcome. The scorer now permits this explicit contract only when the
  candidate oracle passes; actual command failures and timeouts still fail.
- Gson's `JsonWriter.value(String)` annotation starts at line 526; its method
  header is line 527. Both tools correctly locate the declaration at 526.
  The reviewed anchor now uses the annotation start. These were not extraction
  failures; the matching policy remains exact.
- Axum's suite uses paths relative to the `axum/` package, while the separate
  checkout is the monorepo root. The corrected replay uses that package as the
  source root, and preflight now rejects roots missing reviewed files.

The source patch and Graphify distribution file hashes were retained alongside
this run. Timings from the debug Compass executable are not release-performance
evidence. Preliminary score totals must not be presented as accuracy results.

## Corrected focused comparison

The corrected replay uses the exact source roots and reviewed declaration starts
and accepts Graphify's documented ambiguity exit status. Both executable
identities remained unchanged and all five source roots remained clean/pinned.
Its suite and runner copies, tool hashes, graphs, logs and raw responses are
retained under `runs/v2-corrected-02` in the evaluation workspace.

| Measured item | Compass | Graphify 0.9.67 |
| --- | ---: | ---: |
| Text-oracle passes, all rows | 50/50 | 44/50 |
| Non-excerpt rows | 45/45 | 44/45 |
| Source-excerpt rows | 5/5 | 0/5 |
| Exact reviewed declaration anchors | 15/15 | 14/15 |
| Median estimated tokens on 44 shared passes | 308 | 111.5 |

The largest difference is a source-excerpt feature gap: Graphify's `explain`
returns metadata rather than the requested declaration text. Those five rows
are separated above instead of treating them as five independent relationship
accuracy wins. Among the other 45 rows, the difference is one Axum file-path
lookup. Both path inputs resolve to the same unrelated test module. A follow-up
using the full `src/routing/...` paths produced the same failure; the raw retry
is retained as `axum-exact-path.stdout`/`.stderr`. This is evidence of that file
lookup failure, not proof that Graphify cannot traverse an explicit-ID path.
Its missing Zod anchor is the implementation at `classic/schemas.ts:303`; its
graph retains the interface declaration at line 72 instead.

Graphify has the lower paired token estimate. These are bytes/4 estimates, not
measured model tokens. No speed comparison is claimed: Compass used a debug
binary, other native checks ran concurrently, and timings were single samples.
The later release checkpoint reproduced every score and paired token median
in the table. Its single-run paired median latency was 259.5 ms for Compass and
216 ms for Graphify; this also does not establish a repeatable speed advantage.
The release executable and build log were retained with a source-commit record
and SHA-256, and both tool identities were unchanged across the replay.
All 50 questions are an existing development suite. The six extra passes do
not establish held-out precision, execution-path accuracy or overall superiority.
The subsequent native trail defects demonstrate gaps this text suite misses.
The separate 500-query relevance qualification runs AI-reviewed synthetic
phrasing-equivalence cases over the shared fixture graph, as its test and
corpus notes explicitly state. It is useful regression coverage, not 500
independent real-repository judgments or production telemetry.

## Source-grounded path review

The five positive path responses from `v2-corrected-02` were audited separately
against the printed ordered hops, unique graph node identities, semantic edge
direction, relation, declaration anchors, and reviewed source occurrences.
Both tools pass all five navigation witnesses in `path-audit-02.json`.
This is post-output development review, not held-out precision or recall.

The first diagnostic incorrectly required Gson's construction line 801 even
when allowing the coarse `references` relation. Graphify's actual edge is a
valid return-type reference at line 797. The corrected witness accepts that
site for `references` and line 801 for `instantiates`; Compass returns the
latter. The report preserves this specificity difference. The earlier
`path-audit-01.json` remains as an oracle-error diagnostic, not a competitor
failure. Zod's route uses reverse/forward file containment in both tools;
it supports navigation but proves no runtime call chain.

The checked-in auditor rejects missing/reversed edges, wrong occurrences,
wrong declarations, ambiguous labels, invalid identities, endpoint-only text,
and inconsistent hop counts. It checks recorded graph/suite digests and pinned
source state, bounds inputs, and records hashes for its own code, witnesses,
and captured responses. It currently audits successful positive path rows;
unreachable, ambiguous, truncated and limit outcomes still need a broader
source-reviewed corpus.

## Synthetic boundary diagnostics

Separate shared-graph cases were recorded before executing either tool, with
raw outputs and exact arguments retained under `path-boundary-diagnostic` in
the evaluation workspace. These isolate query behavior from extraction quality.

| Case | Compass release checkpoint | Graphify 0.9.67 |
| --- | --- | --- |
| Two disconnected components | Explicit no path | Explicit no path |
| Absent endpoint | Nonzero no-match error | Nonzero no-match error |
| Two `Worker` nodes in different components | Lists both IDs and refuses a path | Warns on stderr, selects one, returns its path |
| Exact ID in the disconnected component | Explicit no path | Explicit no path |
| Reverse traversal of a stored call | Preserves reverse arrow | Preserves reverse arrow |
| Both endpoints resolve to the same node | Explicit refusal | Explicit refusal |

The ambiguity row records a policy difference: Graphify does disclose the
ambiguity, but still picks an endpoint. It must not be described as hiding the
warning. The two hop-limit cases are Compass-only contract checks because
Graphify exposes no documented hop-bound option. A one-hop request correctly
returns a bounded no-path result; the two-hop request reproduced the third
navigation defect above. These are not extra head-to-head accuracy wins.

## Next evidence to collect

1. Keep exact build/source provenance for subsequent release comparisons;
   the latest query correction has native and fixed-graph regression evidence.
2. Expand hub review beyond candidate eligibility to source-reviewed design
   judgments, separating connectivity from responsibility/cohesion defects.
3. Add independent edge/path judgments: ordered adjacent edges, relation kinds,
   traversal direction, source occurrences, ambiguity, unreachable nodes, and
   bound exhaustion. A negative or limit outcome must never count as a path.
4. Use held-out repositories/questions and publish all failures, including
   competitor wins. Separate extraction gaps, resolution gaps, retrieval gaps,
   rendering gaps and oracle mistakes using actual source evidence.
5. Improve the owning production layer for reproduced failures, retain native
   regressions, then rerun equivalent questions. Report category-level evidence
   and uncertainty rather than claiming universal dominance.
