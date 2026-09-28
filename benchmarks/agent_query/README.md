# Agent query evaluation

`benchmarks/agent_query` measures how well Compass answers the agent questions
in its suites compared with Graphify on the same pinned checkouts. It is
developer-side tooling: Compass never runs it, and it never installs Graphify.

[`COVERAGE_PLAN.md`](COVERAGE_PLAN.md) tracks the broader real-repository
question/evidence matrix, including ask, communities, clusters, and god nodes.
The audit report distinguishes completed checks from surfaces still awaiting
source or design-quality judgments.

Four suites share the harness:

| Suite | Questions | Shape |
| --- | ---: | --- |
| `suite.toml` | 47 | The first five-repository suite, including Compass's compact and paged projections |
| `suite_v2.toml` | 50 | A blackbox-fair extension: same questions for both tools, default output forms, no tool-specific projections |
| `suite_fd.toml` | 12 | Separate pinned `sharkdp/fd` sample, recorded from source before either tool's first extraction/query run |
| `suite_ask.toml` | 10 | Same natural-language caller/callee questions and 2,000-token budget for Compass `ask` and Graphify `query` across five languages |

`suite_v2.toml` states its fairness contract inline and keeps it in the rows:
both tools are blackboxes over the same pinned checkout, every oracle is read
from source, each row asks the same question of the same declaration through the
closest documented operation on each side, and continuations are each tool's own
(Compass `--cursor`, Graphify `--budget`). `path` rows pass `--undirected` to
Graphify because Compass `path` searches relationships in both directions. A row
that a tool cannot answer fails and is reported as a recall gap.

Compass continuation requires one `Pagination:` footer with a nonterminal,
previously unseen cursor. `next=none` ends pagination; `next=` text in source
excerpts or prose does not authorize a follow-up. Multiple footers or a repeated
cursor stop continuation while preserving the unmet answer requirements.
Both LF and CRLF output are supported. Query-only replays must use each pinned
source checkout as their working directory, matching the full runner, so
digest-verified source excerpts remain available.

Name-resolution rows (`ambiguity`, `negative`) use Graphify's `explain`, the
command that reports its candidate list for an ambiguous name and its explicit
no-match, rather than `query`, which traverses the neighbourhood of a single
matched node. `broad` rows use a 600-token page budget on both sides: below
that, one tool's fixed metadata can consume the whole page. No v2 row repeats a
question from the first suite; the audit compares repository, kind and addressed
symbol across both files.

`suite_ask.toml` reuses reviewed caller/callee facts from v2 to compare the
natural-language interface. These are ten interface checks, not ten additional
independent source judgments. Both sides receive identical question text and
one 2,000-token response budget, with no continuations. The initial scorer checks
selected-fact recall; extra statements still need source review for precision.

The suite covers five real repositories in five languages:

| Repository | Language | Focus |
| --- | --- | --- |
| `spf13/cobra` | Go | CLI command resolution and execution |
| `pallets/flask` | Python | request dispatch into views |
| `google/gson` | Java | overloaded serialization APIs |
| `colinhacks/zod` | TypeScript | schema parse and safe-parse helpers |
| `tokio-rs/axum` | Rust | routing and service dispatch |

The first two suites contribute source-reviewed questions across `explain`,
`explain_source`, `callers`, `callees`, `impact`, `path`, `file_path`,
`ambiguity`, `negative`, and `broad` (the first suite adds the `brief`,
`brief_callers`, and `paged_callers` projections). Every question declares the
exact per-tool argument vector, the expected outcome, and the file, line, or
symbol anchors the reviewer read in the pinned checkout. Every repository also
declares reviewed declarations to look for in each graph.

## Run

```bash
python3 benchmarks/agent_query/runner.py doctor \
  --compass-binary /path/to/compass \
  --graphify-binary /path/to/graphify \
  --source cobra=/Volumes/Workspace/Github/spf13/cobra \
  --source flask=/Volumes/Workspace/Github/pallets/flask \
  --source gson=/Volumes/Workspace/Github/google/gson \
  --source zod=/Volumes/Workspace/Github/colinhacks/zod \
  --source axum=/Volumes/Workspace/Github/tokio-rs/axum/axum

python3 benchmarks/agent_query/runner.py run \
  --suite benchmarks/agent_query/suite_v2.toml \
  --workspace /Volumes/Workspace/CrabData/compass-evaluations/agent-query \
  --compass-binary /Volumes/Workspace/crabbuild-target/compass/release/compass \
  --graphify-binary "$(command -v graphify)" \
  --source cobra=/Volumes/Workspace/Github/spf13/cobra \
  --source flask=/Volumes/Workspace/Github/pallets/flask \
  --source gson=/Volumes/Workspace/Github/google/gson \
  --source zod=/Volumes/Workspace/Github/colinhacks/zod \
  --source axum=/Volumes/Workspace/Github/tokio-rs/axum/axum
```

`--suite` defaults to `suite.toml` beside the runner. Passing
`suite_v2.toml` runs the 50 blackbox questions instead.

`doctor` fails unless the source is a clean working checkout at the suite's
pinned commit and contains the reviewed files. Axum's suite uses its `axum/`
package as the source root, not the monorepo root. `run`
builds `compass extract --code-only --no-viz --store sqlite` and
`graphify extract --code-only` once per repository under
`WORKSPACE/runs/<run-id>/artifacts`, then writes `run.json` and `REPORT.md` under
`WORKSPACE/runs/<run-id>/`.

Every run builds fresh graphs and refuses an existing run ID. The retained
`--force` flag is a compatibility no-op. Prior artifact directories are never
silently reused or removed. The runner checks the pinned commit and clean Git
status before extraction and after queries, records graph digests and build
logs, and checks executable identity before and after the run. The executable
hash covers only that file: a Python launcher hash does not pin its imported
packages. Use an immutable environment when comparing installations.

## Metrics

### Source-defined community task diagnostic

`community_task_pairs_panel_a.json` records 30 source declarations grouped into
15 collaborator pairs across Chi, Click, jsoup, Redux and WalkDir. Run the
artifact audit against the registered `java-varargs-panel-a-02` capture:

```bash
python3 -m benchmarks.agent_query.community_tasks \
  --registration benchmarks/agent_query/community_task_pairs_panel_a.json \
  --run /path/to/java-varargs-panel-a-02/run.json \
  --output /path/to/new-community-audit.json
```

