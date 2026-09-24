# Command reference

This reference groups the public `compass` command surface by responsibility.
Run `compass <command> --help` for the exact options in the installed version;
this page explains how the families fit together and which outputs are stable
for automation.

## Global entry points

```bash
compass --help
compass --version
compass <command> --help
```

The shipped product executable is `compass`; there is no legacy command
frontend or alias.

## Build and analysis

### `init`

Configure repository scope and perform the first structural build:

```text
compass init [PATH]
  [--include PATH_OR_GLOB]
  [--exclude GLOB]
  [--program]
  [--store json|sqlite]
  [--inference-level low|medium|high|max]
  [--yes]
  [--force]
```

Includes and excludes are repeatable. Interactive mode previews the effective
corpus before writing `.compass/config.toml`; scripts must pass `--yes`.
Replacing an existing configuration requires `--force`. Init builds the
structural graph by default; pass `--program` when the initial workspace also
needs Program IR.
The initial build publishes JSON and a SQLite query snapshot by default. Pass
`--store json` when only the portable JSON artifact is wanted. The database
lives below the output root at `store/store.sqlite3`; the
snapshot contains only the small reference beside `graph.json`.

### `update`

Make a saved current-tree graph match the project:

```text
compass update [PATH]
  [--program]
  [--program-artifact PATH]
  [--out DIR]
  [--store json|sqlite]
  [--inference-level low|medium|high|max]
  [--no-program]
  [--no-cluster]
  [--force]
  [--no-viz]
  [--no-gitignore]
  [--exclude PATTERN]
  [--resolution N]
  [--exclude-hubs N]
```

Use for normal cold/incremental structural builds. The default publishes the
structural graph only; pass `--program` when Program IR inspection or graph
enrichment is needed. `--no-program` remains accepted as an explicit
structural-only compatibility flag. Supply a verified offline SCIP index with
repeatable `--program-artifact` (which also enables Program IR). Fresh exact
Java symbol evidence can disambiguate AST-proven call sites. Python call
enrichment additionally requires an offline `scip-python` artifact whose
`<artifact>.compass-manifest.json` contains a complete frozen
`compass.managed-analyzer-profile/1` profile. Compass never runs or installs
`scip-python` during the build. Stale, generic, unverified, inexact, or
conflicting Python artifact evidence is not projected. `--no-program`
conflicts with `--program-artifact`.
Graph storage defaults to `sqlite`; `--store json` opts out of the validated
local store sidecar without replacing `graph.json`. JSON remains the portable
authority, while the sidecar keeps large graphs queryable under bounded memory.
Inference defaults to `low` and publishes exact relationships only. Use
`medium` for source-backed inferred resolution, `high` to additionally retain
explicitly qualified external references, or explicit `max` to retain all
inferred relationships including deferred receivers.

### `ensure`

Ensure the active checkout or linked worktree has a current local graph:

```text
compass ensure [PATH] [UPDATE_OPTIONS]
```

With no `PATH`, `ensure` resolves the active Git worktree root even when the
agent starts in a nested directory. It uses the same incremental, atomic
pipeline and build profile as `update`. It reports whether the worktree-local
graph was `initialized`, `updated`, or already `current`. Run it once when an
agent session starts, resumes in a different worktree, or acquires a new
working directory. Do not pass `--force` during normal session bootstrap;
compatible manifests and caches make repeated calls inexpensive.

Keep the default `compass-out/` below each worktree. Multiple worktrees may
contain different uncommitted changes and must not write one shared mutable
output directory. Repository-wide immutable history and its verified-content
cache remain shared through the Git common directory.

Clustered builds use deterministic fixed-resolution Leiden over the typed
evidence topology. Omitting `--resolution` uses `1`; `--resolution N` uses the
single positive finite value `N`. Higher values generally create smaller
communities. The three-candidate automatic selector is qualification-only and
is not enabled by omitting this option. `--no-cluster` skips community
membership, analysis, labels, `community-quality.json`, and
`community-hierarchy.json`.

Clustered typed builds also publish `community-hierarchy.json`, the bounded
navigation hierarchy over the published partition. Read it back unchanged with
`compass export hierarchy-json`, which refuses an unknown schema major or a
graph identity mismatch.

`compass export html` and `compass export workbench-json` embed that hierarchy
for the selected graph, so the page navigates levels instead of one flat
partition. `--hierarchy-level N` opens the page on level `N` (default `0`); an
unknown level, or a graph whose build published no hierarchy, fails with a
bounded error rather than rendering a different page.

### `extract`

Expose the full build surface:

```text
compass extract [PATH]
  [--program]
  [--program-artifact PATH]
  [--no-program]
  [--code-only]
  [--cargo]
  [--google-workspace]
  [--postgres DSN]
  [--backend NAME]
  [--model MODEL]
  [--mode deep]
  [--ocr off|auto|always]
  [--ocr-profile NAME]
  [--ocr-language BCP47]
  [--token-budget N]
  [--max-concurrency N]
  [--max-workers N]
  [--api-timeout SECONDS]
  [--allow-partial]
  [--dedup-llm]
  [--timing]
  [--out DIR]
  [--store json|sqlite]
  [--inference-level low|medium|high|max]
  [--no-cluster]
  [--force]
  [--no-viz]
  [--no-gitignore]
  [--exclude PATTERN]
  [--resolution N]
  [--exclude-hubs N]
```

