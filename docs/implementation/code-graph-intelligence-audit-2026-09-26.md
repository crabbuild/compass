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
| Better navigation and walks | Valid ordered edges, direction, hop bounds, alternatives, ambiguity and negative cases | Five registered real-source directed call chains yield 2/5 source-supported answers per tool; ambiguity, relationship restrictions and broader path quality remain open |
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

## MCP path identity, bounds, and source-route diagnostics

The next development arm uses the same retained Cobra (Go), Flask (Python),
Gson (Java), Zod (TypeScript), and Axum (Rust) graphs. There are 28 questions per
tool: five forward paths, five reverse paths, five one-hop-too-small bounds,
five nonexistent source IDs, four ambiguous source names, and four disconnected
pairs. Exact IDs are prepared symmetrically from source anchors and are not
scored as retrieval. Each side has the same external 60-second/16-MiB RPC and
64-MiB session bounds. Graphify receives `undirected=true`; Compass's public
MCP operation is undirected navigation. These results do not establish directed
call-flow accuracy.

Disconnected pairs are selected from common source-anchored nodes in different
components in each graph. They include files/modules/configuration as well as
function declarations. Preparation examined at most 2,048 common anchors;
no Cobra pair was selected, which does not prove none exists. Positive source
witnesses reuse development witnesses reviewed after earlier path outputs.

**Registration correction:** inputs were written before the first requests,
but a transient Python runner cleanup `PermissionError` prevented the intended
pre-execution commit. The initial capture was launched before that command
failure was noticed. Its retained manifest's preregistration claim is wrong;
this report and the current input scope supersede it. This is a development
diagnostic. The original log is `mcp-path-python-01.log`; the child process was
subsequently absent and the full 81-test rerun passed. The runner was not changed
for an unreproduced failure.

### Baseline findings and the separate label arm

All 56 requests execute in `mcp-path-paired-01`. The independent auditor checks
transcript/input/graph hashes, actual path bodies, exact endpoint identity,
minimum distance, each ordered edge's relation and direction, and source-route
witnesses separately. It never resolves an ambiguous displayed intermediate
node using the expected answer.

| ID-input diagnostic | Baseline Compass | Graphify 0.9.67 |
| --- | ---: | ---: |
| Verifiable minimum-hop path | 8/10 | 0/10 |
| Explicit hop-bound outcome | 5/5 | 3/5 |
| Missing source stays unresolved | 5/5 | 0/5 |
| Ambiguous source is not selected | 0/4 | 0/4 |
| Correct global disconnection | 4/4 | 1/4 |

Compass's two positive misses use a Zod intermediate label shared by several
nodes; the text does not identify the selected node. This does not prove the
route itself wrong. Graphify's exact-ID inputs frequently select other nodes:
Cobra `.Find()` becomes `Command`, Axum `validate_path` becomes `routing()`,
and some pairs resolve to the same node. Its missing-ID sentinel also resolves
to fuzzy candidates. These are strict identity diagnostics, not a general
claim about Graphify's advertised label/keyword interface.

A separate label-input arm was committed before its requests, after the ID
failures were observed. It uses the same positive source endpoints and bounds,
with each tool's actual display labels: 15 questions per tool, all 30 executed.
**Graphify wins this baseline comparison:** 10/10 verifiable paths versus
Compass's 8/10; both give 5/5 explicit hop-limit outcomes. Graphify matches all
five reviewed source routes; Compass verifies four of five, with Zod's identity
unverified. This arm is development evidence, not held-out confirmation.
Artifacts are `mcp-path-labels-01` and `mcp-path-labels-audit-01.json`.

### Production corrections and the intermediate failure

MCP path endpoints now require an exact ID or uniquely matching normalized
symbol/qualified name. Ambiguity returns stable candidates and exact IDs;
missing names remain unresolved. The public hop bound is validated from zero
to 64. Search uses the query crate's shared bounded engine, minimizing hops,
then structural relation cost, then a stable tie key. Its one-million-adjacency
and 16-MiB cumulative path-key budgets fail explicitly. Reaching a depth
frontier is distinguished from proving disconnection; conservative depth
reports may remain incomplete even when the full stored graph is disconnected.

The versioned `compass.mcp.path/1` structured result preserves ordered node IDs,
source anchors, and selected edges with their stored and traversal directions.
Legacy text remains available. Equal-hop routes can change under the structural
tie rule, and callers relying on fuzzy endpoint selection must migrate; the
compatibility reference and migration guide document both changes.

The first corrected binary (`3fc64bc00f1f47e36fb2158c9dfbfb360cf81915c6b81f588c84f5f29b023ed1`)
failed every positive structured-anchor check: compact traversal nodes omit
numeric source lines, so projection emitted null. Both `*-02` captures and
audits preserve this failure. Projection now reads the full stored node
records, as hub projection does, and a native regression requires line 9 to
survive actual structured invocation. The oracle was not relaxed.

The intermediate ID run already refused all four ambiguous endpoints. It
reported global disconnection for only two of four disconnected pairs; Gson
and Axum reached the depth bound. Those two remain incomplete answers rather
than being credited as proven disconnections.

The next frozen binary (`78e2a3528ef864937b7160fb523826b621218e05e14ccad51faba6dc8caf1082`)
restored all node source anchors, but `*-03` audits exposed null edge IDs:
the compact cache omits these too. All positive results therefore still failed
the structured-edge checks. The implementation now selects and renders paths
from one full bounded document snapshot, retaining parallel-edge IDs during
the search itself. It does not attach an arbitrary full-record edge after
traversing a lossy projection. The native regression uses two parallel calls
and requires the stable selected ID (`edge-a`), as well as the source line.
This full-snapshot load can cost more than compact traversal; no timing or
memory improvement is claimed.

### Final path replay at this checkpoint

The frozen executable in `mcp-path-identities-provenance` has SHA256
`b16d7f755a81bf68110737a445701158ee541a4cc0f070b9e4e800cff693c988`.
Its base commit, full source patch, and changed-file hashes are retained.
`mcp-path-paired-04` completes all 56 RPCs and `mcp-path-labels-04` completes
all 30. Corresponding `mcp-path-audit-04.json` and
`mcp-path-labels-audit-04.json` validate raw transcripts and stored graph
identities. The current auditor also rechecks both baseline captures in
`mcp-path-baseline-current-audit.json` and
`mcp-path-labels-baseline-current-audit.json`; all 86 original outcomes remain
unchanged. Errors cannot pass from error text, and structured negative statuses
must agree with the required outcome.

| ID-input diagnostic | Baseline Compass | Corrected Compass | Graphify |
| --- | ---: | ---: | ---: |
| Verifiable minimum-hop path | 8/10 | 10/10 | 0/10 |
| Explicit hop-bound outcome | 5/5 | 5/5 | 3/5 |
| Missing source stays unresolved | 5/5 | 5/5 | 0/5 |
| Ambiguous source is not selected | 0/4 | 4/4 | 0/4 |
| Correct global disconnection | 4/4 | 2/4 | 1/4 |

The corrected label arm **ties Graphify**: each returns 10/10 verifiable
minimum-hop paths and 5/5 hop-bound outcomes. Each matches all five reviewed
source routes. Compass also matches those five routes in the ID arm; Graphify
matches none of the five with ID inputs. Failure to establish identity remains
unverified source-route evidence, not evidence that every underlying edge is
wrong. These short selected navigation routes do not establish representative
path precision, source occurrence recall, execution feasibility, or broad
superiority.

Compass's two lost global-disconnection completions are explicit depth-limit
outcomes for Gson and Axum at eight hops. The original implementation searched
the whole component before applying the requested hop limit. The corrected
one stops at the bound; neither this report nor the auditor credits those
incomplete answers as disconnections. Retain that completion tradeoff alongside
the identity and ambiguity improvements.

Verification: 1,212 native tests pass, zero fail, two are ignored. This includes
workspace library/binary tests, CLI query/product contracts, output agent-query
contracts, MCP coverage, typed traversal, and the three-engine independent
path oracle (729 four-node graphs × three depth bounds × three engines =
6,561 queries). The small oracle covers hop/cost optimality and actual edge
chains; it does not establish large-graph performance or source extraction
accuracy. Workspace and changed integration-target Clippy pass with warnings
denied. All 86 Python benchmark tests, formatting, diff checks, and the product
boundary gate pass. Logs are `mcp-path-corrections-tests-04.log`,
`mcp-path-corrections-clippy-04.log`, and `mcp-path-python-07.log`.
The test build emits existing core `unused_mut` and macOS linker unwind
warnings. Full extraction fixture qualification remains the earlier receiver
checkpoint: this correction changes query/MCP behavior on retained graphs,
not extraction. Concurrent debug replays are not timing evidence.

## Hub explanations and source-role census

The next development diagnostic inspects the original 100 MCP hub entries,
50 per tool. `benchmarks/agent_query/hub_role_reviews.json` records manual
source-role judgments, exact graph IDs when recoverable from the response,
pinned source-file hashes, anchors, and excerpts. It deliberately follows
output inspection. It is neither preregistered design-quality evaluation nor a
representative precision sample. Different returned sets prevent a shared
accuracy score.

The independent `hub_evidence_audit.py` verifies the original capture and source
provenance. It checks 87 reviewed identities against the pinned source; the 13
ambiguous Graphify labels remain unknown. This verifies where the judgments
came from, not that a source hash can prove a semantic role or design judgment.
The original report is `hub-source-role-audit-01.json`.

| Reviewed role among returned hubs | Compass | Graphify |
| --- | ---: | ---: |
| Production type declaration | 21 | 19 |
| Production callable | 12 | 4 |
| Test helper callable | 6 | 5 |
| Test suite type | 0 | 3 |
| Whole production source module | 6 | 0 |
| Whole test module | 4 | 0 |
| Example callable | 1 | 1 |
| Benchmark callable | 0 | 1 |
| Generic implementation target | 0 | 2 |
| Trait implementation block | 0 | 2 |
| Unidentified from returned label | 0 | 13 |

Types include classes, structs, aliases, interfaces, and public testing API
types. “Production” distinguishes the source declaration from repository test
helpers, not its release stability or architectural importance. All ten
Compass Zod results are whole-file module nodes, four spanning test files.
These can be useful file navigation hubs, but do not identify ten god objects.
Graphify's Axum `S` and `T` entries are generic implementation subjects, and
`Router<S>` and `Bytes` refer to trait implementation blocks rather than new
type declarations. Earlier source diagnostics establish two wrong references
to `S`; this census does not extrapolate their frequency.

### Exposing why a hub is connected

Both original MCP hub responses primarily state degree. Compass now adds
stored node kind and a bounded relation/direction breakdown, in text and
`connectivity` with schema `compass.hub-connectivity/1`. Analysis lives in
`compass-graph`; MCP projects it. Eligibility, degree, and ranking stay the
same. Undirected ranking metadata is corrected to name its existing policy.
The summary counts all valid incident records, including parallel records,
separately from distinct-pair degree. Directed self-loops count once in the
incident total and once per direction. Undirected records receive no invented
arrow. Sixteen relation categories are shown by descending record count then
name, with omitted category and record totals.

Concrete observations from the retained Compass graphs illustrate why this
matters:

- Cobra `Command`: 525 incident records, including 346 incoming references and
  154 outgoing containment records. Its degree is 443, a different quantity.
- Flask `Flask`: 110 records include 38 outgoing containment and 34 incoming
  references; several other relation kinds contribute too.
- Gson `JsonReader`: 530 records include 240 incoming instantiations and 97
  outgoing containment records.
- Zod `to-json-schema.test`: 1,156 records include 789 containment, 321
  references, and 46 calls, all outgoing. It is a whole test module.
- Axum `.route()`: 584 records include 306 incoming calls, one outgoing call,
  and 272 incoming `tests` records. A high degree does not mean this method
  makes hundreds of outgoing calls.

These counts are observations about the stored graph, not independently
verified counts of source occurrences or evidence of excessive responsibility.
The previous neighbor interface groups by neighbor and can omit parallel
relations; a summary should not be reconstructed from that grouped view.
The native implementation uses complete stored records. No automatic
production/test role classifier or god-object judgment is introduced.

The summary auditor separately checks numeric types, direction, self-loops,
parallel records, ordering, omission accounting, and each hub's text block.
Original Compass and Graphify responses have no such summary. That absence is
reported as unavailable, not as a wrong answer or lack of another workflow.
Graphify's neighbor and CLI interfaces remain available; this diagnostic does
not measure their multi-step explanation cost or quality.

### Hub explanation replay and verification

`mcp-hub-evidence-01` completes all 58 original shared MCP questions on the
same graphs. All original graph-consistency judgments and all returned hub
identity/degree/rank records remain unchanged. The independently verified
`hub-evidence-audit-01.json` checks **50/50 Compass connectivity summaries and
50/50 matching text blocks**. Graphify's direct hub responses offer no such
summary; this is recorded as unavailable, not 50 incorrect explanations.
The latest source-role check (`hub-source-role-audit-02.json`) also verifies
each hub RPC's raw request and response. Graphify's stored graphs are
undirected, so their oracle record counts remain undirected; the summary does
not infer semantic directions from their serialized endpoints.

For the same five Compass top-ten hub responses, text grows from 9,136 to
26,620 bytes and captured response-wire bytes grow from 24,146 to 75,299.
Graphify remains at 1,489 text / 1,989 wire bytes. This is additional evidence
with a payload cost, not an efficiency win or a controlled multi-step workflow
comparison. Initialization traffic is retained separately. Ranking sets differ
between products, and these runs do not establish comparative design quality.

The frozen executable in `hub-evidence-provenance` has SHA256
`5382b51b86ab76032173a17e638a3ad525600ec1726364d3244edddb7abd2c19`;
the collector records the same executable digest. Base commit, source patch,
and changed-source hashes are retained. Native verification passes **1,162
tests, zero failed, two ignored**, covering the full workspace library/binary
baseline plus `analyze_coverage`, `coverage_paths`, and `compass_product`.
This is a different integration selection from the prior path checkpoint,
whose larger count also included CLI/output path suites. New regressions
cover parallel edges, self-loops, missing endpoints, directed/undirected
records, permutation stability, top-zero, relation omission totals, text
projection, and actual MCP transport. Workspace and selected integration
Clippy pass with warnings denied. All **93 Python tests**, formatting, diff
checks, and product boundary pass. Logs: `hub-evidence-native-04.log`,
`hub-evidence-clippy-03.log`, and `hub-evidence-python-03.log`.

