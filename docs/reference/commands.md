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
  [--store json|sqlite]
  [--yes]
  [--force]
```

Includes and excludes are repeatable. Interactive mode previews the effective
corpus before writing `.compass/config.toml`; scripts must pass `--yes`.
Replacing an existing configuration requires `--force`.
The initial build publishes JSON only by default. Pass `--store sqlite` to add
the shared SQLite snapshot and `store.ref` to that generation. The database
lives below the output root at `.compass-store/compass-store.sqlite3`; the
generation contains only the small reference beside `graph.json`.

### `update`

Make a saved current-tree graph match the project:

```text
compass update [PATH]
  [--program-artifact PATH]
  [--out DIR]
  [--store json|sqlite]
  [--no-program]
  [--no-cluster]
  [--force]
  [--no-viz]
  [--no-gitignore]
  [--exclude PATTERN]
  [--resolution N]
  [--exclude-hubs N]
```

Use for normal cold/incremental structural builds. The default also publishes
`program.json`; use `--no-program` when only `graph.json` is required. Supply a
verified offline SCIP index with repeatable `--program-artifact`. For Java,
fresh exact symbol evidence can disambiguate AST-proven call sites in
`graph.json`; stale, unverified, conflicting, and non-call references are not
projected. `--no-program` conflicts with `--program-artifact`.
Graph storage defaults to `json`; `--store sqlite` adds the validated local
store sidecar without replacing `graph.json`.

### `extract`

Expose the full build surface:

```text
compass extract [PATH]
  [--program-artifact PATH]
  [--no-program]
  [--code-only]
  [--cargo]
  [--google-workspace]
  [--postgres DSN]
  [--backend NAME]
  [--model MODEL]
  [--mode deep]
  [--token-budget N]
  [--max-concurrency N]
  [--max-workers N]
  [--api-timeout SECONDS]
  [--allow-partial]
  [--dedup-llm]
  [--timing]
  [--out DIR]
  [--store json|sqlite]
  [--no-cluster]
  [--force]
  [--no-viz]
  [--no-gitignore]
  [--exclude PATTERN]
  [--resolution N]
  [--exclude-hubs N]
```

Use `--code-only` for an explicit fully local structural profile.

`update`, `extract`, and watch rebuilds may succeed with a warning that Compass
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
  [--store json|sqlite]
  [--out DIR]
  [--no-cluster]
  [--no-viz]
  [--no-gitignore]
  [--exclude PATTERN]
  [--poll]
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

## Read and query

### `query`

Natural-language graph discovery:

```text
compass query "<question>"
  [--dfs]
  [--context VALUE]
  [--budget N]
  [--graph PATH | --at REV]
```

Query seeds prefer source-backed declarations over unresolved external-symbol
placeholders with the same callable label. Source-less placeholder nodes retain
an explicit `wiring=FILE:LOCATION` site, and traversed relationships render
their occurrence as `at=FILE:LOCATION`; neither is presented as a declaration
location.

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

Canonical language contract: [CompassQL](../COMPASSQL.md).

### `path`

```text
compass path "<source>" "<target>" [--graph PATH | --at REV]
```

Renders a shortest known graph path while preserving relationship direction.

### `explain`

```text
compass explain "<node>" [--graph PATH | --at REV]
```

Shows one node and incoming/outgoing connections. An exact node ID resolves
directly. When a label names multiple source-backed declarations, Compass lists
the candidates and their source ranges and asks for the full node ID instead of
silently selecting one. Connection lines include the stored relationship site.

### `affected`

```text
compass affected "<node-or-label>"
  [--relation R]
  [--depth N]
  [--graph PATH]
```

Traverses incoming impact-relevant relations.

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
rebuild the newer revision with `--profile-from OLD` when needed.
`--format html` requires `--output PATH` and writes a self-contained
interactive report containing the reviewer findings, unified/split source
diffs, the exact Git patch fallback, and meaningful code-graph changes.
`--output` is rejected for text and JSON; there is no alternate semantic-diff
export command.

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
mode, and prompt contract.

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

Downloads the latest stable Compass release for the current platform, verifies
its SHA-256 checksum and reported version, then replaces the running executable.
If the installed version is current or newer, the command exits successfully
without changing it.

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

Built-in provider names cannot be overridden.

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
  `--engine default|json|store` option selects the engine; both `default` and
  `json` use JSON, while `store` requires a validated SQLite sidecar.
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
`.compass-store/compass-store.sqlite3`, active snapshot, and generation
`store.ref`; a mismatch is an error, never an empty graph.
`backup` creates a new digest-bound directory after checkpointing SQLite.
`restore` validates that bundle and writes only to a new or empty destination.
The commands currently operate on the local SQLite adapter. The redb adapter is
library-only, and PostgreSQL/DynamoDB are future backends.

`graph.json` is the default complete engine. Use `--engine store` with typed
query commands only after a `--store sqlite` build. The explicit rebuild
runbook is [`scripts/rebuild_compass_store.sh`](../../scripts/rebuild_compass_store.sh);
the detailed durability, backup, GC, quota, and recovery policy is in the
[Compass Store operations guide](../guides/compass-store-operations.md).

## IDE and graph-inspection commands

```text
compass capabilities --format json
compass export json [--community ID]
compass export callflow-json --output PATH
compass program call-graph (--symbol SYMBOL | --source FILE --byte BYTE)
  [--direction callers|callees|both] [--depth N] --format json
compass history timeline [--rev REV] [--limit N [--after CURSOR]] --format json
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
`history diff` streams an exhaustive,
deterministic record-level diff for selected immutable roots. It may lazily
materialize a missing revision, requires identical complete build profiles and
compatible graph engines, refuses to overwrite `--output`, and bounds stdout
for safety. This is distinct from the ranked `compass diff` semantic-review
report. Guided writers accept `--events jsonl`; stdout then contains
`compass.ide.progress/1` events and human diagnostics move to stderr.

`json` is the canonical versioned graph-presentation export. `viewer-json`
remains accepted as a deprecated compatibility alias.

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
