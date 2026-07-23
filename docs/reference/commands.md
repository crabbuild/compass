# Command reference

This reference groups the public `compass` command surface by responsibility.
Run `compass <command> --help` for the exact options in the installed version;
this page explains how the families fit together and which outputs are stable
for automation.

> **Who this reference is for:** users and integrators looking up a command
> without reading a tutorial.
>
> **You will learn:** public command families, primary inputs/outputs, shared
> conventions, and where exact subcommand contracts live.
>
> **Prerequisites:** Compass installed.
>
> **Reading time:** 12–15 minutes as a survey; use the tables for lookup.

## Global entry points

```bash
compass --help
compass --version
compass <command> --help
```

The shipped product executable is `compass`. Scripts and new documentation
should not depend on the internal Graphify compatibility frontend.

## Build and analysis

### `update`

Make a saved current-tree graph match the project:

```text
compass update [PATH]
  [--out DIR]
  [--no-cluster]
  [--force]
  [--no-viz]
  [--no-gitignore]
  [--exclude PATTERN]
  [--resolution N]
  [--exclude-hubs N]
```

Use for normal cold/incremental structural builds.

### `extract`

Expose the full build surface:

```text
compass extract [PATH]
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
  [--no-cluster]
  [--force]
  [--no-viz]
  [--no-gitignore]
  [--exclude PATTERN]
  [--resolution N]
  [--exclude-hubs N]
```

Use `--code-only` for an explicit fully local structural profile.

### `watch`

```text
compass watch [PATH]
  [--debounce SECONDS]
  [--out DIR]
  [--no-cluster]
  [--no-viz]
  [--no-gitignore]
  [--exclude PATTERN]
  [--poll]
```

Long-running filesystem watcher. Use a manual `update` as its recovery oracle.

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

Canonical language contract: [CompassQL 1](../COMPASSQL.md).

### `path`

```text
compass path "<source>" "<target>" [--graph PATH | --at REV]
```

Renders a shortest known graph path while preserving relationship direction.

### `explain`

```text
compass explain "<node>" [--graph PATH | --at REV]
```

Shows one node and incoming/outgoing connections.

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
compass history build REV [build-profile options|--profile-from REV|REALIZATION] [--format text|json]
compass history rebuild REV [build-profile options] [--replace-corrupt] [--format text|json]
compass history list [REV] [--format text|json]
compass history show REALIZATION [--format text|json]
compass history prefer REV REALIZATION [--format text|json]
compass history export REV --format graph-json|compass-out --output PATH
compass history gc [--prune-non-preferred] [--yes] [--format text|json]
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
  [--detailed]
  [--format text|json]
  [--topology-only]
  [--include-locations]
  [--include-analysis]
  [--include-metadata]
  [--fingerprint SHA]
  [--allow-profile-mismatch]
```

`--detailed` is a human format and cannot be combined with JSON.

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
  [--project]
  [--strict]
  [--platform P | P]
```

Run `compass install --help` for the version's platform list. `--strict`
requires a supported project-scoped hook target.

### `uninstall`

```text
compass uninstall
  [--project]
  [--purge]
  [--platform P | P]
```

Review targets before `--purge`.

### `hook`

```text
compass hook [install|uninstall|status]
```

### `hook-check`

```text
compass hook-check
```

Managed integration probe installed for supported assistants. It is normally
invoked by generated integration configuration rather than by a person.

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
- `--at REV` selects an exact historical graph for supported reads.
- `--graph` and `--at` are mutually exclusive.
- Build `PATH` defaults are command-specific; run help before scripting.
- `COMPASS_OUT` can change the default output root for several compatible
  command families; explicit `--out` is clearer in automation.

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
- [CompassQL 1](../COMPASSQL.md)
- [Versioned history guide](../guides/versioned-history.md)

**Next step:** run `compass <command> --help` for the command you will automate,
then pin its input, structured output, and exit expectations in an integration
test.