Earlier development logs preserve a test fixture type mismatch and an expected
legacy-kind mismatch (`symbol`, not `unknown`); both were corrected before the
final checks. Existing core `unused_mut` and macOS linker unwind warnings remain
in test builds. The full extraction fixture gate remains the prior receiver
checkpoint: no extractor, publication, or viewer format changes are made here.
God-object detection, community cohesion, source-edge precision, and explanation
usefulness on held-out tasks remain open.

## Frozen confirmation panel A: competitor advantages remain

Inputs were committed as `7c70fbea` before extraction or queries: 55 questions,
21 selected relationship pairs, and ten forward/reverse path witnesses.
Chi (Go), Click (Python), jsoup (Java), Redux (TypeScript), and WalkDir (Rust)
are new repositories for this checkpoint. Both tools use the same five clean,
pinned checkouts. Selection is purposive, not representative. The Compass
executable is the frozen `5382b51b…` hub-evidence build above; Graphify reports
0.9.67. `heldout-a-01` completed all ten builds and 110 question workflows.
Source commits, registered input bytes, captured graph digests, runner digest,
and capture byte totals were rechecked. The CLI records executable hashes;
Graphify's launcher hash does **not** attest to all Python package/dependency
bytes during execution. Do not conflate the separate MCP environment check
with a pre/post CLI package check.

### Original text-recall scores

| Surface | Compass | Graphify |
| --- | ---: | ---: |
| Explain | 10/10 | 8/10 |
| Direct callers plus incoming-call ask | 8/10 | 9/10 |
| Callees | 4/5 | 4/5 |
| Symbol paths, both directions | 6/10 | 10/10 |
| File paths | 5/5 | 1/5 |
| Ambiguity | 5/5 | 4/5 |
| Missing symbols | 5/5 | 5/5 |
| Broad natural query | 3/5 | 5/5 |
| Total text matches | 46/55 | 46/55 |

There are 38 shared passes, eight exclusive passes per tool, and one shared
failure. On shared text passes, median estimated answer tokens are 158 Compass
versus 104 Graphify. The single-run overall wall-time medians are 738 versus
236 ms; these are observations, not a controlled performance benchmark.
Graphify sometimes exceeds the requested text budget (for example the Chi ask
response is 2,744 estimated tokens against a requested 2,000). All consumed
responses and documented follow-ups remain in the cost totals.

These are **not correctness scores**. Manual response inspection catches four
Compass text passes that do not resolve the requested operation: Redux's
primary explanation, both WalkDir explanations, and WalkDir callees merely
list ambiguity candidates. The registered text criteria find names/lines in
those lists. Graphify's two WalkDir symbol paths also pass by printing the
requested words while selecting same-named **test** functions and walking
through their containing test file. Its stderr warns of ambiguity; the body
still provides a different route. The frozen run is preserved, with these
judgments recorded separately in `heldout_panel_a_review.json`.

For incoming-call ask, Compass prints the requested callee and incoming
relationships on Chi, Click, jsoup, and Redux; WalkDir remains unresolved.
Compass's usage results include references/imports where explicitly labeled,
not just calls. Graphify's query answers return broader neighborhoods that
contain the selected incoming call; WalkDir combines library and test seeds.
These inspected facts do not establish the precision of every returned edge.

### Source and identity checks

The path auditor now retains failed commands as failed rows rather than
aborting and dropping the remaining denominator. Nonzero exit, timeout, and
unsupported multi-response execution cannot pass even if stdout prints a valid
path. Source drift and mismatched graph provenance still abort the audit.
All ten witnesses remain in each tool's denominator. Source-verified paths
are **6/10 Compass versus 8/10 Graphify**: both pass Chi, Click, and jsoup;
Graphify also passes Redux; neither establishes the requested WalkDir route.
Compass refuses WalkDir's genuinely ambiguous unqualified names and Redux's
export/function name collision. An ambiguity refusal is safer than selecting
a wrong declaration, but it is still not a completed path task.

The initial edge audit also exposed an **oracle mistake**. The registered
Click witness omitted `open_stream`'s second call to `_wrap_io_open` at
`src/click/_compat.py:450`. Compass preserved both calls, while Graphify kept
only line 397. The original witness and audit remain unchanged. A separately
named `edge_witnesses_heldout_click_corrected.json` adds line 450 after reviewing
the entire function; it is an explicitly post-output diagnostic correction.

| Selected-pair evidence | Compass | Graphify |
| --- | ---: | ---: |
| Unique endpoint identity, original or corrected | 17/21 | 21/21 |
| Relationship/absence matches, original or corrected | 15/21 | 20/21 |
| Full occurrence agreement, registered witness | 14/21 | 18/21 |
| Reviewed positive occurrences, registered witness | 13/18 | 15/18 |
| Full occurrence agreement, corrected diagnostic | 15/21 | 17/21 |
| Reviewed positive occurrences, corrected diagnostic | 14/19 | 15/19 |

The 21 pairs comprise 16 positive pairs and five direct-edge negatives.
Negatives pass only with both endpoints uniquely identified. Four Compass
Redux pairs fail identity checks because separate export and function nodes
share the exact file/start-line/terminal name; they must not be described as
four missing call edges. The plain-symbol ambiguity is a real workflow issue,
while the source audit cannot choose between those nodes using its registered
identity rule. Compass also misses Chi's source-proven `rctx.URLParam` call.
Both tools miss jsoup's chained `new Cleaner(...).isValidBodyHtml(...)` call.
Graphify loses repeated occurrences in jsoup and WalkDir as well as corrected
Click. This checks selected pairs, not complete callee sets or graph precision.

Artifacts are `heldout-a-path-audit-01.json`, five
`heldout-a-*-edge-audit-01.json` files, and
`heldout-a-click-edge-audit-corrected-01.json` under the evaluation root.
The checked-in review records their digests, input digests, response-review
digests, and limits. Auditor regressions pass all 95 Python tests, including
unsuccessful execution and source-drift cases (`heldout-a-auditor-tests-01.log`).
This panel contradicts a broad superiority claim. Its first run remains a
confirmation checkpoint; subsequent product tuning on it is development and
requires another independent confirmation panel. MCP/community workflows,
directed or long walks, source-level cohesion, and god-object judgments remain
unproven here.

### Post-output path selection diagnostics

Two additional arms use the **original frozen binaries and graphs**, with
source-reviewed callable endpoints prepared symmetrically from the graphs.
They are post-output diagnostics, not node retrieval or workflow-cost scores:
preparing an endpoint from the oracle does not prove that a user found it.
The four questions cover forward/reverse Redux and WalkDir paths only.

| Prepared endpoint form | Compass source-path matches | Graphify source-path matches |
| --- | ---: | ---: |
| Full stored IDs | 4/4 | 2/4 |
| Exact stored display labels | 0/4 | 2/4 |

Both graphs contain the selected WalkDir call edges. Full IDs let Compass
navigate those edges and Redux's function declarations. Graphify's CLI path
help describes source/target strings without promising exact-ID semantics;
its stored-ID and display-label attempts still choose different WalkDir
endpoints. For example the full-ID forward request returns a test function,
its containing test file, and an import to `WalkDir`. Compass normalizes display
labels and still detects the export/function or library/test collisions.
Neither arm replaces the original 6/10 versus 8/10 source-path result, and the
ID diagnostic is not a claim that Graphify lacks other disambiguation workflows.
Artifacts: `heldout-a-explicit-path-diagnostic-01` and
`heldout-a-label-path-diagnostic-01`, each retaining prepared inputs, executable
hashes, script, captured streams, and source-witness audit results.

### Development correction: actionable ambiguity answers

The frozen panel exposed an unrelated presentation defect: a typed ambiguous
relationship request said “No exact match” and named the first candidate as a
fallback. A typed ambiguous node-trail response could instead claim no directed
path. `compass-output` now handles `needs_resolution` before those answer
branches, asks for exact IDs, and uses the operation as the answer basis.
Every retained ambiguity candidate has its exact ID in paged and full text,
even if qualified labels differ. Candidate selection, raw query results,
relationship resolution, schema majors, and path algorithms are unchanged.
Changed text pages use the existing cursor-prefix rejection rules; restart a
rejected continuation from page one.

This is **development after observing panel A**, not an improved held-out
score. `heldout-a-ambiguity-replay-01` repeats all 55 Compass questions against
the original hash-verified graphs. Text passes remain **46/55**, with no changed
pass/fail rows. All four inspected Redux/WalkDir ambiguous callers/callees/ask
responses now show exact IDs and the corrected headline. The text-oracle false
positives documented above remain false positives; better ambiguity wording
does not turn an unresolved task into a successful answer. The replay does not
re-extract either graph or rerun Graphify.

The new frozen binary has SHA256
`d561d7c762410899cb6039f409d17401239ee26ee525f8bbfd2e95cebcb82857`.
`ambiguity-headline-provenance` records its base commit, patch, and source hashes.
Verification passes **1,155 native tests, zero failed, two ignored**, comprising
workspace library/binary tests plus `agent_query`, `code_query_cli`, and
`compass_product`. The new regressions cover callers, callees, impact, node
trails, candidate permutation, full/paged text, agent JSON, and the actual CLI
callers/callees/ask boundary. Workspace and the same selected integration Clippy
pass with warnings denied. All **95 Python tests**, formatting, diff checks,
and product-boundary checks pass. Logs are `ambiguity-headline-native-02.log`,
`ambiguity-headline-clippy-01.log`, and `ambiguity-headline-python-01.log`.
The first targeted test compile used a nonexistent test-options default; the
retained `ambiguity-headline-targeted-01.log` records that development error.
Explicit options corrected it before the full pass. Existing core unused-mut
and macOS linker warnings remain in build/test logs. Extraction qualification
and JavaScript gates were not rerun: this change only projects query ambiguity
and does not change extraction, publication, or viewer assets.

### Development correction: Go control initializers and receiver shadowing

The registered Chi miss at `context.go:18` came from a supported factory return
that was lost in an `if` initializer. The universal Go producer now searches
control initializers and nearer lexical bindings before outer parameter aliases.
Unknown locals, callback factories, closure parameters, and type-switch aliases
must not borrow a same-named outer type or package function. Case-specific
type-switch narrowing remains unsupported. Existing parameter/import evidence
still supports project-wide field and return resolution. Disposable AST cache
semantics advance from 3 to 4; users must rebuild old Go graphs to get these
edges. Published history is unchanged.

The first frozen development binary is retained under
`go-initializer-provenance`, SHA256
`bd0168b9d1258199a23820c1888aba577a41869fec820baeb5d9a54db2a4087d`.
Fresh paired runs `go-initializer-chi-01` and `go-initializer-cobra-01` use
unchanged question suites and pinned sources. Text scores remain **Chi 11/11
versus 10/11**, and **Cobra 10/10 versus 9/10** (Compass versus Graphify).
These scores did not improve. Concurrent builds make these runs unsuitable
for latency claims. The Graphify launcher/package provenance limitation above
still applies.

Every graph relationship delta was inspected, using source/target declaration
identity, relationship kind, exact site, and multiplicity. Chi adds two calls
and removes no relationships:

- `URLParamFromCtx` to `Context.URLParam` at `context.go:18`, supported by
  `RouteContext`'s declared `*Context` return and the `if` initializer.
- `compressResponseWriter.Flush` to the `compressFlusher.Flush` interface method
  at `middleware/compress.go:364`, supported by the type assertion at line 363.
  This proves an interface-method target, not the runtime implementation.

The four original Chi pair/occurrence witnesses now all match both tools;
Compass previously matched three. A separately recorded **post-output** fifth
witness checks the compression interface call: Compass matches it; Graphify
has both unique endpoints but lacks the call. This diagnostic selection is
not an independent precision or recall sample, and does not replace panel A.

The same complete-delta review caught a regression on Cobra: four new type
references to `Command` at `command.go:479,521,921,1153` came from invoking
callbacks returned by `UsageFunc`, `HelpFunc`, and `FlagErrorFunc`. The outer
invocation has no proven named target. A native regression reproduced the
extra edge (two site edges instead of one). The producer now omits that
unsupported outer call candidate while retaining the separately visited inner
factory call. The intermediate binary and its faulty output remain recorded;
the four references are not counted as gains.

The checked-in `go_receiver_development_review.json` records intermediate run
digests, every delta, source anchors, and judgments.
`edge_witnesses_go_receiver_chi_diagnostic.json` preserves the expanded diagnostic
separately from the original registered witness file.

The corrected frozen binary has SHA256
`bb09a5335c80fd7dd710e008b4a2f30f9403108e4883ed74e7d7c0de1b0aaca9`
under `go-initializer-final-provenance`. Fresh paired repeats
`go-initializer-chi-02` and `go-initializer-cobra-02` complete all 42 requests.
Chi retains exactly the two reviewed additions; Cobra removes exactly the four
intermediate false references and has no relationship delta from its baseline.
Text scores and the five-pair diagnostic result are unchanged. The delta script,
graph/run digests, source excerpts, and both failed/passing callback regression
logs are retained. These are development results after panel A.

Final-source native checks pass **1,410 tests, zero failed, two ignored**:
workspace library/binary tests plus `universal_evidence`, `universal_resolution`,
`contracts`, and `compass_product`. Workspace and the same integration Clippy
selection pass with warnings denied. All **95 Python tests**, formatting,
product boundary, and diff checks pass. Both full fixture qualification runs
completed with exit zero, including semantic, topology, Markdown, and React
fixture checks. The second run started after the final Go production edit.
These gates do not cover the subsequently discovered Chi route-parent defect
below. Logs use `go-initializer-qualification-02`, `go-initializer-native-03`,
`go-initializer-clippy-02`, and `go-initializer-python-02` prefixes. Existing
core unused-mut and macOS linker warnings remain in build/test logs.

### Panel A MCP extension: communities, neighbors, and hubs

Commit `3ec3732a` registered 60 MCP requests before executing them on the original
panel-A graphs and frozen hub-evidence Compass binary. This is a **development
extension after observing CLI output**, not a new independent confirmation panel.
It uses the separate Graphify MCP environment, checking its recorded package
files before and after execution. This does not retroactively attest the CLI
extraction environment. All 60 expected requests completed; raw transcripts,
input digests, and the exact question multiset were checked.

| Stored-graph diagnostic | Compass | Graphify |
| --- | ---: | ---: |
| Counts match | 5/5 | 5/5 |
| Complete largest-community membership | 5/5 | 5/5 |
| Missing community handled | 5/5 | 5/5 |
| Selected call-neighbor label/direction triples match | 5/5 | 5/5 |
| Ambiguous name remains ambiguous | 5/5 | 4/5 |
| Returned hub identity established | 50/50 | 41/50 |
| Direct hub connectivity summary matches | 50/50 | unavailable |

