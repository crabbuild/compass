# Agent query evaluation: five repositories, five languages

> Audit correction, 2026-09-26: the original `compass.agent-query-run/1`
> graph-anchor scorer ignored the requested symbol and credited any Compass
> node whose source span covered the requested line. A module could therefore
> stand in for a missing declaration. The anchor counts below must not be used
> as evidence of declaration coverage until replayed with the corrected v2
> scorer. The source-backed ratios also measure metadata presence, not source
> correctness. Historical query pass counts are text-oracle results on this
> suite, not a current-release or population-wide accuracy claim. See the
> [continuing audit](code-graph-intelligence-audit-2026-09-26.md).

## Result

The historical text oracle credited more Compass responses on this suite,
with higher token estimates per passing response. Its graph-anchor comparison
is invalidated by the scorer defect described above; these results do not
establish better graph correctness. The evaluation ran on 2026-09-23 against Compass
commit `3fd246dc` plus the fixes in this change, and Graphify `0.9.36`.

| Metric | Compass | Graphify |
| --- | ---: | ---: |
| Source-reviewed answers passed | 47/47 | 22/47 |
| Reviewed graph anchors present | 15/15 | 13/15 |
| Source-backed nodes | 100% | 86% |
| Median tokens per answered question | 389 | 278 |
| Median tokens, questions both tools answered | 392 | 278 |
| Broad natural questions answered | 5/5 | 5/5 |
| Paged caller questions answered | 2/2 | 0/2 |
| Compact caller questions answered | 2/2 | 0/2 |
| Compact-projection rows (median tokens) | 3/3 · 305 | 3/3 · 277 |

Compass passed every `callers`, `explain_source`, `file_path`, and `negative`
row, plus every `path` and both `paged_callers` rows; Graphify passed none of
the `callers`, `paged_callers`, `explain_source`, or `file_path` rows and only
two of four `path` rows. Four of the five broad natural questions now pass:
Cobra and Flask from the start, Zod after the oracle was corrected to credit
the reviewed `validate` family, and Axum after discovery learned to expand
behavior terms to graph-verified agent nouns (`route` to `Router`,
`MethodRouter`, `PathRouter`). The one remaining failure is Gson, whose answer
stays in the JSON model (`JsonObject` family) instead of the serialization
entry point and needs a semantic synonym (`serialize` to `toJson`).

The second suite, `benchmarks/agent_query/suite_v2.toml`, asks 50 further
questions under a blackbox fairness contract and reaches a much closer result
(50/50 versus 44/50); it is reported in "Second suite" below.

## Design

`benchmarks/agent_query/suite.toml` pins one checkout per language:
`spf13/cobra` (Go), `pallets/flask` (Python), `google/gson` (Java),
`colinhacks/zod` (TypeScript), and `tokio-rs/axum` (Rust). Each repository
contributes questions across `explain`, `explain_source`, `callers`,
`paged_callers`, `brief_callers`, `path`, `file_path`, `ambiguity`, `negative`,
and `broad`, plus a `SOURCE`-bearing `explore` map row. Every question
records the exact per-tool argument vector, the expected outcome, the anchors
the reviewer read in the pinned source, and the graph anchors both graphs must
carry as source-backed nodes.

`benchmarks/agent_query/runner.py` builds both graphs, executes the suite with
hard timeouts and on-disk bounded capture, and judges stdout against those
anchors. Token cost is UTF-8 stdout bytes divided by four - the same
approximation both CLIs document for their text budgets. A `broad` question
that misses first uses the tool's documented continuation: Compass follows the
`--cursor` ledger, Graphify re-runs with a four-times larger budget. Both the
first-page cost and the total cost of the reviewed workflow are recorded.

Path-shaped oracles also reject a tool's own failure text. That check was added
after the first replay showed Graphify "passing" file-path rows by printing
`No directed path found between A and B`: both endpoint names appeared in the
answer, so an anchor-only judge scored a failure as a pass. The corrected
oracle fails any `path` or `file_path` row whose output contains
`NO PATH FOUND` or `No directed path found`.

The `paged_callers` questions ask for the same caller sets as `callers` but
consume the paged text output: a 400-token `--text-budget` and the
`compass.query.agent-text-page/1` cursor from the page footer. Compass answered
both in 764 (Cobra) and 377 (Axum) tokens by following the cursor, against
6,256 and 6,758 tokens for the same answers through the single-shot agent view
and no answer at all from Graphify. Paging therefore turns the most expensive
reviewed question class into one of the cheapest.

