# Code graph intelligence audit: 2026-09-26

## Status and acceptance criteria

Broad superiority over Graphify is **unproven**. The objective covers hub
analysis, code graph correctness, queries, explanations, and navigation/path
finding. A focused text-recall score cannot establish all of those properties.

| Requirement | Evidence needed | Current evidence |
| --- | --- | --- |
| Reliable hub analysis | Declaration-aware candidates, stable rankings, source-reviewed false positives and negatives | Five hub defects fixed; no reviewed god-object corpus yet |
| Accurate code graph | Reviewed declaration and relationship precision/recall, direction, occurrences, unresolved/ambiguous cases | Source-first fd pair/occurrence audit added; receiver-shadowing correction has native regressions; fd loop recall remains open |
| Better query answers | Held-out equivalent questions, independent source judgments, precision and recall | Five-repository development suites plus a separately selected source-first fd sample; neither establishes representative accuracy |
| Better explanations | Correct target, source provenance, callers/callees and explicit uncertainty | Fresh paired fd answers expose a Compass callees miss and a Graphify wrong-owner edge hidden by the text oracle |
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

## Separate source-first fd sample

The inputs in `benchmarks/agent_query/suite_fd.toml` and
`edge_witnesses_fd.json` were committed at `fe1a1fb5` before either tool was
run on this checkout. Source selection used `sharkdp/fd` at
`b422e5d8c9cffaa1ae43ba68e7b97a60fb3e8ae5`, with a separate clean working
checkout because the existing repository was bare. The sources stayed pinned
and clean before/after extraction. This is a source-first selected sample,
not a representative held-out corpus. It becomes development evidence once
used to guide fixes.

The fresh run `fd-source-first-01` uses the frozen debug Compass executable
with all eight preceding fixes. It records both graph digests, executable
identities, build logs and every query response. It is not performance evidence.

| Measured item | Compass | Graphify 0.9.67 |
| --- | ---: | ---: |
| Predeclared text-oracle passes | 11/12 | 9/12 |
| Non-excerpt passes | 10/11 | 9/11 |
| Exact reviewed declaration anchors | 5/5 | 5/5 |
| Predeclared positive direct-call pairs present | 10/10 | 10/10 |
| Reviewed line-level call occurrences preserved | 16/16 | 10/16 |
| Predeclared wrong-target pairs correctly absent | 2/2 | 2/2 |
| Post-output positive path witnesses | 2/2 | 2/2 |
| Median estimated tokens on eight shared passes | 227 | 88 |

Relationship presence and occurrence preservation are different measurements.
Graphify retains one occurrence for each reviewed pair, losing six repeated
sites across four pairs. These are selected-pair results, not whole-graph
precision or recall. The positive path review confirms an adjacent directed
call and a compatible source occurrence in each graph; neither rendered path
names a particular parallel edge or proves runtime execution.

The Compass failure is real: the `execute_batch` callees answer omits
`CommandBuilder::finish` at `src/exec/mod.rs:111`. The graph also lacks that
method's calls to `CommandBuilder::push` at line 104 and `exit_code` at line
116. The enclosing function maps constructors into a collected result,
destructures its success case, then iterates the builders. Graphify finds all
three method targets. Compass's Rust value-type collector handles function
parameters and local lets, but does not establish the loop/match/iterator
binding chain needed here. This is an extraction/resolution gap, not an
answer-rendering error.

The same answer exposes a Graphify precision error: its call at line 97 targets
`CommandTemplate::new` (declaration line 220), although the source explicitly
calls `CommandBuilder::new` (declaration line 136). Compass targets the correct
constructor. The post-output manifest `edge_witnesses_fd_diagnostic.json`
records those five checks separately. Graphify's text-oracle pass is retained;
it does not prove every returned edge correct.

Graphify's other text failures are the source-excerpt feature gap, a file
lookup resolving `src/exec/mod.rs` to the `Exec` declaration in
`src/cli.rs:858`, and a broad question that never returns `handle_cmd_error`
within its three budget increases. Compass needs two follow-up pages for
that broad question (1,769 estimated total tokens), so its pass also identifies
room to improve ranking and answer size. The lower paired token median remains
a Graphify advantage.

