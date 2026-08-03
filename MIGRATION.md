# Migrate from Graphify to Compass

Compass uses its own executable, output directory, environment variable, and sidecars. The first public release makes a clean break from Graphify compatibility paths.

## Install Compass

Install the latest macOS release:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/crabbuild/compass/releases/latest/download/install.sh | sh
```

The release contains only the `compass` executable. It doesn't install `graphify` or `graphify-mcp` compatibility entry points.

## Rebuild project output

Compass doesn't read `graphify-out/` or `GRAPHIFY_OUT`. Run a new build to create `compass-out/`:

```bash
cd your_project_directory
compass update .
```

Set `COMPASS_OUT` before running Compass when you need a custom output directory.

Rename repository and user configuration before the first build:

```text
.graphifyignore                  -> .compassignore
~/.graphify/providers.json       -> ~/.compass/providers.json
GRAPHIFY_*                       -> COMPASS_*
merge.graphify.*                 -> merge.compass.*
graphify://... MCP resources     -> compass://...
```

Compass does not fall back to the old names.

## Compass Store sidecar upgrades

The `0.3.x` line supports the logical majors `compass.store/1`,
`compass.store.graph-snapshot/1`, and `compass.store.ref/1`. A patch release
can reopen and validate a same-major SQLite sidecar. Unknown majors, a missing
or corrupt `store.ref`, and redb or prototype files are rebuildable hard cuts;
they are not migrated in place.

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
now publish JSON only and remove store files from the new generation. Typed
queries use JSON by default; use `--engine store` to select a retained sidecar.

Downgrades must validate the output with the target binary. Do not reuse a
newer physical SQLite/redb file merely because its filename matches. Rebuild
when the target binary reports an unsupported major or adapter.

## Regenerate HTML graph exports

Current Compass releases use the shared graph workbench for `graph.html` and
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