`--backend` and `--model` select the semantic provider and model. The
non-secret `COMPASS_BACKEND` and `COMPASS_MODEL` environment values are used
when the flags are omitted; explicit flags win. Built-ins are `claude`, `kimi`,
`ollama`, `gemini`, `openai`, `deepseek`, `azure`, `bedrock`, and `claude-cli`.
Set only the selected provider's documented credential variable, or register a
custom OpenAI-compatible provider with `compass provider add`. OCR remains
local and does not require an LLM credential; document semantic enrichment may
use the selected provider.

Use `--code-only` for an explicit fully local structural profile; it limits
structural node and edge extraction to code-classified files while retaining
the scanned file inventory. Program IR is opt-in with `--program`;
`--program-artifact` also enables it. `--no-program` is retained for callers
that already use the structural-only spelling.

OCR is off by default. `auto` processes scanned/low-text PDF pages and eligible
embedded Office images; `always` processes every bounded visual candidate.
Both are local and require an explicitly installed verified profile. Extraction
never downloads a model. `--ocr-language` is repeatable, and
`--allow-partial` also authorizes visibly incomplete OCR coverage.

### `document` and `models`

```text
compass document inspect FILE
  [--format text|json]
  [--ocr off|auto|always]
  [--ocr-profile NAME]
  [--ocr-language BCP47]
  [--allow-partial]

compass models list [--format text|json]
compass models install pp-ocrv6-small|pp-ocrv6-medium
compass models verify pp-ocrv6-small|pp-ocrv6-medium
```

`document inspect` is read-only and does not publish a graph. JSON uses
`compass.document.inspect/1`; text marks OCR-derived evidence visibly. Native
PDF, DOCX, PPTX, and XLSX processing requires no additional installation.
`models install` is the only command here that uses the network. It downloads
only pinned artifacts from the Compass allowlist, validates size and SHA-256,
and publishes an atomic verification marker. `list` and `verify` are offline.
On Intel (`x86_64`) macOS, managed OCR is unavailable because the pinned ONNX
runtime has no self-contained distribution; `models install` fails before any
download, while native document processing and `--ocr off` remain available.

`ensure`, `update`, `extract`, and watch rebuilds may succeed with a warning that Compass
published a partial graph. The warning reports exact omitted node, omitted
edge, and identity-collision counts. The retained `graph.json` remains strictly
valid and queryable; record examples and the exact summary are in
`graph.diagnostics`. Document-level corruption, an unsafe inventory, no usable
nodes, serialization failure, and atomic publication failure still return a
nonzero exit.

### `watch`

```text
compass watch [PATH]
  [--debounce SECONDS]
  [--program]
  [--program-artifact PATH]
  [--no-program]
  [--store json|sqlite]
  [--inference-level low|medium|high|max]
  [--out DIR]
  [--no-cluster]
  [--no-viz]
  [--no-gitignore]
  [--exclude PATTERN]
  [--poll]

Watch builds omit Program IR by default. Pass `--program` or
`--program-artifact PATH` when the watcher should maintain the artifact.
```

Long-running adaptive filesystem watcher. Compass synchronizes once at startup,
then coalesces native filesystem events with a 150 ms quiet window and a 750 ms
maximum delay. `--debounce` changes the quiet window; the maximum becomes five
times that value, capped at five seconds.

Only one build runs at a time. Changes received during a build queue one
follow-up, transient build failures retry with bounded backoff, and an idle
five-minute reconciliation catches missed events. Native watcher startup
automatically falls back to content-aware polling; `--poll` forces that backend.
A manual `compass update` remains the recovery oracle.

### `cluster-only`

Recluster/analyze an existing graph or path:

```text
compass cluster-only [PATH]
  [--graph PATH]
  [--no-viz]
  [--no-label]
  [--resolution N]
  [--exclude-hubs N]
  [--min-community-size=N]
```

For a typed `compass.graph/1` input this uses the same fixed-resolution Leiden
profile as a normal build and atomically republishes graph-bound
`community-quality.json`. A schema-less legacy graph retains compatibility
Louvain behavior and does not publish quality evidence. The command never
interprets a missing older quality artifact as successful evidence.

### `label`

Generate/update semantic community labels:

```text
compass label [PATH]
  [--graph PATH]
  [--backend NAME]
  [--model NAME]
  [--missing-only]
  [--no-viz]
  [--resolution N]
  [--exclude-hubs N]
  [--max-concurrency N]
  [--batch-size N]
  [--min-community-size=N]
  [--timing]
```

`--min-community-size` controls which communities are presented for labeling
and in the bounded architecture report. It does not remove nodes, edges, or
community assignments from the graph; omitted communities remain queryable and
are included in the report's coverage disclosure. The default is `3`.

When labeling first reclusters a typed graph, its resolution behavior and
quality artifact are the same as `cluster-only`.

## Read and query

### `query`

Natural-language graph discovery:

```text
compass query "<question>"
  [--traverse]
  [--dfs]
  [--context VALUE]
  [--direction auto|incoming|outgoing|both]
  [--scope KIND:VALUE]
  [--format text|json]
  [--result-envelope]
  [--text-budget N]
  [--cursor TOKEN]
  [--evidence]
  [--budget N]
  [--page N]
  [--max-nodes N]
  [--max-edges N]
  [--graph PATH | --at REV]
```

