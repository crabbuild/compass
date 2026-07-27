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
contracts advertised by the extension. If a non-Compass or incompatible binary
is found first on `PATH`, Compass stops before running a workflow and offers
**Select Compass Binary** instead of displaying raw CLI usage output.

## Workflows

- Initialize, update, and watch a repository from the Compass activity bar.
- Explore the current graph with the active `compass export html` visual
  language adapted to the current VS Code theme. Single-click a node to inspect
  it; double-click an overview community to load its detailed graph, use
  **Overview** to go back, and double-click a detail node with an exact file and
  line/byte location to open source.
- Start a caller/callee graph from the function under the cursor in any
  call-capable language already represented in the Compass graph. Expand it by
  depth while retaining resolved, inferred, ambiguous, and unresolved calls.
- Read the broader architecture flow document in a separate editor tab.
- Run natural-language queries or deterministic CompassQL.
- Browse every reachable Git commit with graph states: graph available, not
  materialized, building, or failed.
- Explicitly build missing historical graphs, load exact revisions, and compare
  a commit with any parent using structural and semantic findings.

## Using the Compass activity bar

### Workspace

Workspace is the single command surface for Compass. Repository rows report
**Graph ready**, **Not initialized**, **Building**, or **Build failed** without
repeating workflow actions under every folder.

- **Explore** contains Code graph, Architecture flow, Call graph from cursor,
  Ask codebase, and Codebase evolution.
- **Maintain** contains Update graph and Watch for changes. It changes to
  **Stop watching** while the current repository watcher is active.
- **Active operations** appears only while a build or watcher is running.
- **Initialize repository** and **Retry graph build** appear only when the
  current workspace state requires them.

Choosing **Initialize repository** opens a dedicated setup wizard. Review the
repository scope, add include or exclude rules when needed, and start the first
index from the final review step. While Compass builds, the page shows the
completed and total file count plus the file currently being indexed.

**Refresh Compass Status** in the Workspace title reads repository state again.
It never initializes, updates, or watches a graph.

Open **Code graph** to inspect connected nodes, filter communities, search, and
open source. Drag the inspector divider to resize it, or use its header control
to collapse and expand it.

### Trace calls from the editor

1. Open an indexed source file and place the cursor anywhere inside a function
   or method body.
2. Right-click in the editor and choose **Compass Call Graph**.
3. Choose **Show Callers**, **Show Callees**, or **Show Callers & Callees**.
4. In the graph tab, use **Callers**, **Both**, and **Callees** to switch
   direction without returning to the editor. Use an **Expand** action to trace
   a continuation one level farther.
5. Double-click a located function node to open its source.

The same three direction commands are available from the Command Palette.
Compass uses the repository's structural graph for every supported language
and enriches it with Program IR when that artifact exists. The coverage notice
identifies structural-only or combined evidence and says when results are
partial. An empty direction means that Compass found the function but has no
represented relationship in that direction; it does not prove no runtime call
exists.

Compass discovers the CLI automatically from the configured location and then
from `PATH`. A setup row appears only when the executable is missing or
incompatible; a healthy CLI does not occupy the Workspace view.

In a multi-root workspace, Compass uses the repository containing the active
editor. If no repository is implied and more than one is eligible, Compass asks
you to choose one.

### Git commits and revision graphs

Open **Codebase evolution** from **Workspace > Explore** or the Command Palette.
The left rail lists every reachable Git commit and shows whether its Compass
graph is available, not materialized, building, or failed. Compass opens the
newest 100 commits first and loads more as you scroll; loaded pages remain
cached while the Codebase Evolution panel is open.

If revision graphs are disabled, choose **Enable revision graphs** inside
Codebase Evolution, then select either the recommended local **Code only**
profile or the **Compass default profile** resolved by the CLI. The timeline
reloads automatically after enablement.

Select a commit, then:

1. Choose **Build graph** if the revision is not materialized. Select the configured
   history profile, a local code-only build, or a profile reused from another
   revision.
2. Choose **Open graph** to explore an available revision.
3. Choose **Compare parent** to see structural and semantic changes after both
   revision graphs are available.
4. Choose **Query this revision** to run a query against that exact commit.

In a changed graph, select an aggregate community and choose **Inspect
changes**. Compass lazily compares that community at both revisions and shows
the exact added, removed, and changed symbols and relationships. Selecting a
changed symbol reveals a Before/After table containing only modified fields,
including signatures and source ranges, with source actions for both commits.
If either community exceeds `compass.graphNodeLimit`, the detail view labels
its counts as partial and explains how to increase the limit.

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
