# VS Code Architecture Flow Design

**Date:** 2026-07-28

**Status:** Approved

**Implementation root:** `/Users/haipingfu/graphify/compass`

## Context

The VS Code architecture panel currently invokes:

```text
compass export callflow-json --graph <graph.json>
```

The CLI returns one JSON document containing every section node and every
intra-section edge. `CompassProcessManager` buffers command output and rejects a
stream after 8 MiB. The Django fixture produces a 35,110,985-byte architecture
document from 50,944 symbols and 190,401 edges, so the extension terminates the
otherwise successful export with:

```text
Compass output exceeded the 8 MiB safety limit
```

Raising the limit alone would permit parsing, but the extension would still post
the complete document to the webview and React would search, sort, and retain all
rows. That would move the bottleneck from process capture to webview hydration.

The current call-flow model also retains only intra-section edge details.
Cross-section calls become aggregate `sourceSection`, `targetSection`, and
`calls` records. The UI can report that two subsystems communicate, but it cannot
show which symbols make those calls. Its card grid communicates a relationship
inventory rather than a system architecture.

## Goals

- Load the 35.1 MB Django architecture export without weakening the ordinary
  8 MiB command safety boundary.
- Keep every symbol and call reachable through search, filtering, paging, and
  source navigation.
- Preserve detailed caller, callee, relation, confidence, and source evidence
  for cross-subsystem calls.
- Make production architecture legible first while keeping tests, generated
  code, vendor code, and unknown scopes explicitly available.
- Replace the overview card grid with a directional, interactive architecture
  map that reflects subsystem topology and call volume.
- Keep the extension responsive by sending bounded view slices rather than the
  complete model to the webview.
- Preserve local-only operation, VS Code theme integration, keyboard access,
  high-contrast support, reduced motion, and cancellation.

## Non-goals

- Rendering tens of thousands of symbols simultaneously.
- Replacing the symbol-centered Call Graph workspace.
- Adding a database or persistent architecture index.
- Reimplementing Compass graph extraction or semantic inference in TypeScript.
- Automatically excluding tests or generated code without visible disclosure.
- Raising the output limit for unrelated Compass commands.
- Adding runtime network requests, telemetry, or external diagram services.

## Considered approaches

### 1. Architecture-specific 128 MiB capture with host-side projection — selected

The extension allows up to 128 MiB of stdout for the architecture export only.
It validates and retains the complete model in the extension host, then sends
small overview, search, subsystem, and route pages to the webview. The shared
viewer consumes a provider interface; VS Code uses a message-backed provider and
offline documents can use an in-memory provider.

This fits the measured Django payload with substantial headroom, keeps ordinary
commands at 8 MiB, avoids repeated CLI derivation, and makes webview work
proportional to what is visible.

### 2. Paged CLI commands

Compass could expose separate overview, section, route, and search commands.
This lowers extension-host memory but either reloads and re-derives the graph for
each page or requires a new indexed artifact. It creates a larger public CLI
surface and more process latency than the current payload requires.

### 3. Disk-backed streaming artifact

The CLI could write an indexed architecture artifact and the extension could
read records lazily. This provides the highest ceiling, but introduces artifact
lifecycle, atomicity, schema migration, and cleanup work disproportionate to the
35.1 MB reproduction.

## Selected architecture

The system keeps three authority layers:

```text
Compass output model
  complete symbols, internal calls, cross-subsystem calls, evidence, counts
          |
          | validated compass.viewer.callflow/1 JSON (up to 128 MiB)
          v
VS Code architecture controller
  owns full model, scope projection, search, paging, selection generation
          |
          | bounded typed messages
          v
Shared React architecture workspace
  owns visible interaction state, SVG map, tables, inspector, accessibility
```

The extension host is a presentation index, not a semantic engine. It groups,
filters, sorts, and pages fields already supplied by Compass. It does not infer
new calls or read private Compass storage.

### Process capture

`CompassProcessManager` gains per-command output limits. Existing callers retain
an 8 MiB stdout and stderr ceiling. The architecture controller requests a
128 MiB stdout ceiling while stderr remains bounded at 8 MiB.

Limits are measured in UTF-8 bytes rather than JavaScript string length. A limit
error identifies the stream and configured ceiling. Cancellation and child
termination behavior remain unchanged.

The 128 MiB value is a safety ceiling, not a webview payload target.

### Call-flow contract

The CLI keeps `compass.viewer.callflow/1` and adds optional fields alongside the
existing provenance and statistics:

- section summary counts independent of loaded row arrays;
- a source scope for each node: `production`, `test`, `generated`, `vendor`, or
  `unknown`;
- complete cross-section call records with source section, target section,
  caller, callee, relation, and confidence;
- coverage counts for internal, cross-section, and unassigned calls; and
- enough endpoint source metadata for the inspector and source navigation.

Every graph edge must be classified as internal, cross-section, or unassigned.
The model exposes all three totals. Section derivation places otherwise
unassigned nodes in `Other`; if an edge still cannot be represented, the UI
discloses its count instead of presenting the view as complete.

The capability report and VS Code compatibility requirement continue to
advertise `/1`. Original v1 payloads remain valid: the host preserves their
subsystem and aggregate route totals, treats their nodes as visible, and clearly
marks individual cross-route evidence as unavailable. Additive v1 payloads
provide production scoping and complete caller/callee evidence.

### Host projection and messaging

The architecture controller replaces its one-shot `hydrate` message with a
request/response protocol carrying repository identity, request identity, and
generation:

- `architectureOverview` supplies section summaries, overview connections,
  statistics, coverage, provenance, and active scope counts;
- `requestSection` / `sectionPage` supplies a bounded symbol or internal-call
  page for one section;
- `requestRoute` / `routePage` supplies the detailed calls behind one
  cross-section connection;
- `searchArchitecture` / `architectureSearchResults` searches the complete
  host model and returns ranked, bounded results;
- `setArchitectureScope` rebuilds summaries for production or all-code scope;
- `openSource`, `retry`, and `showOutput` retain their current responsibilities.

Responses that do not match the active repository, generation, and request are
ignored. The controller holds the model only for the panel lifetime and releases
it on disposal. Pages are deterministic and disclose total rows, current range,
and active filters.

Initial and interactive messages target less than 1 MiB. Row pages default to
100 calls or 100 symbols and remain user-pageable to the end of the collection.

## Information architecture and interaction

The workspace uses three coordinated regions:

```text
┌─ Subsystems ──────┬─ System map ──────────────────────┬─ Inspector ─────────┐
│ Core              │                                   │ Selected subsystem   │
│ Integrations      │   API ━━━━━━━▶ Models             │ Incoming routes      │
│ Infrastructure    │    ┃            ┃                  │ Outgoing routes      │
│ Tests             │    ▼            ▼                  │ Symbols and calls    │
│ Generated         │   Auth ──────▶ Storage             │ Evidence and source  │
└───────────────────┴───────────────────────────────────┴─────────────────────┘
```

The left rail groups sections by source scope and displays symbol and call
counts. It remains searchable and collapses to a selector at narrow widths.

The central SVG map is the signature interaction. It uses deterministic layered
layout, directed curves, arrowheads, and restrained motion:

- node area encodes subsystem symbol count;
- connection width encodes call volume using a capped logarithmic scale;
- solid lines indicate extracted-majority evidence;
- dashed lines indicate inferred-majority evidence;
- selecting a subsystem highlights its incoming and outgoing neighborhood;
- selecting a connection opens every underlying cross-subsystem call;
- unrelated topology dims but remains visible; and
- zoom, fit, reset, pan, keyboard traversal, and a table alternative are
  available without external diagram services.

The right inspector explains the current selection. For a subsystem it shows
scope, counts, top incoming and outgoing routes, source-file groups, symbols,
and internal calls. For a connection it shows source and target, total calls,
evidence distribution, and the paged underlying call records. Caller, callee,
and source paths open code through the extension host.

A compact top toolbar owns:

- global architecture search;
- `Production` and `All code` scope;
- extracted, inferred, and ambiguous evidence filters;
- fit/reset controls; and
- a visible coverage summary.

Production is the initial scope. The toolbar states, for example, “Production ·
17,674 of 50,944 symbols.” Tests and generated code are never silently removed:
their counts remain visible in the rail and the `All code` action makes them
fully searchable and inspectable.

The existing symbols and calls tables remain as accessible, exhaustive
alternatives to the map. Card grids are removed from the primary architecture
path because they obscure direction and topology at scale.

## Visual language

The architecture workspace should feel like a live engineering blueprint inside
VS Code, not a dashboard embedded in VS Code.

- Editor, sidebar, input, focus, selection, warning, and link colors continue to
  derive from VS Code variables.
- Subsystem colors are stable data colors with light, dark, and high-contrast
  variants; color is never the only evidence cue.