Graphify silently selects the Chi `Context.URLParam` method for `URLParam`,
despite the separate top-level declaration. It preserves the other four
ambiguities. Compass preserves all five, but substring matching produces broad
candidate lists, including documentation nodes; this is not a candidate-precision
win. Neighbor consistency also does not establish source-edge correctness:
Compass text omits call-site occurrences, while Graphify prints retained sites.
The frozen Chi graph still has the already-documented receiver miss.

Largest-community sizes are Chi **96/108**, Click **389/84**, jsoup **752/295**,
Redux **271/71**, and WalkDir **116/47** (Compass/Graphify). These are different
partitions on different extracted graphs, not shared correctness denominators.
The reviewed file distributions include many tests: Compass Redux's largest
community includes 156 nodes from `test/createStore.spec.ts`; Graphify WalkDir's
includes 45 from `src/tests/recursive.rs`. Neither fact alone establishes good
or bad functional cohesion.

The complete post-output hub-role census verifies 50 Compass and 41 Graphify
source identities; nine Graphify display identities remain unresolved. Compass
uses explicit MCP IDs here; Graphify's CLI JSON can supply IDs, so the nine
unknowns are specific to this MCP display, not missing graph identities. Compass
returns 22 production types, seven production callables, eight test helpers,
two test types, five test modules, two source modules, two documentation-tooling
callables, and two route records from a test/example. The different returned
sets cannot establish a shared design-quality score. The roles, source excerpts,
file hashes, and all unknowns are preserved in `hub_role_reviews_panel_a.json`.

Full answer text totals **125,033/42,550 bytes**, with **174,128/45,989 actual
response-wire bytes**. Hub responses alone total **24,401/1,473 text bytes** and
**69,504/1,973 wire bytes**. Compass supplies more metadata and different members;
these costs do not prove greater efficiency. Concurrent compilation excludes
latency claims. `mcp_panel_a_review.json` records all denominators and artifact
digests. The collector now accepts safely named repositories from the captured
run instead of a hard-coded five-name list; all **97 benchmark tests** pass,
including duplicate-record and path-escape regressions.

### Source defect exposed by a consistent hub summary

Compass's second Chi hub is the route `GET /users/1` inside `TestCleanPath`,
with degree **59** from **68 outgoing containment records** and no other
incident relationship kinds. Its graph-consistent summary faithfully exposes
unsupported hierarchy. For example, it claims to contain `GET /` inside
`TestThrottleBacklog`, although both tests independently construct a local
`r := chi.NewRouter()` and neither mounts the other's route.

The separately recorded negative witness fails on both the original panel
graph and the newer Go-corrected graph. Graphify has no matching route endpoint
identities, so it cannot receive credit for the negative. The source review
traced the defect to shared publication applying filesystem parent selection
to programmatic receivers named `r`. This can inflate hubs and connect unrelated
tests; passing self-graph consistency and existing fixture gates did not detect
it. The recorded real-source negative is not a population precision estimate.

A native regression reproduced the defect. Publication now admits only
recognized filesystem-convention facts to that parent-selection step;
programmatic mounts/groups remain owned by framework composition rules. Tests
cover six programmatic frameworks, input-order reversal, a positive filesystem
case, and mismatched origin/rule/framework negatives. All **17 framework-route
tests** pass. Framework-pack semantics advance from 6 to 7 and build-state seals
include that identity. Existing filesystem conventions still
need broader independent semantic review; this correction is not proof of their
complete correctness.

#### Fresh development comparison after the hierarchy correction

The frozen `route-hierarchy-provenance` binary has SHA256
`872aff05f6c1723d27ee96230a4e315d3f0d6e1849387de09530b907f0ff3f40`.
`route-hierarchy-panel-a-01` builds both tools afresh on all five pinned sources
and repeats every original question. All **110 requests ran**, with zero
timeouts; nonzero exits remain in the denominator. Text scores remain **46/55
for both**, with no changed pass/fail rows. Source-path matches remain **6/10
versus 8/10**. Corrected selected-pair relationship matches are **16/21 versus
20/21**, and full occurrence agreement is **16/21 versus 17/21**. Compass's
one-pair improvement over the original panel comes from the earlier Go receiver
fix, not the hierarchy correction. Original and corrected Click witnesses are
still separate.

A complete relationship delta against the Go-corrected Chi graph removes
**88 `contains` records**, adds none, and retains all 729 nodes. The other four
repositories have no relationship changes from the original panel. The recorded
independent-router negative now passes with both Compass endpoints present;
Graphify's endpoints remain unavailable. This is source-backed defect recovery,
not a new broad recall score.

`mcp-panel-a-02` repeats all **60 MCP requests**, each successfully, against the
fresh graphs. All earlier consistency and ambiguity outcomes are retained.
Compass's two Chi route hubs disappear from the top ten, replaced by the
production type `compressResponseWriter` and test helper `bigMux`; both new
source roles were inspected. The other four hub rankings are unchanged.
`hub_role_reviews_panel_a_after_routes.json` retains all 100 rows and only reuses
earlier judgments after exact identity, anchor, excerpt, and file-hash equality.
Chi's selected largest Compass community changes from 96 to 90 nodes; complete
enumeration still does not prove functional cohesion. Compass/Graphify answer
text totals are **125,192/42,550 bytes**, with **174,698/45,989 wire bytes**.
No latency or efficiency win is claimed.

Verification passes **1,431 native tests, zero failed, two ignored**, covering
workspace libraries/binaries and the selected universal-evidence, resolver,
cache/contracts, product, framework-route, and framework-qualification tests.
The first expanded Clippy invocation exposed an existing `expect_err` in a
qualification test. An explicit `Err(MissingRoute)` assertion replaced it;
all four qualification tests were rerun and the full selected Clippy invocation
then passed. All **97 benchmark tests**, formatting, diff, and product-boundary
checks pass. The subsequent full fixture qualification **failed** at its topology
floor: 1,270 edges versus a required 1,284. The previous Go fixture pass does not
verify this production change. The follow-up below retains that failure and
finds an additional semantic defect.

#### Fixture topology review and a remaining false filesystem parent

The full run passed its native scale checks, source-integrity and repeated-build
comparisons, and existing semantic assertions before stopping at the count
floor. It did not reach the subsequent Markdown and React qualification stages.
Fresh clustered production builds of the identical fixture corpus with the
frozen Go-receiver and route-hierarchy binaries isolate exactly **19 removed
`contains` records, zero added records**, and the same **1,276 nodes**. Node
contents are unchanged except for community assignments. The removed records
represent 18 unique typed endpoint pairs.

Source inspection covers every removed relationship:

| Independent fixture groups | Removed records |
| --- | ---: |
| Drupal entity-type hook and YAML routes | 4 |
| Express apps in separate modules | 3 |
| Flask factory and separately instantiated app | 3 |
| Independently instantiated FastAPI apps | 2 |
| Vue Router route arrays in separate modules, both directions | 2 |
| Kotlin and Java Spring controllers in separate packages | 4 |
| TanStack and React Router route modules | 1 |

The [complete review](../../benchmarks/agent_query/route_hierarchy_fixture_review.json)
records every removed identity, both binary and graph hashes, source hashes,
and every policy adjustment. Each count bound changes by exactly its measured
before/after delta, retaining its previous margin. Community count changes
from 225 to 232 and component count from 221 to 229; neither change establishes
better functional cohesion.

Count recalibration alone is insufficient. Qualification manifest version 3
adds eight source-reviewed independent-route assertions. Both endpoint files
must retain route nodes; the specified directed containment must be absent at
every confidence level. The old graph fails all eight groups. The corrected
production graph passes **seven and fails one**: a convention-derived
`react-router::PAGE::/tanstack` still contains `/home`, with the child endpoint
remapped to the AST route. These flat source modules do not declare that
parent-child relationship.

The remaining resolver helper selects the first other route module found in a
nearby directory. The frontend source oracle's `routeParent` helper uses the
same rule. Their agreement therefore cannot independently validate framework
parentage. **The semantic gate remains failing**; no successful full rerun is
claimed. Fixing this requires framework-specific, source-proven parent rules
and independently specified positive and negative fixtures.

All **87 Python script tests** pass, including 23 code-graph oracle tests. On a
synthetic graph with the remaining false edge removed solely for oracle
validation, restoring each of the 20 known false records individually fails
the new assertion. This mutation exercise validates the checker; it is not a
production correction. Existing native results describe the unchanged Rust
production code, not a fix for this newly exposed defect.

#### Framework-specific filesystem parents and independent source review

The next correction replaces directory-order selection with an indexed lookup
of framework parent roles: Next layouts, the supported Svelte layout facts,
Nuxt parent pages, and default React Router/Remix/TanStack flat-route names.
Pages, standalone HTTP endpoints, and Astro routes no longer establish parents
by proximity. Ambiguous nearest parents remain unresolved. Custom configuration
and extraction gaps are documented in the framework reference; this change does
not claim complete routing support. Framework semantics advance to version 8;
the product version remains 0.3.30 and historical graphs remain immutable.

The frozen binary SHA256 is
`9efe548537b106c69d1da7a5ac7955e764e43103e0d1e4a754a3c1880c292bea`.
On the **identical original fixture corpus**, it removes exactly **12 more
unsupported containment records**, adds none, and retains all **1,276 nodes**
unchanged except for community assignments. All common edges are unchanged.
The removed records are two React Router sibling links, two Next page-parent
links, five Astro links, one Nuxt endpoint link, and two Svelte page/server
links. The source-reviewed negative manifest now covers **18 file pairs** and
passes on the production graph. Restoring each of these 12 false records
individually makes the checker fail.

Adding two explicit Next layout fixtures is measured separately: the sources
add 16 nodes and 24 edges to the corrected original graph, including four
expected layout relationships. On identical expanded sources, old versus new
production removes 12 false links and adds the missing root-to-nested-layout
link. No other common edge or node content changes. The frontend fixture gate
requires exactly the four source-specified layout pairs; omitting any one fails
its mutation check. The topology bounds move by the measured original-before to
expanded-after deltas, preserving every previous margin. The
[complete fixture review](../../benchmarks/agent_query/semantic_route_parent_fixture_review.json)
separates the production delta, fixture addition, mutation checks, and each
policy adjustment. Increased fragmentation is not evidence of better cohesion.

The JavaScript source oracle now uses a separate pairwise convention predicate,
with 15 manually specified cases in both input orders. The native route matrix
contains 23 cases, also in both orders. Agreement is still insufficient by
itself: the historical Next, React Router, and TanStack hierarchy scorecards
are explicitly **invalidated**, retaining their old counts and digests until
their sources are independently reviewed. Pinned hierarchy qualification
cannot use those records as passing evidence.

A real Next diagnostic uses 273 files (1,398,028 bytes) projected read-only from
shadcn/ui commit `a87a63b2ca25143d26c8bd0903e4e9bc77b3f824`. Four positive and
three negative pairs were recorded before inspecting either graph for this
projection. Old production passes **6/7** and corrected production **7/7**.
The complete graph delta contains **13 removed** links (nine to HTTP handlers,
two page-to-layout reversals, and two loading-to-layout reversals) and **four
added** AppLayout-to-nested-layout links. Every changed pair was source-reviewed;
the 14,762 nodes are unchanged except for communities and all common edges are
unchanged. This is a source-selected development diagnostic over a partial
repository, not held-out evidence or a graph-wide precision estimate.

Fresh paired builds on the unchanged five-language panel produce byte-identical
Compass graphs to the preceding route correction. The same 110 requests yield:

| Measure | Compass | Graphify |
| --- | ---: | ---: |
| Query text oracle | 46/55 | 46/55 |
| Source-checked paths | 6/10 | 8/10 |
| Selected source relationships | 16/21 | 20/21 |
| All reviewed occurrences, corrected Click oracle | 16/21 | 17/21 |

The [development review](../../benchmarks/agent_query/semantic_route_parent_development_review.json)
records pins, binary and graph hashes, source witnesses, and raw-artifact hashes.
This correction does not improve the panel scores or prove superiority. The
shared-machine timing run overlaps other verification and supports no speed
claim. MCP scores from the earlier run are not presented as a fresh rerun.

Verification: **1,436 native tests passed, zero failed, two ignored** before a
trivial Clippy needless-borrow correction; the final production binary built
successfully. Workspace library/binary and selected-integration Clippy passes.
A broader invocation including the untouched `react_frontend` test failed on
39 existing `unwrap`/`expect` lint violations; its failure log is retained.
All **88 script tests**, **15 JavaScript oracle cases**, and **97 benchmark
tests** pass, as do formatting and the product-boundary check. The first full
fixture invocation stopped because its default parser-source path was absent.
A second invocation uses the existing parser bundle in this worktree's target
directory and completed with exit 0. It passed the scale, semantic, topology,
Markdown, lifecycle determinism, and release frontend qualification stages.
This full pass qualifies production commit `c20db15b`; the pinned hierarchy
scorecards remain invalidated and require separate source review.

#### Java constructor receiver correction

The earlier frozen-binary diagnostic and failed native regression reproduced
the jsoup constructor receiver gap. They remain retained, including the initial
graph-review mistake (`relation` versus graph-v1 `kind`). The producer now reads
the constructor's AST type for direct calls and up to seven parenthesis wrappers,
then uses existing qualified-type and overload resolution. It rejects anonymous
and enclosing-instance receiver assumptions. A separate correction retains
`outer.new Cleaner()` as unresolved instead of binding an unrelated imported
`Cleaner`. AST cache semantics advance from 4 to 5; product version, graph and
evidence schemas, and advertised capabilities are unchanged.

On identical three-file Java diagnostic sources, the old binary produces two
invented external method targets. The correction removes those two nodes and
three false relationships, and adds nine source-supported calls. The native
regression checks exact name spans, repeated occurrences, argument overloads,
type/value namespace distinction, provenance, direction, and reverse-input
determinism. `javac`/`javap` independently corroborate the fixture's target
distinctions; compiled classes were not executed. The deliberately deep receiver
is a real call beyond bounded inference, **not a true absence negative**. A copy
of the old cache rebuilds to a graph byte-identical to a clean extraction.