Plain questions against a typed graph use bounded
`compass.query.discovery/1` discovery. Direction, repeatable OR scope,
relationship context, and DFS compose within that contract. `--result-envelope`
requires `--format json` and opt-in wraps the unchanged discovery result in
`compass.query.discovery-result/1` with a query-owned `semanticResultDigest`.
Without this flag, the existing JSON shape remains unchanged. `--traverse`,
`--budget`, or `--page` explicitly select legacy relevance traversal and cannot
be mixed with discovery controls. CompassQL routing is unchanged.
The default focused neighborhood is 64 nodes and 128 edges. Use
`--max-nodes 500 --max-edges 1000` when a query intentionally needs the full
supported breadth; these remain hard ceilings rather than new defaults.

`--context VALUE` is a relationship filter for traversal evidence contexts such
as `call`, `import`, or `route`. It is not a node, file, package, community, or
subsystem selector. Use repeatable `--scope KIND:VALUE` for explicit OR scope
over `community`, `source`, `package`, or `node`.

The default text projection is concise: it prints match confidence, seed terms,
nodes, edges, and source locations without expanding provenance records or the
semantic digest. `--evidence` selects the full audit projection. Exact-looking
operands that do not resolve emit `NO EXACT MATCH`; bounded fuzzy and lexical
candidates can still follow as suggestions but are not represented as exact.

`--text-budget` bounds the discovery text projection and defaults to 8,000
approximate tokens. Its opaque cursor binds
the contract version, normalized request/options, selected graph generation and
digest, semantic-response digest, evidence tier, and next stable section/item.
Fetch the next page with `--cursor TOKEN` and otherwise unchanged semantic inputs. The
presentation-only `--text-budget` may change between pages. Pages contain whole
deterministic entries; changed inputs fail instead of silently continuing a
different result. JSON rejects text pagination controls. Legacy `--budget` and
numeric `--page` retain their existing meaning only with legacy traversal.

Query seeds prefer source-backed declarations over unresolved external-symbol
placeholders with the same callable label. Source-less placeholder nodes retain
an explicit `wiring=FILE:LOCATION` site, and traversed relationships render
their occurrence as `at=FILE:LOCATION`; neither is presented as a declaration
location.

Typed intent routing:

```text
compass ask "<question>"
  [--graph PATH | --at REV]
  [--format text|json]
```

`ask` chooses a bounded typed search, callers, callees, impact, or node-trail
operation and returns `compass.query/1`. `--at REV` reads one immutable trusted
revision graph; it does not fall back to a legacy projection. Rebuild a revision
whose realization does not contain the current trusted graph contract.

CompassQL:

```text
compass query --cql QUERY
  [--param NAME=VALUE]
  [--format table|json|jsonl]
  [--graph PATH | --at REV]

compass query --cql --file PATH
  [--params-file PATH]
  [--output PATH]

compass query --cql --stdin
compass query --cql --repl
```

Limits:

```text
--timeout-ms N
--max-rows N
--max-path-depth N
--max-expanded-relationships N
--max-memory-bytes N
```

`--budget` and `--page` apply only to natural-language query rendering. Page
CompassQL rows explicitly with a stable `ORDER BY` plus `SKIP` and `LIMIT`, for
example `RETURN n.id ORDER BY n.id SKIP 100 LIMIT 100`.

Canonical language contract: [CompassQL](../COMPASSQL.md).

### Typed query commands

The focused typed commands share one output profile:

```text
compass ask "<question>"       [--format text|agent-json|json]
compass search "<query>"       [--format text|agent-json|json]
compass callers "<symbol>"     [--format text|agent-json|json]
compass callees "<symbol>"     [--format text|agent-json|json]
compass impact "<symbol>"      [--format text|agent-json|json]
compass explore "<symbol>" ... [--format text|agent-json|json]
compass node "<source>" "<target>" [--format text|agent-json|json]
```

`explore --format text` closes its bounded page with a `SOURCE` section: the
recorded line range of each primary anchor, rendered from the digest-verified
file the command already reads below `--root`. Blocks are bounded per anchor
and by the page budget; a truncated file read is labeled, and a stale digest
leaves that anchor out of the source section instead of presenting unverified
text.

`text` is the answer-first Agent View projection. It starts with one `RESULT`
line carrying the result state and whichever of match, evidence, execution and
coverage qualify the answer; `match=exact`, `evidence=exact` and
`execution=complete` are what a resolved answer means and are left out, so an
ordinary page opens `RESULT answered · coverage=incomplete`. `ANSWER` and any
blocking `CAVEATS` follow, then the source-located entities, paths,
relationships, and bounded next actions. Warning caveats print their actionable
sentence and leave the explanatory remainder to `agent-json`/`json`, and a
caveat retained more than once is stated once. An entity prints its stable
identifier only where the page has to resolve a name *and* the label it printed
cannot pick that row out of the answer, which is when two retained rows share
the label; every other row is addressed by the qualified name and source anchor
it prints.
`agent-json` emits the strict
`compass.query.agent-view/1` object. `json` remains the unchanged raw
`compass.query/1` response and is the right choice when an audit consumer needs
every evidence record. The natural `query` command accepts the same
`agent-json` format for discovery; its text header is answer-first while the
existing discovery entry ledger and v2 cursor semantics are unchanged.

