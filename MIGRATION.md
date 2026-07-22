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