The output path must be new. The auditor verifies source commits, file hashes,
exact witness text and graph hashes, then records exact declaration identity
and community membership. Missing or ambiguous anchors cannot pass as separate
communities. It reports within-task co-location separately from cross-task
co-location, including same-file and cross-file strata. Community IDs are never
compared across tools. This measures grouping granularity on a small development
panel, not native answer quality, ideal architecture or god-object defects.

The [review](community_task_pairs_review_panel_a.json) retains every selected
declaration and the five source-supported calls crossing community boundaries.
Ten focused auditor tests cover ambiguity, missing assignments, identity,
bounds, zero-valued community IDs and ordering invariance.

### Query metrics

- **Correctness**: stdout with an accepted exit status and without timeout or
  output-limit failure is judged against the suite's anchors.
  Graphify `explain` deliberately returns exit 1 for ambiguity: that status is
  accepted only for a pick-list question with its explicit ambiguity header and
  at least two candidate IDs; the source-anchor oracle must still pass. Other
  nonzero exits fail. A `negative` question passes only with an explicit
  no-match signal, and a
  `pick_list` question passes only when the answer shows at least two distinct
  candidates and at least one reviewed candidate for the name; it deliberately
  does not require a specific pair, because a bounded page can only show part
  of the candidate set.
  An `answer` row requires every `required` anchor and, when it also declares
  `required_one_of`/`min_one_of`, at least that many of those alternatives, so
  a question with several source-reviewed answers (for example the Zod
  validation family) accepts any reviewed one without weakening the row.
- **Compact rows**: `brief` and `brief_callers` questions run the compact
  `compass.query.agent-view.brief/1` projection (or the typed agent path) so the
  suite reports the token cost of the lean agent answer beside the full
  projection.

- **Tokens**: UTF-8 stdout bytes divided by four, the same approximation both
  CLIs document for their text budgets. `run.json` records the first-page cost
  and the total cost of the reviewed workflow.
- **Follow-ups**: a `broad` question that misses the oracle first runs the
  documented continuation - Compass uses the `--cursor` ledger, Graphify
  re-runs with a four-times larger budget - up to `max_follow_ups`, so
  pagination and budget guessing are priced rather than hidden.
- **Paired tokens**: the aggregate table also reports token medians restricted
  to the questions where the same source-reviewed oracle passed for both tools.
  A per-tool median over each tool's own passing rows prices different
  questions; the paired number is the like-for-like comparison, and
  `run.json` carries the per-kind split of both-only, Compass-only,
  Graphify-only, and neither.
- **Latency**: wall-clock milliseconds per tool invocation, including
  follow-ups.
- **Graph coverage metadata**: node and edge counts, source-located node ratio,
  dangling edges, duplicate IDs, and reviewed declaration anchors. Metadata
  presence does not verify that the source or relationship is correct.
  `compass.agent-query-run/2` uses `exact-file-start-terminal-symbol/1` on
  both tools: exact repository-relative file, exact declaration start line,
  and case-sensitive terminal symbol name. Qualification separators and
  parameter lists are removed symmetrically; the pinned line distinguishes
  overloads. This metric does not verify owner or parameter-type accuracy.
  Enclosing module spans and unrelated names at the right line do not count.
  Each graph metric records `anchorPolicy` and `missingAnchors` for review.
  Historical v1 scores used file/line coverage without symbol identity and
  must be recalculated before comparison with v2 scores.

## Limits

### Source-grounded path diagnostics

The five positive `path` rows have a separate, stricter audit:

```bash
python3 -m benchmarks.agent_query.path_audit \
  --run /path/to/workspace/runs/run-id \
  --output /path/to/new-path-audit.json
```

`path_witnesses.json` records reviewed declaration and occurrence lines in the
pinned source. The auditor checks the printed hop chain, unique node identity,
relation, direction, captured graph edge, and source occurrence. It refuses
ambiguous display labels instead of using the expected answer to select a node.
It verifies source state, suite and graph digests, and records response, witness,
and auditor hashes. The output must be new; earlier reports are retained.

These witnesses were reviewed after observing output, so they are development
diagnostics, not held-out accuracy estimates. Gson accepts either an
instantiation at its construction line or a reference at its return-type line;
the report preserves the relation and reviewed site. Zod's file-containment
route proves navigation only. Neither is automatically credited as a call path.
The auditor currently requires successful, single-response executions of these
positive rows; it does not score negative or truncated path outcomes.

### Source-first direct-call sample

`suite_fd.toml` and `edge_witnesses_fd.json` were committed together before the
first run on `sharkdp/fd` at `b422e5d8c9cffaa1ae43ba68e7b97a60fb3e8ae5`.
The 12 questions and 10 positive direct-call pairs were selected by source
inspection. The positive pairs contain 16 call occurrences; two additional
pairs must be absent. This is a selected sample, not a representative held-out
corpus or an estimate of whole-repository precision/recall.

```bash
python3 benchmarks/agent_query/runner.py run \
  --suite benchmarks/agent_query/suite_fd.toml \
  --workspace /path/to/evaluations --run-id fd-fresh \
  --compass-binary /path/to/compass --graphify-binary /path/to/graphify \
  --source fd=/path/to/pinned-clean-fd

python3 -m benchmarks.agent_query.edge_audit \
  --run /path/to/evaluations/runs/fd-fresh \
  --witnesses benchmarks/agent_query/edge_witnesses_fd.json \
  --output /path/to/new-edge-audit.json
```

The edge auditor requires both exact declaration identities even for negative
pairs, checks semantic direction and relation, and separately scores relationship
presence and occurrence coverage. A duplicate at one line cannot recover a
missing occurrence at another. Source files, captured suite/graph digests,
pinned checkout state, and auditor code identity are checked. Reports record
missing and unexpected occurrences and require a new output path. Occurrence
matching covers start lines, not column accuracy or runtime execution.

`edge_witnesses_fd_diagnostic.json` adds five **post-output** checks for the
missing loop calls and constructor-owner mistake discovered in that run. Keep
those results separate from the preregistered sample. `path_witnesses_fd.json`
likewise audits the two positive path responses after output review using the
existing path auditor's `--witnesses` option. It verifies a compatible graph
occurrence; the text path does not identify a particular parallel edge.

### Interpretation

