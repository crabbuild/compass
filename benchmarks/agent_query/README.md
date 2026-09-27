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