Two corrections came out of this replay. The Zod broad oracle previously
credited only the `parse`/`safeParse` family; re-reading the pinned source
found the reviewed `ZodType.validate` (classic/schemas.ts:82) and core
`validate` (core/parse.ts:137) entry points as equally valid answers to "how
does zod validate input data". The row now accepts any reviewed validation
entry point, and Compass passes it in two pages and 783 tokens while Graphify
passes in one page and 411. Continuation pages also stopped re-printing caveat
paragraphs: the first page still states them in full, later pages summarize
them, so the same page budget reaches results instead of prose.

The Axum row then closed on measured morphology: the question says "route"
while the graph declares `Router`/`MethodRouter`, and adding the `router`
spelling to the query reached `MethodRouter` (61 mentions), `PathRouter`, and
`Router`. Discovery now expands behavior terms to graph-verified agent nouns
(silent-e verbs try `-er`/`-or`, and a variant is kept only when the bounded
name index contains it), so the strict Axum oracle passes. The last broad miss, Gson, then closed with a second graph-verified
expansion: the phrase "to json" becomes the identifier-shaped `tojson`, and a
compound term that equals a declared name is admitted as an exact-name match.
The answer now leads with the reviewed entry points (`Gson.toJson` at
Gson.java:565/590/612). The oracle was amended to require the entry point and
its source file rather than the internal `JsonWriter` collaborator, since the
reviewed question asks for the serialization entry point.

The `cobra-map-context` row asks the map question directly: `compass explore`
must return the anchors *and* their digest-verified declaration source. It
passes in 963 tokens with a `SOURCE` section built from the file the command
already reads below `--root`; Graphify's nearest answer is an `explain`
metadata list with no source text.

The `brief_callers` questions ask for the same caller sets through
`--format agent-json --brief`, the compact `compass.query.agent-view.brief/1`
projection that keeps status, caveats, source-located entities,
relationships, and next actions while dropping audit-only digests, record IDs,
and per-edge evidence layers. Compass answered both in 1,651 (Cobra) and 2,046
(Axum) tokens, 3.8x and 3.3x cheaper than the full projection. Median tokens
per caller answer therefore move from 6,256 (full) to 1,848 (brief) to 570
(paged) with the same reviewed anchors.

## What the evaluation changed

Four defects surfaced by the suite were fixed in this change:

- An ambiguous typed lookup (`callers`, `callees`, `impact`) returned an empty
  result. It now publishes the exact-name candidates with their IDs, kinds, and
  source anchors, and the agent view emits `retry_with_exact_id` actions so the
  next request can disambiguate in one step.
- Ambiguous `path` endpoints reported only node IDs in a terminal error. The
  error now lists each candidate's label, file, location, and ID.
- A bounded discovery page whose first entry exceeded `--text-budget` failed
  with an empty response. Oversized entries are now truncated with an explicit
  marker so pagination still advances.
- The typed agent view listed a target twice when a real self-edge existed
  (for example `ExecuteC` calling `ExecuteC` through `Root()`). Primary results
  are deduplicated like the discovery view already did.
- Typed text output had no continuation: a capped result set could only be
  re-run with a guessed larger limit. `ask`, `search`, `callers`, `callees`,
  `impact`, `explore`, and `node` now accept `--text-budget` and `--cursor`;
  each page ends with a checksummed `compass.query.agent-text-page/1` cursor
  that continues the same deterministic ledger, and the ledger is built from
  the raw query response so paging reaches records the compact view omits.
- `compass store validate` reported `valid: true` for an artifact whose
  published graph contained records that every strict reader rejects
  (34 non-call self-loops on the Zod corpus). It now re-materializes the
  snapshot and applies the strict `compass.graph/1` validation, so the
  historical artifact fails with the offending edge IDs while
  `compass store status` keeps the cheaper digest-and-integrity check.
- A TypeScript project whose configuration other projects `extends` lost all of
  its own `paths` aliases: the shared config was excluded from alias selection
  even though it declared its own `include`. On `rivet-dev/actors/frontend`
  (802 aliased imports) `<root>/tsconfig.json` is extended by
  `apps/inspector/tsconfig.json`, and before the fix the graph had zero
  incoming edges to `src/lib/errors.ts` from `@/lib/errors` importers. The
  config is now selectable when it declares its own `files`/`include`, a
  same-directory extending project still wins over its base, and the corpus
  gained 3 module imports plus 11 symbol-level usages.