Anchor matching is a deterministic text-recall proxy over bounded output, not
an independent precision oracle. The suites are focused source-reviewed
samples; they do not estimate population-wide accuracy. Graphify prints an
installation warning on stderr, which `run.json` records separately and the
token metric excludes.

Each subprocess stream is capped at 16 MiB during capture. Exceeding either
cap terminates the process group and fails the observation; truncated text is
never scored as a successful response. An invalid Compass snapshot pointer
fails preparation instead of selecting an arbitrary unpublished snapshot.

## Held-out confirmation panel A

`suite_heldout_a.toml` contains 55 source-reviewed CLI questions on five new
pinned repositories. `heldout_panel_a.json` records selection scope, source
hashes, and the frozen Compass build. Run with the normal `runner run --suite`
interface and explicit `--source NAME=PATH` for Chi, Click, jsoup, Redux, and
WalkDir; keep all generated artifacts outside the source checkouts.

Run `edge_audit` separately with each `edge_witnesses_heldout_NAME.json`, and
`path_audit --witnesses benchmarks/agent_query/path_witnesses_heldout_a.json` on
the captured run. These witnesses are registered before either tool executes
the panel. The question score remains a text-recall proxy; report independent
identity/direction/occurrence checks and all failures separately. Repository
selection is purposive, so this does not estimate population accuracy.

The Go receiver development follow-up is recorded separately in
`go_receiver_development_review.json`. Its complete relationship-delta review
retains the intermediate Cobra callback regression. The additional Chi
compression-interface witness lives in
`edge_witnesses_go_receiver_chi_diagnostic.json`; it is a post-output diagnostic,
not an addition to the original held-out score. Run it with the same
`edge_audit --run ... --witnesses ... --output ...` interface.

The first frozen results and post-output review are recorded in
`heldout_panel_a_review.json` and the main code-graph intelligence audit report.

`suite_mcp_panel_a.json` extends the same frozen panel graphs to 60 MCP requests
after observing the CLI results. It is a development extension, with questions
committed before MCP execution. `mcp_panel_a_review.json` records graph-consistency
and payload results, and `hub_role_reviews_panel_a.json` records the complete
post-output source-role census. Neither establishes functional community quality
or god-object defects. `edge_witnesses_chi_route_hierarchy_diagnostic.json`
separately records an unsupported Compass containment edge discovered through
the hub review; missing Graphify route identities cannot pass that negative.
Keep the original Click edge witness: its missing second `_wrap_io_open` site
is corrected only in `edge_witnesses_heldout_click_corrected.json`. Report both
registered and corrected diagnostic scores. The path auditor retains nonzero,
timed-out, and unsupported multi-response executions as failed rows; it still
rejects source/graph provenance drift. Later product tuning on panel A is
**development**, not another held-out confirmation.

## Shared MCP comparison

`suite_mcp.json` preregisters 29 questions per tool across the same five-language
panel: graph statistics, top-ten hubs, largest-community enumeration, absent
communities, call neighbors of a reviewed declaration, and four ambiguous
neighbor lookups. These use public MCP tools on both sides. Exact input IDs and
community IDs are prepared symmetrically from retained graphs; this preparation
is not scored as node retrieval.

```bash
python3 -m benchmarks.agent_query.mcp_compare \
  --run /path/to/captured/five-repository-run \
  --output /path/to/new-mcp-run \
  --compass /path/to/frozen/compass \
  --graphify-python /path/to/isolated-graphify-mcp-env/bin/python \
  --graphify-environment /path/to/graphify-mcp-environment.json
python3 -m benchmarks.agent_query.mcp_audit \
  --run /path/to/new-mcp-run --output /path/to/new-audit.json
```

The stdio collector currently requires POSIX pipe selectors (macOS/Linux); it
fails explicitly before starting a server on unsupported platforms.

The environment manifest records `files` with package-relative `file` and
`mcpEnvironmentSha256` entries plus the installed package/version list. The
collector validates package hashes before and after collection. It records
executable, graph, input, and collector hashes, JSON-RPC transcripts, errors,
text bytes, and timing. The auditor verifies captured answers against those
transcripts and reports actual response-wire bytes separately from text bytes.
The original collector's `protocolBytes` field estimates JSON serialization
size; use the auditor's `wireResponseBytes` for captured transport size.

Full-enumeration questions use Graphify's large explicit token allowance and
Compass's whole-result interface with common external byte/time limits. This
arm does not claim equal 2,000-token answers. Community membership is compared
with each tool's own partition, not an arbitrary shared cluster number.
Neighbor checks cover displayed direction/label/relation triples and report
ambiguous labels separately. Hub checks recompute displayed degrees for
uniquely identified labels; they do not prove complete ranking eligibility,
source correctness, functional cohesion, or god-object design quality.

### Hub explanation and source-role diagnostics

`hub_evidence_audit.py --run /path/to/mcp-capture --output /path/to/new-audit.json`
checks each returned hub's optional connectivity summary against its own graph.
It distinguishes incident records, ranking degree, direction, self-loops, and
bounded relation rows. Absent summaries are unavailable, not incorrect answers;
this diagnostic does not measure a neighbor/CLI follow-up workflow. It must not
be used as a cross-tool precision score on the different returned hub sets.

The post-output `hub_role_reviews.json` records manual declaration-role reviews
of the original MCP panel, preserving the 13 ambiguous Graphify identities as
unknown. Pass `--reviews benchmarks/agent_query/hub_role_reviews.json` only with
the original digest-matched capture. The auditor checks pinned source commits,
whole-file hashes, exact anchors, and excerpts. This validates the review's
source provenance; it does not automate semantic role or design-quality judgment.

### MCP path diagnostics

Pass `--inputs benchmarks/agent_query/suite_mcp_paths.json` to the same collector
for 28 questions per tool: forward/reverse minimum-hop routes, hop bounds,
missing and ambiguous endpoints, and disconnected pairs. Use
`python3 -m benchmarks.agent_query.mcp_path_audit --run /path/to/new-mcp-run
--output /path/to/new-path-audit.json` to check actual ordered identities,
relations, directions, and minimum hops against the stored graphs. Structured
Compass results must also preserve node source anchors and edge identities.
Reviewed source-route witnesses are reported separately from graph consistency.

