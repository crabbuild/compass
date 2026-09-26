# Agent query evaluation

`benchmarks/agent_query` measures how well Compass answers the agent questions
in its suites compared with Graphify on the same pinned checkouts. It is
developer-side tooling: Compass never runs it, and it never installs Graphify.

[`COVERAGE_PLAN.md`](COVERAGE_PLAN.md) tracks the broader real-repository
question/evidence matrix, including ask, communities, clusters, and god nodes.
Those planned surfaces must not be described as already evaluated.

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
