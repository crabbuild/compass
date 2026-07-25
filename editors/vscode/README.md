# Compass for VS Code

Compass brings the local-first Compass code graph into VS Code. The extension
uses the same React graph viewer and versioned models as Compass's offline
exports.

## Requirements

Install the `compass` CLI separately on the same machine or remote extension
host where VS Code opens the workspace. The extension never bundles a native
binary and never sends telemetry.

If `compass` is not on `PATH`, set **Compass: CLI Path** or choose **Select
Compass Binary** from the guided setup.

The CLI must support `compass capabilities --format json` and the versioned
contracts advertised by the extension. If an older or Graphify-compatible
binary is found first on `PATH`, Compass stops before running a workflow and
offers **Select Compass Binary** instead of displaying raw CLI usage output.

## Workflows

- Initialize, update, and watch a repository from the Compass activity bar.
- Explore the current graph with the active `compass export html` visual
  language adapted to the current VS Code theme. Single-click a node to inspect
  it; double-click an overview community to load its detailed graph, use
  **Overview** to go back, and double-click a detail node with an exact file and
  line/byte location to open source.
- Start a caller/callee graph from the function under the cursor and expand it
  by depth while retaining resolved, inferred, ambiguous, and unresolved calls.
- Read the broader architecture flow document in a separate editor tab.
- Run natural-language queries or deterministic CompassQL.
- Browse every reachable Git commit with graph states: graph available, not
  materialized, building, or failed.
- Explicitly build missing historical graphs, load exact revisions, and compare
  a commit with any parent using structural and semantic findings.

Compass runs only in trusted workspaces. Browser-only `vscode.dev` is not
supported; Remote SSH, WSL, and Dev Containers run Compass on the remote
extension host.

## Privacy and safety

All graph and query processing is local unless you explicitly configure a
Compass semantic provider. Webviews contain only local assets. CLI processes
are spawned with argument arrays and no shell.