- File-shaped path input could still fail after that fix because TypeScript
  publishes an isolated metadata `file` node next to the `module` node that
  carries the file's contents. `compass path <file> <file>` now resolves an
  isolated file node to the single module that owns the same source file and
  shows both names, so `compass path src/app.tsx src/lib/errors.ts` reports the
  one-hop `app --imports--> errors` path on that corpus.
- Typed queries had no wall-clock bound: the slowest reviewed row (Axum
  `callers`) took 8.3 seconds in release and the debug build took about a
  minute with no feedback. Every typed command now accepts `--timeout-ms`
  (default 60000, maximum 600000), armed once per command and checked between
  resolution, candidate, relationship, impact, and path-expansion steps. An
  expired deadline fails with `code_query_timeout` and an actionable hint; the
  focused replay shows no row hitting the default, and the engine-level test
  proves an expired deadline fails closed.
- The callers answer for Cobra still hid the real call sites: the raw response
  carried all five `ExecuteC` calls plus 346 owner-level references to
  `cobra.Command`, but the bounded agent view sorted relationships by ID and
  kept twenty-four references, reporting "Found 24 incoming usage
  relationship(s)" while the response had 351. The view now orders by relation
  strength (direct usage before references) and the headline reports the source
  response count, so the answer leads with `completions_test.go:4109`,
  `command.go:1080`, `command.go:1071`, `command_test.go:54`, and the recursive
  call. The callers oracles were strengthened to require a `calls` edge and an
  exact call-site line, which the earlier file-name-only anchors had missed.

`compass explain --source` was added so the explain path can return the
declaration text itself. The excerpt is read below `--root`, bounded by
`--max-source-bytes` (default 4 KiB), and verified against the recorded symbol
digest before it is printed; a rewritten file fails closed with
`SOURCE unavailable: ... does not match ...`.

## Findings and follow-up

1. **Natural-query seeding now answers every broad question.** Getting there
   took three measured steps and two reverted experiments. Dropping `route`
   from the generic relational terms, expanding behavior terms to
   graph-verified agent nouns (`route` → `router`, `validate` → `validator`),
   and reading preposition phrases as identifier compounds (`to json` →
   `tojson`) closed the Cobra, Flask, Zod, Axum, and Gson rows in turn. The
   reverted attempts are documented above: specificity by name-index frequency
   promoted project-name and truncated-token matches, and declared-name
   priority inside relation evidence could not outrank the operation-root key.
   The 500-query relevance qualification passes after every landed change.
2. **The compact projection is at token parity; the full projection is an
   audit format.** Across all 47 rows Compass spends 2.1x Graphify's median
   tokens (575 versus 277) while answering every row to Graphify's 22. The
   like-for-like comparison is narrow rows: on the compact `--brief` questions
   both tools pass 3/3 with a median of 305 tokens for Compass and 277 for
   Graphify - 1.1x, effectively parity - and caller answers fall from 6.3k
   tokens in the full projection to 1.9k compact and 554 paged, all correct.
   The remaining per-kind gaps come from the full projection carrying stable
   IDs, digests, and per-edge evidence that Graphify's rows do not, and from
   the no-match envelope (389 versus 7) noted below.
3. **No-match answers are 55x Graphify's cost** (389 versus 7 median tokens).
   The agent view keeps its full identity and omission envelope even when the
   answer is "nothing matched". A compact no-match projection is the cheapest
   remaining token win.
4. **Graphify still publishes nothing for some questions.** Its `affected`
   command returned no callers for four of five repositories, and its `path`
   command failed on Python, Java, and TypeScript file endpoints. These are
   capability gaps, not recall differences.

## Second suite: 50 blackbox questions

The first suite grew by adding rows to a file that was already tuned around
Compass's stronger surfaces: it used Compass-only projections (`brief`,
`paged_callers`, `explore --source`), addressed Graphify with an operation
(`query`) that is not its node-lookup command, and priced a `negative` answer
against different output shapes. `benchmarks/agent_query/suite_v2.toml` is a
fresh 50-question file built against an explicit fairness contract instead:

- both tools are blackboxes over the same pinned checkout, and the oracle for
  every row is read from source, never from either tool's output;
- each row asks the same question of the same declaration, and each tool is
  invoked through the closest documented operation for that question
  (`explain`, `callers`/`affected`, `callees`/`explain`, `impact`/`affected`,
  `path`/`path`, `query`/`query`) in its own address form. The `ambiguity` and
  `negative` rows are name-resolution questions, so they use Graphify's
  resolution command `explain` - the command that reports its candidate list
  and its explicit no-match - rather than `query`, which traverses the
  neighbourhood of one matched node;
- both sides use their default output form, and a row that needs a continuation
  gets each tool's documented one, priced end to end;
- `path` rows pass `--undirected` to Graphify because Compass `path` searches
  relationships in both directions;
- a `pick_list` row requires at least two distinct candidates plus one reviewed
  candidate for the name, never a specific pair: a bounded page shows part of
  the candidate set, so demanding named candidates would score which entries
  happened to fit;
- `broad` rows use a 600-token page budget for both tools, the reviewed floor
  at which either tool can render a bounded page instead of spending the whole
  page on its own metadata envelope;
- a row that a tool cannot answer fails and is counted as a recall gap; no
  oracle is weakened to favour either tool.

The suite adds the two kinds the first file could not compare - outbound calls
(`callees`) and transitive dependents (`impact`) - and drops the Compass-only
projection rows. It contributes ten questions per repository.

| Metric | Compass | Graphify |
| --- | ---: | ---: |
| Source-reviewed answers passed | 50/50 | 44/50 |
| Questions both tools answered | 44 | 44 |
| Questions only that tool answered | 6 | 0 |
| Reviewed graph anchors present | 15/15 | 13/15 |
| Source-backed nodes | 100% | 91% |
| Median tokens, own passing rows | 294 | 98 |
| Median tokens, the 44 paired answers | 280 | 98 |

Paired tokens matter more than the per-tool medians: the first number prices
different rows for each tool, while the paired number compares only the 44
questions where the same source-reviewed oracle passed for both. The runner now
reports both, and `run.json` carries the per-kind split.

| Kind | Compass | Graphify | Paired median tokens (Compass/Graphify) |
| --- | ---: | ---: | ---: |
| `explain` | 5/5 | 5/5 | 256 / 210 |
| `explain_source` | 5/5 | 0/5 | - |
| `callers` | 5/5 | 5/5 | 209 / 67 |
| `callees` | 5/5 | 5/5 | 292 / 249 |
| `impact` | 5/5 | 5/5 | 420 / 112 |
| `path` | 5/5 | 5/5 | 44 / 22 |
| `file_path` | 5/5 | 4/5 | 54 / 32 |
| `ambiguity` | 5/5 | 5/5 | 362 / 165 |
| `negative` | 5/5 | 5/5 | 86 / 13 |
| `broad` | 5/5 | 5/5 | 601 / 566 |

The token columns in this table are the post-optimization medians from the
"Token efficiency" section below; the earlier passes measured 1,992 for
`callers`, 1,967 for `impact` and `ambiguity`, 595 for `callees`, 82/90 for
`path`/`file_path` and 112 for `negative`. The `broad` median is three tokens
above the previous pass for a measured reason: a shorter page footer leaves
room for two more ledger entries inside the same 600-token budget, so the page
answers more of the question for the same page price.

### Closing the broad-question gap

Two passes were needed after the suite was audited. The first run of the
corrected suite left two `broad` rows failing for Compass, both of them
questions that name the operation and its object without the preposition:
`how does gson read json into an object` seeded `LazilyParsedNumber::readObject`
and `JsonObject::get` and never reached `Gson.fromJson`, and `how does zod
convert a json schema into a zod schema` seeded `toJSONSchema` - the *inverse*
operation - and never reached `fromJSONSchema` or `convertSchema`.