The first generated report incorrectly describes every suite as containing
five repositories. Its original report is retained. The renderer now uses the
recorded repository count, with singular/plural regression coverage; this
wording correction changes no scores or raw observations.

### Rust receiver correctness defect and correction

Reducing the loop miss uncovered false-positive calls in Compass. The frozen
eight-fix executable emits `Decoy::finish` for both calls below, although the
loop call is on `Actual`:

```rust
struct Actual;
impl Actual { fn finish(&self) {} }
struct Decoy;
impl Decoy { fn finish(&self) {} }
fn run(builder: &Decoy, builders: &[Actual]) {
    for builder in builders { builder.finish(); }
    builder.finish();
}
```

A nested `let builder = factory();` also inherits the outer parameter's
type when the local initializer does not yield a producer-known type.
Unknown local types currently fall through the value-type lookup, while the
parameter's alias remains available to call resolution. These are
precision defects, not acceptable substitutes for unresolved evidence.
Retained reductions are under `rust-loop-diagnostic-01` and
`rust-shadow-diagnostic-02`; the latter uses a source-defined factory returning
a different declared type.

The new edge auditor and report/input regression coverage pass 48 Python
tests. No production Rust change is included in this evaluation checkpoint;
the prior native verification ledger still describes the eight-fix executable.

The subsequent correction is in the Rust evidence producer. Value lookup now
distinguishes an absent binding from an existing binding of unknown type.
Local bindings suppress stale parameter aliases, including loop and closure
patterns, match arms/guards, conditional lets, and let-chain guards. Binding
visibility is bounded by lexical scope and source range; initializers and else
branches keep their proper outer bindings. Destructuring records the bound
names without assigning the whole container's type to each field. If an inner
receiver's type remains unknown, its source occurrence and unresolved candidate
remain visible; no target is invented.

A native regression failed before the fix by returning two calls to `Decoy`
where only the unshadowed outer call is valid. The expanded regression covers
ten scope/pattern forms, initializer timing, preserved typed inner calls, and
else-branch visibility. The current targeted checks pass 27 Rust language
tests, all 206 universal resolver integration tests, and 33 filesystem
contracts. Workspace and changed integration-test Clippy pass with warnings
denied. An existing needless borrow in the touched filesystem test was also
removed to allow that test target's Clippy check.

AST cache semantics advance from 2 to 3 so warm builds cannot keep previously
incorrect relationships. A cache regression checks that version-2 facts are
discarded. This is a correction to the existing producer contract, not a new
advertised capability or a new universal-pipeline promotion. Evidence/graph
schema majors and the Rust producer capability identity remain unchanged;
historical graphs are not rewritten.

The full workspace native baseline passes 1,083 tests with zero failures and
two ignored tests. The qualifying debug executable was copied and hashed after
its successful native build; its source snapshot, patch, and committed Rust
file hashes are retained under `shadow-fix-provenance`. The replay in
`rust-shadow-corrected-03` removes the wrong `Decoy` edge in both source
reductions while preserving exactly one valid outer call in each. The still
unproven inner receiver remains unresolved.

Fresh extraction/query run `fd-shadow-02` preserves all 12 query outcomes,
all 16 reviewed Compass call occurrences, and both source-grounded path
witnesses. The paired token medians remain 227 versus 88 on eight shared
passes. The separate diagnostic still records the three missing Compass
loop/callback calls and Graphify's wrong constructor target. All 92 recorded
Graphify distribution-file hashes remained unchanged; this does not pin every
transitive dependency. The Compass executable is a debug build and native
qualification ran concurrently, so these timings are not performance evidence.

Fixture qualification initially stopped at preflight because the default
parser-source bundle directory was absent. It was restarted using the complete
bundle already present in this checkout's build directory, whose language
definition hash matches the vendored manifest. The full fixture gate completed
successfully, including independent Markdown and React frontend qualification.
The release executable is frozen under `shadow-fix-provenance/compass-release`
with SHA256 `6baa1cabccdea6357ad5e9653a008efa1b384250557c1ce27278402e85399719`.

## Natural-language interface comparison

