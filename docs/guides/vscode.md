# Use Compass in VS Code

The Compass extension is a local, workspace-hosted interface to the separately
installed `compass` CLI. It works in desktop VS Code and on Remote SSH, WSL, and
Dev Container extension hosts. Browser-only `vscode.dev` is not supported.

## Set up

1. Install Compass and confirm `compass --version` works on the workspace host.
2. Install the Compass VSIX.
3. Open a trusted repository.
4. If the extension cannot find Compass on `PATH`, choose **Select Compass
   Binary** or set `compass.cliPath`.
5. Open the Compass activity bar and run **Initialize Repository**.

Initialization previews include and exclude globs before it writes
`.compass/config.toml`. The extension never installs a CLI automatically.
The CLI must support `compass capabilities --format json`. If capability
negotiation fails or a required versioned contract is missing, the extension
does not run the incompatible command; upgrade Compass or use **Compass: Select
CLI Binary**, then reload VS Code.

## Current graph

Choose **Open Code Graph** to use the same versioned graph model as
`compass export json`, rendered with the active `compass export html`
canvas structure, community palette, force layout, evidence styling, hover
metadata, and inspector concepts. VS Code colors take priority so the canvas
follows the active light, dark, or high-contrast theme; the Compass export
palette remains the fallback.

Search symbols and files, pause or resume the layout, fit or reset the view,
show labels, filter communities, and inspect connected nodes. Single-click a
node to select it without leaving the graph. Double-click a node to open its
exact source range when Compass provides a non-empty file plus line or byte
location. Nodes without that exact location remain inspectable and do not
trigger navigation. Graphs above 5,000 nodes use Compass's community overview.
Double-click an overview community to lazily load its complete member graph
through `compass export json --community ID`; then double-click a located source
node to open it. **Overview** returns to the community map. Historical graphs
use the same interaction against their exact commit.

## Calls and architecture

Place the cursor inside a function and run **Open Call Graph from Cursor**.
Compass resolves the cursor as a UTF-8 byte position, selects the innermost
function, and expands callers, callees, or both with explicit depth and size
bounds. Resolved, inferred, ambiguous, and unresolved calls use distinct labels
and visual treatment. Unresolved calls never prove that another call is absent.

Use **Open Architecture Flow** for the broader subsystem call-flow document,
cross-community relationships, symbol lists, call tables, confidence, and
source links.

## Query

Use **Query Codebase** for natural-language discovery or deterministic
CompassQL. CompassQL parameters and limits are sent as literal process
arguments. Use Cmd/Ctrl+Enter to run a query.

## Evolution

**Open Codebase Evolution** lists every commit reachable from all local refs.
Use `--rev` at the CLI boundary when a consumer intentionally wants one
reachable subgraph. Each commit
has one clear state:

- **Graph available** — an exact preferred realization can be opened.
- **Not materialized** — no graph exists; selecting it does not start a build.
- **Building** — an explicit history build is active.
- **Failed** — the latest build attempt failed without affecting other commits.

Choose **Build graph** to materialize a missing commit explicitly using the
configured profile, a code-only build, or a profile inherited from another
revision. Historical
exports are validated against the full commit and realization identity, decoded
in a three-entry memory cache, and removed from temporary storage after use.
The configured graph node limit applies to both current and historical
overview/detail exports.
Choose a parent to see structural counts and Compass semantic findings. Queries
can target the selected available revision; unavailable revisions stay
disabled and are never materialized implicitly.

The changed-graph tab starts with the bounded community overview. Select a
changed community and choose **Inspect changes** to load that community from
both revisions. The detail view provides a searchable affected-symbol list,
status-aware relationships, and a Before/After table for every modified symbol
field. **Open before** and **Open after** display read-only source from the
owning Git commit. If the configured `compass.graphNodeLimit` prevents either
community export from being complete, Compass keeps the aggregate comparison
available and marks the detailed counts as partial.

## Security

The extension is disabled for untrusted workspaces. It starts Compass with
argument arrays and `shell: false`, bounds captured output, validates every
versioned payload, rejects repository mismatches and path escapes, loads no
remote webview resources, and sends no telemetry.