Text output is paged. `--text-budget <N>` sets the approximate token budget of
one page (default 2000) and the closing `Pagination:` line carries a
`compass.query.agent-text-page/1` cursor in `next=` (a compact, checksummed
envelope; cursors from an earlier release are rejected with an explicit
version error). One page renders at most the Agent View's profile - 12 primary
results, 24 relationships, 5 paths - and reports the ledger's true total, so
the page stops at the strongest evidence instead of filling the budget with the
tail of a long relation list. Passing that token back as
`--cursor` continues the same deterministic ledger at the same page budget, so
a capped result set is read page by page instead of re-run with a guessed
larger limit. Each continuation re-runs the same query with wider internal
record bounds and verifies the reviewed prefix before rendering, and an invalid
or stale cursor fails explicitly. The ledger is derived from the raw
`compass.query/1` response, so paging reaches records that the compact Agent
View bounds omit; when the underlying query bound itself is reached, the page
reports it and asks for wider `--max-nodes`/`--max-edges` limits.

Page one states the result state, answer, and every caveat. Continuation pages
keep the state and answer and replace the caveat block with a single
`CAVEATS: N unchanged from page 1 (code×count)` line, so the same budget is
spent on result entries rather than repeated prose. Blocking caveats are always
stated in full; warning caveats state their actionable sentence.

`agent-json` and `json` are incompatible with text-only `--cursor`,
`--text-budget`, `--evidence`, and `--result-envelope` controls. Agent View JSON contains
bounded `nextActions` as argv arrays or JSON argument objects; clients should
use those values instead of reconstructing shell commands from result text.

Every typed query accepts `--timeout-ms <N>` (default 60000, maximum 600000).
The deadline is armed once per command, so continuation pages and the internal
bound widening share it, and it is checked between resolution, candidate,
relationship, impact, and path-expansion steps. An expired deadline fails with
`code_query_timeout` and a hint to raise the deadline or lower the record
bounds; no partial response is published.

`--format agent-json --brief` emits the compact
`compass.query.agent-view.brief/1` projection: the same status, answer,
caveats, source-located entities, relationships, paths, and next actions, with
audit-only identities, digests, omission counters, per-relationship IDs, and
per-edge evidence layers removed. Use it when an agent needs the reviewed
answer cheaply; use `--format json` or the plain `agent-json` projection when it
needs exact identity, digests, or full evidence. `--brief` is rejected for any
other format. On the evaluation corpus the same caller answers cost 1.7k-2.0k
tokens in brief form instead of 6.3k-6.8k.

`callers` returns incoming relationship evidence: calls, routes, references,
imports, exports, and aliases. `callees` remains the direct outgoing call view.
When an import or reference ends at a containing module rather than the
selected declaration, `callers`, `impact`, and `affected` retain the real
owner-targeted edge and emit an `incomplete_coverage` precision warning.
Such an edge proves a module-level dependency, not a direct symbol call;
impact paths include the containment hop instead of silently jumping from
the selected symbol to the importer.

### `architecture`

```text
compass architecture
  [--graph PATH]
  [--labels PATH]
  [--format text|json|agent-json]
```

Returns the existing bounded architecture projection as a first-class command.
The text form is answer-first and names the graph totals, groups, routes,
diagnostics, and any omitted groups. `agent-json` adds the versioned
`compass.architecture.agent-view/1` envelope while preserving coverage counts
and witness group IDs, so an empty displayed section cannot be mistaken for an
empty architecture.

### `path`

```text
compass path "<source>" "<target>" [--max-depth N]
  [--format text|json|agent-json] [--graph PATH | --at REV]
```

Resolves both endpoints by exact node ID, name, or qualified name before doing
any graph search; missing and ambiguous endpoints fail explicitly. The text path
search is bounded to eight hops by default and ranks structural relationships
such as calls, containment, imports, and dependencies ahead of weak references
or documentation links. When a meaningfully weaker route is up to two hops
shorter, Compass shows it separately. Output names the resolved target ID, and
an unreachable target is reported as `NO PATH FOUND` with the depth bound and
visited-node count. Relationship arrows always preserve their stored direction.
Traversal may follow a relationship in either direction; the arrows make that
choice visible rather than rewriting the graph.

Both endpoints accept a file path as well as a symbol. When a language
publishes an isolated metadata `file` node beside the `module` node that
carries the file's contents, an isolated file endpoint resolves to the single
module that owns the same source file and the answer names both
(`schemas (packages/zod/src/v4/classic/schemas.ts)`). A file with several
candidate modules, or a name that matches several declarations, still fails
closed with the candidate list instead of guessing.

### `explain`

```text
compass explain "<node>"
  [--budget N]
  [--page N]
  [--source]
  [--root PATH]
  [--max-source-bytes N]
  [--format text|json|agent-json]
  [--graph PATH | --at REV]
```

Shows one node and incoming/outgoing connections. An exact node ID or unique
exact qualified name resolves directly. When a label or qualified name names
multiple source-backed declarations, Compass lists the candidates and their
source ranges and asks for the full node ID instead of silently selecting one.
Connection lines include the stored relationship site; extraction is the
graph's default provenance, so `[EXTRACTED]` is stated once in the
`Connections (N, extracted unless marked):` header and a line carries a
provenance tag only when the edge is something else.
Connections and ambiguous candidates use the same bounded, deterministic
pagination contract as natural-language queries instead of silently cutting off
after the first group.

`--source` appends the declaration text for a uniquely resolved, source-backed
node. The excerpt is read below `--root` (default: the current directory),
limited to `--max-source-bytes` (default: 4096), and verified against the
symbol digest recorded in the graph before it is printed. A rewritten file
fails closed with `SOURCE unavailable: ... does not match ...`; an ambiguous or
unsourced target keeps the candidate list instead of guessing. The declaration
is what a `--source` request is for, so the connection list beside it is
bounded to its strongest entries by default: the `Pagination:` footer still
reports the list's true total and `--page 2` continues it, while an explicit
`--budget` lists as much of the list as that budget reaches. On the reviewed
corpora this removes about a third of a source answer without putting any
connection out of reach.