- The utility and data face remains the VS Code monospace font. Labels use the
  editor UI font for legibility.
- Borders, labels, route thickness, and negative space carry hierarchy. Shadows,
  gradients, and decorative cards are avoided.
- One orchestrated transition connects selection to highlighted routes and the
  inspector. Reduced-motion mode changes state immediately.

## Loading, empty, and error states

Loading progress distinguishes export, validation, indexing, and map
preparation. Large repositories receive factual copy such as “Indexing 190,401
calls locally” rather than an indefinite spinner.

Errors remain in the panel and provide `Retry` and `Show output` actions:

- exports between 8 MiB and 128 MiB load normally;
- output above 128 MiB reports the architecture-specific ceiling and does not
  suggest that the graph is corrupt;
- malformed `/2` data reports an incompatible export;
- disposal or retry cancels the active process and invalidates late responses;
- no matching filters produce an actionable empty state; and
- unassigned calls appear as a coverage warning with their exact count.

The controller logs repository path, phase, elapsed time, and payload byte count
without logging source-derived graph content.

## Performance and resilience

- The CLI runs once per panel hydration.
- The host retains one validated full model and small derived indexes.
- Search uses pre-normalized labels, paths, relations, and section names.
- The webview receives bounded pages and summary topology only.
- SVG renders subsystem-level topology, never all symbols.
- Tables paginate rather than mounting off-screen rows.
- Scope and evidence changes reuse host indexes.
- Panel disposal releases the model, indexes, listeners, and active process.
- Narrow layouts move the inspector below the map; they do not discard
  functionality.

## Testing strategy

Implementation is organized as context-rich production slices. Each slice is
implemented first, followed immediately by focused regression tests and fresh
verification before the next slice.

### Rust model tests

- `/2` serializes section summaries and source scopes.
- A cross-section edge retains its caller, callee, sections, relation, and
  confidence.
- `internal + cross-section + unassigned` equals total graph edges.
- Production, test, generated, vendor, and unknown fixtures classify
  deterministically.
- The Django-shaped distribution does not lose test or cross-section data.

### Extension-host tests

- Ordinary stdout still fails above 8 MiB.
- Architecture stdout succeeds above 8 MiB and through 128 MiB.
- UTF-8 byte counts, stdout and stderr limits, cancellation, and kill behavior
  remain correct.
- Overview hydration never posts the full retained model.
- Section, route, search, scope, and paging requests return bounded,
  identity-matched results.
- Retry and disposal reject stale responses and release retained state.

### Viewer tests

- Production is the initial, visibly disclosed scope.
- `All code` exposes test and generated sections.
- The map renders directed accessible connections and coverage counts.
- Selecting a subsystem highlights both incoming and outgoing routes.
- Selecting a route exposes complete paginated call evidence.
- Search reaches results outside the current page.
- Keyboard navigation, table alternative, high contrast, narrow layout, and
  reduced motion preserve the same information.

### Integration and qualification

- A synthetic call-flow export larger than 8 MiB and smaller than 128 MiB
  hydrates successfully.
- The 35,110,985-byte Django export loads, reports 50,944 symbols and 190,401
  edges, and exposes detailed cross-subsystem calls.
- Type checks, viewer tests, extension tests, Rust tests, production builds, VSIX
  smoke checks, and `graphify update .` pass before completion.

## Acceptance criteria

1. Opening Django architecture no longer emits the 8 MiB failure.
2. Ordinary Compass commands retain the 8 MiB safety limit.
3. No webview hydration message contains the complete 35.1 MB model.
4. The initial view visibly identifies its production-only scope and full totals.
5. Every section and symbol is reachable in `All code`, including tests and
   generated code.
6. Every represented graph edge is reachable as an internal or cross-section
   call; any unassigned count is explicitly disclosed.
7. Selecting a subsystem connection reveals its complete caller/callee evidence
   with source navigation and pagination.
8. The overview is a directional subsystem map with volume and evidence encoded
   accessibly.
9. Search covers the complete retained model rather than the currently rendered
   page.
10. Loading, cancellation, retry, errors, high contrast, reduced motion,
    keyboard use, and narrow layouts remain functional.

## Rollout

The `/2` CLI contract and matching extension requirement ship together. The
architecture panel does not attempt to interpret `/1` as `/2`; it shows the
standard upgrade path. No graph artifact migration is required because the
model is derived from the existing public graph on demand.