Run `suite_mcp_path_labels.json` as a separate input for the same positive
endpoints and bounds using display labels (15 questions per tool). This arm
was added after the ID diagnostic exposed Graphify endpoint substitution;
Graphify's public tool describes label/keyword inputs. Report both arms,
including competitor wins. Neither is held-out evaluation. All comparisons
request undirected navigation explicitly for Graphify; these are not evidence
of directed call-flow quality.

The first ID diagnostic was captured before its planned registration commit
because a preliminary test command failed. Its archived input incorrectly says
preregistered; the development designation in the current manifest and audit
report supersedes that claim. Preserve original artifacts. Disconnected pairs
include source files/modules as well as declarations. A search that reaches
its depth or work bound has not proved global disconnection and must remain an
incomplete answer to that question.

### Community-to-neighbor development workflow

`community_navigation_panel_a.json` freezes the one-follow-up label baseline
before capture in commit `7a6d6c97`. Run it on the same paired graph manifest:

```bash
python3 -m benchmarks.agent_query.community_navigation \
  --policy benchmarks/agent_query/community_navigation_panel_a.json \
  --registration benchmarks/agent_query/community_task_pairs_panel_a.json \
  --run /path/to/paired-graph-run/run.json \
  --output /path/to/new-community-navigation-run \
  --compass /path/to/frozen/compass \
  --graphify-python /path/to/isolated-graphify-mcp-env/bin/python \
  --graphify-environment /path/to/graphify-mcp-environment.json
```

Starting community IDs are prepared symmetrically. Follow-up labels use only
the community text and public seed file/symbol. All matching rows are retained;
multiple distinct labels stop the workflow. Graphs are used for scoring only
after requests, never to substitute an expected follow-up ID. One request has
60 seconds and 1 MiB of external response allowance; sessions are bounded at
64 MiB. Graphify receives an explicit generous token budget; Compass exposes a
whole-result interface. Those native controls are not equal token budgets.

The matching review records seed identity support on 5/15 Compass versus 9/15
Graphify tasks, and direct collaborator support on 4/14 versus 8/14. These are
reused-repository development results for one particular workflow. Broader
matching, duplicate member labels and a missing WalkDir call explain distinct
Compass failures. Additional disambiguation calls and richer member handles
remain unmeasured. Report bytes alongside these unequal successful sets, not as
a matched-success efficiency claim.

`neighbor_exact_match_review_panel_a.json` records the unchanged workflow after
MCP neighbor lookup began preferring exact matches. Compass improves to 8/15
seed identities and 6/14 direct collaborator identities; Graphify stays at 9/15
and 8/14. The ten graph hashes, follow-up labels, community texts and all Graphify
neighbor texts remain identical. Missing/extra neighbor diagnostic arrays now
sort deterministically; older captures need order normalization for those two
arrays only. Genuine collisions, two ambiguous community selectors and the
missing WalkDir call remain open.

### Source-coordinate-assisted resolver workflow

Use `community_identity_navigation_panel_a.json` as `--policy` with the same
`community_navigation` collector. It is frozen in `f660408b`. After community
membership, Compass uses structured `search_symbols` results and Graphify uses
`get_node` with its documented `path::symbol` selector. Both policies receive
the exact seed file, declaration start and terminal symbol, then pass only a
uniquely source-matched returned ID to `get_neighbors`. A search result that is
truncated or whose matching IDs remain ambiguous cannot supply the next input.
The expected collaborator is scoring-only and cannot gate requests.

The matching review records correct seed IDs and completed neighbor lookups on
15/15 tasks for each tool. Compass displays the reviewed collaborator label on
13/14 direct-call tasks; Graphify on 14/14. Three target labels remain ambiguous
in each tool's neighbor text, leaving 10/14 and 11/14 unambiguous target labels.
The missing WalkDir call remains a graph gap. Earlier label-only failures do
not imply that these existing resolver APIs cannot complete seed navigation.

All 90 tool calls succeed, and all 129 harness tests pass. Complete session bytes
are 785,989 for Compass and 102,213 for Graphify under this fixed policy. Compass
search returns more candidates and structured evidence; native bounds and
semantic payloads differ. Report these actual workflow costs with that context.
This is source-assisted development evidence, not natural-language discovery,
comprehensive assertion precision, or held-out performance.


### Rust indexed-receiver correction

See `rust_index_receiver_development_registration.json` and
`rust_index_receiver_development_review.json` for the fixed-protocol rerun on
Chi, Click, jsoup, Redux and WalkDir. One reviewed Rust call is recovered;
the 15-task source-assisted workflow now ties Graphify at 14/14 collaborator
labels and 11/14 unambiguous target labels. Other existing graph records and
all community assignments remain unchanged. Final-source validation passed,
including the native baseline and production fixture qualification. These reused
development tasks do not establish broad precision,
god-object diagnosis or overall superiority.


### Explicit neighbor identities

`neighbor_identity_development_registration.json` freezes the unchanged-graph
five-repository comparison; `neighbor_identity_development_review.json` records
the verified capture. Compass now returns the exact reviewed destination ID
in all 14 direct neighbor responses. Graphify does not emit neighbor IDs, but
with one additional source-anchored public resolver call **both resolve 14/14**.
Label presence remains 14/14 and global target-label uniqueness remains 11/14
for each. All 15 Compass full-record projections match their graphs, preserving
181 record appearances. This is graph consistency, not source precision for all
records. The extra source coordinates, richer Compass payload cost and reused
development scope remain explicit; no overall superiority claim follows.


### State-access evidence prerequisite

`state_access_development_registration.json` fixes 20 source access sites across
Go, Python, Java, TypeScript and Rust. This known-subject diagnostic inspects full
frozen graphs, not public query output or a god-object classifier. Both tools
lack all 20 selected state-contact links; Compass represents six of ten state
slots and Graphify none at the pinned coordinates. Missing edges must not be
interpreted as method independence or low cohesion.

Replay the committed per-site results against the original external artifacts:

```sh
python3 -m benchmarks.agent_query.state_access_audit \
  --registration benchmarks/agent_query/state_access_development_registration.json \
  --artifact-root /path/to/code-graph-audit-20260926 \
  --output benchmarks/agent_query/state_access_development_review.json --verify
python3 -m unittest benchmarks.agent_query.tests.test_state_access
```