### `affected`

```text
compass affected "<node-or-label>"
  [--relation R]
  [--depth N]
  [--format text|json|agent-json]
  [--graph PATH]
```

Traverses incoming impact-relevant relations. Typed `compass.graph/1` inputs use
the same bounded resolver and source-backed relationship postings as callers
and impact; legacy node-link inputs retain the compatibility traversal. JSON
and agent JSON retain diagnostics, evidence, and explicit ambiguity candidates.

### `context`

```text
compass context explain|modify|debug|test TARGET
  [--graph PATH] [--program PATH] [--root PATH] [--memory PATH]
  [--engine default|json|store] [--format text|json]
  [--max-depth N] [--max-nodes N] [--max-edges N]
  [--max-paths N] [--max-candidates N] [--max-source-bytes N]
  [--max-knowledge-items N] [--max-response-bytes N]
```

Emits `compass.task-context/1` after exact target resolution. It composes
digest-verified source, exact calls, related tests, bounded impact, and
identity-linked reflection memory. Ambiguous and fuzzy-only targets retain
candidates but do not compose structural evidence.

### `tree`

```text
compass tree
  [--graph PATH]
  [--output HTML]
  [--root PATH]
  [--max-children N]
  [--top-k-edges N]
  [--label NAME]
```

Defaults:

- graph: `compass-out/graph.json`;
- output: `compass-out/GRAPH_TREE.html`;
- max children: 200;
- top outbound edges: 12.

After a successful interactive HTML export, Compass asks whether to open the
page in the default browser. The answer defaults to no. With redirected input
or output, in pipes, and in CI, Compass neither prompts nor launches a browser.

### `benchmark`

```text
compass benchmark [GRAPH_JSON]
```

Runs the native graph-query benchmark surface.

## Versioned history and diffs

### `history`

```text
compass history enable [build-profile options]
compass history disable
compass history blind-spots [--rev REV] [--limit N] [--format text|json]
compass history status [REV] [--format text|json]
compass history build REV [--all [--first-parent]] [build-profile options|--profile-from REV|REALIZATION] [--format text|json]
compass history rebuild REV [build-profile options] [--replace-corrupt] [--format text|json]
compass history list [REV] [--format text|json]
compass history show REALIZATION [--format text|json]
compass history prefer REV REALIZATION [--format text|json]
compass history export REV --format graph-json|compass-out --output PATH
compass history gc [--prune-non-preferred] [--yes] [--format text|json]
```

`history build REV --all` resolves `REV` once, then builds every locally
reachable commit (including merged branches) in oldest-first topological order.
Add `--first-parent` to limit the batch to the ref's first-parent lineage.
The selected build profile is fixed for the whole batch. Validated preferred
realizations with that profile are skipped, so rerunning the command resumes
without rebuilding completed commits. Compass continues after individual
commit failures, emits a complete final report, and exits `1` if any failed.

```bash
compass history build main --all --code-only
compass history build main --all --first-parent
```

`history blind-spots` reads the preferred immutable realization for each
reachable commit and compares typed graph-insights IDs. It reports active and
resolved gaps/components, preserves explicit omission counts, and treats
missing sidecars from older realizations as unavailable evidence rather than
as empty reports.

Build-profile options include:

```text
--code-only
--backend NAME
--model NAME
--exclude PATTERN
--cargo
```

### `diff`

```text
compass diff OLD NEW
  [--format text|json|html]
  [--output PATH]
  [--limit N]
  [--all]
  [--explain FINDING_ID]
  [--fingerprint SHA]
```

The default output is an actionable PR-review summary: likely breaks, behavior
changes, affected callers/modules, and test evidence. Routine symbol churn is
collapsed; `--limit N` changes the visible per-section budget, while `--all`
expands routine findings and is exhaustive. `--explain` prints the evidence
and reasoning for one finding. Diff requires comparable build profiles;
rebuild the newer revision with `--profile-from OLD` when needed. `diff` never
materializes a revision: build each uncached revision explicitly with
`compass history build REV --code-only` before comparing.
`--format html` requires `--output PATH` and writes a self-contained
interactive report containing the reviewer findings, unified/split source
diffs, the exact Git patch fallback, and meaningful code-graph changes.
`--output` is rejected for text and JSON; there is no alternate semantic-diff
export command.

### `review`

```text
compass review --base REV --head REV
  [--repo OWNER/REPO] [--host HOST] [--pull-request-number N]
  [--fingerprint SHA256]
  [--format text|json|markdown|sarif]
  [--readiness]
  [--output PATH]
  [--max-findings N --max-output-bytes N]

compass review --pr NUMBER --repo OWNER/REPO [--host HOST]
  [--fingerprint SHA256]
  [--format text|json|markdown|sarif]
  [--output PATH]
```

`review` emits the canonical `compass.pr_intelligence.report/1` result or one
of its deterministic projections. Local mode resolves exact objects and never
fetches. `--repo`, `--host`, and `--pull-request-number` bind a frozen CI
identity without selecting the GitHub adapter. `--pr` selects bounded GitHub
metadata/file pagination through `gh` and requires those full objects locally.