Fresh paired builds at the same five repository pins and all 110 unchanged CLI
requests are recorded in `java-constructor-panel-a-01`. The other four Compass
graphs are byte-identical to the preceding checkpoint. jsoup retains all 6,116
nodes and common edges, except community assignments, and adds **122 call
occurrences** (10 production, 112 test). Complete delta review checks each
receiver, package/import context, target declaration, overload argument types,
source span, and direction. This is static review of a development delta, not a
representative precision sample or proof of runtime calls. It also exposes
existing incomplete varargs display signatures; the exact source declarations
support those edges, while signature completeness remains follow-up work.

| Measure | Compass | Graphify |
| --- | ---: | ---: |
| Query text oracle | 46/55 | 46/55 |
| Source-checked paths | 6/10 | 8/10 |
| Selected source relationships | 17/21 | 20/21 |
| All reviewed occurrences, corrected Click oracle | 17/21 | 17/21 |

The one newly matched selected pair is `Jsoup.isValid` to
`Cleaner.isValidBodyHtml`; both tools previously missed it. The
[development review](../../benchmarks/agent_query/java_constructor_development_review.json)
records executable, graph, source and artifact hashes, all added occurrences,
target declarations, and remaining limitations. Reusing these repositories after
they informed the fix is development evidence, even though the suite filename
contains `heldout`. No new MCP or god-object/community quality claim follows.
Timings overlap native verification and support no speed claim.

Verification: **1,439 native tests passed, zero failed, two ignored**. Clippy first
found redundant error wrapping in the new test; after that test-only correction,
workspace library/binary plus selected integration Clippy and the focused
regression pass. The fixed qualification corpus has zero node/edge changes
excluding communities, so no topology thresholds changed. Full Java fixture
qualification completed with exit 0, including native scale ceilings, semantic
and topology checks, lifecycle determinism, Markdown checks, the release build,
and frontend precedence/positive/negative checks. This pass qualifies Java
production commit `22814e59`; it does not establish comparative superiority.

#### Export binding query resolution and traversal cache fidelity

Redux's `miniKindOf` has separate export and function records. The graph already
contains its three reviewed local calls, but ordinary name lookup previously
refused the export/function pair as ambiguous. Lookup now removes a redundant
export candidate only when complete exact, nondeferred `contains` and `exports`
evidence proves one declaration at the binding occurrence. The declaration must
already be a candidate with the same normalized name, nonempty qualified name,
and source file. Genuine competing declarations remain ambiguous; exact export
IDs retain their original meaning. Proof is bounded to 256 candidates and 1,024
examined edges plus one truncation probe. Exhaustion retains the original
candidates and reports incomplete typed resolution.

A failed-before regression also exposed the compact traversal reader retaining
only the first evidence confidence and dropping `deferred`. It now retains the
weakest confidence across evidence and the compatibility field, plus deferred
state. Disposable traversal cache magic advances from `TRAILT05` to `TRAILT06`;
published graphs, identities, schemas, and AST cache semantics remain unchanged.

Fresh paired extraction and all 110 unchanged requests on the same Go, Python,
Java, TypeScript, and Rust pins produce **byte-identical graphs for both tools**
relative to the Java checkpoint. The three newly passing text questions are
Redux callees and the two directions of the undirected `miniKindOf`/`ctorName`
path. Source review confirms the three local callees and the call occurrence;
the reverse traversal does not claim a reversed call.

| Measure | Compass | Graphify |
| --- | ---: | ---: |
| Query text oracle | 49/55 | 46/55 |
| Source-checked paths | 8/10 | 8/10 |
| Selected source relationships | 17/21 | 20/21 |
| All reviewed occurrences, corrected Click oracle | 17/21 | 17/21 |

The unchanged relationship oracle still cannot identify four Redux pairs
uniquely because export and function records remain distinct. These failures
are retained; they are neither four missing calls nor recovered extraction
relationships. Both WalkDir ordinary-name path witnesses still fail both tools:
Compass preserves genuine ambiguity, while Graphify returns an unreviewed
route. A separate post-output source census reviews 126 coincident Redux
bindings; this establishes static correspondence, not 126 successful public
queries or representative precision. The
[development review](../../benchmarks/agent_query/export_binding_development_review.json)
records source, executable, graph, oracle, and raw-artifact hashes.

Verification: **1,180 native tests passed, zero failed, two ignored** across
workspace libraries/binaries and the selected export, cache, traversal, store,
CLI query, and product integration targets. The four export regressions include
direct SQLite, JSON, generic materialized store, cold/warm projection, preserved
ID/ambiguity behavior, and proof exhaustion. The cache regression includes
mixed-confidence order, deferred state, and stale cache rebuilding. Workspace
and selected-integration Clippy pass with warnings denied. Full fixture
qualification for production commit `9b63873a` completed with exit zero,
including native scale ceilings, semantic/topology checks, lifecycle
determinism, Markdown, release compilation, and frontend precedence,
positive/negative, and independent source-anchor checks. The release binary
and qualification log hashes are recorded in the development review.
No MCP rerun, full-answer precision, community cohesion, god-object quality,
directed/long-path, latency, or general superiority claim follows. This reused
panel remains development evidence.

#### Directed call-path development comparison

[Five source-reviewed chains](../../benchmarks/agent_query/directed_path_development_registration.json)
were committed as `bc4374a7` before their first execution: Chi registration to
radix-prefix matching (four calls), Click file opening to binary-reader testing
(four), jsoup validation to safe-node copying (two), Redux kind detection to
constructor-name inspection (two), and WalkDir entry handling to loop-error
creation (three). Both tools receive identical short endpoint names and the
same verified stored graphs. Compass uses `node` with depth eight and one path;
Graphify uses `path --directed`. Each command has the same external 60-second
deadline and 16 MiB capture ceiling. Their internal work limits differ.

All 20 requests completed: five positive requests and five reverse-direction
probes per tool. Positive **source-supported directed call chains score 2/5
for each tool**, with different failures:

| Repository | Compass | Graphify |
| --- | --- | --- |
| Chi | Refuses interface/implementation name ambiguity | Returns a five-hop mixed-reference/method route, not a call chain |
| Click | Returns the four-call source chain | Returns the four-call source chain |
| jsoup | Refuses two genuine `isValid` declarations | Reports no directed route |
| Redux | Refuses function/import/module name ambiguity | Returns the two-call source chain |
| WalkDir | Returns the three-call source chain | Returns a seven-hop mixed-reference/method route, not a call chain |

The Compass Click path selects the second `_is_binary_reader` occurrence at
line 188, while the frozen witness names line 181. Both occurrences were visible
in the source window reviewed before execution. The alternate is explicitly
adjudicated after output; the original witness is unchanged. Literal frozen
occurrence-site agreement is **Compass 1/5, Graphify 2/5**. The separate 2/5
source-supported figure accepts this valid occurrence under the registered
policy and does not claim complete occurrence recall.

Compass's ambiguity refusals are safe but do not complete the requested positive
paths. Graphify's mixed routes do not satisfy a call-chain request; this alone
does not establish that every structural relationship in those routes is false.
Neither tool returns a reverse path. Compass reports three ambiguities and two
bounded direction mismatches; Graphify reports five missing directed paths.
These receive **no global unreachability credit**. Forward source witnesses
cannot prove global reverse absence.

The [review](../../benchmarks/agent_query/directed_path_development_review.json)
retains the executable/graph/registration hashes, raw outputs, identities,
source spans, alternate-occurrence adjudication, and all failures. Its first
audit attempt stopped because it assumed Rust occurrences covered only the
terminal method name; the corrected check requires the exact qualified source
expressions. No graph, request, or witness changed. This is selected development
evidence on reused graphs, with concurrent fixture qualification and no speed
claim. It establishes no directed-navigation lead.

#### Terminal-cursor harness correction and remaining broad-query gap

The benchmark previously treated any `next=` token as a continuation, including
the published terminal marker `next=none`. Redux's broad question therefore
received an unnecessary second request and an invalid-cursor error. The harness
now accepts one pagination footer, stops at `none` or a repeated token, and
ignores source/prose occurrences. Regressions cover real cursors, multiple
footers, legacy/discovery/typed footer forms, and LF/CRLF. All **99 benchmark
tests pass**. Graphify's documented budget continuation is unchanged.

A query-only replay runs all 110 unchanged requests with the same binaries,
graph digests, source pins, and source working directories. Scores remain
**Compass 49/55, Graphify 46/55**. The sole outcome change removes the invalid
Redux follow-up: the question still fails its `miniKindOf` recall requirement,
now with exit zero and zero follow-ups. Every retained Compass capture is
byte-identical to its baseline page. Three Graphify query outputs vary in edge
order/content within their output budgets while scores and byte counts remain
unchanged; no whole-answer equivalence or causal claim follows.

The first replay used the Compass checkout as its working directory, making
eight source excerpts unavailable. That attempt is explicitly invalidated and
retained, along with its later summary-generation error. No cache defect or
token improvement is inferred from it. The successful second replay preserves
both tools' original working-directory setup. A failed byte-equality assertion
also remains documented; it exposed the three Graphify variations rather than
a scoring change. The
[correction review](../../benchmarks/agent_query/cursor_harness_correction_review.json)
records exact runner, executable, graph, source, and raw-capture provenance.

A separate post-output diagnostic leaves the product gap open: the original
Redux question reads 431 candidates in 73 probes but admits zero seeds and
reports bounded truncation. Shorter `kind of`, `kindOf`, `miniKindOf`, and
qualified-name questions recover source-located candidates. These are diagnosis
inputs, not replacements for the original failed question or extra comparative
passes. The specificity filter and phrase recall need a native reduction and
broader positive/negative evaluation before changing their behavior.

During this investigation, help output incorrectly displayed an examined-edge
default of 128 even though runtime JSON and `DiscoveryLimits` use 10,000. A
numeric-prefix replacement intended for the 1,000 returned-edge ceiling also
matched 10,000. The help correction requires the closing delimiter. A native
regression failed before the change and checks both help entry points against
the runtime defaults. Verification passes **1,107 native tests, zero failed,
two ignored** across workspace libraries/binaries and help/product integration
targets. Workspace and help-integration Clippy pass with warnings denied, as do
formatting, diff, and the product boundary. Runtime query limits and graph/query
schemas do not change. Full fixture qualification was not repeated for this
help-only correction; the recorded pass qualifies query/cache commit `9b63873a`.

#### Java varargs reduction: signature and call recall failures

A [compiler-valid reduction](../../benchmarks/agent_query/fixtures/java_varargs/VarargsDemo.java)
now isolates the varargs issue found during jsoup review. Expected declarations
and call targets were written before extracting this diagnostic. `javac
17.0.8.1` compiles it, and `javap` descriptors/instructions independently confirm
the target overloads; compiled classes were not executed.

The frozen export-binding binary preserves only **two of five** reviewed
declaration signatures: the boolean overload and ordinary arrays. Both
varargs-only `join` declarations render `join()`, and the prefixed overload
omits its trailing varargs parameter. More seriously, it preserves only **two
of five** reviewed calls: the boolean call and the uniquely named prefixed call.
Calls with string arguments, integer arguments, and an explicit string array
are missing. This reduction emits no wrong target for those three calls.

The [diagnostic review](../../benchmarks/agent_query/java_varargs_diagnostic_review.json)
retains exact identities, source and graph hashes, compiler evidence, and the
unfixed failures. Initial inspection looked for a top-level signature; corrected
inspection reads `details.data.signature`. At that checkpoint, code inspection
suggested the producer assumed every spread parameter had a named `type` field;
AST inspection and a failed-before native regression were still required. This
was a post-jsoup-output development reduction. The correction below preserves
that original failure record.

## Java varargs correction and repeated development comparison

The native failed-before regressions confirmed the earlier diagnostic: the
pinned Java grammar represents a spread type as an unnamed child and its name
inside a variable declarator. The producer now preserves that type, its array
rank, the spread signature, and parameter references. Explicit array creation
arguments retain their dimensions. The resolver checks strict fixed-arity,
loose fixed-arity, then variable-arity applicability and requires sufficient
evidence for a unique overload. Receiver parameters do not add call arguments;
array parameters do not borrow their element class as a method receiver.
AST cache semantics advance from 5 to 6; the product remains 0.3.30.

The original five-call reduction now resolves all five calls. A larger,
compiler-checked reduction exposes both missing and wrong targets:

| Selected source occurrences | Before | After |
| --- | ---: | ---: |
| Correct target | 6/25 | 24/25 |
| Wrong target | 5/25 | 0/25 |
| Missing | 14/25 | 1/25 |

Four wrong targets treated array arguments as scalar strings; another selected
boxing before primitive widening. `javac`/`javap` confirm the expected targets;
the classes were not executed. The remaining missing call passes a method
result with no proven type in Compass. Its preserved ambiguity is a known
recall gap, not a negative success. These deliberately constructed development
cases do not estimate population precision.

The fresh paired five-language run `java-varargs-panel-a-02` completed all 110
requests with the original questions and witnesses. Text checks remain
**49/55 Compass, 46/55 Graphify**; reviewed source paths remain **8/10 each**.
Selected relationship identity checks remain **17/21 versus 20/21**;
all-occurrence checks remain **17/21 each** with the disclosed corrected Click
witness. This repair produced **no score gain on those existing questions**.
All four non-Java Compass graphs and all five Graphify graphs are byte-identical
to the export-binding checkpoint.

The pinned jsoup graph keeps 6,116 nodes and grows from 21,095 to 21,110 edges:
43 signature corrections, nine added calls (five production, four test), and
six added production type references, with no removed relationships. Every
changed signature and added relationship was reviewed against the pinned
source, including overload sets, receiver declarations, inheritance, and
occurrence spans. This reviews the entire observed delta, not the remaining
graph or community responsibilities.

An upgrade from copied AST-v5 artifacts extracts both files again; a subsequent
run reuses both AST-v6 entries. Links, node fields excluding community labels,
and community member partitions match a clean build. The initial byte-equality
assertion failed because community numeric IDs are remapped against prior
state. The two upgraded graphs are byte-identical to each other; whole-graph
equality with the clean build is not claimed.

