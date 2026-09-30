# Agent query evaluation

`benchmarks/agent-query` measures how well Compass answers the agent questions
in its suites compared with Graphify on the same pinned checkouts. It is
developer-side tooling: Compass never runs it, and it never installs Graphify.

Three suites share the harness:

| Suite | Questions | Shape |
| --- | ---: | --- |
| `suite.toml` | 47 | The first five-repository suite, including Compass's compact and paged projections |
| `suite_natural.toml` | 25 | Same natural-language question on both tools, source-reviewed v2 oracles, 800-token pages, no follow-ups |
| `suite_v2.toml` | 50 | A blackbox-fair extension: same questions for both tools, default output forms, no tool-specific projections |

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

The suite covers five real repositories in five languages:

| Repository | Language | Focus |
| --- | --- | --- |
| `spf13/cobra` | Go | CLI command resolution and execution |
| `pallets/flask` | Python | request dispatch into views |
| `google/gson` | Java | overloaded serialization APIs |
| `colinhacks/zod` | TypeScript | schema parse and safe-parse helpers |
| `tokio-rs/axum` | Rust | routing and service dispatch |

Both suites contribute source-reviewed questions across `explain`,
`explain_source`, `callers`, `callees`, `impact`, `path`, `file_path`,
`ambiguity`, `negative`, and `broad` (the first suite adds the `brief`,
`brief_callers`, and `paged_callers` projections). Every question declares the
exact per-tool argument vector, the expected outcome, and the file, line, or
symbol anchors the reviewer read in the pinned checkout. Every repository also
declares graph anchors that both graphs must contain as source-backed nodes.

## Run

```bash
python3 benchmarks/agent-query/runner.py doctor \
  --compass-binary /path/to/compass \
  --graphify-binary /path/to/graphify \
  --source cobra=/Volumes/Workspace/Github/spf13/cobra \
  --source flask=/Volumes/Workspace/Github/pallets/flask \
  --source gson=/Volumes/Workspace/Github/google/gson \
  --source zod=/Volumes/Workspace/Github/colinhacks/zod \
  --source axum=/Volumes/Workspace/Github/tokio-rs/axum

python3 benchmarks/agent-query/runner.py run \
  --suite benchmarks/agent-query/suite_v2.toml \
  --workspace /Volumes/Workspace/CrabData/compass-evaluations/agent-query \
  --compass-binary /Volumes/Workspace/crabbuild-target/compass/release/compass \
  --graphify-binary "$(command -v graphify)" \
  --source cobra=/Volumes/Workspace/Github/spf13/cobra \
  --source flask=/Volumes/Workspace/Github/pallets/flask \
  --source gson=/Volumes/Workspace/Github/google/gson \
  --source zod=/Volumes/Workspace/Github/colinhacks/zod \
  --source axum=/Volumes/Workspace/Github/tokio-rs/axum
```

`--suite` defaults to `suite.toml` beside the runner. Passing
`suite_v2.toml` runs the 50 blackbox questions instead.

`doctor` fails when a checkout is not at the suite's pinned commit. `run`
builds `compass extract --code-only --no-viz --store sqlite` and
`graphify extract --code-only` once per repository under
`WORKSPACE/artifacts`, then writes `run.json` and `REPORT.md` under
`WORKSPACE/runs/<run-id>/`.

## Metrics

- **Correctness**: bounded stdout is judged against the suite's anchors. A
  `negative` question passes only with an explicit no-match signal, and a
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
- **Graph quality**: node and edge counts, source-backed node ratio, dangling
  edges, duplicate IDs, and how many reviewed anchors the graph contains.

## Limits

Anchor matching is a deterministic text-recall proxy over bounded output, not
an independent precision oracle. The suite is a focused five-repository
sample; it does not estimate population-wide accuracy. Graphify prints an
installation warning on stderr, which `run.json` records separately and the
token metric excludes.