The preregistered `suite_ask.toml` sends identical caller/callee questions and
2,000-token budgets through Compass `ask` and Graphify `query`, with no
continuations. The ten questions reuse reviewed facts from the five-language
panel; they are interface checks, not ten independent source judgments.
Captured run `ask-paired-01` passes 9/10 selected-fact text checks for Compass
and 10/10 for Graphify. Compass's Cobra callees page omits `Find` before its
12-primary-result projection bound, even though the requested token budget
has room. Graphify wins that preregistered row. Do not change its oracle or
silently add a follow-up to erase this failure.

Review beyond the text oracle finds seven Compass relationship headlines
attributing the answer to the wrong subject: Cobra callers, both Flask rows,
both Gson rows, Zod callees, and Axum callees. Their correct fact anchors do
not make these answers fully correct. The CLI executes parsed operands but
supplies the whole natural-language question as a query operand to the
renderer; the first node by ID then substitutes for the requested subject.
This also affects answer basis, primary ordering, and suggested next actions.
The native CLI regression reproduced the wrong subject before correction.
The CLI now passes the planner operands with their symbol/source/target roles
and retains the original question separately. The first correction passes
36 CLI query and 9 product tests, workspace Clippy, and the 1,083-test native
baseline. The expanded regression also checks a missing subject and follow-up
actions. Python benchmark tests pass 49 cases.

Graphify exceeds the requested 2,000-token budget on four answers (Cobra
callers: 2,854; Gson callers and callees: 2,773 each; Axum callees: 3,160),
using the recorded stdout bytes/4 estimate. Each explicitly discloses the
overrun. The original selected-fact scores remain recorded; they are not
hard-budget success scores or complete precision judgments. On the nine
shared text passes, median output is 291 versus 1,210 estimated tokens.
This does not establish equal-budget correctness or performance superiority.

The diagnostic query-only replay `ask-operands-replay-02` uses retained,
digest-checked graphs from `ask-paired-01`. Both tools now pass 10/10 text
checks, including Cobra: correct subject ordering puts its missing fact on
page one without changing the budget or oracle. Median text output is 279.5
versus 1,439.5 estimated tokens on the ten shared passes. Additional JSON
requests compare all ten Compass ask projections with their direct commands;
those diagnostic requests are excluded from paired workflow costs.

All ten agree with direct commands, but source review still finds two wrong
headlines in both interfaces (Zod callees and Axum callees): function labels
such as `convertSchema()` and `validate_path()` carry trailing parentheses,
and the renderer's exact-string lookup does not share the query engine's
symbol normalization. Consequently Zod still names `convertBaseSchema` and Axum still names
`validate_v07_paths` as their subjects. The first correction resolves five of
seven observed headline failures; direct-command equivalence alone cannot prove source correctness.
A separate renderer regression failed before correction. The renderer now
shares the query engine's existing normalization for case, leading dots, and
trailing empty parentheses, while exact IDs and uniqueness remain explicit.
The original run, intermediate replay, and their scores remain retained
independently.

Final query-only replay `ask-operands-replay-03` passes 10/10 text checks for
both tools. A separate post-output subject diagnostic verifies all ten Compass
headlines and node bases against exact reviewed declaration labels, files,
and start lines (including Zod and Axum). Those witnesses are retained as
`ask-subject-witnesses.json`; they do not replace the original text oracle.
The frozen final debug executable has SHA256
`a748d0fa8eb08fec09a43647e29be0b056a706eb986d70c47049463c9dbb123c`.
The source patch and file hashes are retained under
`ask-normalization-provenance`. Output medians remain 279.5 versus 1,439.5
estimated tokens on the same ten passing text questions.

The complete native run passes 1,145 tests: 1,083 workspace lib/bin tests,
36 CLI query tests, 9 CLI product tests, and 17 output integration tests;
two existing tests remain ignored. Workspace and changed integration-target
Clippy pass with warnings denied. The first expanded Clippy run found an
existing `expect_err` in the touched output test; it now returns the failed
assertion as an error. The 17-test output suite was rerun after that test-only
cleanup. Formatting, diff checks, product boundary, and 49 Python benchmark
tests pass. The full extraction fixture gate passed at the preceding receiver
checkpoint; these presentation corrections use native query/CLI tests and
retained real graphs rather than claiming a new extraction qualification.