The command creates or reuses comparable immutable graph realizations and
fails explicitly on profile mismatch. A clean candidate is analyzed at its
deterministic synthetic merge; a conflict uses the PR-head realization and
reports unavailable/indeterminate merge-dependent conclusions. `--output`
writes atomically. Markdown-only budgets report exact projection omissions.
Advisory risk and typed gate state do not change the CLI success code.
With `--readiness`, JSON or Markdown emits the additive
`compass.pr-readiness/1` envelope referencing the unchanged report digest.

See [PR Intelligence](pr-intelligence.md) for schema, rubric, bounds, MCP, and
gate semantics.

## Service

### `serve`

```text
compass serve [GRAPH_PATH]
  [--graph PATH]
  [--transport stdio|http]
  [--host HOST]
  [--port PORT]
  [--api-key KEY]
  [--path PATH]
  [--json-response]
  [--stateless]
  [--session-timeout SECONDS]
```

Prefer stdio for a single local client. Avoid putting secret values directly in
shell history; use the deployment's supported secret mechanism.

## Export and visualization

### `export`

Formats include:

```text
html
json
workbench-json
callflow-html
obsidian
wiki
svg
graphml
cypher / graph database formats represented by current help
neo4j
falkordb
```

Each format has its own exact flags:

```bash
compass export --help
compass export callflow-html --help
```

Common inputs include `--graph PATH`, labels/report/sections, output directory,
node/diagram limits, and database connection arguments.

`callflow-json` and `callflow-html` retain their command names for script
compatibility but now publish one Rust-owned architecture projection. JSON is
`compass.viewer.architecture/1`; HTML embeds the same model in the shared
workbench. Production scope is classified before communities are grouped, and
Generated, Vendor, Test, Documentation, and Unknown sources cannot influence
Production names or boundaries. Relationships are classified as Execution, Dependency, Type,
Structure, Contextual, or Unknown. The default Architecture lens admits only
Execution and Dependency relationships. Aggregate metrics remain labeled
relationships because the Execution class also includes handlers, routing,
messaging, and other executable flow; an individual exact `calls` record keeps
its original relation name in the inspector.

`--max-sections N` bounds overview groups. It does not discard groups or merge
them into `Other`: the model reports exact omissions and retains every group
for search and drill-down. Use `--architecture-overlay PATH` for a strict
`compass.architecture-overlay/1` JSON or TOML file. The canonical current
project discovers `.compass/architecture.toml`; arbitrary and historical graph
paths do not inspect live configuration. `--sections PATH` remains a deprecated
alias and adapts legacy section JSON.

`html`, `json`, and `workbench-json` accept repeatable graph views. Compass
preserves their command-line order and puts them in one navigable workbench:

```bash
compass export html --code-graph --architecture-graph
compass export html --call-graph checkout --impact-graph checkout
compass export html --affected-graph checkout --relation calls --relation imports
compass export html --artifact-lens routes --artifact-lens data
compass export html --history-graph main~10..main
```

The equivalent generic syntax is repeatable `--view` with `code`,
`architecture`, `call:SYMBOL`, `impact:SYMBOL`, `affected:NODE`,
`history:OLD..NEW`, or `artifact:LENS`. Call, impact, and affected views share
bounded `--depth`, `--max-nodes`, and `--max-edges` controls. `--direction`
applies to call views, `--include-heuristic` to impact views, repeatable
`--relation` to affected views, and `--program PATH` enriches call views with
Program IR evidence. Unsupported, misspelled, and format-incompatible options
fail instead of being ignored.

With no requested view, `html` contains a code-graph workbench and plain
`json` retains the existing `compass.viewer.graph/1` response. Any requested
view makes `json` return `compass.viewer.workbench/1`; `workbench-json` always
returns that contract. `--community` remains a graph-only JSON operation and
cannot be combined with workbench views. `--output PATH` selects the HTML file.

For `html` and `callflow-html`, an interactive terminal asks before opening the
generated page in the default browser. Non-interactive commands never prompt or
launch a browser.

For database credentials, prefer supported environment variables over
`--password`.

### `tree`

Listed under read/query; produces a filesystem/symbol HTML visualization.

## Graph diagnostics and merge operations

### `diagnose`

The `diagnose` command groups integrity checks for saved graph artifacts. Its
current public diagnostic is `multigraph`.

#### `diagnose multigraph`

```text
compass diagnose multigraph
  [--graph PATH]
  [--json]
  [--max-examples N]
  [--directed | --undirected]
  [--extract-path PATH]
```

### `merge-graphs`

```text
compass merge-graphs graph1.json graph2.json [...]
  [--out merged.json]
```

Inputs must have compatible directed/multigraph semantics.

### `merge-driver`

```text
compass merge-driver BASE CURRENT OTHER
```

Low-level managed integration surface for graph merge behavior.

### `cache-check`

```text
compass cache-check FILES_FROM
  [--root DIR]
  [--mode M | --deep]
  [--prompt-file PATH]
```

Checks whether cached semantic results can be reused for a file list, root,
mode, and prompt contract. Results are written visibly below the selected
output root as `cached.json` (when hits exist) and
`uncached.txt`.

### `merge-chunks`

```text
compass merge-chunks CHUNK_FILES... --out PATH
```

Validates and combines semantic chunk files into one output artifact.

### `merge-semantic`

```text
compass merge-semantic
  --cached PATH
  --new PATH
  --out PATH
```

These are pipeline helpers; use them when implementing or diagnosing semantic
workflows.

## Assistant and hook lifecycle

### `install`

```text
compass install
  [--project | --user]
  [--strict]
  [--platform P ... | --all]
  [--dry-run]
  [--require-all]
  [--format text|json]
```