Verification so far: 1,092 workspace library/binary tests passed with two
ignored; 250 product/cache/resolution contract tests and nine focused tests
passed; workspace and focused-test Clippy passed with `-D warnings`; format,
diff, product-boundary, and all 99 benchmark harness tests passed. The full
fixture qualification passed against final production sources, including release
frontend precedence, activation, determinism, and source-anchor checks. The first
final-source attempt was interrupted during release compilation; its missing
handle and absent process were confirmed before retrying. Its partial log is
retained and is not counted as a full pass. Review found
that a missing optional type vector could incorrectly prove a zero-parameter
declaration; that regression failed before an added exact-match guard and passed
afterward. The earlier fixture run passed but is superseded by that source
change. The second frozen comparison reproduces all ten first-run graph hashes
and the same scores. An additional array-receiver regression passed for spread,
ordinary-array, and trailing-dimension parameters without changing production
code. The first
harness discovery command pointed at the wrong directory and ran zero tests;
the corrected command ran all 99. The
[development review](../../benchmarks/agent_query/java_varargs_development_review.json)
records binaries, sources, graph hashes, all source judgments, cache differences,
and retained failures. No speed, fresh held-out, new MCP/directed-path, community
quality, or god-object diagnosis claim follows from this checkpoint.

## Responsibility explanation evidence on the five-language panel

Commit `232608ee` froze five natural-language questions and 20 source-backed
implementation facts before either tool answered those questions. This is a new
development arm on previously evaluated repositories, with the final Java
comparison graphs reused. It is not held-out or representative evidence.
Both tools received the same question, a requested 2,000-token budget, the
source checkout as their working directory, and no follow-up allowance.
All ten requests completed successfully.

| Complete fact coverage in the returned response | Compass | Graphify |
| --- | ---: | ---: |
| Explicit correct implementation facts | 0/20 | 0/20 |
| Sufficient returned evidence for the full fact | 0/20 | 0/20 |
| Total stdout bytes across five requests | 39,740 | 53,306 |

These native queries primarily return graph context. Names, navigation anchors,
and partial relationships remain useful, but they do not establish such facts
as shared router state, a file wrapper's exception behavior, listener snapshots,
or symlink-loop conditions. The result does not show fabricated prose answers;
there were no task-level mechanism assertions to score for prose precision.
Four Compass responses request resolution; the Click response returns
candidates. A source-reading/disambiguation workflow remains to be compared.
Compass `explain` advertises verified source; Graphify's other public operations
and a symmetric agent source-reading workflow are not excluded by this arm.

Graphify's WalkDir response explicitly reports that its complete answer exceeds
the requested budget. All 26,609 bytes were retained, including 244 edge rows;
equal requested budgets did not produce equal answer sizes. No latency claim
is made while compilation shares the machine.

Post-output inspection identifies one wrong Graphify call target:
`IntoIter::get_deferred_dir` is linked to `IntoIter::pop`, but the receiver at
that source occurrence is the `Vec<DirEntry>` field `deferred_dirs`. Compass
keeps the valid call to `skippable` and has no corresponding wrong internal
`pop` edge. That does not recover the external vector call or establish overall
precision. The remaining additional node/edge assertions have not all been
source-reviewed; full response precision remains incomplete.

The [frozen questions](../../benchmarks/agent_query/responsibility_questions_panel_a.json)
and [fact review](../../benchmarks/agent_query/responsibility_review_panel_a.json)
preserve exact source witnesses, raw-response hashes, all omissions, the budget
discrepancy, and the diagnosed edge. God-object responsibility judgments,
functional community quality and overall superiority remain unproven.

### Bounded source follow-up for responsibility questions

The [follow-up protocol](../../benchmarks/agent_query/responsibility_source_followup_panel_a.json)
was frozen in `62f60b29` before the source windows were read. Both sides get
one contiguous window of at most 8,000 bytes, beginning at the earliest exact
subject anchor returned by their initial query. A read requires one unique
file. All matching declaration anchors remain visible; choosing the window
origin does not resolve declaration identity. Oracle witness ranges do not
select the window.

| Sufficient evidence after query plus source read | Compass | Graphify |
| --- | ---: | ---: |
| Chi | 2/4 | 2/4 |
| Click | 4/4 | 4/4 |
| jsoup | 3/4 | 3/4 |
| Redux | 1/4 | 3/4 |
| WalkDir | 1/4 | 1/4 |
| Total | 11/20 | 13/20 |
| Source payload bytes | 35,692 | 35,692 |
| Initial query plus source payload bytes | 75,432 | 88,998 |

Graphify's two additional Redux facts concern reducer state updates and the
listener snapshot. Its returned anchor starts at implementation line 86;
Compass also returns overload declarations at lines 41 and 75, so its window
starts earlier and ends before those mechanisms. This is a workflow advantage
under the specified policy. It does not establish that omitting overloads is
universally correct. Compass likewise retains WalkDir's associated type alias
at line 538 and struct at line 566; neither ambiguity is silently resolved.

The [review](../../benchmarks/agent_query/responsibility_source_followup_review_panel_a.json)
records all 40 evidence judgments and source/response hashes. Strict containment
of every complete frozen witness yields 10/20 and 12/20. Semantic review credits
one additional Chi fact per tool: the complete middleware construction and
shared pool/tree assignments appear before the partial last line, although the
window omits the later return in the witness. jsoup attribute filtering remains
incomplete because the window ends before the destination write and rejection
branch. These distinctions are preserved rather than treating partial ranges
uniformly as successes or failures.

All ten reads succeeded. Click's window reaches EOF at 3,692 bytes; every other
window returns 8,000 bytes. The collector reads bounded whole files to validate
and hash them before slicing; payload bytes do not measure disk IO. No model
generated an explanation. This development arm measures evidence available to
an agent under one reading policy, with unequal initial query response sizes.
It does not measure best-possible workflows, source-reading latency, native
explanation quality, community quality, or god-object detection.

### Literal identifiers in natural questions

The responsibility responses exposed a retrieval gap: Click's question names
`_AtomicFile`, but generic `close` methods occupy all three seed slots. Commits
`090140f5` and `6711e3a3` add bounded declared-name lookup for underscore and
mixed-case compound identifiers embedded in prose. Ordinary words retain their
lexical rank; a missing compound does not turn partial names into exact hits.
All lookups share the existing work bounds, and incomplete name postings or
candidate admission cannot establish uniqueness.

The first implementation exposed a second problem in the real Redux response:
the top `createStore` seed lacked an ambiguity flag while the other same-name
declarations had one. A regression with same-name declarations of different
kinds failed before correction. Exact source-name collisions now stay ambiguous
across ranking evidence; heuristic ranking orders them without resolving their
identity. Matched identifier components are retained even when later recall
channels cannot admit more candidates.

The final fixed-graph rerun executes all 110 existing suite requests and the ten
responsibility queries against the same graph files and source pins. The
existing recall proxy remains **49/55 for Compass and 46/55 for Graphify**, with
no verdict changes or timeouts. The responsibility diagnostics show:

- Click's first seed is now `_AtomicFile`, with exact-name provenance and a
  matched identifier component. Generic secondary candidates and partial
  coverage remain.
- All three Redux `createStore` declarations are exact-name seeds and all
  three carry ambiguity warnings.
- WalkDir's associated alias and `IntoIter` struct are both exact-name seeds
  with ambiguity warnings.
- Chi `Mux` and jsoup `Cleaner` outputs remain byte-identical. Single-word
  capitalized subjects still use the earlier ranking.

These are anchor-selection diagnostics on a development panel, not a new
explanation score. The earlier source-follow-up score belongs to its frozen
pre-change workflow. The
[development review](../../benchmarks/agent_query/literal_identifier_development_review.json)
records both candidate runs, failed-before regressions, final verification,
source and binary hashes, and actual response sizes. The first workspace
verification batch was intentionally stopped for the ambiguity correction and
is not counted as a completed baseline. Extraction was not rerun for these
query-only changes. Full response precision, new source-follow-up coverage,
held-out confirmation, god-object diagnosis and community quality remain open.

### Source-defined community task pairs

Commit `234753eb` freezes
[30 exact declarations](../../benchmarks/agent_query/community_task_pairs_panel_a.json)
and their source mechanisms before this task-pair membership audit. Earlier
comparisons had already exposed parts of these graphs, so this is a development
diagnostic. The three task pairs per repository yield 15 within-task pairs and
60 cross-task pairs. Both tools use the unchanged captured native graphs.

All 30 declarations resolve uniquely on both sides. The
[review](../../benchmarks/agent_query/community_task_pairs_review_panel_a.json)
records identities, memberships, source witnesses and whole-community sizes.

| Repository | Compass collaborator pairs co-located | Graphify collaborator pairs co-located | Compass cross-task pairs co-located | Graphify cross-task pairs co-located |
| --- | ---: | ---: | ---: | ---: |
| Chi | 3/3 | 3/3 | 0/12 | 0/12 |
| Click | 2/3 | 3/3 | 2/12 | 0/12 |
| jsoup | 2/3 | 0/3 | 0/12 | 0/12 |
| Redux | 3/3 | 3/3 | 4/12 | 0/12 |
| WalkDir | 3/3 | 3/3 | 12/12 | 12/12 |
| Total | 13/15 | 12/15 | 18/60 | 12/60 |

These columns describe different tradeoffs. They are not a combined accuracy
score, and cross-task co-location is not a false-positive count. Only two of
the 15 collaborator pairs cross source files; both tools co-locate those two.
The purposefully small task labels do not partition all source responsibilities.

Source review of all five split collaborator pairs finds the expected direct
call edge in the corresponding graph:

- Compass splits Click `term_len` from `strip_ansi`, while placing `term_len`
  with help-table layout. The source shows `measure_table` using `term_len`,
  so there is a concrete reason for that cross-task grouping.
- Both tools split jsoup `isBlank` from `isWhitespace`.
- Graphify also splits jsoup `clean` from `copySafeNodes`, and
  `parseBodyFragment` from the selected three-argument `parseFragment`.

The five boundaries are not missing-call findings. They motivate testing
navigation to collaborators outside a community. Likewise, both tools place
WalkDir's handle budgeting, deferred-directory output and symlink-loop methods
in one community. Those mechanisms share iterator state; this does not prove
excessive responsibility. Whole-community size comparisons also reflect
different extraction granularity, including fields, parameters and containers.

The bounded auditor reproduces all 150 tool/pair outcomes after hardening input
validation. All **109 developer-harness tests** pass, including ten new tests
for exact identity, ambiguity, missing assignments, integer-zero communities,
bounds and deterministic pair outcomes. A separate direct recomputation checks
the captured IDs and pairs, but it is not an independent semantic reviewer.
No Rust code or clustering algorithm changed in this iteration. Public
community/navigation workflow costs, broader membership precision, positive
god-object evidence, independent review and fresh confirmation remain open.

### Public community-to-neighbor workflow

Commit `7a6d6c97` freezes the
[one-follow-up protocol](../../benchmarks/agent_query/community_navigation_panel_a.json)
before capture. Every task starts at the community containing its first
source-defined declaration, prepared symmetrically from each tool's graph.
Starting-community discovery is not scored. The neighbor selector comes only
from returned member text matching the public seed file and terminal symbol.
Distinct matching labels remain unresolved; no expected ID is substituted.

The [complete review](../../benchmarks/agent_query/community_navigation_review_panel_a.json)
records all 30 tool/task outcomes and the source-backed failure distinctions.

| Measure | Compass | Graphify |
| --- | ---: | ---: |
| Community lists matching stored membership | 15/15 | 15/15 |
| Unambiguous follow-up label selected | 13/15 | 15/15 |
| Seed identity supported after one lookup | 5/15 | 9/15 |
| Direct collaborator identity supported | 4/14 | 8/14 |
| Neighbor calls reporting ambiguity | 8 | 6 |
| Required direct-call pairs present in graph | 13/14 | 14/14 |

All 58 executed tool calls succeeded at the protocol/tool level. Two Compass
lookups were skipped because multiple distinct member labels matched. Successful
seed responses matched their stored displayed adjacency. That consistency does
not establish the precision of every returned edge. Chi's request-ID pair shares
context state and is excluded from the direct-call denominator.

The failures expose separate improvement opportunities:

- Compass's `get_neighbors` consumes the full exact/prefix/substring candidate
  list from `find_node`. Unique displayed names `RequestID()`, `term_len()` and
  `.follow()` still produce ambiguity with broader matches such as
  `NextRequestID()`, `test_term_len()` and `.follow_links()`.
- Compass's Redux communities contain several member labels for the same seed
  file and terminal symbol. Without declaration lines or IDs in the member
  text, this policy cannot select the intended declaration.
- Click's `__exit__` and the selected jsoup names collide in both tools. Their
  ambiguity responses often expose candidate IDs, and Graphify suggests
  `path::symbol`. A longer disambiguation workflow remains a valid unmeasured
  alternative; these failures do not prove navigation is impossible.
- Compass's successful WalkDir `push` response omits the reviewed call to
  `DirList.close` at `src/lib.rs:906`. Both declarations are uniquely present,
  but the captured Compass graph lacks that edge. Graphify stores and displays
  it. This graph gap is separate from community grouping or selector ambiguity.

Neither tool reaches its split collaborator pairs under this policy: Compass
has two such pairs, Graphify three. The earlier source review found the required
edges in all five cases. These different subsets are not equal denominators.

The 60-second/1-MiB external response bounds are common, but native controls
are unequal: Graphify receives a generous explicit token budget and Compass
exposes whole results. Actual call text totals are 145,715 versus 51,605 bytes;
call response wire totals are 150,769 versus 55,173. Including initialization,
listing, requests and stderr, the session totals are 238,216 versus 94,443 bytes.
Different community sizes and success counts prevent a matched-success output
or latency efficiency claim. No cap or timeout occurred.

All **119 developer-harness tests** pass, including ten new tests for selector
ambiguity, duplicate identities, direction and distinct-neighbor multiplicity.
A separate same-agent script checks all saved requests/responses, graph and
support-file hashes and identity/count summaries; it is not an independent
semantic reviewer. No Rust code changed or Rust tests ran in this iteration.
The workflow is development evidence on reused repositories, not a held-out
result, god-object diagnosis or overall superiority claim.

### Exact-first neighbor lookup correction

Commit `8f5eb5e5` fixes the broad-match ambiguity demonstrated above.
`get_neighbors` delegates first to the existing `find_exact_nodes` query helper;
only an empty exact candidate set uses the broader lookup. Exact IDs retain
case and precedence. Genuine normalized-name collisions remain ambiguous and
keep the same bounded, stable candidate list. The implementation reuses the
shared evidence-gated export-binding behavior rather than adding a second
resolver. Graphs and clustering do not change.

The [complete rerun review](../../benchmarks/agent_query/neighbor_exact_match_review_panel_a.json)
uses the same frozen 15 tasks and one-follow-up policy. All ten graph hashes,
source declarations, selected labels and community texts are unchanged. Every
Graphify neighbor response is byte-identical to the baseline.

