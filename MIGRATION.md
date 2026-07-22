# Migrating from Graphify to Compass

Compass can be adopted side by side. It reads and writes the existing
`graphify-out/` graph, cache, manifest, labels, analysis, memory, and sidecar
formats, so no data migration is required.

## Install

Use a release archive for a prebuilt binary, or install from the workspace:

```bash
git clone https://github.com/crabbuild/compass.git
cd compass
cargo install --locked --path crates/compass-cli
```

The installation provides:

- `compass` for the primary `compass <command>` interface;
- `graphify` for strict legacy CLI compatibility;
- `graphify-mcp` for existing MCP configurations.

No Python runtime or separately installed tree-sitter/native library is needed.

## Side-by-side qualification

Keep the Python executable available for one release cycle. Before changing an
automation, preserve or commit the current `graphify-out/` directory, then run
read-only commands through both interfaces:

```bash
python -m graphify query "authentication flow"
compass query "authentication flow"

python -m graphify path Router Database
compass path Router Database
```

Next, run Compass against a disposable working-tree copy and compare the full
`graphify-out/` tree after a cold build, unchanged warm build, and one-file
incremental update. The repository qualification script automates the Phase 1
graph and performance comparison:

```bash
COMPASS_BENCH_CORPUS=/path/to/corpus scripts/qualify_phase1.sh
```

Do not run two writers against the same output directory concurrently. Query
commands and the MCP server may read a completed graph while a guarded atomic
update is in progress.

## Cut over

Replace legacy invocations mechanically:

```text
graphify <command>       -> compass <command>
python -m graphify ...  -> compass ...
```

If exact legacy messages or scripts are important, use the native `graphify`
compatibility executable instead; it dispatches to the same Rust services.

Reinstall assistant integration from the native binary after cutover so hooks
and skill assets point at the intended command:

```bash
compass install --platform codex --project
```

Semantic and network-backed operations remain opt-in. Review provider keys,
custom endpoints, database DSNs, and TLS flags before enabling them in a new
environment. `extract --code-only` is the explicit local-only mode.

## Roll back

Rollback does not require converting graph data:

1. Stop Compass watchers and MCP servers.
2. Restore the preserved `graphify-out/` directory only if an interrupted or
   unwanted write occurred; completed Compass output is Python-compatible.
3. Resume the pinned Python `graphify` command.
4. Reinstall the desired Python-era assistant integration if its command path
   differs from the native installation.

Do not delete caches merely to roll back. Python and Compass share their baseline
layout, and retaining them makes rollback faster. If a future ledger entry adds
optional forward-compatible metadata, older Python ignores it; a future
breaking migration is prohibited without an explicit versioned converter and
separate rollback procedure.

## Troubleshooting

- Use `compass --help` to see only completed native commands.
- Use `graphify --help` when validating a legacy script's exact surface.
- Run `compass cache-check` before discarding a cache.
- Run `compass diagnose multigraph` before converting a suspect graph.
- Keep API keys out of command output and bug reports; native diagnostics
  redact configured secrets.