Run `compass install --help` for the version's platform list. `--strict`
requires a project-scoped Claude target. With no explicit platform, Compass
detects agents and also installs the portable Agent Skills package. Dry-run
output includes the complete skill and adapter path plan and performs read-only
preflight checks.

An installed skill is owned through its `.compass-install.json` manifest, and
`compass ensure` reports drift before it builds: a managed skill that is
missing, or one whose files no longer match the manifest, is named on the
build's output with the command that repairs it. A missing managed file is an
incomplete install, so a plain `compass install` restores it. A file that was
edited since it was installed is never overwritten: the install fails with
`was modified since Compass installed it and will not be overwritten`, and the
operator decides whether to remove it and reinstall.

### `uninstall`

```text
compass uninstall
  [--project]
  [--purge]
  [--platform P | P]
```

Review targets before `--purge`.

### `upgrade`

```text
compass upgrade
```

Reads the bounded `compass.release/1` static manifest from the latest stable
Compass release, downloads the exact immutable-tag archive for the current
platform, verifies its declared size, SHA-256 digest, and reported version,
then replaces the running executable. Release discovery does not use the
rate-limited GitHub REST API. If the installed version is current or newer,
the command exits successfully without changing it.

### `hook`

```text
compass hook [install|uninstall|status]
```

### `hook-check`

```text
compass hook-check
```

Managed integration probe invoked by older Compass-generated integration
configuration. Current contextual integrations use `hook-guard`; people do not
normally invoke this command directly.

### `hook-guard`

```text
compass hook-guard [search|read [--strict]|gemini]
```

Managed stdin/stdout adapter used by installed search, read, and Gemini
integration hooks. Treat its input/output behavior as an internal integration
contract unless a release explicitly documents it as a public automation API.

## Providers and optional sources

### `provider`

```text
compass provider list
compass provider show NAME
compass provider add NAME
  --base-url URL
  --default-model MODEL
  --env-key KEY_VARIABLE_NAME
  [--pricing-input N]
  [--pricing-output N]
compass provider remove NAME
```

Built-in provider names cannot be overridden. The built-ins are `claude`,
`kimi`, `ollama`, `gemini`, `openai`, `deepseek`, `azure`, `bedrock`, and
`claude-cli`; credentials are read from each provider's documented environment
variable and are never written to the registry.

### `add`

```text
compass add URL
  [--author NAME]
  [--contributor NAME]
  [--dir ./raw]
```

Remote ingestion changes the filesystem and network state.

### `clone`

```text
compass clone GITHUB_URL
  [--branch BRANCH]
  [--out DIR]
```

Treat cloned content as untrusted.

## Cross-project and collaboration

### `global`

```text
compass global add graph.json [--as REPO_TAG]
compass global remove REPO_TAG
compass global list
compass global path
```

### `prs`

```text
compass prs [NUMBER]
  [--triage]
  [--worktrees]
  [--conflicts]
  [--wrong-base]
  [--base BRANCH]
  [--repo OWNER/REPO]
  [--graph PATH]
```

GitHub/network credentials may be required.

## Result memory and reflection

### `save-result`

```text
compass save-result
  --question Q
  (--answer A | --answer-file PATH)
  [--type T]
  [--nodes N1 N2 ...]
  [--outcome useful|dead_end|corrected]
  [--correction TEXT]
  [--memory-dir DIR]
```

### `reflect`

```text
compass reflect
  [--memory-dir DIR]
  [--out PATH]
  [--graph PATH]
  [--analysis PATH]
  [--labels PATH]
  [--half-life-days N]
  [--min-corroboration N]
  [--if-stale]
```

### `check-update`

```text
compass check-update PATH
```

Managed integration/update probe.

## Input selection conventions

- Current read commands default to `compass-out/graph.json`.
- `--graph PATH` selects a graph JSON.
- Typed code-query commands (`search`, `callers`, `callees`, `impact`,
  `explore`, and `node`) use `graph.json` by default. Their
  `--engine default|json|store` option selects the engine; `default` uses the
  validated SQLite sidecar when the build published one and otherwise falls
  back to JSON, `json` always reads graph.json, and `store` requires the
  sidecar and fails closed when it is missing or corrupt.
- `--at REV` selects an exact historical graph for supported reads.
- `--graph` and `--at` are mutually exclusive.
- Build `PATH` defaults are command-specific; run help before scripting.
- `COMPASS_OUT` can change the default output root for several compatible
  command families; explicit `--out` is clearer in automation.

## Store health and recovery

```text
compass store status [OUTPUT] [--format text|json]
compass store validate [OUTPUT] [--format text|json]
compass store backup [OUTPUT] --output BACKUP_DIR [--format text|json]
compass store restore --from BACKUP_DIR --into OUTPUT [--format text|json]
```

`status` is read-only and reports graph, shared SQLite store, selector, schema,
and digest state. `validate` requires a matching
`store/store.sqlite3`, the current snapshot, and that snapshot's `store.ref`;
a mismatch is an error, never an empty graph.
`backup` creates a new digest-bound directory after checkpointing SQLite.
`restore` validates that bundle and writes only to a new or empty destination.
The commands currently operate on the local SQLite adapter. The redb adapter is
library-only, and PostgreSQL/DynamoDB are future backends.

