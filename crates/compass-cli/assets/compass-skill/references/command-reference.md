# Compass command reference

Load this reference when selecting a command, reviewing automation, or checking
whether a Compass capability is covered by the installed skill. Run
`compass <command> --help` for the current option syntax.

## Read and navigate

- `compass query`: natural-language graph traversal or deterministic CompassQL.
- `compass program`: inspect canonical Program IR functions, coverage, callers,
  and call evidence, or run CompassQL over its read-only graph projection.
- `compass path`: shortest known graph route between two matched nodes.
- `compass explain`: one matched node plus its local neighborhood.
- `compass affected`: downstream review candidates, optionally filtered by
  relation and depth.
- `compass tree`: repository hierarchy enriched with graph metadata.
- `compass history`: configure, materialize, inspect, export, prefer, or garbage
  collect immutable commit realizations.
- `compass diff`: compare two revision graphs and optionally topology, locations,
  analysis, or metadata.
- `compass benchmark`: measure query behavior for one graph on the current
  machine.
- `compass diagnose multigraph`: validate graph shape and report problematic
  parallel or reciprocal relationships.
- `compass check-update`: report whether watched semantic changes left a pending
  refresh marker.

These are read-only unless a missing historical realization must be materialized.
An `--at REV` query can therefore create local history-store artifacts even
though the query itself does not edit the working tree.

## Build and enrich

- `compass update`: deterministic structural refresh.
- `compass extract`: structural plus optional semantic, Cargo, PostgreSQL, or
  Google Workspace layers.
- `compass watch`: long-running deterministic refresh and semantic-staleness
  detection.
- `compass cluster-only`: recompute communities and visual/report artifacts from
  existing extraction.
- `compass label`: generate or complete human-readable community labels.
- `compass add`: download an external URL into the selected local source folder.
- `compass clone`: clone or update a supported GitHub repository checkout.

`extract`, `label`, `add`, and `clone` may cross a network boundary. `watch` is
long-lived. Every command in this group writes local state.

## Publish, compose, and serve

- `compass export`: create HTML, call-flow HTML, Obsidian, wiki, SVG, GraphML,
  Neo4j, or FalkorDB artifacts; database `--push` is a remote write.
- `compass serve`: long-running MCP server over stdio or HTTP.
- `compass merge-graphs`: compose explicit graph JSON files into one artifact.
- `compass global`: add, remove, list, or locate graphs in the cross-project
  registry.
- `compass prs`: inspect pull requests, worktrees, likely conflicts, base
  branches, and graph impact.
- `compass merge-driver`: three-way merge entry point for a configured Git merge
  driver, not an ordinary file merge command.

Keep repository origins and graph revisions visible when composing results.
Never treat PR impact as proof of a textual conflict.

## Knowledge and semantic operations

- `compass provider`: add, list, show, or remove semantic provider definitions.
- `compass save-result`: persist a useful, dead-end, or corrected query result.
- `compass reflect`: synthesize corroborated saved results into project lessons.
- `compass cache-check`: split semantic inputs into cached and uncached work.
- `compass merge-chunks`: combine valid semantic result chunks.
- `compass merge-semantic`: combine cached and newly produced semantic layers.

Provider mutations change user configuration. Result saving and reflection write
durable project knowledge. The three cache/merge commands are low-level recovery
tools and must not replace normal extraction without a reason.

## Installation and hooks

- `compass install`: install the canonical skill and platform integration.
- `compass uninstall`: remove managed integrations; `--purge` additionally
  removes Compass output and requires explicit user intent.
- `compass hook`: install, inspect, or uninstall repository refresh hooks.
- `compass hook-check`: no-op probe owned by installed hook configuration.
- `compass hook-guard`: adapter owned by installed search/read/Gemini guards.

Do not call `compass hook-check` or `compass hook-guard` as ordinary user
workflows. Their stdin/stdout contracts are platform integration details.

`history-worker`, `hook-spawn`, and `hook-refresh` are intentionally absent from
the public command list. They are process and hook implementation details. Do
not invoke them directly; use `compass history`, `compass hook`, or
`compass install` to manage the owning lifecycle.
