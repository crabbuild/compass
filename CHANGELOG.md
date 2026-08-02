# Changelog

## Unreleased

- Improve native Python, Rust, Go, Java, TypeScript, and JavaScript graph
  extraction and resolution, with three-run deterministic qualification on six
  pinned real repositories and explicit reporting of comparator defects and
  remaining parity/performance gaps.
- Preserve Rust module identity for source-backed crates rooted directly below
  `crates/<name>` instead of `src/`, allowing exact sibling imports, trait
  implementations, type relationships, and calls to resolve. Generic impl
  blocks retain their exact local outer type owner without treating references,
  foreign types, or generic parameters as local declarations. Rust adapter
  evidence advances to version 2 so cached version-1 facts refresh.
- Parse TypeScript `export type * from` declarations without quarantining the
  surrounding barrel file, preserving its exact import and re-export facts.
- Publish exact JavaScript and TypeScript class/interface heritage, including
  aliased imported bases, and preserve valid surrounding facts when the pinned
  TypeScript grammar encounters an indexed `typeof import(...)` type query.
- Resolve JavaScript-family imports through unique repository-local npm
  package exports and bounded wildcard barrels, including NodeNext source
  extension aliases, while leaving duplicate packages and export targets
  unresolved.
- Publish imported JavaScript-family function values used as arguments and
  collection members as exact declaration references instead of inferred
  calls. Extraction semantics advance to version 4 so cached facts refresh.
- Add `update`/`extract --no-program` for bounded structural-only builds that
  omit the independent Program IR artifact and its analysis memory.
- Reduce structural resolver peak memory by transferring semantic evidence
  into the resolution index instead of cloning the full fact corpus.
- Avoid constructing Program IR evidence during `--no-program` builds and
  publish portable AST cache entries one file at a time rather than retaining
  a second full extraction corpus through cross-file resolution.
- Bound medium structural builds to sequential discovery, cache publication,
  resolution, and graph normalization paths, and stream prepared graph edges
  into publication instead of retaining a second edge corpus. This lowers the
  ripgrep structural-only six-run median peak from 379 MiB to 336 MiB without
  changing the published graph bytes.
- Preserve exact declaration ownership when same-named overloads or Python
  property accessors share a qualified identity; publish these relationships
  as `contains` edges instead of the legacy node-kind-like `method` relation.
- Preserve parser-proven Java callable ownership through exact declaration
  identities, including same-name/same-arity overloads and methods nested in
  enum or anonymous bodies. Java adapter evidence advances to version 2 so
  cached v1 facts refresh automatically. Java declarations and call sites now
  retain bounded canonical parameter and argument type vectors; a complete,
  uniquely exact vector selects one overload, while unknown or competing
  signatures remain unresolved.
- Resolve Java overloads through proven language conversions only after exact
  type-vector matching fails. Complete source hierarchy evidence, primitive
  widening, boxing and unboxing, arrays, and a bounded set of stable
  `java.lang` supertypes may select one uniquely most-specific overload;
  incomplete or competing conversions remain unresolved. Java adapter
  evidence advances to version 3 so cached v2 facts refresh automatically.
- Publish exact Go interface method declarations and ownership, and adjudicate
  Graphify cross-type containment only when Compass has one source-anchored
  code-type owner rather than treating Graphify's target collision as truth.
- Distinguish Go named-type conversions from function calls, exclude receiver
  methods from package-member lookup, and propagate exact receiver types
  through chained local returns and named result parameters.
- Resolve Go range-value method calls from exact collection return/member
  types while keeping one-variable slice/map indexes and keys unresolved.
- Resolve Go indexed method receivers through exact slice, array, map, and
  channel element evidence without crossing same-named element owners.
- Preserve exact element types for local Go named collection declarations,
  including parameters and built-in `make` initializers.
- Follow owner-qualified Go field chains without crossing same-named fields,
  and resolve imported field/interface members to exact repository declarations
  instead of retaining full-import-path external placeholders.
- Publish Go result annotations as exact `returns` relationships, including
  methods named after their result type, and use that evidence to distinguish
  additional named-type conversions from calls.
- Follow unique Go method return types across files and imported packages for
  local and directly chained receiver calls, while leaving unpositioned
  multi-result or ambiguous flows unresolved. Root-package joins require an exact bounded
  `go.mod` module-path match, and foreign same-named packages remain unresolved.
  Go adapter evidence advances to version 2 so existing caches refresh
  automatically.
- Preserve the addressable member type of embedded Go fields so explicit
  selectors such as `options.baseOptions.AddFlags()` resolve to the embedded
  receiver method. Go adapter evidence advances to version 3 so cached source
  facts refresh automatically. Go package identity now also honors the parsed
  package clause, keeping external `_test` packages distinct from production
  packages in the same directory. Positional multi-result assignments retain
  their selected output index, allowing exact chained member calls without
  treating all callable returns as interchangeable.
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