Omit `--verify` with a new output path to produce a fresh report. Existing reports
are never overwritten. The verifier checks source commits and witnesses, graph
hashes, all candidate/connecting records and the auditor's own code hash. It
preserves Graphify's undirected container flag; stored endpoint order is not a
native directed-path claim. Same-agent review and purposive development scope
remain explicit.


### Rust field-access correction

`rust_state_access_development_registration.json` freezes the correction and a
known-ID public neighbor control. `rust_state_access_development_review.json`
records the five-repository rebuild: Compass now supports 4/20 registered access
sites (all four Rust sites), versus Graphify's unchanged 0/20. The four other
Compass graphs are byte-identical. WalkDir adds 174 field-reference records and
changes 122 community assignments while preserving all earlier nodes/edges.
The four public neighbor requests retrieve the selected identities and anchors
for Compass; Graphify lacks those fields. This known-subject gain does not
establish overall superiority, source precision for every added edge, improved
clustering, read/write effects or god-object defects. Raw captures and replay
scripts remain under the registered external artifact directory.


### Java field-scope compiler challenge

`java_state_scope_registration.json` registers 44 synthetic development cases
before either graph capture. The source fixture exercises positive field
selection, repeated occurrences, local/parameter shadowing, block and loop
lifetimes, lambda/catch/resource bindings, flow-scoped patterns, inheritance,
field hiding, static access, nested/anonymous/local classes, casts and array receivers.
The installed Corretto 17.0.8 compiler confirms all 44 source expectations:
36 positive cases, eight negative controls and 45 field occurrences.

`java_state_scope_oracle.json` retains each compiler instruction and source line.
`java_state_scope_baseline.json` records fresh native graph captures of that same
fixture: Compass publishes 12/14 registered fields, Graphify 0/14, and neither
publishes a contact to any registered field. Both therefore miss all 45 selected
occurrences. Empty negatives do not establish positive precision. Caller
ownership is explicitly unscored by this target/line inventory. These synthetic
results do not replace the five-repository state-access comparison or establish
cohesion, god-object defects or overall superiority.

Capture with an already installed JDK; classes and logs belong on the external
workspace volume. This compiles with processors disabled and never runs fixture
code. There is no JDK requirement for normal Compass execution or Python unit
tests. The parser is a bounded fixture-specific oracle, not a general Java
compiler front end. In particular, nonconstant fixture fields avoid constant
folding; source/class-field inventory mismatches fail, and only compiler-marked
synthetic fields are excluded from source targets.

```sh
python3 -m benchmarks.agent_query.java_state_scope_audit \
  --registration benchmarks/agent_query/java_state_scope_registration.json \
  --java-home /path/to/installed/jdk \
  --artifacts /Volumes/Workspace/CrabData/java-scope-new/capture \
  --output /Volumes/Workspace/CrabData/java-scope-new/oracle.json
python3 -m unittest benchmarks.agent_query.tests.test_java_state_scope
```

To replay the committed oracle, omit `--java-home`, point `--artifacts` at the
saved `java-state-scope-01/capture` directory, and use
`--output benchmarks/agent_query/java_state_scope_oracle.json --verify`.
Add `--graph-manifest /path/to/java-state-scope-01/graphs.json` and
`--oracle benchmarks/agent_query/java_state_scope_oracle.json` to replay the
baseline into
`--output benchmarks/agent_query/java_state_scope_baseline.json --verify`. Compilation/disassembly outputs, compiler identities, source,
registration, class files and graph hashes are retained. Historical exploratory
reports remain in the external artifact directory.

### Java field-access correction

`java_state_access_development_registration.json` retains all 20 real-source
sites and all 44 compiler cases; `java_state_access_public_registration.json`
registers four known-ID public neighbor requests. The corrected review is
`java_state_access_development_review.json`, with source, binary, graph,
registration and verification hashes. Complete captures and replay scripts are
under external `java-state-access-02`; `java-state-access-01` is explicitly
superseded after compiler and native counterexamples exposed wrong type
precedence and duplicate-receiver selection.

Compass recovers 8/20 real-source sites (four Java and four Rust), versus
Graphify's unchanged 0/20. The Java fixture recovers 39/45 occurrences versus
0/45, retaining six misses and all eight negative controls. A separate
post-capture check verifies the supported compiler field/enclosing-method pairs
and source ranges; the original target/line inventory still does not score
caller ownership. This is a fixture-specific consistency check, not a general
compiler oracle or blinded precision result.

All five repositories are rebuilt; four graphs are byte-identical to the Rust
baseline. jsoup retains previous records and adds 3,896 field references. All
added records pass endpoint/occurrence checks, including independent AST
ownership ranges for 26 field initializers. Four public requests retrieve the
selected field identities and anchors for Compass, versus none for Graphify;
Compass's complete response payload is substantially larger. The unchanged
75 task-pair community outcomes show no measured improvement. These results do
not establish authored explanations, cohesion, god-object defects, exhaustive
edge precision or overall superiority. See the audit document for remaining
misses, payload costs, partition changes, warnings and exact verification.

### Compiler source-binding census for real Java fields

`java_real_field_registration.json` freezes all 88 Java 8 base-source files in
jsoup before compiler binding capture. It retains the existing native graph
hashes, project source pin, build configuration and cached dependency digests.
`java_real_field_review.json` records the comparison and all remaining misses.
This previously observed development repository is not held-out evidence.

The public JDK `JavacTask`/`Trees` oracle parses and attributes source without
code generation, annotation processing or project execution. Source positions
are converted from UTF-16 to UTF-8 with split-surrogate checks. The auditor
joins source declarations without choosing between ambiguous candidates and
requires exact occurrence, target and source-owner evidence for credit. It
inventories graph contacts even when their identity or anchor cannot be verified;
line-only and unordered contacts are not upgraded to exact/directed evidence.

Use an installed JDK 17 and the digest-matched cached dependencies named by the
registration. Every capture requires a new external artifact directory:

```sh
python3 -m benchmarks.agent_query.java_field_capture \
  --registration benchmarks/agent_query/java_real_field_registration.json \
  --root /Volumes/Workspace/Github/jhy/jsoup \
  --java-home /path/to/installed/jdk17 \
  --classpath /path/to/jspecify-1.0.1.jar \
  --classpath /path/to/re2j-1.8.jar \
  --artifacts /Volumes/Workspace/CrabData/java-field-capture-new
python3 -m benchmarks.agent_query.java_source_fields \
  --capture /Volumes/Workspace/CrabData/java-field-capture-new/bindings.stdout \
  --manifest /Volumes/Workspace/CrabData/java-field-capture-new/manifest.json \
  --root /Volumes/Workspace/Github/jhy/jsoup \
  --graph /path/to/frozen-graph.json --tool compass \
  --output /Volumes/Workspace/CrabData/java-field-review-new.json
```

