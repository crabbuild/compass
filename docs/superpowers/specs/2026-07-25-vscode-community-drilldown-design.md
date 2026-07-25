# VS Code Community Drill-Down Design

Date: 2026-07-25

## Goal

Give the Compass VS Code graph the same conditional double-click semantics as
the active `compass export html` experience:

- double-click an aggregated community to load and enter its detailed graph;
- double-click a regular node with an exact source location to open that file
  and range; and
- return from a community detail to the already-loaded overview.

Community details must load lazily through the installed Compass CLI. The
extension must not invoke Graphify or read Compass graph artifacts directly.
The active HTML exporter remains unchanged.

## Public CLI

`json` becomes the canonical public name for the versioned graph presentation
model:

```text
compass export json [--graph PATH] [--labels PATH] [--node-limit N]
compass export json --community ID [--graph PATH] [--labels PATH] [--node-limit N]
compass history export REV --format json [--community ID] --output PATH
```

The payload remains `compass.viewer.graph/1`. The schema name does not change
because command spelling and payload compatibility are separate concerns.

`viewer-json` remains a deprecated compatibility alias for existing extension
versions and scripts. It is removed from primary help, examples, and new
extension invocations, but continues to accept the same options and return the
same payload. Historical `--format viewer-json` receives the same alias
treatment.

## Community Detail Contract

Without `--community`, the existing node-limit behavior remains:

- graphs within the limit return their ordinary symbol graph;
- explicitly limited larger graphs return the aggregated community overview;
  and
- overview community nodes include `memberCount`.

With `--community ID`, Compass loads the original graph and community
assignment, selects only members of that community, and emits:

- every selected member node;
- internal edges whose source and target are both selected members;
- the selected community metadata and color;
- relevant hyperedges that can be represented without dangling members; and
- `stats.aggregated = false`.

The returned detail uses the existing graph-view schema. The request fails
closed when the community ID does not exist, is malformed, or exceeds the
configured detail node limit. It never silently returns a partial community.
The error reports the member count and explains that the VS Code graph node
limit can be increased.

## VS Code Data Flow

The React canvas emits a node ID on double-click. `CompassGraph` resolves the
node and routes exactly one action:

1. If the current model is aggregated and the node has `memberCount`, call
   `openCommunity(communityId)`.
2. Otherwise, if the node has a non-empty file plus a line or byte location,
   call `openSource(source)`.
3. Otherwise, do nothing beyond the single-click selection that already
   occurred.

The current graph webview sends an `openCommunity` message containing the
repository identity and non-negative community ID. `GraphPanel` verifies that
the requested community exists in its active overview, then runs:

```text
compass export json --graph GRAPH --node-limit LIMIT --community ID
```

The history webview sends the same semantic request with the currently loaded
revision identity. `HistoryPanel` runs the corresponding historical export:

```text
compass history export REV --format json --community ID --output TEMP
```

Every response is validated with `GraphViewModelSchema` before it crosses into
the webview.

## Viewer State and Navigation

`CompassGraph` receives an overview model and an optional active detail model.
It maintains no filesystem or CLI knowledge.

While loading:

- the overview remains visible;
- the selected community remains focused;
- the toolbar reports that the community is loading; and
- duplicate requests for that community are ignored.

On success, the canvas switches to the detail model and shows a visible
**Back to community overview** action. Back navigation is immediate and uses
the overview already in memory. Selection, hover state, hidden-community
filters, and saved canvas view are reset when crossing the overview/detail
boundary so state from one graph cannot leak into the other.

On failure, the overview remains active and an accessible error appears in the
inspector or toolbar. The user can retry by double-clicking again.

The panel keeps a bounded three-entry community-detail cache for its current
graph identity. Updating the current graph, loading a different historical
revision, or disposing the panel clears that cache.

## Messages and Security

Host/webview contracts add discriminated messages for:

- requesting a community detail;
- delivering a validated community detail;
- reporting a community-load error; and
- returning to the overview.

Repository identity checks remain mandatory. Community IDs are integers,
non-negative, and must exist in the active overview. CLI arguments remain
arrays with `shell: false`. Historical temporary files retain the existing
owner-only storage, validation, and cleanup behavior.

## Compatibility and Capabilities

The capability schema continues to advertise
`graph_viewer: compass.viewer.graph/1`. A feature flag indicates lazy community
detail support so the extension can gate the interaction when paired with an
older Compass CLI. If unavailable, community nodes remain inspectable and the
inspector explains that Compass must be upgraded for drill-down.

## Error Cases

- Unknown community: keep the overview and report that the community is no
  longer present.
- Oversized community: keep the overview and report its member count and the
  relevant node-limit setting.
- CLI cancellation or panel disposal: stop the process and publish no result.
- Graph refresh or revision change during a request: discard the stale response
  using the request and graph identities.
- Invalid payload: reject it at the host boundary and show the standard Compass
  graph-load error without hydrating untrusted data.

## Verification Strategy

Implementation will precede test creation, per the explicit request not to use
TDD. Regression coverage is added after the behavior works.

Coverage must include:

- Rust model filtering for nodes, internal edges, community metadata,
  hyperedges, unknown IDs, and node-limit failures;
- current and historical CLI tests for canonical `json`, `--community`, and
  deprecated `viewer-json` compatibility;
- contract tests for all new host/webview messages;
- reducer/component tests for loading, success, failure, and back navigation;
- Chromium interaction proving actual community-node double-click enters
  detail, source-node double-click still opens source, and Back restores the
  overview; and
- real VS Code host coverage proving only Compass CLI argument arrays are used.

Final qualification includes Rust formatting, strict Clippy, relevant Rust and
JavaScript suites, Chromium accessibility/responsive checks, deterministic
asset verification, Compass graph refresh, VSIX packaging, and VSIX smoke
inspection.