| Measure | Compass before | Compass after | Graphify both runs |
| --- | ---: | ---: | ---: |
| Seed identity supported | 5/15 | 8/15 | 9/15 |
| Reviewed direct collaborator supported | 4/14 | 6/14 | 8/14 |
| Neighbor responses reporting ambiguity | 8 | 5 | 6 |

Chi `RequestID`, Click `term_len` and WalkDir `follow` now resolve their exact
seed. The latter two expose their reviewed direct collaborators; the request-ID
pair has no direct-call requirement. No task loses a previously supported seed
or collaborator. Graphify remains ahead in this specific arm.

Two Compass Redux member selectors still have multiple distinct labels. Click's
`__exit__`, the three jsoup seeds and Redux's `bindActionCreators` still produce
genuine neighbor ambiguity. WalkDir's `push -> DirList.close` edge remains
missing in the frozen Compass graph. The next navigation arm should use each
tool's documented source-qualified or exact-ID handles and examine whether
neighbor results identify the selected declarations. This label-only arm does
not measure the best possible longer agent workflow.

All 58 executed tool calls succeed. Call text totals are 139,718 bytes for
Compass and 51,605 for Graphify; complete captured session bytes are 232,183
and 94,443. Unequal community sizes and success counts still preclude a
matched-success efficiency claim. The public inputs, outputs and product
version remain compatible at 0.3.30.

Verification:

- Both failing exact-priority/collision-count regressions are preserved in the
  initial log; the four focused neighbor tests then pass.
- All 58 MCP tests pass, including 38 library tests and 20 integration tests.
- Workspace library/binary tests: **1,101 pass, two ignored**. The MCP library
  tests are included in this count, not additive.
- Product contract tests: **nine pass**. Workspace formatting, Clippy with
  warnings denied, product boundary and the CLI build pass.
- Developer harness: **120 tests pass**. A comparison verifier exposed unstable
  ordering in older missing-neighbor diagnostic lists. The auditor now sorts
  missing/extra lists; the failing regression and initial capture remain.
  Final recapture preserves all public responses and verdicts. Older diagnostic
  list ordering is canonicalized only for comparison, not source interpretation.
- The first full MCP run also exposed a stale transport assertion from before
  the shared renderer's compact `RESULT` header. Commit `500a4565` updates it
  and verifies text equality with the rendered structured Agent View. The
  failed run remains recorded; the complete rerun passes.

Validation logs retain the existing core-test `unused_mut` warning and a macOS
linker unwind-table warning. No extraction, language or viewer code changes, so
those full qualification/JavaScript gates were not rerun. Source assertion
precision, god-object judgments, broader community usefulness, longer walks
and held-out confirmation remain incomplete.

### Source-coordinate-assisted public navigation

Commit `f660408b` freezes a stronger
[public resolver workflow](../../benchmarks/agent_query/community_identity_navigation_panel_a.json)
before capture. Both tools receive the exact task seed file, declaration start
line and terminal symbol. Starting communities are still prepared symmetrically.
After finding matching member rows, Compass uses structured `search_symbols`
results and Graphify uses `get_node` with its documented `path::symbol` form.
Only a uniquely source-matched returned ID becomes the next `get_neighbors`
input. The expected collaborator is scoring-only and cannot gate requests.
Each task permits one resolver and one neighbor lookup after the community.

The [complete review](../../benchmarks/agent_query/community_identity_navigation_review_panel_a.json)
records every task and the provenance of its selected ID.

| Measure | Compass | Graphify |
| --- | ---: | ---: |
| Correct returned seed ID | 15/15 | 15/15 |
| Completed neighbor lookup with that ID | 15/15 | 15/15 |
| Reviewed outgoing collaborator label present | 13/14 | 14/14 |
| Unambiguous target label after validated seed lookup | 10/14 | 11/14 |

This is meaningful counterevidence to treating the earlier label-only failures
as inability to navigate. Existing public APIs can resolve all selected seeds
with the additional source-assisted step. No product code changed in this arm,
so the stronger results are not attributed to an extraction improvement.

A completed lookup here means the returned ID matches the frozen declaration,
the request uses that ID, and the response has the expected successful heading.
It does not mean the legacy neighbor body explicitly reports its seed ID.
Likewise, Click's `close`, jsoup's `parseFragment`, and jsoup's `isWhitespace`
labels each map to multiple target declarations in both graphs. Their displayed
labels are not counted as unique destination identities. The earlier name-only
identity audit remains separately recorded; these different metrics must not
be silently substituted for one another.

The only reviewed direct pair absent from the frozen Compass graph is still
WalkDir `push -> DirList.close` at `src/lib.rs:906`. Graphify stores and displays
it. All five earlier split-community pairs now expose the collaborator label
(two Compass, three Graphify); these unequal subsets remain descriptive.
All community memberships and displayed neighbor multiplicities agree with
their stored graphs. That does not establish precision for every extra edge.

All **90 public tool calls succeed** with no timeout or cap failure. Compass
call text totals 156,976 bytes and response-wire totals 694,354; Graphify totals
55,402 and 60,410. Including initialization, listing, requests and stderr, the
session totals are **785,989 versus 102,213 bytes**. These are real costs of this
fixed workflow. Compass search supplies multiple candidates, source anchors and
richer structured evidence, while Graphify's lookup returns one node. Native
limits and semantic payloads differ; these totals are not a universal efficiency
ranking or an equal-token result.

All **129 developer-harness tests** pass. Eight new resolver tests cover exact
anchors, collisions, truncation, schema errors and returned-ID provenance. A
missing-target-oracle regression ensures seed navigation can still be scored
when no expected destination ID is available. Final recapture after removing
target-oracle availability from workflow control flow retains all 90 response
packets and 30 task audits identically. A separate same-agent verifier checks
raw requests/responses, graph hashes, source-coordinate identity and adjacency;
it is not an independent semantic reviewer. Rust tests were not rerun because
this iteration changes only evaluation code/docs and reuses the validated binary.

Next work should address the missing Rust indexed-receiver call, exact
destination identities in public call/navigation output, and bounded resolver
cost. Natural-language seed discovery, broader source precision, god-object
responsibility evidence, longer directed walks and held-out confirmation remain
open; the supplied declaration coordinates make this a different task from
unassisted discovery.

## Rust indexed-receiver recovery

The development protocol is frozen in
`benchmarks/agent_query/rust_index_receiver_development_registration.json`
(commit `6c70df5e`). Implementation `18bb37ea` follows bounded Rust receiver
syntax through source-proven scalar indexes into standard vectors, arrays and
slices. Field types retain their declaration context; nested qualified names
retain every module segment. Ambiguous imports/layouts, custom containers,
ranges, unknown index types, raw pointers and root-container method fallbacks
cannot establish an element-method target. AST cache semantics advance to 7;
the package stays at 0.3.30 and published graph schemas/history remain unchanged.

All five Compass graphs were rebuilt from the same pinned source commits with
the original native-only arguments into fresh outputs. The frozen Graphify
graphs were retained byte-for-byte; both public MCP workflows were recaptured.
This arm does not compare extraction timing. The review and artifact hashes are
in `benchmarks/agent_query/rust_index_receiver_development_review.json`;
complete logs, graphs and transcripts are under `rust-index-receiver-03`.

The complete graph comparison finds **one added call and no removed or changed
existing records**: WalkDir `IntoIter::push` calls `DirList::close` at
`src/lib.rs:906`. Source establishes `stack_list: Vec<DirList>` and
`oldest_opened: usize`; the target method begins at line 1008. A separate
same-agent verifier checks the full graph-record delta, source bytes, caller
range and target identity. All five node arrays and community assignments are
unchanged; the other four Compass graphs are byte-identical. This is evidence
for the changed call, not precision for every existing graph assertion.

The original 15 tasks, source witnesses, selectors and limits remain unchanged:

| Source-assisted navigation measure | Compass before | Compass after | Graphify |
| --- | ---: | ---: | ---: |
| Correct seed ID and completed lookup | 15/15 | 15/15 | 15/15 |
| Reviewed outgoing collaborator label | 13/14 | 14/14 | 14/14 |
| Globally unambiguous target label | 10/14 | 11/14 | 11/14 |

All 90 public calls succeed. All 45 Graphify response packets are identical to
the prior capture. Compass changes one neighbor text to include `close`; three
WalkDir resolver packets change only graph identity/view digests. The other
41 packets are identical. Compass totals 157,011 call-text bytes, 694,390 wire
response bytes and 786,025 full-session bytes; Graphify remains at 55,402,
60,410 and 102,213. Native payloads and controls differ, so these are actual
workflow costs rather than an equal-token efficiency ranking.

Three target labels remain ambiguous for both tools: Click `close`, jsoup
`parseFragment`, and jsoup `isWhitespace`. The fix closes a known extraction
gap and produces a tie on this development panel. It does not establish overall
superiority, god-object diagnosis, broad source precision or held-out quality.
Explicit destination identities, source-grounded explanations and longer
source-verified directed walks remain next work.

Final validation against unchanged implementation `18bb37ea` passed: formatting,
38 Rust language tests, 211 universal resolver tests, workspace Clippy, 1,101
workspace tests (2 ignored), 9 product tests, the product-boundary check and the
complete production fixture qualification. The Python benchmark harness passed
129 tests. Source hashes remained unchanged before and after each native gate;
the frozen evaluated binary is byte-identical to the final build. Commands,
counts and log hashes are recorded in the review artifact. Broader real-repository
qualification and fresh held-out evaluation were not run in this development arm.

## Explicit neighbor identities and relationship evidence

Protocol `bde19bb3` freezes this development arm before implementation
`d51698dc`. It reuses all ten graph artifacts and the same five pinned source
repositories from the Rust recovery arm. All 15 tasks retain their original
community, seed resolver and neighbor calls. A new symmetric arm permits one
additional destination resolver call on each of the 14 direct-call tasks,
gated on an actually returned outgoing collaborator label. Both tools receive
the same target file, declaration start and terminal symbol for that follow-up.
This is source-assisted navigation on reused tasks, not held-out discovery.

Compass neighbor results now carry `compass.query.neighbors/1`: exact seed and
destination node records and all matching relationship records. The query layer
preserves parallel records, self loops, endpoint direction, source anchors and
provenance under explicit adjacency/record/byte bounds. Text retains compact
neighbor lines and adds escaped identity/source details. The graph artifacts,
extraction, source trees, package version and historical realizations do not
change in this arm.

A separate same-agent verifier recomputes every request, response, source-anchor
selection and full incident-record multiset. All 15 Compass projections match
their graphs, totaling 181 record appearances across the requests. That includes
records beyond the 14 reviewed source collaborators and is **graph consistency,
not broader source precision**. All 45 prior Graphify payloads and all 30 Compass
community/seed resolver payloads match the baseline, ignoring request IDs that
shift after the extra calls. All 15 Compass neighbor payloads change.

| Development measure | Compass | Graphify |
| --- | ---: | ---: |
| Correct seed ID and completed neighbor lookup | 15/15 | 15/15 |
| Reviewed outgoing collaborator label | 14/14 | 14/14 |
| Globally unambiguous target label | 11/14 | 11/14 |
| Explicit target ID in direct neighbor response | 14/14 | Unavailable (0/14) |
| Correct target ID after one extra source-anchored resolver | 14/14 | 14/14 |

Before this change neither tool emitted explicit neighbor IDs. Graphify's
neighbor response still emits labels, relations and source sites; the lack of a
full record projection is not a wrong-edge judgment. Its additional public
`get_node` call resolves all 14 source-matched targets, including the three
ambiguous labels. Compass's richer direct response avoids that extra identity
lookup for these tasks, but the extended workflow remains a tie. The extra
resolver uses supplied target coordinates; it does not autonomously reconstruct
an ambiguous edge from its label. All 14 reviewed pairs on both sides also have
the required directed call in their frozen graph.

All 118 public calls succeed. With all four steps executed symmetrically,
Compass uses 193,385 text bytes, 1,228,383 response-wire bytes and 1,323,516
full-session bytes. Graphify uses 57,404, 63,756 and 107,645 respectively. For
just the original three steps, response-wire totals are 1,017,434 versus 60,415
bytes. These are observed costs with different native payloads and controls;
there is no equal-content efficiency or latency claim.

Artifact hashes, commands and limitations are recorded in
`benchmarks/agent_query/neighbor_identity_development_review.json`; raw captures
and logs are under `neighbor-identity-02`. Final validation passed: formatting,
3 focused query tests, all 60 MCP tests, workspace Clippy, 1,106 workspace tests
(2 ignored), 9 product tests, the product-boundary check, complete production
fixture qualification and the final CLI build. The Python harness passed all
134 tests. Runtime/test hashes stayed unchanged during validation and match
`d51698dc`; the evaluated binary is byte-identical to the final build. The gate
retains its existing fixture-omission and compiler warnings in the logs.
God-object responsibility judgments, richer source explanations, broader
assertion precision, longer directed walks and fresh held-out confirmation
remain outstanding.

## Directed endpoint identity and depth-limit correctness

The registered development replay reuses the five source-reviewed two-to-four
call chains in Chi (Go), Click (Python), jsoup (Java), Redux (TypeScript), and
WalkDir (Rust), with all ten graphs frozen from `rust-index-receiver-03`.
Both tools receive the same endpoint file, start line and symbol. One public
MCP resolver call per endpoint returns candidates; only a unique exact source
match may supply an ID to the directed CLI path request. No graph ID is used to
construct requests. The protocol was committed as `98759ae8`, before capture.
This is source-assisted navigation on known development tasks, not autonomous
endpoint discovery, fresh extraction or held-out evidence.

A separate native-label control was registered at `2cc02501` after observing
ID-input failures, before executing the control with the final binary. It uses
only the original five short-name pairs, with no retries or candidate changes.
Its results are kept separate: taking the best answer from either arm would
misrepresent both protocols. External deadlines and stream bounds match, but
native internal work limits differ; Graphify exposes no matching CLI depth
control. There is no timing or efficiency ranking.

| Source-supported directed call chains | Compass | Graphify |
| --- | ---: | ---: |
| Public resolver IDs supplied to CLI | 4/5 | 1/5 |
| Original native short labels | 2/5 | 2/5 |