Use `--tool graphify` with its native graph. Add `--verify` to replay an existing
review; default output creation rejects overwrites. Offline replay and unit tests
need no JDK. Compiler errors, incomplete output, source drift and limits fail the
capture; they never become empty successful inventories.

The final census contains 3,785 ordinary source-field references and 529 enum
constant references. Compass verifies 3,047 ordinary references and all 3,047
returned contacts in scope; Graphify has no field-contact records. Both tools
represent all 131 enum constants but miss all 529 reference occurrences. Compass
represents 614/616 ordinary fields; Graphify represents none. The remaining
44 external fields, 55 array lengths and 31 class literals are reported
separately. Arrays and class literals are javac pseudo-fields, not source fields.

All 44 previous scope cases and their 45 bytecode-checked occurrences agree with
this source oracle. The additional fixture covers overload ownership, Unicode,
compound uses, constant folding, intrinsics, initializers and anonymous/local
classes. All 176 auditor tests pass. Repeated full captures and offline reviews
are identical. External `jsoup-java-field-oracle-02` retains final evidence;
round 01's mixed non-source category remains explicitly superseded. These results
strengthen source-declaration precision evidence for this configuration; they
do not score read/write effects, explanations, paths, community quality or
actual god-object defects.

### Explanation source-budget sensitivity

`explanation_budget_registration.json` freezes the existing five repositories,
20 facts, exact identity constraints, and five source quotas before capture.
`explanation_budget_review.json` publishes every result and actual byte cost.
The latest graphs still score 14/20 versus 15/20 at 8,000 bytes. At 16,000 the
scores are 19/20 versus 18/20; at 32,000 they are 20/20 versus 18/20. Both tie
8/20 at 2,000 and 4,000. These measure source evidence, not authored answers or
overall superiority; larger quotas also have unequal actual source usage.

`source_windows.py` plans bounded windows solely from public membership anchors
and scores the unchanged witnesses. It rejects missing anchors and unsafe paths,
retains clipped raw bytes, and applies the historical Click header allowance only
with verified identity. All 187 benchmark tests pass, including 11 new tests:

```sh
python3 -m unittest benchmarks.agent_query.tests.test_source_windows
```

External `explanation-budget-01` contains the collector, independent verifier,
20 public-call transcripts, 50 source-window arms and payload deltas. All ten
8,000-byte source controls reproduce the old bytes and outcomes. The audit
explains source-order starvation, Graphify's two Redux interval gaps, and the
much larger Compass graph payloads. No production code changes in this arm.

### Native member-name focus

`explanation_focus_registration.json` fixes a Compass before/after experiment
using each full original question and the same 8,000-byte source quota. Product
commit `293582c3` adds optional `--source-members --member-focus TEXT`: rank
recorded callable names by distinct normalized term matches, then source order.
No source is read to rank; unmatched members and all existing limits remain.

`explanation_focus_review.json` records a gain from 14/20 to 15/20 supported
facts, with no losses on this known development panel. Chi gains routing
implementation evidence. Literal coverage is 5/20 versus 6/20 before indentation
normalization. Actual retained source stays 28,129 bytes; stdout and charged
verification work increase. Six Redux excerpts remain explicitly unverified
because their graph nodes lack stored source digests.

All 15 native invocations, ordering decisions, source intervals and provenance
statuses replay under external `member-focus-02`; round 01 retains the initial
compile failure. The corrected verifier retains its failed assumption about
Redux digests. Native and benchmark checks are listed in the main audit.

This is not a paired Graphify result or authored-answer score. The separate
symmetric 8,000-byte neighbor-window control remains 14/20 versus 15/20. Remaining
WalkDir loop evidence shows why lexical name matching alone is insufficient.

### Paired public-member focus: retain the negative result

`paired_member_focus_registration.json` fixes a common helper and five source
quotas for both tools before capture. `member_focus.py` ranks public labels with
a frozen lexical policy; equal file/line groups use the maximum single-label
score, and ties keep source order. `source_windows.py` accepts an exact group
permutation while preserving interval ends computed from original source order.
No tool-specific kind filter, source-body ranking or private graph lookup selects
source. The helper is an evaluation policy, not a new native Graphify feature.

At the primary 8,000-byte quota, common focus regresses **Compass 14/20 → 12/20**
and **Graphify 15/20 → 14/20**. Both trade Chi shared-state evidence for routing
evidence; Compass loses two Redux facts and Graphify loses one WalkDir fact.
The committed `paired_member_focus_review.json` publishes all five quotas and
all fact-level gains/losses. This policy is rejected as the default. Keep its
negative result separate from the native callable-span improvement.

All 202 benchmark tests pass, including 15 new tests. All 20 fresh public calls,
153 anchors and 100 window/scoring arms replay independently; all historical
source-order controls remain identical. External `paired-member-focus-01`
retains raw responses, windows, ranked labels, source-boundary diagnosis and
verification scripts. Product code/version is unchanged in this checkpoint.

### Longer call paths: preserve the native-label tie

`longer_path_registration.json` and `longer_path_witnesses.json` freeze five new
source-grounded endpoint pairs in the known five-language panel. Four-to-seven
call witnesses include explicit conditional/overload semantics. The public
source-assisted workflow scores Compass **2/5** and Graphify **0/5**; a separately
registered native short-name control ties at **1/5**. Both tools return valid
six-call Click chains, through different source branches. The post-output
Graphify alternative receives source support, not frozen-occurrence credit.

`longer_path_review.json` retains every result, source identity, occurrence,
confidence, depth probe, failed request and artifact hash. Compass's jsoup and
WalkDir structural shortcuts fail the call-chain task even though private-graph
diagnostics find the necessary call edges. Neither graph exposes Redux's named
`combination` function at the frozen coordinate. Public answer quality, graph
availability and cross-interface ID handling remain separate measurements.