The fresh `v2-shadow-05` run remains 50/50 versus 44/50, with five source
excerpt availability differences and one Axum file-resolution difference.
Its five source-grounded path witnesses pass for both tools in
`path-audit-v2-shadow-05.json`. Graphify uses less output on its 44 shared
passes (112 versus 308 estimated tokens). All of these runs use a frozen
debug executable during concurrent qualification; timings are not speed
comparisons. The new ask failures demonstrate why that earlier suite is
insufficient to establish the requested superiority.

## Shared MCP comparison across five languages

Commit `152efdd7` records `suite_mcp.json` and the bounded stdio collector before
execution. There are 29 questions per tool on the retained five-repository
panel: graph counts, top-ten hubs, largest-community enumeration, missing
communities, calls adjacent to reviewed declarations, and four ambiguous
neighbor names. Both products expose the same MCP operations. Input node IDs
and community IDs are prepared symmetrically from the graphs and are not
counted as successful node retrieval. Sources and graph hashes are checked
before and after use.

Graphify runs in the isolated 0.9.67 MCP environment described in the coverage
plan. Its optional SDK dependencies are separately recorded, and all 227
compared Graphify package files match the original installation. This avoids
counting an absent optional SDK as a product failure. Full-enumeration requests
use its explicit 262,144-token allowance; Compass exposes whole-result output.
The external limits are 60 seconds per RPC, 16 MiB per response/stderr, and
64 MiB per session. These are complete-enumeration tasks, not an equal
2,000-token arm. Text bytes and actual response-wire bytes are reported
separately. Concurrent debug timings are not performance comparisons.

All 58 RPCs completed in `mcp-paired-01`. The original run exposed two production
defects: the compact traversal projection retained community names but dropped
labeled community IDs, and `get_neighbors` selected the first ambiguous match.
The independent auditor checks source-run/graph/input/collector hashes and
captured responses against raw JSON-RPC transcripts. Its checks implement the
preregistered graph-consistency policies; these are not complete source-precision
or functional-clustering oracles.

| Graph-consistency check | Initial Compass | Corrected Compass | Graphify in both runs |
| --- | ---: | ---: | ---: |
| Node/edge/community totals | 0/5 | 5/5 | 5/5 |
| Largest-community member multiset and count | 0/5 | 5/5 | 5/5 |
| Absent community is reported absent | 5/5 | 5/5 | 5/5 |
| Filtered neighbor direction/label/relation triples | 5/5 | 5/5 | 5/5 |
| Ambiguous neighbor lookup preserves ambiguity | 0/4 | 4/4 | 4/4 |

The first two failures share one cache defect; they are not ten independent
implementation bugs. Native regressions failed before both corrections.
The model now retains the community ID and display name in its compact cache,
and advances cache magic to `TRAILT05` so previously deficient `TRAILT04`
projections rebuild. Cold, warm, and stale-cache regression cases pass.
MCP now returns a stable candidate list (at most 20 plus an omission count)
with source paths and exact IDs. Exact IDs preserve case. Graph schemas and
historical artifacts remain unchanged.

Replay `mcp-paired-02` uses the same questions, source graphs, and Graphify
environment. It rebuilds the old Compass traversal caches automatically and
passes the graph-consistency checks above. The corrected debug binary SHA256
is `4b3b755fd8a3e8d74c4eac8c336f719506dcfee9f8ac89cea2eafbe181042efb`;
source/file hashes are under `mcp-corrections-provenance`. No oracle was retuned
to replace the initial failures. Reports and transcripts for both runs remain.

The hub outputs expose another unresolved limitation. Only 29/50 Compass and
37/50 Graphify entries have labels that uniquely identify a graph node.
All of those identifiable entries have matching independently computed degree;
the remaining entries are unverified because their labels collide. Matching a
label plus its expected degree to choose a convenient identity would conceal
this limitation. Both MCP outputs need stronger identity presentation for
reliable navigation. These counts do not verify ranking eligibility or source
correctness, and high degree does not establish excessive responsibility or
poor cohesion. Zod's highly connected module nodes also need explicit role
interpretation before any design conclusion.

