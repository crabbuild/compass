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

## Using the Compass activity bar

### Repository

Repository shows the Compass state of every folder in the current VS Code
workspace. Expand a repository to use the actions that match its current state:

- **Initialize repository** creates the first local graph in
  `<repository>/compass-out/`.
- **Open graph** opens the current code graph. Use the right inspector to search,
  inspect connected nodes, filter communities, and open source. Drag the inspector
  divider to resize it, or use its header control to collapse and expand it.
- **Codebase evolution** opens Git and graph-build history.
- **Update graph** retries a failed build.

Compass discovers the CLI automatically from the configured location and then
from `PATH`. A CLI row appears only when the executable is missing or incompatible;
a healthy CLI path does not occupy the Repository view.

### Operations

Operations is the command center. Its groups expose the workflows that can run for
the current workspace:

- **Build** — initialize a repository, update a graph, and start or stop watch.
- **Explore** — open the graph, trace a call graph from the cursor, read the
  architecture flow, or query the codebase.
- **History** — open Codebase Evolution.
- **Active operations** — see builds and watchers currently running. Select an
  active watcher to stop it.

In a multi-root workspace, Compass uses the repository attached to the clicked
Repository action. Operations asks you to choose a repository when an action could
apply to more than one folder.

### Git commits and revision graphs

Open **Codebase evolution** from Repository, Operations, the Repository title bar,
or the Command Palette. The left rail lists every reachable Git commit and shows
whether its Compass graph is available, not materialized, building, or failed.

Select a commit, then:

1. Choose **Build graph** if the revision is not materialized. Select the configured
   history profile, a local code-only build, or a profile reused from another
   revision.
2. Choose **Open graph** to explore an available revision.
3. Choose **Compare parent** to see structural and semantic changes after both
   revision graphs are available.
4. Choose **Query this revision** to run a query against that exact commit.

Opening Codebase Evolution never builds historical graphs automatically. Revision
builds are explicit because they can take time and may use a configured semantic
provider.

Compass runs only in trusted workspaces. Browser-only `vscode.dev` is not
supported; Remote SSH, WSL, and Dev Containers run Compass on the remote
extension host.

## Privacy and safety

All graph and query processing is local unless you explicitly configure a
Compass semantic provider. Webviews contain only local assets. CLI processes
are spawned with argument arrays and no shell.
