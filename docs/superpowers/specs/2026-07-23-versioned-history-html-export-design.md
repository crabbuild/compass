# Compass versioned-history HTML export

## Purpose

Compass will export an offline, self-contained time-travel viewer for every
materialized commit in its immutable history store. A user opens one HTML file,
selects a commit in a Git-style rail, and sees the complete code graph from that
exact commit. The viewer requires neither the source repository nor a server.

## Scope and command line

Add an all-history export form:

```text
compass history export [--format html] --output PATH [--force]
```

`html` is the default format for this no-revision form. It exports one preferred,
validated realization for every materialized commit. No revision positional is
accepted in this form; the output is inherently a history-wide viewer.

Retain the existing revision-specific forms without changing their meanings:

```text
compass history export REV --format graph-json --output PATH
compass history export REV --format compass-out --output PATH
```

The CLI must report a clear usage error for ambiguous combinations, such as a
revision with `--format html`, or no revision with a non-HTML format.

## Export source and manifest

The history store remains authoritative. The exporter enumerates all commits
with a preferred materialized realization, validates each realization before
reading its artifacts, then reconstructs its exact `GraphDocument`.

Each commit appears once. If a commit has alternate realizations, the current
preferred realization is exported; users can make a deliberate selection before
exporting with `compass history prefer`. The initial release does not expose
alternates in the viewer.

Embed a versioned manifest containing:

- repository display identity and export time;
- exporter/viewer schema and renderer versions;
- one entry per commit: full SHA, parent SHAs, subject, author, timestamp,
  preferred realization ID, extraction fingerprint, and node/edge counts;
- the embedded graph payload associated with each manifest entry; and
- an integrity digest for each payload.

Git metadata is display and navigation data. The embedded graph document is the
only graph source used by the viewer. The exported document must preserve the
full SHA and realization ID so its displayed selection is auditable.

## Self-contained HTML

The output is a single HTML file. It contains the viewer CSS and JavaScript,
the commit manifest, all graph payloads, and the graph-rendering dependency.
It makes no runtime HTTP requests. In particular, the current `vis-network`
CDN dependency must be bundled inline or replaced by an equivalent local,
licensed asset in the generated document.

The exporter may use deterministic payload deduplication and compression, but
only when the generated viewer can decode it without a network dependency. A
browser that cannot decode the selected payload must show a clear local error;
it must not load a different revision or fetch a substitute.

Writes are staged beside the target and atomically published only after the
entire document is generated. The output path must not already exist; `--force`
only confirms a large export and never authorizes overwriting a prior bundle.

## Viewer interaction

The viewer is a three-pane layout:

1. **Commit rail.** A searchable, filterable Git-style list shows commit SHA,
   subject, author/date, merge-parent count, and concise graph-change counts.
   The initial selection is the newest exported commit.
2. **Graph canvas.** Selecting a commit replaces the canvas with that commit's
   complete graph. Graph search, zoom, layout, filters, and node inspection are
   local viewer state and stay usable after a commit selection where possible.
3. **Inspector.** It shows the selected commit's exact identity, realization
   identity, parents, metadata, graph statistics, and an optional graph-change
   summary.

The primary view is always the complete graph at the selected commit. A
secondary `Compare with parent` control opens comparison with a user-chosen
parent for merge commits; it never replaces the full-graph default.

Commit selection updates the document title and URL fragment to
`#commit=<full-sha>`. Opening that fragment, and browser back/forward, restores
the corresponding commit selection. Invalid or unavailable fragments fall back
to the newest exported commit and surface a non-blocking message.

## Size, failures, and privacy

Before writing, Compass calculates and reports the estimated output size. When
it exceeds a documented safety threshold, the command requires `--force` to
continue. This check protects users from unexpectedly creating very large
history bundles without preventing intentional exports.

The command fails before publishing if history is unavailable, no preferred
realizations exist, a selected realization cannot be validated, Git metadata
cannot be resolved, the output path is unsafe, or the output cannot be written.
No partial destination may remain after failure.

At runtime, a corrupted embedded record results in a clear commit-local viewer
error. Other valid commits stay selectable. The viewer does not contact the
network, send telemetry, or infer data from the local checkout.

## Verification

Tests must cover:

- command parsing, including the HTML default and compatibility of the existing
  revision-specific formats;
- inclusion of every and only preferred materialized commit, in deterministic
  order, with exact SHA and realization provenance;
- validation failure and atomic cleanup before publication;
- self-contained output with no external script, stylesheet, data, or network
  reference;
- selection of a commit rendering its exact embedded graph;
- fragment selection and browser history restoration;
- optional comparison behavior for ordinary and merge commits;
- corrupt payload isolation in the viewer; and
- size warning/`--force` behavior.

## Non-goals

- Exporting alternate realizations in the first viewer release.
- Materializing Git commits during export.
- Replacing the existing `graph-json` or `compass-out` revision export forms.
- Hosting the viewer or introducing a server/API requirement.