The ID workflow resolves four Compass endpoint pairs and all five Graphify
pairs. Compass returns the reviewed chains for Chi, Click, jsoup and WalkDir;
Redux remains unresolved because `search_symbols` returns a function and an
export with the same source line and name. The frozen selector refuses to pick
one. Graphify succeeds for Redux; its Chi route starts at `Mux` rather than the
resolved `MethodFunc` and mixes references/membership with calls. It reports
no path for Click, jsoup and WalkDir with these ID inputs. Inspection of the
pinned Graphify CLI confirms it passes IDs through label scoring without an
exact-ID check. This arm measures endpoint handling across public interfaces
as well as path availability; it is not a pure path-search comparison.

The native-label control preserves Graphify's Click and Redux successes.
Compass succeeds for Click and WalkDir and refuses three ambiguous requests.
Graphify's Chi and WalkDir routes mix structural relationships with calls;
jsoup reports no directed route. Those structural routes are not credited as
call chains, but this does not establish that the structural edges themselves
are wrong. Every successful chain was checked against exact declarations,
edge orientation, occurrence sites and pinned source. Compass selects a second
valid Click call at line 188 rather than the frozen line 181; literal frozen-site
agreement is therefore 3/5 and 1/5 for its ID and label arms, versus Graphify's
1/5 and 2/5. Conditional static call chains do not imply guaranteed runtime
execution sequences.

The replay also exposed a correctness defect independent of the paired score:
a failed depth-limited directed search could claim a direction mismatch using
a shorter undirected route, even while a longer forward route existed. A native
regression reproduced that claim before the fix. A second regression reproduced
`explore` dropping incomplete status when no connecting path was returned.
The query layer now checks whether the depth frontier remains open after a
failed bounded search and preserves that status through exploration. Frontier
checks share the work budget and occur after the positive search, so they cannot
consume the budget of a still-viable shorter route. Closed dead ends and cycles
can still prove a complete negative result within the graph.

All nine registered low-depth requests changed from `truncated: false` to
`truncated: true` with `bounded_truncation`; none now claims `direction_mismatch`
or `no_match`. All four positive Compass ID-path payloads and all ten Graphify
forward/reverse payloads are byte-identical to baseline. The path scores above
were already present before this fix: the improvement is truthful bound reporting.
Reverse probes receive no source-global absence credit. In particular, Chi's
reverse diagnostic combines a closed directed search with an incomplete
undirected search that nevertheless finds a connection; its bounded flag
remains visible.

A separate same-agent verifier checks saved public request/response transcripts,
source-coordinate selections, CLI arguments, graph/source/binary hashes, full
paths and typed provenance projections. Its first attempt incorrectly required
raw graph and typed query evidence to have identical JSON structure; the retained
failure led to checking their explicit field/anchor projection instead. It is
not independent human adjudication. Raw captures, both native reproductions,
the failed verification attempt and the final verifier are retained under
`directed-identity-01` and `directed-identity-02`; artifact hashes and detailed
outcomes are in `benchmarks/agent_query/directed_identity_development_review.json`.

Final validation passed: formatting, 24 focused query tests (including the
2,187-request four-node oracle), 38 CLI query tests, 60 MCP tests, workspace
Clippy, 1,106 workspace tests (2 ignored), 9 product tests, product boundary,
complete production fixture qualification and the final CLI build. The Python
harness passed 137 tests. Validated source hashes match implementation
`1ec835bf`; the evaluated binary is byte-identical to the final build. Existing
fixture-omission and compiler warnings remain in the retained logs. No version
or machine-schema bump was made.

God-object responsibility judgments, richer explanations, broader source
precision, longer walks and fresh held-out confirmation remain unproven.

## Native responsibility explanations and source verification

Registration `4e6ccb25` froze a native `explain` comparison for the existing five
responsibility subjects and 20 implementation facts. It reuses the ten graphs
from `rust-index-receiver-03`. Both tools receive the same short subject name
and run in the pinned source checkout. A separate arm supplies the same subject
file, start line and symbol to one public MCP resolver, then passes a unique
source-matched returned ID to `explain`. No graph lookup chooses requests and
there are no retries or external source follow-ups. These are known development
questions, not held-out evaluation or automatic god-object judgments.

Compass receives a 2,000-token connection budget and an 8,000-byte source cap;
Graphify's native `explain` has no equivalent flags. Both have the same external
120-second timeout and 16 MiB stream caps. Source availability is an observed
capability of these commands, not an equal-I/O or efficiency comparison.

| Complete responsibility facts in returned evidence | Compass | Graphify |
| --- | ---: | ---: |
| Native short-label `explain` | 7/20 | 0/20 |
| Source-coordinate-assisted `explain` | 7/20 | 0/20 |
| Earlier query plus symmetric bounded source read (separate workflow) | 11/20 | 13/20 |

Neither native command authors a mechanism explanation: explicit native
responsibility assertions remain **0/20 for both**. Compass's excerpts contain
all four Click `_AtomicFile` facts and three jsoup `Cleaner` facts. Their full
frozen source witnesses are returned, including the otherwise easy-to-misread
Click exception behavior: the `delete` argument is not inspected by `close`.
The Java excerpt ends at a partial line 206, before the attribute write needed
for the remaining cleaning fact; it receives no partial-fact credit.
Graphify returns useful relationships and source anchors, but no source excerpts
sufficient for these complete implementation facts. It can still support an
agent source-reading workflow, as the separate 13/20 result demonstrates.

Graphify resolves all five subject identities in both arms. Compass resolves
three native subjects and four assisted subjects. Native `createStore` is
ambiguous across 27 source-backed candidates; its assisted search exceeds the
frozen 256-candidate bound and is refused. Native `IntoIter` is ambiguous between
an associated type alias and a struct; the supplied line resolves the struct in
the assisted arm. Go `Mux` and Rust `IntoIter` declaration excerpts include their
fields, but their separately defined methods remain outside those source spans.
Names and method relationships do not establish the missing responsibility
facts. This identifies a concrete next explanation gap: gathering the relevant
implementation evidence beyond the subject's declaration span.

Native stdout totals are 24,476 bytes for Compass and 6,286 for Graphify;
Compass includes 9,620 raw source bytes. In the assisted arm, the totals are
24,308 and 6,286, with 11,448 Compass source bytes. These totals exclude resolver
traffic, whose raw transcripts are retained. A source excerpt is evidence rather
than an authored answer. Other printed graph relationships have not all received
source review; these fact scores are not full response-precision scores.

Inspection exposed a separate provenance defect: `explain` labeled every source
excerpt `digest-verified`, including nodes without a stored digest. A native
regression reproduced that label, and checks cover same-length source changes,
matching and mismatching full-span digests, truncated returned prefixes, absent
files, malformed digests and all three CLI output formats. The query layer now
returns an explicit verification flag. A missing digest produces an unverified
current excerpt; malformed or mismatching digests prevent source output. The
existing containment and bounded-read primitives are retained. Verification
matches recorded bytes; it does not authenticate the graph or its semantics.

A separate post-output Redux diagnostic selects the `kindOf` function from an
already captured public resolver response by its explicit kind and source site.
That diagnostic does **not** change the frozen ambiguous selection policy or any
paired score. The published function has no source digest. Its old explanation
claims verification; the new explanation says `unverified: no recorded source
digest`. All remaining output, graph bytes and source bytes are unchanged.

All 19 paired explanation payloads and all ten resolver response payloads are
unchanged after the provenance fix: the selected excerpts in the comparison
already had matching digests. Thus the 7/20 evidence result is an observation
about existing native capabilities, not an improvement attributable to this fix.
A separate same-agent verifier checks saved requests/responses, commands,
source/graph/binary hashes, every rendered source line and the full-span digest.
It reuses the frozen resolver helper and separately verifies selected source
anchors; it is not independent human adjudication.

Implementation `3a58309f` passed formatting, 4 query source tests, 39 CLI query
tests, all 60 MCP tests, workspace Clippy, 1,106 workspace tests (2 ignored),
9 product tests, the product-boundary check and a final CLI build. Validated
source hashes match the commit, and the evaluated binary matches the final
build. Extraction/viewer qualification and JavaScript gates were not rerun:
extraction, resolution, publication and viewer assets are unchanged. No benchmark
library changed, so the Python harness was not rerun; the new external collectors
and evidence verifier completed successfully. The nonfatal macOS linker warning
is retained in the reproduction log. Version remains 0.3.30.

Detailed judgments and artifact hashes are in
`benchmarks/agent_query/native_explanation_development_review.json`; captures are
under `native-explanation-01` and `native-explanation-02`. God-object diagnosis,
responsibility synthesis, broader source precision, longer walks and fresh
held-out confirmation remain outstanding.

## Recorded member source and a shared neighbor/source-window control

Registration `bac936f9` freezes the existing five subjects, 20 responsibility
facts, graph snapshots and ten public endpoint-resolver responses. This is
known development evidence. No selector substitutions rescue Compass's Redux
candidate-limit failure. The opt-in `explain --source-members` implementation
(`6d4df994`) follows recorded outgoing containment through nested types and
returns callable spans in source order under one 8,000-byte source budget.
Default declaration excerpts are unchanged. This new Compass capability has
no claimed equivalent Graphify flag; its feature delta is not a paired win.

A separate control gives each tool one public `get_neighbors` call with the
previously selected ID. It groups outgoing `contains` / `method` rows by their
returned file/start-line anchors, sorts them, and reads up to the next returned
anchor in that file (at most 4,096 bytes for its final anchor). All windows share
8,000 source bytes per subject. There are no extra pages, retries, fact-guided
selection or end-line advantages. Graphify's displayed relation sites are
checked against source declarations after capture. Windows do not establish
unique target identity. This is one reproducible agent policy, not an optimal
retrieval strategy or an equal-I/O experiment.

| Subject | Compass member mode | Compass shared control | Graphify shared control |
| --- | ---: | ---: | ---: |
| Chi / Go | 3/4 | 3/4 | 3/4 |
| Click / Python | 3/4 | 4/4 | 4/4 |
| jsoup / Java | 4/4 | 3/4 | 4/4 |
| Redux / TypeScript | 0/4 | 0/4 | 2/4 |
| WalkDir / Rust | 3/4 | 1/4 | 2/4 |
| **Complete facts supported by source** | **13/20** | **11/20** | **15/20** |

The earlier declaration-only native result remains Compass 7/20, Graphify
0/20; the earlier query-plus-source-read result remains Compass 11/20,
Graphify 13/20. Keep these workflows separate. None of these tools' native
renderings authors the requested mechanism explanations: explicit native
responsibility assertions remain zero. Source evidence is not a synthesized
answer or a god-object defect judgment.

Member mode gains seven facts and loses one against declaration excerpts.
Separately defined Go and Rust methods become available, and the Java attribute
write now fits. However, Click's recorded getter span excludes `@property`.
The initializer and getter body alone cannot establish property semantics, so
that entire fact is denied. The shared windows retain the decorator and earn
that fact for both tools. Chi's route handler remains outside the native budget;
WalkDir's loop-check body is partial. Redux remains unresolved for Compass.

Graphify leads the shared control by four facts: one Java, two TypeScript and
one Rust. Compass's returned Java field and Rust field/type anchors consume
window budget before later methods; Graphify exposes fewer such anchors. The
same policy therefore produces different coverage. No missing or partial code
is credited to close the gap.

Exact literal witness coverage is 4/20 for member mode, 10/20 for Compass's
control and 14/20 for Graphify's control. The semantic scores above separately
allow leading indentation differences at callable-span starts and missing blank
separators between complete bodies. Both controls also omit the Click class
header, whose identity is already supplied by the verified selected owner;
all initialization, decorator and getter code is present. Every exception and
rejected fact is recorded individually. This is same-agent adjudication with a
separate verifier, not independent human review.

All 54 native member spans match stored full-span digests and pinned source
bytes, including the returned prefixes of truncated members. Seven members are
omitted by the byte budget across Chi and WalkDir; no member source read fails
in this sample. All 130 returned membership anchors (69 Compass, 61 Graphify)
were checked against declaration lines. Nine saved neighbor request/response
pairs and every source window were verified. Compass reports no neighbor or
transport truncation; no Graphify truncation marker was observed. Neither fact
proves complete graph membership or general relationship precision.

Native member mode returns 23,082 source bytes in 48,177 stdout bytes. The
shared control returns 27,673 source bytes for Compass and 35,673 for Graphify;
Compass's unresolved Redux subject contributes zero. Neighbor text totals are
26,497 versus 6,980 bytes; full MCP responses total 281,242 versus 7,534 bytes.
These figures exclude previously captured resolver traffic and do not measure
latency or equal computational work.

Validation passed: formatting, 13 focused query tests, 40 CLI query tests,
60 MCP tests, workspace Clippy, 1,106 workspace tests (2 ignored), 9 product
tests, the product boundary and a final CLI build. Recorded Rust source hashes
match `6d4df994`; the final build matches the evaluated binary. JavaScript/viewer
and extraction/resolution publication gates were not rerun because those
surfaces are unchanged. Version remains 0.3.30. Discovery, metadata, source and
verification-work bounds have native regressions; stale or missing source
status remains explicit.

The first external collector failed before any public request because of an
invalid hash-helper read bound. Its log and script are retained; the corrected
capture uses a fresh directory. A verifier parser initially included the summary
heading as a member; its failed attempt is also retained. The corrected verifier
passes, including byte-to-line consistency checks. Detailed judgments and hashes
are in `benchmarks/agent_query/member_source_development_review.json`; external
artifacts are under `member-source-02`. The next explanation work needs better
source context and selection, actual responsibility synthesis, and fresh
confirmation. This result does not establish overall superiority.

## Exact source-constrained lookup and the remaining retrieval gap

Registration `2ef40e95` keeps the same five repositories, graph/source pins and
20 facts, and adds explicit identity constraints: symbol, source file, declaration
start line and stored kind. These are supplied task inputs in a new development
arm; they do not retroactively rescue the old unscoped Redux result. Prior
sources and outputs were known when this arm was designed.

Implementation `9cc6ba9e` adds `search --exact` and MCP `search_symbols` with
`exact: true`, plus optional file, start-line and kind filters. Exact IDs take
precedence; otherwise the existing normalized name index is read within the
unchanged candidate cap. All matching records survive unless an explicit filter
excludes them. No lexical fallback, inferred ownership, export-binding collapse,
or arbitrary winner is introduced. Filtering happens after the bounded index
read: a truncated singleton or empty result proves neither uniqueness nor
absence. Exact text requests keep the supplied bounds without automatic widening.