`graph.json` remains the complete portable authority. The default query engine
uses the validated SQLite sidecar when present; use `--engine json` to force
JSON or `--engine store` to require the sidecar. The explicit rebuild
runbook is [`scripts/rebuild_compass_store.sh`](../../scripts/rebuild_compass_store.sh);
the detailed operational workflow is in the
[Operations guide](../guides/operations.md).

## IDE and graph-inspection commands

```text
compass capabilities --format json
compass export json [--community ID]
compass export workbench-json [VIEW ...]
compass export html [VIEW ...]
compass export callflow-json --output PATH
compass program call-graph (--symbol SYMBOL | --source FILE --byte BYTE)
  [--direction callers|callees|both] [--depth N] --format json
compass history timeline [--rev REV] [--limit N [--after CURSOR]] --format json
compass history blind-spots [--rev REV] [--limit N] [--format text|json]
compass history change-counts REV [--parent REV] --format json
compass history diff OLD NEW [--root NAME] [--output PATH] --format jsonl
compass history export REV --format json [--community ID] [--node-limit N] --output PATH
```

`history timeline` is inspection-only and defaults to all commits reachable
from local refs. `--limit` returns the newest bounded page, and `--after` uses
the preceding page's opaque `nextCursor`. A cursor rejects local-ref changes
instead of silently mixing snapshots. Responses include `hasMore`,
`nextCursor`, and `totalEntries` once the final page establishes the exact
count. `history change-counts` requires existing preferred realizations with
the same complete build profile and never builds them. Its bounded structural
counts exclude source-coordinate, clustering/layout, and anchor-derived edge
identity churn while preserving topology and relationship multiplicity.
`history export` is also read-only; run `compass history build REV --code-only`
before exporting an uncached revision.
`history diff` streams an exhaustive,
deterministic record-level diff for selected immutable roots. It is read-only:
both revisions must already be materialized, and an uncached revision returns
the exact `compass history build REV --code-only` prerequisite instead of
starting extraction. It requires identical complete build profiles and
compatible graph engines, refuses to overwrite `--output`, and bounds stdout
for safety. This is distinct from the ranked `compass diff` semantic-review
report. Guided writers accept `--events jsonl`; stdout then contains
`compass.ide.progress/1` events and human diagnostics move to stderr.

`json` is the canonical versioned graph-presentation export. `viewer-json`
remains accepted as a deprecated compatibility alias.

## Grounded Agent Graph overlays

```text
compass agent-graph status [OPTIONS]
compass agent-graph prepare --source-span FILE:START_BYTE:END_BYTE [OPTIONS]
compass agent-graph apply --request FILE --enable-writes [OPTIONS]
compass agent-graph show ASSERTION_ID [OPTIONS]
compass agent-graph history [OPTIONS]
compass agent-graph audit --revision REVISION [OPTIONS]
compass agent-graph diff OLD_REVISION NEW_REVISION [OPTIONS]
compass agent-graph query --revision REVISION --cql QUERY [OPTIONS]
compass agent-graph export --revision REVISION --output FILE [OPTIONS]
compass agent-graph rebase-plan --revision SOURCE_REVISION [OPTIONS]
compass agent-graph rebase-commit --request FILE --enable-writes [OPTIONS]
```

Common selectors are `--root`, `--overlay`, `--revision`, `--profile
augment|curated`, and `--format text|json`. Select the current Base Generation
with `--graph`, or an exact immutable historical Base Generation with
`--realization`; these selectors are mutually exclusive. Non-Git current-tree
use requires `--state-root`. Writes are disabled unless the invocation includes
`--enable-writes`; curated masks additionally require `--allow-masks`.

`prepare` is read-only. Repeat `--base-node ID`, `--base-edge ID`, and
`--source-span FILE:START_BYTE:END_BYTE` as needed. Compass returns the selected
Base Generation, current `expectedRevision`, canonical digest-bound Base
references, and an apply-ready grounding submission. At least one source span
is required. Do not pass `--revision`: preparation pins the active revision
atomically with the selected Base Generation.

Apply accepts `compass.agent-graph.batch/1`; rebase commit accepts
`compass.agent-graph.rebase-commit/1`. Query remains read-only CompassQL.
Export writes a self-describing `compass.agent-graph.effective/1` document and
refuses an unsafe or existing destination. Usage errors exit `2`; typed domain,
conflict, authorization, verification, storage, and limit errors exit `1`.

For continuous coding-session enrichment, there is no separate long-running
mutation command. The assistant repeats `status` → `prepare` → `apply` →
`audit`/`diff` at explicit milestones and pins the receipt revision after each
successful batch. A changed Base Generation puts the loop behind
`rebase-plan`/`rebase-commit`; writes must stop until every rebase item is
resolved. The bundled Compass skill documents this lifecycle while keeping
ordinary query and watch operations read-only.

## Output and exit conventions

Human text goes to stdout on success. Diagnostics go to stderr.

History:

- success and read-only no-store status/list operations: exit `0`;
- usage: exit `2`;
- Git/provider/validation/corruption/storage: exit `1`.

CompassQL:

- source/options/compile: exit `2`;
- graph loading: exit `3`;
- execution/limit/cancellation/output: exit `4`.

Other command families preserve documented compatibility-specific codes. Test
the exact command boundary your automation uses.

## Related pages

- [Configuration reference](configuration.md)
- [Output reference](outputs.md)
- [CompassQL](../COMPASSQL.md)
- [Versioned history guide](../guides/versioned-history.md)

**Next step:** run `compass <command> --help` for the command you will automate,
then pin its input, structured output, and exit expectations in an integration
test.