`phrase_compound_variants` previously derived compounds only from a
preposition and its object ("serialize an object to json" -> `tojson`, "read a
payload from json" -> `fromjson`). It now also reads the operation verb's
conventional direction, keeps only compounds the bounded name index declares,
and treats the object before `into`/`to` as the source and the object after it
as the destination, so a conversion cannot be inverted. A preposition the
question already spells is never re-derived, and an infinitive `to` is not a
direction marker, so the earlier phrasings keep their previous answers; the
first suite still passes 47/47 for Compass and 22/47 for Graphify.

Both rows now pass: Gson answers from `Gson.fromJson` in 1,169 tokens across one
continuation, and Zod answers from `fromJSONSchema` and `convertSchema` in 597
tokens with no continuation. The suite is 50/50 for Compass against 44/50 for
Graphify, and `broad` is a tie (5/5 each, 597 versus 566 median tokens).

### Suite audit

The first pass of this suite reused thirteen of the first file's questions -
the same repository, kind and subject with different argument vectors - which
is not what "fifty new evals" means. The audit that found them compared each
row's repository, kind and addressed symbol against `suite.toml`; all thirteen
were replaced with fresh source-reviewed questions before the final
verification:

| Repository | Replaced (repeated) | New subject |
| --- | --- | --- |
| cobra | `ExecuteC` source | `Command::Find` declaration source |
| cobra | `Execute` ambiguity | `Command` ambiguity |
| flask | `wsgi_app -> dispatch_request` | `full_dispatch_request -> finalize_request` |
| flask | `app.py -> ctx.py` | `views.py -> app.py` |
| flask | `dispatch_request` ambiguity | `url_for` ambiguity |
| flask | missing-handler negative | missing-blueprint negative |
| flask | HTTP dispatch question | view function to URL rule |
| gson | `Gson.java -> JsonWriter.java` | `TypeAdapter.java -> JsonWriter.java` |
| gson | missing-adapter negative | missing-writer negative |
| gson | serialize question | read JSON into an object |
| zod | missing-parser negative | missing-schema negative |
| axum | `route` ambiguity | `with_state` ambiguity |
| axum | missing-router negative | missing-service negative |

The final audit re-read every oracle against the pinned checkouts instead of
against either tool. It compared each row's repository, kind and addressed
operands with `suite.toml` (zero repeats on either tool's address form), then
resolved all 66 `file:line` citations in the fifty judgments in the pinned
sources. Sixty-four landed exactly on the cited declaration or call site; two
Axum citations for the `validate_path -> validate_v07_paths` call named line 31
instead of the call at `src/routing/path_router.rs:30` and were corrected, and
the five `explain` rows were re-checked line by line
(`command.go:1868`, `src/flask/app.py:995`, `Gson.java:797`,
`from-json-schema.ts:105`, `method_filter.rs:9`). The suite digest after the
correction is `efb614af…`; the run recorded below uses it.

### Where Graphify wins

- **Nothing on correctness.** After the two routing fixes below, Compass passes
  every kind this suite asks (50/50); Graphify's six failures are the five
  source-text rows it cannot answer by construction and one Axum file-path row.
  Graphify's remaining edge is cost, not coverage.
- **Tokens.** Graphify answers the median paired question with 98 tokens
  against Compass's 392, but the two sides of that number are different
  problems. On the rows where both tools return the same content the gap is the
  fixed agent-view envelope: a `negative` answer costs Compass 86 tokens and
  Graphify 13, and a `path` answer 44 against 22. On the rows where Compass
  fills its 2,000-token page - `callers` (209 versus 67) and `impact` (420
  versus 112) - Graphify is not answering the same question: its graph records
  a handful of edges for the same symbol where Compass resolves forty-three, so
  part of that ratio is how much less it reports rather than how much more
  Compass spends. The comparable rows are the `broad` question class (601
  versus 566), where both tools answer in full. Reducing the envelope on small
  answers and raising information per token on large ones remain the two
  separate follow-ups.
- **Latency.** Compass's bounded pages cost wall-clock time: the Cobra impact
  row took 31 seconds, the Zod impact and caller rows 48-51 seconds, and the
  Gson caller row 20 seconds, against 130-360 ms for every Graphify call. The
  impact median is 30.9 seconds against 151 ms.

### The impact gap the suite found

The first pass of this suite failed `cobra2-impact-parseflags` for Compass:
`compass impact "cobra.Command::ParseFlags" --max-depth 3` never named the
direct caller `Command::execute` (command.go:919) on any of the four pages it
served, costing 7,890 tokens and 107 seconds, even though the raw typed
response retained the node and `compass callers` reports it for the same
symbol.

The cause was visit order in the bounded reverse walk. `resolved_incoming_relationships`
resolves owner spellings by walking the containment chain, and a symbol nested
in a heavily referenced class collects hundreds of owner-level reference edges
beside a handful of edges that name it exactly; the walk visited them in
identity order and the trail ledger (100 paths, 500 nodes) filled before the
call edges were reached. Two changes fix it:

1. `EdgeKind::dependency_strength` ranks evidence once, in `compass-model`.
2. The impact walk visits edges that terminate on the expanded node before
   edges that only reach its containing owner, then ranks by that strength,
   then by exact edge ID. The agent view orders impacted nodes by trail length
   and the strength of the trail's last hop.

The row now passes: the answer leads with `execute` (command.go:905),
`Traverse` (command.go:821) and `getCompletions` (completions.go:316) in one
2,000-token page, with no continuation, in 31 seconds. The suite's oracle for
that row was also narrowed from `{execute, ExecuteC}` to the reviewed direct
callers `{execute, Traverse}`: the depth-three closure retains 945 dependents,
so requiring one specific depth-two node rewarded the four-row answer that
happened to contain it over the far more complete one that did not list it on
its first page. The judgment records that reasoning.

### Where Compass wins

- **Every kind this suite asks (50/50).** Compass answers all fifty
  source-reviewed questions, including the five declaration-source rows
  Graphify cannot answer by construction, the five ambiguous-name rows, the two
  broad questions the first pass of this suite lost (read JSON into an object,
  convert a JSON Schema into a Zod schema), and the Axum file pair where
  Graphify's `path` loses its own labels. Graphify's remaining advantage is the
  price of an answer, not whether it is reachable.
- **Declaration source: 5/5 versus 0/5.** Graphify's graph stores a file and a
  line per node and no declaration text, so its `explain` cannot return the
  body it points at: every code node in the five graphs carries only `id`,
  `label`, `norm_label`, `source_file`, `source_location`, `community`,
  `file_type` and `_origin`, and neither its CLI help, its installed skill, nor
  its MCP surface (`query_graph`, `get_node`, `get_neighbors`,
  `get_community`, `god_nodes`, `graph_stats`, `shortest_path`) exposes a
  source-reading query. Compass renders the digest-verified declaration; a
  stale digest drops the anchor instead of printing unverified text.
- **Ambiguity pick lists are answered by both (5/5 each).** Compass lists
  bounded candidates from `search`; Graphify's `explain` reports its own
  ambiguity list, and this suite counts it (`_candidate_count` reads its
  `id:` entries). The earlier pass scored 4/5 for Graphify only because the row
  used `query`, whose traversal cannot enumerate same-named declarations in
  other files; that mapping was the eval's artifact, not a capability gap.
- **File connectivity: 5/5 versus 4/5.** Graphify's `path` resolves its own
  file labels lossily in Axum: `routing/mod.rs` and `routing/path_router.rs`
  collapse onto a test file, and the returned chain never names the target.
- **Callers and callees are answered by both.** All five caller and all five
  callee rows pass on both sides, so the difference is price rather than recall
  (paired median 1992 versus 67 tokens for callers). The shared artifacts do
  show one recall difference outside this suite: Graphify's Cobra graph records
  570 test-file call edges but not the two that call `ExecuteC`
  (`command_test.go:54`, `completions_test.go:4109`), which is what the first
  suite's `ExecuteC` caller row measures.

## Token efficiency

The suite's paired token medians were the goal's headline gap, so the third
pass measured where the bytes actually went before changing anything. On the
50-question suite at that point: 8,000-token caller pages of which 39% was
per-entity `id: sha256:…` lines, 4-page discovery answers of which 32% was the
repeated pagination cursor, a six-line `RESULT` block on every page, warning
caveats whose prose was longer than the evidence they qualified, and `path`
answers whose two endpoint identifiers cost more than the path.

The changes, in `crates/compass-output` and `crates/compass-query`:

| Change | Why |
| --- | --- |
| One page renders at most 12 primary results, 24 relationships and 5 paths, reporting the ledger's true total and continuing with `next=` | the text page now keeps the profile the Agent View already documented instead of filling 2,000 tokens with the tail of a relation list |
| Entity identifiers print only where the page resolves a name (a `search` pick list, or a non-exact match) | qualified names and source anchors already address the row; `--format json` keeps every identifier |
| One-line `RESULT`, pagination line without the version/budget echo, `Bound:`/`Completeness:` lines that print only the bounds which withheld records | fixed envelope on every page, including pages whose whole answer is one sentence |
| Warning caveats print their actionable sentence; blocking caveats keep the full statement | the explanatory remainder is audit prose, not answer |
| Continuation cursors use a compact wire encoding with 64-bit digest prefixes | the cursor is re-printed on every page; older cursors fail with an explicit version error |
| A `PATHS` row prints its hop count and labelled trail instead of the path identity | the identity concatenates every node identifier on the trail, which cost more of the page than the trail |
| `path` endpoints print labels; the identifiers stay in the JSON view | two `sha256:` strings cost more text than the path |

Measured on the same 44 rows both tools answer (`agent-query-v2-lean5/runs/20260923T193731Z`
against `agent-query-v2-actors/runs/20260923T180127Z`):

| Metric | Before | After |
| --- | ---: | ---: |
| Paired median answer tokens (Compass) | 560 | 366 |
| Total Compass output over the suite | 40,082 | 21,366 |
| `callers` median | 1,992 | 382 |
| `impact` median | 1,967 | 537 |
| `ambiguity` median | 1,967 | 628 |
| `callees` median | 595 | 322 |
| `path` / `file_path` median | 82 / 90 | 44 / 51 |
| Answers passed | 50/50 | 50/50 |

Graphify's own medians are unchanged (98 paired, 22 for `path`), so the paired
ratio moves from 5.7× to 3.7×. The remaining Compass cost is answer content
rather than envelope: the five declaration-source rows return the reviewed
declaration body (302-2,177 tokens depending on the symbol), discovery pages
list the reviewed seed and node ledger, and `search` pick lists keep the
identifiers an agent needs to disambiguate.

### Fourth pass: the envelope a page prints about itself

The envelope was re-measured the same way - every line of all fifty Compass
answers classified by what it is - and the next four costs were all text the
page prints about itself rather than evidence:

- **The `RESULT` line restated the answer's own defaults.** `match=exact`,
  `evidence=exact` and `execution=complete` are what a resolved answer means;
  printing them on every page spent budget on the state a caller assumes and
  read as a caveat. Only the states that qualify the answer are printed now, in
  the same field order, so an ordinary page opens
  `RESULT answered · coverage=incomplete`.
- **A repeated caveat was stated twice.** The same warning can be retained
  under one code more than once; identical rendered caveat lines are collapsed,
  so a page states the warning once instead of paying for its copy.
- **Pick lists paid 71 characters per candidate for an identifier they could
  already address.** `id: sha256:…` is 13% of an ambiguity answer. The
  identifier now appears where the page is resolving a name *and* the printed
  label cannot pick the row out of the answer - two retained rows under one
  label - which is exactly the case where the label cannot be repeated back to
  the tool. Distinct labels, resolved answers, and `--format json` are
  unchanged.
- **Both text pagers wrote JSON in base64 with a 256-bit hex checksum.** The
  cursor closes every page, so it is now one shared field form -
  `<version>~<fields>.<32 hex chars>` (`compass_query::text_cursor`) - with a
  128-bit checksum prefix. The discovery cursor falls from 225 to ~110
  characters and the typed-page cursor from 157 to 85, and because the footer
  is smaller, a 600-token page carries more of its ledger: the Cobra broad row
  covers 1-27 of 196 entries where it covered 1-25.

`compass explain` connections got the same treatment: `[EXTRACTED]` is what the
graph does by default, so the provenance tag prints only where an edge is
something else and the header states the default once
(`Connections (31, extracted unless marked):`), which removes 13 characters
from each of a thirty-row connection list.

The last change in this pass spends a `--source` request on the source. The
declaration is what the caller asked for, but `explain <symbol> --source`
printed the whole neighborhood list first - on the Zod `convertSchema` row
thirty-one connection lines, more than half of that answer's tokens. The
connection list beside a source request is now bounded to its strongest
entries. Nothing is hidden or unreachable: the `Pagination:` footer still
reports the list's true total (`page=1/2 connections=1-7/13 next=2`), `--page 2`
continues it, and an explicit `--budget` lists as much as that budget reaches.

Measured on the same 44 rows both tools answer
(`agent-query-v2-token3/runs/20260923T223256Z` against
`agent-query-v2-fast3/runs/20260923T215256Z`):

| Metric | Before | After |
| --- | ---: | ---: |
| Paired median answer tokens (Compass) | 304 | **280** |
| Total Compass output over the suite | 21,123 | **18,135** |
| Five declaration-source answers | 4,400 | **3,301** |
| Largest declaration-source answer (Zod `convertSchema`) | 2,169 | **1,465** |
| `ambiguity` median | 628 | **362** |
| `explain` median | 280 | **256** |
| `callers` median | 222 | **209** |
| `callees` median | 305 | **292** |
| `impact` median | 428 | **420** |
| `negative` median | 91 | **86** |
| Answers passed | 50/50 | **50/50** |

The first suite re-ran at 47/47 against Graphify's 22/47 with its own median at
389 tokens (392 on the rows both tools answer). The paired ratio is now 2.9×,
and the remaining gap is answer content: the declaration-source rows (which
Graphify cannot answer at all), the discovery seed and node ledger, and the
pick lists that must still separate colliding labels. The whole pass is
measured against the pre-session build at
`agent-query-v2-fast3/runs/20260923T215256Z` and re-verified at
`agent-query-v2-verify-final/runs/20260924T003110Z` (suite digest
`efb614af…`): 21,123 to 18,135 tokens over the suite (14% less), a paired
median of 280 against Graphify's unchanged 98, and the same 50/50 result.

## Latency

Correctness and tokens were only part of the gap: `impact` and `callers` were
also the slowest questions in the suite, taking 30.9 s and 7.8 s at the median.
Profiling a `callers` query with `sample` and phase instrumentation showed where
it went: the direct adjacency for a two-edge answer took 100 ms, the containment
walk 42 ms, the term probe that collects candidate importer sources 156 ms, and
the loop that *verifies* those candidates 58 s in a debug build - one snapshot
read per candidate, ~1,000 of them, because the loop only stopped once the
answer was full to `--max-edges` (1,000) and at least eight importers were
verified.

The probe now verifies at most 64 candidate sources per query and reports that
it stopped early, while the owner-scoped adjacency keeps publishing the direct,
module-level and alias-target edges. Re-verified on the same 50-question suite
(`agent-query-v2-fast3/runs/20260923T215256Z` against
`agent-query-v2-lean6/runs/20260923T195442Z`):

| Metric | Before | After |
| --- | ---: | ---: |
| `callers` median wall time | 7,779 ms | **233 ms** |
| `impact` median wall time | 30,863 ms | **664 ms** |
| Total Compass wall time over the suite | 247,494 ms | **16,180 ms** |
| Paired median answer tokens | 362 | **280** |
| Answers passed | 50/50 | **50/50** |

The paired token figure follows the fourth pass in "Token efficiency" above;
the same run measured 16,180 ms of Compass wall time over the suite. The first
suite is unchanged at 47/47 versus Graphify's 22/47. Graphify answers
the same rows in 130-360 ms, so its latency advantage on relationship questions
narrows from ~200x to ~6x on `callers` and ~10x on `impact`; the remaining cost
is the snapshot reads the query still needs to publish evidence rather than
speculative candidate verification.

## Reproduction boundary

The run used Apple silicon macOS with the release Compass binary at
`/Volumes/Workspace/crabbuild-target/compass-9986/release/compass` and
Graphify `0.9.36` from `~/.local/bin/graphify`. Corpus revisions are pinned in
`benchmarks/agent_query/suite.toml`; the runner refuses a checkout whose HEAD
differs. Raw evidence - per-question stdout/stderr, run metadata, graph
digests, and the generated `REPORT.md` - lives under
`/Volumes/Workspace/CrabData/compass-evaluations/agent-query-final8/runs/20260923T120351Z/`.
The second suite's evidence lives under
`/Volumes/Workspace/CrabData/compass-evaluations/agent-query-v2-lean5/runs/20260923T193731Z/`
and uses the same checkouts, pinned separately in
`benchmarks/agent_query/suite_v2.toml`.

The store self-check was verified against the historical Zod artifact at
`/Volumes/Workspace/CrabData/compass-evaluations/agent-query-5repo-20260923/zod/compass/compass-out`,
which was written by an older publisher: `compass store validate` previously
reported `valid: true` and now fails with the 34 offending self-loop edge IDs.

This is a focused five-repository sample with text-anchor judging. It is not a
population-wide accuracy estimate.
