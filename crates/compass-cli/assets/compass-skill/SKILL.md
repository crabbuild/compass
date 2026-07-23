---
name: compass
description: "Use for questions about a codebase, its architecture, dependencies, history, change impact, or project artifacts—especially when compass-out exists or the user invokes /compass."
---

# Compass

Compass is the first navigation layer for codebase work. It builds and queries a
local knowledge graph with native commands. Use the graph to find the smallest
relevant source set, then verify important conclusions in the cited source.

## Invocation contract

If the user invokes `/compass --help` or `/compass -h` without another request,
run `compass --help`, return its current command summary, and stop.

Otherwise:

1. Treat an explicit user command as authoritative.
2. If no path is supplied for a build or refresh, use `.`.
3. Run `compass <command> --help` before inventing options or relying on a
   remembered flag.
4. Use the installed native `compass` executable. Never substitute another
   product, a Python module, or an unsupported command.
5. Keep the user's requested graph, revision, output directory, and provider
   explicit throughout the workflow. Do not silently fall back to another one.

If `compass` is unavailable, report that fact and provide the exact command that
would have been run. Do not emulate a successful Compass result with broad
source searches.

## Select the evidence before acting

Resolve these inputs first:

- Source root: the supplied path, otherwise `.`.
- Current graph: explicit `--graph`, otherwise `compass-out/graph.json`.
- Historical graph: explicit `--at REV`; never combine it with `--graph`.
- Output root: explicit `--out`, otherwise `compass-out/`.
- Semantic provider: only a provider explicitly selected or already configured.

Check whether graph output exists and whether repository guidance requires a
refresh. A historical request must stay pinned to its resolved commit. A merged
or global graph must preserve repository origin. If a command fails to load the
selected graph, stop and diagnose that selection instead of answering from a
different graph.

## Fast path: use an existing graph

When `compass-out/graph.json` exists and the user asks a natural-language
codebase question:

1. Run `compass reflect --if-stale`.
2. Read `compass-out/reflections/LESSONS.md` if it exists and is relevant.
3. Run `compass query "<question>"` before broad source searches.
4. Inspect the returned nodes, relations, and source locations.
5. Open only the source files needed to verify the answer.

Use the specialized navigation commands when they fit:

- `compass path "<source>" "<target>"` for a shortest known dependency path.
- `compass explain "<concept>"` for one node and its neighborhood.
- `compass affected "<symbol>" --depth N` for downstream review scope.
- `compass query --cql "..."` for exact, deterministic graph patterns.
- `compass tree` for a graph-aware repository tree.
- `compass query "<question>" --at REV` for an immutable historical graph.

Read `compass-out/GRAPH_REPORT.md` for repository-wide architecture, hubs, and
communities. When `compass-out/wiki/index.md` exists, navigate from the index
instead of opening wiki pages indiscriminately.

The graph is an evidence index, not permission to guess. Preserve edge direction,
confidence, and source provenance. Say when a path is absent or evidence is
ambiguous. Do not claim that an inferred edge is a directly observed call.

For a graph without useful matches, check freshness, selected graph, spelling,
and terminology before reading broadly. A targeted source search may verify or
debug a graph result; it should not silently replace the graph-first workflow.

## Build or refresh

Choose the least expensive command that satisfies the request:

- `compass update .` for local, deterministic structural extraction.
- `compass extract PATH --code-only` for explicit no-model extraction with
  optional native integrations.
- `compass extract PATH` when the user wants semantic facts from documents,
  papers, Office files, or images and accepts the configured provider.
- `compass cluster-only` when extraction is current and only communities or
  visual outputs need regeneration.
- `compass watch .` for continuous deterministic refresh during active work.

`update`, local queries, reports, and local exports do not require network
access. Semantic providers, URL ingestion, repository cloning, database pushes,
and HTTP serving may use the network; do not start them unless the request
requires them.

After modifying project code, run `compass update .` unless the repository gives
a more specific Compass instruction. If the refresh fails, report the failure
and do not describe the graph as current. Confirm the expected graph and report
exist after a successful build; an old file surviving a failed command is not a
successful refresh.

Community naming is a separate semantic operation. Use `compass label` only when
the user wants human-readable community labels and accepts provider use. Use
`--missing-only` to preserve existing curated labels when appropriate.

## Command routing

Do not force every request through `query`:

- Architecture or concept: `query`, then `explain`.
- Dependency route: `path`.
- Change-review scope: `affected`.
- Exact relationship or automation: `query --cql`.
- Repository structure: `tree`.
- Revision-specific evidence: `history`, `diff`, or `--at REV`.
- Stale structural output: `update`; stale semantic output: `extract`.
- Existing extraction with stale communities: `cluster-only`; stale names only:
  `label --missing-only`.
- Artifact delivery: `export`.
- Invalid or suspicious graph: `diagnose multigraph`.
- Cross-repository view: `global` or `merge-graphs`.

For the full public command inventory, mutability, and internal-command boundary,
load the complete command reference from the on-demand index below.

## Answering workflow

For architecture, dependency, and impact questions:

1. Query the graph with the user's terminology.
2. If results are weak, retry with concrete symbol, file, crate, or community
   names found in the report—do not broaden immediately to the whole repository.
3. Use `path`, `explain`, `affected`, or CompassQL to test the relationship.
4. Verify decisive facts in source.
5. Answer with the relevant path or source locations and distinguish observation
   from inference.
6. When the result will help future work, record it with `compass save-result`
   only if the user asked to preserve project knowledge or repository guidance
   says to do so.

For saved or generated artifacts, give the actual path. For long-running
commands such as `watch` and `serve`, report the process state and endpoint or
watched root. For mutating commands, report what changed and what was left
untouched.

## On-demand references

Load only the reference needed for the current request:

- Complete command inventory and lifecycle: `references/command-reference.md`
- Query, CompassQL, paths, explanations, impact: `references/query.md`
- Incremental refresh, clustering, output freshness: `references/update.md`
- Semantic extraction, providers, caches: `references/semantic-extraction.md`
- Community labeling and report regeneration: `references/labeling.md`
- Immutable commit graphs and diffs: `references/history.md`
- Hooks and assistant registration: `references/hooks.md`
- Watch mode and added external sources: `references/add-watch.md`
- Wiki, visual, graph-database exports: `references/exports.md`
- MCP serving and client boundaries: `references/serve.md`
- Repository cloning, PRs, global and merged graphs: `references/github-and-merge.md`
- Saved answers and learned project lessons: `references/reflections.md`
- Diagnostics, benchmarks, and recovery tools: `references/operations.md`
- Graph schema, confidence, and provenance: `references/extraction-spec.md`
- Network, credentials, destructive actions, and trust: `references/security-and-boundaries.md`

## Completion rules

- Prefer concise graph output and targeted source reads over dumping whole files.
- Treat `affected` as review scope, not proof that every result must change.
- Treat an empty query or missing path as evidence that the graph does not encode
  the relationship, not proof that the relationship cannot exist.
- Do not expose provider credentials, MCP API keys, or database passwords.
- Report the graph path or revision used when it is not the default current graph.
- Report whether a requested refresh, export, installation, or hook change
  actually completed.
- Do not invoke installation-managed commands (`hook-check`, `hook-guard`) or
  process workers directly unless diagnosing the integration that owns them.
- Do not report partial semantic extraction as complete unless the user selected
  and accepts `--allow-partial`; enumerate the warnings and missing scope.
