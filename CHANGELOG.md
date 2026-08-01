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