All 204 benchmark tests and the product boundary pass; repeated verification is
byte-identical. External `longer-paths-01` holds raw MCP/CLI captures and verifier
scripts, including failed attempts. No production change, held-out claim or
overall superiority claim follows from this checkpoint.

### Calls-only trails: native correction on the frozen questions

`calls_only_trails_registration.json` and `calls_only_trails_review.json` retain
the explicit calls-only implementation and replay of the same five questions.
Source-assisted public-ID static call paths improve from **2/5 to 4/5**; native
short-name results remain **1/5**. CLI, explicit call-path `ask` and actual MCP
agree on all four resolved pairs. Redux remains unresolved in the denominator.
All 20 fresh resolver results are unchanged, and nine default stdout/stderr
controls remain byte-identical.

The jsoup route uses `Document.body`'s missing-body fallback despite the caller
initializing a shell containing a body. Its edges have source support, but
execution feasibility in that context is unproven. This post-capture alternative
receives no frozen-route or runtime credit. The stopped initial verification,
source adjudication and existing incomplete-coverage warning are retained.

The report records 56 query, 43 CLI, 11 MCP, 1,106 workspace, nine product and
204 benchmark passing tests, two existing ignored workspace tests, formatting,
Clippy, product-boundary and build results. Repeated verification is identical.
External `calls-only-trails-01` retains raw evidence and logs. This is a native
before/after result, with no equivalent Graphify path filter, held-out claim,
authored-answer score or overall superiority claim. Version remains 0.3.30.

### Shared public-neighbor call-path workflow

`public_neighbor_path_registration.json` freezes a common bounded FIFO search;
`public_neighbor_paths.py` consumes only public MCP results. Both tools receive
the same source endpoint questions and workflow limits. Compass supplies
destination IDs directly; Graphify requires cached public label lookups, whose
ambiguities remain incomplete branches. This compares public navigation
workflows, with identity capabilities and internal work still differing.

`public_neighbor_path_review.json` records **4/5 versus 0/5** source-supported
static call paths. Graphify encounters intermediate ambiguity in Chi/Click,
no outgoing calls at the jsoup start, and unresolved Redux/WalkDir endpoints.
Its separate native Click path success remains visible. Compass still misses
Redux. Its jsoup `createShell` route uses different call occurrences from the
frozen witness, and Click uses the previously reviewed stdin alternative.
Only Chi/WalkDir have a frozen occurrence at every step. No runtime feasibility
or global absence score is assigned.

All 110 public requests replay exactly; 20 endpoint results are unchanged and
76 neighbor projections match the frozen graphs. Response costs are 1,558,220
bytes over 72 Compass calls and 11,001 bytes over 38 Graphify calls, including
failures; they do not establish equal-work efficiency. All 218 benchmark tests
and the product boundary pass on repeat. The first suite's subprocess cleanup
permission error is retained. Fourteen new tests include all 4,096 four-node
directed graphs at three depth bounds. External `public-neighbor-path-01`
retains raw evidence and repeated verification. No product/version change,
held-out evidence or overall superiority claim follows.

### Externally rated Blob classes

`mlcq_god_audit_registration.json` freezes a Java addition to the multilingual
panel using Lech Madeyski and Tomasz Lewowski's [MLCQ v1.1 dataset](https://zenodo.org/records/3666840)
(CC-BY-4.0; paper DOI: 10.1145/3383219.3383264). The full downloaded CSV stays
outside the repository; the registration retains attribution, digests and
selected review metadata.

`mlcq_corpus.py` reduces 4,019 Blob review rows to 2,334 source classes. Duplicate
reviews by one reviewer never add independent votes. It selects every pinned
repository/revision with at least two multi-reviewer unanimous `none` classes
and two classes whose reviewers all assigned `major` or `critical`: CloudStack
and Eclipse Platform UI. All 55 classes and 104 review rows in those revisions
remain visible; only eight classes enter the primary denominator (four in each
rating group). The remaining 47 classes retain single-reviewer, minor or
conflicting judgments. This consensus-enriched sample is not prevalence data.

`mlcq_god_source_witnesses.json` pins exact class declaration coordinates and
source hashes. `mlcq_god_source_review.json` records a complete annotated-span
review for all eight classes. One positive case is a test class. These authored
responsibility observations supplement imperfect external labels; they are not
tool-generated explanations, proven refactoring requirements or new labels.

`mlcq_hubs.py` scores the registered native top-100 hub response at cutoffs
10/50/100, separately for each rating group. Missing graph declarations,
ambiguous graph anchors, ambiguous displayed labels, nonreturned classes and
unavailable captures remain distinct. Source coordinates never disambiguate a
returned label. Unlabeled hubs cannot establish precision-at-k or a false-positive
rate. Source-assisted class neighbors are a separate diagnostic and cannot
replace native ranking retrieval. Neither tool's ordinary degree ranking is an
established god-object classifier. The original extraction and artifact limits
remain part of the outcome, including failures.

`mlcq_god_audit_review.json` preserves the first bounded run: the frozen
unoptimized Compass development binary timed out on CloudStack and exceeded
the registered graph-artifact limit on Eclipse.
Graphify admitted both graphs but retrieved none of the four positive or four
`none` classes in its top 100. All eight existed at their exact source anchors
and resolved through public queries. Eighteen raw tool request/response pairs were verified;
all eight neighbor projections matched stored displayed triples. The repeated
verifier is byte-identical. These outcomes establish no Compass superiority or
validated god-object classifier; they identify extraction/admission work and
the gap between degree ranking and externally rated Blob classes.

## SQL prefix guard: matched development-build diagnosis

`sql_prefix_scan_registration.json` freezes complete CloudStack SQL files and
limits; `sql_prefix_build_context.json` freezes explicit settings matching the
unoptimized baseline. `sql_prefix_scan_review.json` records all 24 fresh-output
observations and repeated verification. Preserve unavailable baseline output,
small-file timing regressions and the distinction between complete old/new graph
equality and candidate-only repeatability. No release or Graphify speed claim
follows. Full scope and validation are in the
[code-graph audit](../../docs/implementation/code-graph-intelligence-audit-2026-09-26.md).


## Declared larger-artifact Blob diagnostic

`mlcq_admission_followup_review.json` reports the same eight reviewed classes
under the separately registered 1 GiB graph cap. Compass CloudStack still times
out; Eclipse is admitted with its 62-edge omission warning. Neither tool returns
either Eclipse positive class in the top 100. Keep unavailable classes distinct
from nonreturned classes and classifier decisions.

