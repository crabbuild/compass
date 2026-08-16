# Migrate from Graphify to Compass

Compass uses its own executable, output directory, environment variable, and
sidecars. Its output root now preserves the familiar flat artifact shape so
file-based workflows can transition while Compass's snapshot and store
layout remains visible and clearly owned.

## Install Compass

Install the latest macOS release:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/crabbuild/compass/releases/latest/download/install.sh | sh
```

The release contains only the `compass` executable. It doesn't install `graphify` or `graphify-mcp` compatibility entry points.

## Rebuild project output

Compass doesn't read `graphify-out/` or `GRAPHIFY_OUT`. Keep the old directory
for rollback and run a new build to create `compass-out/`:

```bash
cd your_project_directory
compass update .
```

Set `COMPASS_OUT` before running Compass when you need a custom output directory.

The new directory exposes the main artifacts directly:

```text
compass-out/
├── graph.json
├── graph.html        # unless --no-viz or the render limit omits it
├── GRAPH_REPORT.md
├── orientation.json  # versioned agent context bound to this exact graph.json
├── manifest.json
└── cache/
```

Scripts that open, copy, serve, mount, or archive these conventional filenames
only need the executable and output-directory rename. The JSON schema and
cache payloads remain Compass contracts: do not copy `graphify-out/cache/` or
`graphify-out/manifest.json` into `compass-out/`. Compass rebuilds them from
source while retaining its own internal snapshot and store protocols.

### Rebuild provisional Compass artifacts after the v1 identity reset

Compass currently makes no backward-compatibility promise for pre-release
internal artifacts. Extraction, cache, publication, store-index, query-index
and ranker, overview, qualification, and semantic-diff identities have been
reset to v1. Artifacts carrying provisional higher version numbers are not
migrated. Run a forced update with the current binary to publish one coherent
v1 artifact set:

```bash
compass update . --force
```

Disposable query indexes rebuild automatically on their next use.

## Update natural-query automation

Plain `compass query "<question>"` now returns structured discovery text by
default on a typed graph. Replace discovery text paging based on `--budget` and
numeric `--page` with `--text-budget` and the opaque `next=<cursor>` token.
Keep the question, discovery options, and graph unchanged while following a
cursor. Use explicit `--traverse` when an existing workflow intentionally needs
the former relevance traversal; its `--budget`/`--page` behavior remains.
CompassQL and explicit `ask`, `search`, `callers`, `callees`, and other typed
commands are unchanged.

Natural discovery now defaults to a focused 64-node, 128-edge neighborhood.
Automation that depended on the former 500-node, 1,000-edge default should pass
`--max-nodes 500 --max-edges 1000` explicitly. Those values remain supported
hard ceilings; the discovery JSON schema and deterministic result ordering are
unchanged.

MCP clients must read structured results from the `result` field of the
`compass.mcp.tool-result/1` envelope and inspect `transportTruncation`
separately from the domain result's own `truncated` field.

Run `compass update .` once after upgrading to publish `orientation.json` with
the exact `graph.json` digest. Agent-facing orientation/report exports fail
explicitly for older, missing, detached, or stale sidecars instead of pairing
evidence by filename alone.

The orientation contract is now `compass.orientation/2`. Consumers that parse
`orientation.json` must accept the new schema and may read its optional typed
`blindSpots` projection; older orientation files should be regenerated with
`compass update .` rather than edited in place.

## Select inference breadth explicitly when upgrading

Structural `init`, `update`, `extract`, and `watch` builds now default to
`--inference-level low`, which publishes exact relationships only. This is a
hard cutover from the former `max` default. If an automation or downstream
consumer requires deferred-receiver and all other retained inferred
relationships, add the former behavior explicitly:

```bash
compass update . --inference-level max
compass extract . --code-only --inference-level max
```

`medium` adds source-backed inferred relationships and `high` additionally
adds explicitly qualified external relationships. Existing schema-1 build
state that omitted the inference field is still interpreted as historical
`max`. Because new low profiles record the level explicitly, the first command
run without an override rebuilds and republishes the graph coherently instead
of reusing the wider graph.

## Opt into Program IR generation

Structural graph builds now omit the optional `program.json` artifact by
default. Add `--program` to `init`, `update`, `extract`, or `watch` when
program inspection or Program-backed enrichment is part of the workflow.
Supplying `--program-artifact` also enables Program IR on update, extract, and
watch. Existing scripts may keep using `--no-program`; it remains accepted as
an explicit structural-only spelling.

Rename repository and user configuration before the first build:

```text
.graphifyignore                  -> .compassignore
~/.graphify/providers.json       -> ~/.compass/providers.json
GRAPHIFY_*                       -> COMPASS_*
merge.graphify.*                 -> merge.compass.*
graphify://... MCP resources     -> compass://...
```

Compass does not fall back to the old names.

## Hard cut to visible Compass output state

Current output no longer hides Compass-owned files. The layout now uses
`snapshots/`, `current-snapshot`, `store/`, and
visible snapshot-local names such as `build-state.json` and
`analysis.json`.

Compass contains no compatibility reader, detector, or in-place migrator for
the former hidden layout. Archive or remove the entire old output directory
before running this version, then build it again:

```bash
mv compass-out compass-out-hidden-layout-backup
compass update .
```

The history realization schema and SQLite store format remain at v1. Compass
does not rewrite immutable realizations or map former hidden artifact paths.
Use `compass history build` to recreate any revision that must materialize with
the current visible artifact layout; archive the existing history database
first only when it is needed for audit or rollback.

## Compass Store sidecar upgrades

The `0.3.x` line supports the logical majors `compass.store/1`,
`compass.store.graph-snapshot/1`, and `compass.store.ref/1`. A patch release
can reopen and validate a same-major SQLite sidecar. Unknown majors, a missing
or corrupt `store.ref`, and redb or prototype files are rebuildable hard cuts;
they are not migrated in place.

The optimized immutable graph-index layout is also a hard cut from store files
created by the pre-release per-snapshot/chunked-payload implementation. New
SQLite state lives at
`compass-out/store/store.sqlite3`; current snapshots contain
only canonical `graph.json` and `store.ref`. Do not move an older database into
that location or copy a newer database into a snapshot directory. Preserve
`graph.json` and rebuild from source:

```bash
compass update . --out compass-out --force --store sqlite
compass store validate compass-out --format json
```

The disposable JSON query index advances to `compass-code-index/2` so its FTS
underscore tokenization matches store term postings. It is rebuilt
automatically on the first JSON query; no graph or store migration is needed.

Check an output before upgrading or collecting support evidence:

```bash
compass store status compass-out --format json
compass store validate compass-out --format json
```

Create and verify a rollback bundle before a planned upgrade:

```bash
compass store backup compass-out --output /safe/compass-store-backup
compass store restore --from /safe/compass-store-backup --into /safe/compass-out-check
compass store validate /safe/compass-out-check --format json
```

If validation fails after an upgrade, retain `graph.json` and rebuild the
sidecar from source:

```bash
scripts/rebuild_compass_store.sh . --out compass-out --compass compass
```

The script preserves existing sidecars in a timestamped rollback directory,
restores them if `compass update --force` fails, and never replaces the JSON
artifact. A normal `compass update --force --store sqlite` is also sufficient
when the old sidecar has already been removed. Builds without `--store sqlite`
now publish JSON only and remove store files from the new snapshot. Typed
queries use JSON by default; use `--engine store` to select a retained sidecar.

Downgrades must validate the output with the target binary. Do not reuse a
newer physical SQLite/redb file merely because its filename matches. Rebuild
when the target binary reports an unsupported major or adapter.

## Update node-trail direction handling

Typed node-trail queries now interpret operands as source then target and only
follow edges in their published direction. A route requiring reverse traversal no longer
appears as if it were a valid forward dependency; the response contains the
typed `direction_mismatch` diagnostic instead.

If a workflow intentionally needs the reverse route, swap its operands:

```bash
compass path "former-target" "former-source"
compass ask "path from former-target to former-source"
```

Consumers that exhaustively decode `compass.query/1` diagnostic codes must add
`direction_mismatch` before upgrading.

## Regenerate HTML graph exports

Normal builds now write the self-contained page directly to
`compass-out/graph.html`; `compass export html` repairs or regenerates the same
root path. Current Compass releases use the shared graph workbench and
do not preserve the previous export's DOM, CSS selectors, or remote
`vis-network` script boundary. Regenerate saved HTML exports with the current
`compass export html` command. Any private CSS overrides or browser automation
that targeted the old document structure must be updated to use visible roles
and labels in the new workbench.

The matching VS Code extension requires a CLI that advertises both `graph` and
`community_detail`. Upgrade Compass and the extension together; the extension
does not fall back to the older non-drill-down graph workflow.

Current VSIX builds require Compass CLI 0.3.0 or newer. If **Select Compass
CLI** labels an installation unsupported, upgrade that CLI or select another
detected installation. Releases below 0.3.0 and 0.3.0 prereleases cannot be
activated, even if they advertise some current capabilities.

## Replace commands

Replace Python and legacy executable invocations with `compass`:

```text
graphify <command>       -> compass <command>
python -m graphify ...  -> compass ...
```

Compass exposes its Model Context Protocol server through `compass serve`. Reinstall assistant integrations so generated hooks and instructions invoke `compass`:

```bash
compass install --platform codex --project
```

Keep the old Graphify installation and `graphify-out/` directory until the new `compass-out/` graph has passed your project checks. The two tools don't share runtime output paths.
