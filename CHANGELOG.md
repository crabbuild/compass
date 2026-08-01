# Changelog

## Unreleased


- Reduce immutable history build overhead with stage-level artifact-load
  instrumentation, verified single-pass graph and Program loading, concurrent
  graph/Program reads, and an in-process typed-artifact handoff for native
  code-only builds. Also remove redundant trusted-graph, ordering,
  source-inventory, and Program module/function storage; encode independent
  records in parallel while retaining exact export and backward read
  compatibility, and reject stale worktree paths in shared AST caches.
- Preserve exact framework route aliases when incremental builds mix portable
  cache paths with absolute universal bindings.

- Hard-cut `graph.html` onto the shared Compass/VS Code graph workbench. HTML
  exports are now self-contained, follow light and dark system themes, validate
  embedded community details on demand, and support double-click community
  drill-down with immediate return to the overview.
- Require the VS Code extension's current graph workflow to negotiate both the
  `graph` and `community_detail` capabilities, accelerate immutable large-graph
  snapshots with copy-on-write filesystem clones when available, and remove
  repeated linear node lookups from graph interactions.

## 0.2.0 - 2026-08-01

- Preserve framework routes and domain facts across incremental AST cache
  reloads by re-rooting cached framework anchors consistently with node and
  edge source paths.
- Publish unresolved external symbols as source-scoped, inferred placeholders
  with deferred incident edges, while retaining canonical resolver work
  internally.
- Hard-cut Java Spring framework extraction to the production universal
  `spring-java` pack, including composed and inherited HTTP mappings, constants,
  bean and injection topology, messaging, scheduling, JPA, transactions, and
  security; Kotlin Spring remains on its established detector.
- Advance universal extraction semantics to `compass.languages.extraction/4`
  so cached Java evidence cannot be reused across the Spring pack cutover.
- Reduce immutable history build overhead by removing redundant trusted-graph,
  ordering, source-inventory, and Program module/function storage; encode
  independent records in parallel while retaining exact export and backward
  read compatibility, and reject stale worktree paths in shared AST caches.
- Use fresh, verified SCIP symbol evidence to disambiguate Java call targets in
  `graph.json` through exact AST call and declaration anchors, while rejecting
  non-call references, stale artifacts, ambiguous definitions, and conflicting
  providers.
- Introduce the first supported Compass code graph contract,
  `compass.graph/1`, as a strict versioned NetworkX-compatible multigraph with
  structural, framework, enterprise, messaging, job, schema, configuration,
  and database kinds.
- Add explicit `routes_to` bindings for supported server and file-routing
  frameworks, preserving middleware order and attributable heuristic wiring.
- Add the shared `compass.query/1` search, callers, callees, impact, explore,
  and node-trail contract across CLI, MCP, the viewer, and VS Code.
- Hard-cut over graph persistence: artifacts without `compass.graph/1` are not
  loaded through an adapter. Run `compass update` to rebuild them.
- Correct C function identities by resolving the callable declarator before
  generic declaration names, including macro-heavy SQLite declarations.
- Preserve repeated Markdown sections and rationale entries as distinct
  positional graph nodes.
- Advance the AST extraction cache namespace to `v0.9.21`. The first update
  after upgrading refreshes deterministic AST facts, then unchanged updates
  reuse the new cache normally.
- Remove the Graphify compatibility frontend, Python oracle, differential
  qualification phases, stale assistant assets, and legacy runtime
  configuration. Compass now builds and tests as an independent product.