The report preserves the correction of an unfiltered-neighbor auditor defect,
all 27 raw tool calls, repeated verification and 238 passing benchmark tests.
It also discloses Graphify's logged omission of all 119 CloudStack SQL files
because its optional parser dependency was absent. Original graphs and outcomes are unchanged.
`sql_regex_cache_registration.json` freezes the separate release-build diagnosis.
`sql_regex_cache_review.json` records all 24 successful observations and full
graph equality across the six runs of each of the four inputs. Both builds use
the same release settings; every timing, peak RSS and host-load observation is
retained. This development panel does not establish whole-product or Graphify
performance superiority. `sql_regex_cache_whole_registration.json` separately
registers the complete-source follow-up. `sql_regex_cache_whole_review.json`
records identical published partial-graph artifacts but a longer candidate observation;
`sql_regex_cache_whole_repeat_registration.json` declares a separate repeated
investigation. `sql_regex_cache_whole_repeat_review.json` retains all six
successful observations, identical full graph hashes, resource logs and repeated
verification. Its medians are 215.288 seconds baseline and 204.082 seconds
candidate (ratio 0.948). These development results do not explain the initial
longer candidate observation or establish general performance superiority.
None of these replaces the original outcomes.

## Release-server Blob follow-up

`mlcq_release_followup_registration.json` retains the same eight classes,
queries, cutoffs and limits, using a fresh admitted CloudStack candidate graph,
the frozen Eclipse graph and one frozen release server. The producer/server
provenance distinction and all partial-graph warnings remain explicit.
`mlcq_release_followup_review.json` records 36 verified raw tool calls and
byte-identical repeated verification. Both tools resolve all eight classes, but
neither returns any of the four consensus-positive classes in its top 100.
These ranking misses do not establish classifier decisions or population
precision. Source-assisted neighbors remain a separate stored-graph diagnostic.


## Java enum-reference correction and preservation controls

`java_enum_access_registration.json` fixes the unchanged compiler-backed jsoup
census before candidate extraction; `java_enum_control_registration.json` fixes
all four other known language repositories before their paired control runs.
`java_enum_access_review.json` records the qualified source/binary, every native
check, complete graph deltas and repeated verification.

The candidate verifies 355/529 enum references (previously zero) and 3,106/3,785
ordinary field references (previously 3,047). All 3,461 returned in-cohort contacts
agree with the compiler; all previously verified occurrences remain. Graphify
retains zero such contacts and no positive-contact precision denominator. The
679 ordinary and 174 enum misses remain in the fixed denominator.

Of 670 added graph references, 256 are in test files outside the compiler cohort;
source-token/provenance checks do not establish their semantic binding precision.
Both jsoup graphs retain two omitted edges. All eight Go/Python/TypeScript/Rust
control extractions succeed with complete graph equality to their paired baseline
and preserved original. These known development results are not held-out evidence.

jsoup changes from 42 to 40 communities; all 75 unchanged source task-pair
outcomes remain identical in a post-change replay. No functional community,
god-object, authored explanation or overall superiority claim follows. The audit
retains the initial native failures, corrected initializer/import cases and the
scope-test correction followed by all 533 resolver tests and focused Clippy.


## Java dotted nominal-owner follow-up

`java_dotted_owner_registration.json` freezes the unchanged jsoup compiler
census and four paired language controls before candidate extraction.
`java_dotted_owner_review.json` records 3,115/3,785 ordinary and 368/529 enum
references, all 3,483 returned contacts compiler-supported and all earlier
verified references retained. Graphify retains zero such contacts. The 670
ordinary and 161 enum misses remain in the denominator.

All eight control extractions reproduce their complete original graphs.
Of 49 new jsoup references, 27 are outside the compiler cohort (21 test and six
Java 11 overlay); anchor checks do not prove their bindings. Community count
changes from 40 to 43 with all 75 existing task-pair outcomes unchanged.
Version-10 AST cache reuse reproduces the fresh candidate graph byte-for-byte.
The separate nested-expression fixture retains three enum misses. The report
binds production `be2fb542`, qualification and repeated verification. These
known development results do not establish broad or held-out superiority.

## Source-first three-language direct-call review

`source_first_three_language_suite.toml` registers 12 selected Go, Python and
Rust declaration/call/path questions before either tool ran. The retained
external run `source-first-3lang-01` contains both freshly built graphs,
commands and raw responses. `review_source_first_three_language.py` requires
that run, the registered suite and the three pinned source roots; it checks
their hashes, exact source call sites and graph edges, then writes
`source_first_three_language_review.json`.

The registered text score is Compass 9/12 versus Graphify 8/12. Two Graphify
path responses contain the required names but follow indirect structural
routes, so the separate source-supported score is 9/12 versus 6/12. The audit
explains each correction and the unresolved FastAPI short-name ambiguity.
These selected development questions do not measure whole-graph accuracy,
authored explanations or god-object diagnosis.

`fastapi_owner_suffix_replay.json` separately records a post-registration
development replay on the same frozen FastAPI graph after the owner-qualified
selector correction. It pins the source, graph, candidate binary, command
output hashes and exact call endpoints. The preregistered 12-question scores
remain unchanged.

## Source-first three-language hub retrieval

`source_first_hub_three_language_registration.json` freezes six source-located
Go/Python/Rust declarations, both existing graph hashes, server environments,
and `god_nodes(top_n=100)` before either hub request. Run
`source_first_hub_capture.py` on the pinned checkouts to retain the bounded
public stdio MCP transcripts, then run `source_first_hub_review.py` to check
exact source identities, top-10/50/100 retrieval and displayed degrees against
each tool's own graph. `source_first_hub_three_language_review.json` is the
result; the raw run is retained externally as `source-first-hubs-01`.

Compass returns 3/6 selected declarations in its top 100; Graphify returns
4/6. Graphify returns `PeerAuth` at rank 55 while Compass does not return it.
Compass's 300 returned rows include explicit identities and matching source
anchors, and all 300 degrees match its graph. Graphify's label-only output
allows 247/300 rows to be uniquely joined; their degrees also match. These
source roles are not god-object defect labels, and the two graphs differ.
The panel measures neither classifier accuracy nor broad superiority.