Verification after the two corrections: 1,148 native tests passed (1,086
workspace lib/bin tests plus 36 CLI query, 9 product, and 17 output tests),
zero failed, two ignored. Workspace Clippy passes with warnings denied.
The benchmark/transport/auditor suite passes 65 Python tests, including
membership mismatches, ambiguous labels, direction errors, parallel edges,
self-loops, transport timeouts/byte limits, and unsupported-platform handling.
Formatting, diff checks, and the native product boundary pass. Full extraction
fixture qualification remains the preceding receiver checkpoint; this change
is verified through native cache/MCP regressions and the retained-graph replay.

### Neighbor filtering diagnostic

The five selected neighbor questions missed another defect: MCP grouped each
neighbor before applying the relation filter. A preceding containment or
reference edge could therefore hide a later call between the same endpoints.
Both incoming and outgoing lookups were affected. A post-output source review
of Flask `tests/test_helpers.py:275–281` establishes the nested `index` to
`generate` call. The retained graph contains both containment and call edges,
but both filtered MCP lookups returned empty adjacency before correction.
This diagnostic is separate from the preregistered comparison scores.

The filter now precedes grouping. Native regression coverage exercises both
directions, both edge orders, and duplicate calls. The tool continues to return
distinct neighbors; it does not promise call-site multiplicity. The unchanged
Flask source and graph now produce the correct outgoing and incoming call in
`mcp-filter-diagnostic-02`, with original failures retained in
`mcp-filter-diagnostic-01`. Similar edge-order candidates exist elsewhere in
the panel, but their presence alone is not source-accuracy evidence.

The final filter executable is retained under `mcp-filter-provenance`, SHA256
`367dfd78301d9c3914049b8c134933dce91934003eafbc2c144d9abd760a7e1a`.
Native verification passes 1,149 tests, zero failed, two ignored; workspace
Clippy passes with warnings denied. Formatting, diff checks, and the product
boundary also pass. This is a query correction on retained graphs; no new
extraction or performance claim follows. Replay `mcp-paired-03` completes all
58 RPCs; `mcp-audit-03.json` confirms the same graph-consistency results as
`mcp-paired-02` for both tools. The separate Flask diagnostic is the evidence
for the additional filter correction.

### Hub identity and direct navigation

The first hub responses omitted the exact IDs already retained by Compass's
analysis layer. MCP now adds ID/source/location text and the versioned
`compass.mcp.hubs/1` structured result inside its existing transport envelope.
It preserves ranking and degree semantics. Labels are sanitized for display;
exact IDs are JSON-escaped in text and retained unchanged in structured data.
Missing source fields remain null. The public description now describes
topology candidates rather than asserting that connectivity proves a core
abstraction or design defect.

A failed-before native regression covers duplicate labels, legacy source
locations, typed anchors, and an ID containing quotes, a backslash, and a
newline. Its follow-up also exposed lookup trimming an ID before checking exact
identity. Exact lookup now precedes the existing trimmed fallback; a separate
query-layer test verifies IDs distinguished by surrounding whitespace.
The transport regression checks that clients receive structured hub results.

`mcp-paired-04` repeats all 58 original RPCs. All previous graph-consistency
checks remain passing for both tools. All 50 Compass hub IDs now resolve to
unique graph nodes, with independently matching degrees and source anchors.
Graphify's MCP result still provides label-only identity: 37/50 entries can be
identified uniquely from their display labels. Its CLI JSON interface supplies
IDs, so this is specifically a finding about these MCP responses. Neither
count proves complete top-N eligibility, source precision, or design quality.

Commit `7c554c2f` adds a separate development diagnostic before its follow-up
requests: use each returned hub ID when available, otherwise the returned
label, for exactly one `get_neighbors` request. The graph may be read by the
oracle but never to substitute a request ID. Explicit ambiguity is safe and
is not counted as completed direct navigation. The same full-enumeration
bounds apply to both tools. Additional disambiguation steps and the alternative
Graphify CLI workflow are outside this diagnostic.