The paired identity control uses the same supplied constraints. Compass receives
its supported fields directly; Graphify receives its existing `file::symbol`
lookup, and its returned source identity and declaration kind are checked against
the constraints afterward. Graphify is not claimed to accept a native kind
filter. Each tool receives one scored lookup, followed by one neighbor request
and the unchanged registered 8,000-byte source-window policy. Diagnostic replays
and negative controls are kept separate from those scored calls.

Both tools resolve **5/5 subjects**. Compass now reaches the `createStore`
function at `src/createStore.ts:86`. Omitting the explicit `function` kind returns
both its declaration and its coincident export record; neither is silently
removed. Asking for declaration line 87 returns no match with complete lookup.
The original ten unscoped resolver response payloads are unchanged, including
Compass's old candidate-limit failure. All nine previously available neighbor
responses and source-window payloads are also unchanged.

| Subject | Compass constrained lookup + windows | Graphify constrained lookup + windows |
| --- | ---: | ---: |
| Chi / Go | 3/4 | 3/4 |
| Click / Python | 4/4 | 4/4 |
| jsoup / Java | 3/4 | 4/4 |
| Redux / TypeScript | 3/4 | 2/4 |
| WalkDir / Rust | 1/4 | 2/4 |
| **Complete facts supported by source** | **14/20** | **15/20** |

The three newly supported Compass facts concern Redux enhancer behavior,
dispatch state/reentrancy, and listener snapshots/unsubscription. Its returned
parameter anchors start early enough to include the enhancer body; Graphify's
first returned member anchor is later. Compass's final window reaches part of
line 312, and the observable/store-API fact remains unavailable. Graphify still
supplies one additional Java fact and one additional Rust fact. Thus the overall
lead in this workflow remains Graphify's, despite Compass's Redux advantage.

Strict literal witness counts are 13/20 and 14/20. Both semantic scores retain
the prior Click allowance: the missing class-header line is supplied by the
independently verified owner identity, and the initialization, decorator and
getter code are all present. Every other credited fact has complete literal
witnesses. Neither tool authors the mechanism answers; these are source-evidence
scores, not synthesized explanations or god-object judgments. The earlier
11/20 versus 15/20 unscoped-window result and 13/20 native member-mode result
remain separate.

Both tools now return 35,673 source bytes across the five subjects. Compass's
neighbor responses contain 33,061 text bytes and 372,829 full MCP response bytes;
Graphify's contain 6,980 and 7,534. Scored resolver responses add 3,039 text /
21,983 wire bytes for Compass and 619 / 1,094 for Graphify. This is not equal
compute, latency or response-size superiority. The verifier checks all 32 saved
public calls, pinned inputs, source windows and unchanged earlier payloads.
The additional 23 Redux membership anchors were source-reviewed, bringing the
reviewed membership-site total to 153; other relationship precision is unproven.

Native checks exposed and corrected two response defects during development:
new exact-mode node limits initially retained excess search hits, and an empty
truncated search failed Agent View validation by claiming `no_match` without a
no-match diagnostic. Exact mode now bounds nodes and hits together; the view
preserves unknown match with partial execution. Search display also uses the
query engine's existing name normalization. Failed fixture construction and
overly strict test assertions are retained separately from these product defects.

Final validation passed formatting, 37 targeted query/backend/binding tests,
20 output-contract tests, 41 CLI tests, 61 MCP tests, workspace Clippy,
1,106 workspace tests (2 ignored), 9 product tests, the product boundary and
CLI build. Validated source hashes match `9cc6ba9e`; evaluated and final binaries
match. JavaScript/viewer and extraction/publication gates were not rerun because
those surfaces are unchanged. Version stays 0.3.30. The separate same-agent
artifact verifier passes; this is not independent human adjudication.

Per-fact judgments, command logs, failed attempts and artifact hashes are recorded
in `benchmarks/agent_query/exact_symbol_development_review.json`, with external
artifacts under `exact-symbol-01` through `exact-symbol-08`. Source selection,
responsibility synthesis, actual god-object defect evidence, broader edge
precision, longer walks and fresh confirmation remain unfinished.

## Shared-state evidence prerequisite for cohesion analysis

Registration `1c6f2a43` freezes 20 source-selected access sites: two state slots,
each used by two methods, in each of the existing five development repositories.
The subjects and earlier graph inventories were already known; this is not a
blind or held-out evaluation. Java includes `Cleaner.CleaningVisitor`, and Redux
uses closure variables rather than class fields. These are prerequisites for
state-sharing analysis, not equivalent whole-class cohesion samples.

The diagnostic scans the same complete, hash-pinned frozen graphs for both
tools, with a 512 MiB bound per graph. It identifies endpoints by exact source
file, declaration line and symbol, retaining ambiguity. It separately checks
method-to-state contact records and occurrence provenance at the selected access
line. Calls on a field's type, containment, owner-type references and excerpts do
not count as state-access edges. It does not measure public query retrieval.

| Subject | Compass state slots represented | Graphify state slots represented | Compass access sites | Graphify access sites |
| --- | ---: | ---: | ---: | ---: |
| Chi / Go | 0/2 | 0/2 | 0/4 | 0/4 |
| Click / Python | 0/2 | 0/2 | 0/4 | 0/4 |
| jsoup / Java | 2/2 | 0/2 | 0/4 | 0/4 |
| Redux / TypeScript | 2/2 | 0/2 | 0/4 | 0/4 |
| WalkDir / Rust | 2/2 | 0/2 | 0/4 | 0/4 |
| **Total** | **6/10** | **0/10** | **0/20** | **0/20** |

All 20 accessing callable coordinates identify one node in both tools. Compass
has missing state endpoints at eight access sites and represented endpoints
with no connecting records at the other twelve. Graphify has no state endpoint
at any of the ten pinned declaration/introduction coordinates. There are no
connecting records of any kind or direction between any candidate endpoints;
the zero result is not caused by the diagnostic's relation whitelist. The
Graphify containers declare `directed: false`; saved endpoint order must not be
interpreted as native directed path support. The report preserves that flag.

These results contradict using absent method/state links as evidence of low
cohesion in these subjects. They do not show independent responsibilities,
a god-object defect, poor whole-class cohesion, or overall tool superiority.
The extra six Compass declarations do not supply the missing access evidence.
Python's state coordinates are first assignments to instance attributes, so
those rows specifically test whether the graph represents those introductions.

Code inspection locates concrete producer gaps at baseline `7aef6a0c`:

- `walk_rust_evidence` in `compass-languages/src/evidence/build.rs` emits calls,
  macro invocations and declaration references, but has no field-expression
  access emission arm. The selected Rust field declarations already exist.
- `walk_java_evidence` in that module emits calls, construction, annotations and
  type relationships, but has no ordinary field-access emission arm. Both
  selected Java field declarations already exist.
- TypeScript identifier traversal calls `emit_callable_reference`; that function
  explicitly skips local declarations without proven callable status. The two
  Redux closure variables are declared but their ordinary value uses are lost.
  Broadening that code requires a truthful value-reference contract, not
  relabeling arbitrary state as callable.

The existing member-access candidate projects to a `references` edge with
member-access provenance, so qualified field-access evidence can use an existing
relationship representation. The next production work belongs in language
fact emission and qualified resolution, with shadowing/ambiguity negatives,
precise occurrence anchors, bounded lookup, cache invalidation and affected
language qualification. Hub ranking cannot reconstruct these missing facts.
No producer, capability, runtime behavior or release version changes in this
checkpoint; the gaps remain open.

`state_access_audit.py` replays source pins, clean checkout state, exact source
witnesses, graph hashes, candidate sets, all connecting records and its own
script hash. The committed registration and review live under
`benchmarks/agent_query/`; raw development artifacts and logs are under
`state-access-01`. The first collector attempt incorrectly required a directed
container and stopped on Graphify's undirected container; retaining that flag
instead permitted the registered stored-endpoint diagnostic. No partial result
was scored. Before registration, source-coordinate assertions also caught and
corrected off-by-one Redux/Rust anchors.

The eleven focused auditor tests pass, including positive contact evidence,
shadowed targets, duplicate candidates, wrong relations/directions, occurrence
mismatch, constructor spelling, parallel records and bounded reads. The complete benchmark suite passes 148 tests. The saved
real-repository review replays byte-for-byte. No Rust/JavaScript tests or
extraction gates were rerun for this benchmark/documentation-only checkpoint;
previous production validation remains tied to its earlier commit.

## Rust field-access correction and paired navigation control

Registrations `4a1be265` and `56d7d559` fix the production contract, unchanged
20-site comparison and four known-ID-assisted public neighbor requests before
rebuilt graphs or follow-up outputs were inspected. Production commit
`390406c1` emits Rust `MemberAccess` occurrences and qualified `AccessesMember`
candidates for explicit field expressions. It reuses bounded source-type
inference and existing universal resolution, restricts targets to fields,
retains unknown/shadowed receivers as unresolved, and excludes method selectors.
Parallel occurrences keep exact field-identifier anchors. It does not infer
read/write effects, aliasing, independent responsibilities or god-object defects.

AST cache semantics advance from 7 to 8 so older facts rebuild. Evidence/graph
schemas, advertised producer capabilities and package version stay unchanged;
published historical realizations are immutable. Graphs must be rebuilt to
receive the new evidence. The CI workflow now runs both new integration suites
explicitly because its library/binary test invocations would otherwise skip them.

All five Compass graphs were rebuilt from the same pinned, read-only sources.
Graphify's frozen native graphs remain unchanged. The original 0/20 versus
0/20 report is preserved; it is not rewritten with the improved graph.

| Evidence | Compass before | Compass after | Graphify |
| --- | ---: | ---: | ---: |
| Registered state-contact access sites, all five repositories | 0/20 | **4/20** | 0/20 |
| Registered Rust access sites | 0/4 | **4/4** | 0/4 |
| State slots represented, all five repositories | 6/10 | 6/10 | 0/10 |

The four recovered sites link `IntoIter::handle_entry` and `get_deferred_dir`
to `deferred_dirs`, and `push` and `pop` to `oldest_opened`, at the registered
lines. Java and TypeScript still miss their eight selected contact edges;
Go and Python still lack the four selected state-slot declarations. Both tools
continue to identify all twenty accessing callable coordinates.

Chi, Click, jsoup and Redux graphs are byte-for-byte identical to their frozen
controls, including all communities. jsoup and Redux still report two omitted
edges each; this checkpoint does not repair those partial publications. WalkDir
keeps all 288 nodes and all 1,206 earlier edge records unchanged, adds 174
member-access references, and changes 122 node community assignments. It now
has 1,380 edge records. Recomputed clustering is an observable consequence, not
proof of improved communities. The verifier checks every added record's field
target kind, exact identifier bytes, enclosing source extent and provenance;
this is occurrence consistency rather than a compiler/type-resolution oracle
or full semantic precision review of all 174 records.

The separate public MCP arm provides each tool its own exact callable IDs as
explicit task inputs, then makes one unfiltered `get_neighbors` request per
method. All eight requests succeed. Compass exposes the field identity and the
selected source-line occurrence in **4/4** replies; Graphify exposes neither
in **0/4**. Compass's complete neighbor nodes and records match its new graph.
This is a known-ID retrieval control with native defaults, not natural-language
identity discovery or authored responsibility explanation. It is not held-out.

Compass returns 10,308 text / 139,303 full response bytes; Graphify returns
1,519 / 1,904. The larger Compass responses retain full records and repeated
occurrences; no token-efficiency or latency advantage is claimed. The scoped
four-site gain must not be substituted for the earlier five-language source
explanation comparison, where Graphify retained a 15/20 versus 14/20 lead.

All six initial extraction regressions failed before the change. The final
seven extraction tests and three resolver/publication tests cover direct and
nested fields, indexed receivers, lexical shadowing, unknown/raw-pointer
receivers, duplicate fields, ambiguous imports, depth exhaustion, trait impls,
callable fields versus method selectors, cross-file targets, parallel anchors
and input-order determinism. An integration fixture initially lacked physical
source inventory and checked the wrong raw kind key; both were corrected.
Another assertion exposed an audit wording error: `member-access` is retained
in provenance, not necessarily in the optional `context` field. The earlier
wording is corrected. Focused Clippy also caught and corrected a test-helper
type-complexity warning. Failed attempts remain in the external logs.

Final validation passed formatting, 45 Rust language integration tests,
237 resolver integration tests, 33 cache contracts, workspace and focused-test
Clippy, 1,106 workspace tests (2 ignored), 9 product tests, the product boundary,
full code-graph fixture qualification (including Markdown and the independent
React release-binary fixture gate), and 148 benchmark tests. The new CI command
also passes all ten new integration tests. The evaluated debug binary matches
the qualifying debug binary, and validated source hashes match `390406c1`.
Warnings retained in the logs include fixture publication omissions, a linker
warning and an existing unused-mut test warning; passing does not mean the logs
are warning-free. Hosted platform/packaging/browser matrices are not claimed.

Verification and artifact hashes are recorded in
`benchmarks/agent_query/rust_state_access_development_review.json`; the complete
capture, fresh graph manifests, graph deltas, raw MCP transcripts, source checks,
verifier and validation logs are under `rust-state-access-01`. Remaining Java,
TypeScript, Go and Python state evidence, actual cohesion/god-object judgments,
authored explanations and fresh held-out confirmation remain unfinished.

## Next evidence to collect

1. Re-review the invalidated pinned hierarchy scorecards from their sources.
   The corrected fixture and selected real-source evidence above do not replace
   those broader checks.
2. Extend source-proven loop/result/iterator inference to recover the remaining
   fd misses. Evaluate TypeScript identity independently of the bounded query
   binding proof above. Extend Java evidence beyond the corrected varargs
   cases, including untyped method results and unresolved receiver forms.
   Keep exact
   build/source provenance for subsequent release comparisons;
   the latest query correction has native and fixed-graph regression evidence.
3. Use the source-role census and connectivity breakdowns to review actual
   responsibilities and source-edge correctness, including containment-heavy
   modules and generic reference targets. Evaluate cluster responsibilities
   and cross-community connections separately from graph consistency.
4. Extend the development navigation-path judgments to directed call paths,
   longer walks, parallel source occurrences, broader ambiguity/unreachable
   cases, and real-repository work exhaustion. A negative or limit outcome
   must never count as a path or proof of global disconnection.
5. Use held-out repositories/questions and publish all failures, including
   competitor wins. Separate extraction gaps, resolution gaps, retrieval gaps,
   rendering gaps and oracle mistakes using actual source evidence.
6. Improve the owning production layer for reproduced failures, retain native
   regressions, then rerun equivalent questions. Report category-level evidence
   and uncertainty rather than claiming universal dominance.
