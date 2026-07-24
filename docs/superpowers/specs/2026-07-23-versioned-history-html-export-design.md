# Compass versioned-history HTML export

## Purpose

Compass will export an offline, self-contained time-travel viewer for every
materialized commit in its immutable history store. A user opens one HTML file,
selects a commit in a Git-style rail, and sees the complete code graph from that
exact commit. The viewer requires neither the source repository nor a server.

## Scope and command line

Add an all-history export form:

```text
compass history export [--format html] --output PATH [--title NAME] [--force]
```

`html` is the default format for this no-revision form. It exports one preferred,
validated realization for every materialized commit. No revision positional is
accepted in this form; the output is inherently a history-wide viewer. `--title`
sets the shareable display title without exposing the absolute local path.

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
History may retain a realization after Git history is rewritten and its commit
object becomes unavailable. That entry must still export using stored SHA,
parents, realization metadata, and graph data; subject, author, and timestamp
are explicitly labeled unavailable rather than failing the whole export.
Timeline ordering follows the stored parent DAG, not SHA order. Materialized
`HEAD` is the default; otherwise use its nearest materialized first-parent
ancestor, then a deterministic newest leaf.

## Self-contained HTML

The output is a single HTML file. It contains the viewer CSS and JavaScript,
the commit manifest, independently compressed per-commit graph payloads, and
the graph-rendering/decompression dependencies.
It makes no runtime HTTP requests. In particular, the current `vis-network`
CDN dependency must be bundled inline or replaced by an equivalent local,
licensed asset in the generated document.

The viewer lazily verifies, decompresses, and parses only the selected payload,
then retains at most three decoded graphs. A browser that cannot decode the
selected payload must show a clear commit-local error; it must not load a
different revision or fetch a substitute. A strict Content Security Policy
enforces the offline boundary.

Writes are staged beside the target and atomically published only after the
entire document is generated. The output path must not already exist; `--force`
only confirms a large export and never authorizes overwriting a prior bundle.

## Viewer interaction

The viewer is a three-pane layout:

1. **Commit rail.** A searchable, filterable Git-style list shows commit SHA,
   subject, author/date, merge-parent count, and concise graph-change counts.
   It visualizes parent/merge lanes and retained disconnected histories. The
   initial selection follows the `HEAD` fallback rule above.
2. **Graph canvas.** Selecting a commit replaces the canvas with that commit's
   complete graph. Graph search, zoom, layout, filters, and node inspection are
   local viewer state and stay usable after a commit selection where possible.
   Shared node IDs retain positions and selection across commits. Graphs above
   5,000 nodes open in community overview mode while keeping the exact graph
   embedded and searchable.
3. **Inspector.** It shows the selected commit's exact identity, realization
   identity, parents, metadata, graph statistics, and an optional graph-change
   summary.

The primary view is always the complete graph at the selected commit. A
secondary `Compare with parent` control opens comparison with a user-chosen
parent for merge commits; it never replaces the full-graph default. Comparison
uses Compass topology diff semantics and is disabled with an explanation when
the parent is absent or extraction profiles are not comparable.

Commit selection updates the document title and URL fragment to
`#commit=<full-sha>`. Opening that fragment, and browser back/forward, restores
the corresponding commit selection. Invalid or unavailable fragments fall back
to the manifest's default commit and surface a non-blocking message.

## Size, failures, and privacy

Compass streams into a staged file so it never retains every graph or a second
copy of the full HTML in memory. When exact staged size exceeds 256 MiB, the
command requires `--force` to publish. Dropping an unconfirmed stage removes it.
Publication is atomic and no-clobber even if another process creates the target
during export.

The command fails before publishing if history is unavailable, no preferred
realizations exist, a selected realization cannot be validated, the output
path is unsafe, or the output cannot be written. Missing optional Git
presentation metadata is not a failure.
No partial destination may remain after failure.

At runtime, a corrupted embedded record results in a clear commit-local viewer
error. Other valid commits stay selectable. The viewer does not contact the
network, send telemetry, infer data from the local checkout, or embed the
absolute local repository path.

The commit rail, graph controls, and inspector meet WCAG 2.2 AA, work at
320 CSS pixels, expose keyboard navigation and live loading/error status, use
44-by-44-pixel touch targets, honor reduced-motion preferences, use shared
design tokens, and support accessible light/dark themes without a network font.

## Verification

Tests must cover:

- command parsing, including the HTML default and compatibility of the existing
  revision-specific formats;
- inclusion of every and only preferred materialized commit, in deterministic
  order, with exact SHA and realization provenance;
- parent-DAG ordering, merge lanes, `HEAD` fallback, and rewritten-history
  metadata fallback;
- validation failure and atomic cleanup before publication;
- self-contained output with no external script, stylesheet, data, or network
  reference, enforced by CSP;
- independent compressed payload round-trips, lazy decode, integrity checks,
  corrupt-commit isolation, and the three-snapshot cache bound;
- selection of a commit rendering its exact embedded graph;
- stable node layout/selection across adjacent commits and large-graph overview;
- fragment selection and browser history restoration;
- optional comparison behavior for ordinary, merge, absent-parent, and
  profile-incompatible commits;
- keyboard, responsive, reduced-motion, and WCAG AA behavior in a real browser;
- exact size warning/`--force`, no-clobber race, and staging cleanup behavior.

## Non-goals

- Exporting alternate realizations in the first viewer release.
- Materializing Git commits during export.
- Replacing the existing `graph-json` or `compass-out` revision export forms.
- Hosting the viewer or introducing a server/API requirement.