| Repository | Compass before IDs | Compass with IDs | Graphify in both runs |
| --- | ---: | ---: | ---: |
| Cobra | 2/10 | 10/10 | 7/10 |
| Flask | 2/10 | 10/10 | 5/10 |
| Gson | 0/10 | 10/10 | 7/10 |
| Zod | 0/10 | 10/10 | 4/10 |
| Axum | 0/10 | 10/10 | 5/10 |
| Total | 4/50 | 50/50 | 28/50 |

Runs `hub-navigation-02` and `hub-navigation-03` each capture all 100 follow-up
RPCs without execution failures. The oracle checks seed identity, response
headline, and the multiset of displayed direction/neighbor-label pairs after
grouping by distinct neighbor. It does not validate individual neighbor IDs,
all parallel relations, source occurrences, or source accuracy. Both products'
broader name matching can be ambiguous even when a case-sensitive display
label is unique. The initial `hub-navigation-01` attempt stopped before
follow-up RPCs because the collector resolved Python's virtual-environment
symlink, losing its package environment. Commit `e3652ae3` fixes that harness
error; it is not counted as a product failure.

The five hub responses plus 50 follow-ups total 593,962 text bytes / 626,338
wire-response bytes for corrected Compass, versus 265,393 / 272,996 for
Graphify. These totals include different completion counts and different
returned hub sets, and are not an efficiency win. Initialization/setup
transcripts are retained separately. A separate transcript integrity pass
checks all 200 follow-up requests and responses against their recorded
arguments, text, and byte counts. Timings remain unsuitable for speed claims.

The frozen executable in `mcp-hubs-provenance` has SHA256
`0c54260eb380c08c18a14c7044a1abce932ede56114d6a515223ab19e1b7be93`.
Verification passes 1,198 native tests, zero failed, two ignored, including
the broader `coverage_paths` suites and the new identity regressions.
Workspace and `coverage_paths` Clippy pass with warnings denied. All 72 Python
benchmark tests, formatting, diff checks, and product boundary pass. Full
extraction fixture qualification remains the receiver checkpoint; this change
adds query/MCP presentation and exact-ID lookup verification on retained graphs.

### Source-role diagnostics from the returned hubs

Two post-output reductions in `hub-source-diagnostics-01.json` retain pinned
source excerpts, file and graph hashes, node records, and selected edges:

- Compass's Zod `to-json-schema.test` module spans the test suite, not a
  production object. Its distinct-pair degree is 789. Its 1,156 incident edge
  records include 789 containment, 321 reference, and 46 call records, with
  overlap between endpoint pairs. Containment-driven degree does not establish
  excessive responsibility.
- Graphify's Axum hubs `S` at `src/service_ext.rs:47` and `T` at
  `src/handler/mod.rs:272` are generic implementation subjects. Their degrees
  are 153 and 87, predominantly reference edges. Two reviewed edges target
  the ServiceExt implementation's `S` from `Router<S>` and the separate
  `Handler<..., S> for T` implementation. Each source declaration binds its
  own `S`; those references do not identify the independently bound `S` in
  `service_ext.rs`. This establishes two wrong reference targets, not that all
  150 references to that hub are wrong or a representative precision rate.

These diagnostics motivate role-aware explanations and independent edge
review. They are not folded into the original graph-consistency scores, and
do not establish that Compass already diagnoses god objects reliably.

## Next evidence to collect

1. Extend source-proven loop/result/iterator inference to recover the fd callees miss. Keep exact
   build/source provenance for subsequent release comparisons;
   the latest query correction has native and fixed-graph regression evidence.
2. Expand the now-identifiable hubs into source-reviewed role and design
   judgments, including containment-dominated modules and generic reference
   targets. Evaluate cluster responsibilities and cross-community connections
   separately from graph consistency.
3. Add independent edge/path judgments: ordered adjacent edges, relation kinds,
   traversal direction, source occurrences, ambiguity, unreachable nodes, and
   bound exhaustion. A negative or limit outcome must never count as a path.
4. Use held-out repositories/questions and publish all failures, including
   competitor wins. Separate extraction gaps, resolution gaps, retrieval gaps,
   rendering gaps and oracle mistakes using actual source evidence.
5. Improve the owning production layer for reproduced failures, retain native
   regressions, then rerun equivalent questions. Report category-level evidence
   and uncertainty rather than claiming universal dominance.
