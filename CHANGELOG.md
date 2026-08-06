# Changelog

## Unreleased

- Keep dense VS Code and HTML code graphs responsive by switching detailed
  views with at least 1,000 nodes or 4,000 relationships to a deterministic
  community-grouped layout with paused physics and reduced edge decoration.
  Users can still enable force-directed physics explicitly.

- Parallelize deterministic per-file AST fact digests for large repositories
  and overlap unchanged pre-merge digest construction with portable AST cache
  publication. Native-volume Django cold-build p50 improves from 9.82 seconds
  to 8.04 seconds while preserving byte-identical graphs, reaching 5.43x
  versus the paired Graphify sample.

## 0.3.3 - 2026-08-05

- Improve Python bound-method and inheritance dispatch, Rust generic/reexport
  resolution, and Go package/call attribution while keeping ambiguous targets
  unresolved.
- Harden incremental graph publication, graph-delta validation, nested
  configuration containment, and qualification bounds for large repositories.
- Improve build performance with adaptive AST workers, bounded graph
  parallelism, cache reuse, source-digest reuse, and extraction-neutral deltas.
- Add focused Fastify, Hono, and Remix framework-pack support and raise the
  VS Code large-graph export limit.

- Respect Rust's separate value and type/module namespaces when extracting
  scoped associated calls. A `module::Type::function()` path now retains its
  visible import binding even when a same-named value is in lexical scope;
  ordinary `value.method()` dispatch remains value-bound, and competing
  imports fail closed. Rust structural facts advance to adapter version 15.

- Follow a unique source-present Rust named reexport when resolving an
  associated callable reached through a glob facade. Receiver-prefix alias
  expansion is depth-bounded; competing aliases and reexport cycles remain
  unresolved.

- Resolve Rust associated calls imported from a source-present sibling-crate
  glob, including chained calls whose receiver type comes from that callable's
  published return evidence. One-component crate names retain Rust `::`
  qualification; competing or unknown glob targets continue to fail closed.

- Resolve published return candidates before using their types as chained-call
  receivers, and require Rust field receiver types to be source-local or
  explicitly imported before qualification. Unresolved prelude types such as
  `Result` and `Option` now use their canonical standard-library ownership
  instead of becoming fabricated crate-local types or methods, while exact
  source-local and explicitly qualified external types remain resolvable.
  Rust structural facts advance to adapter version 14.

- Resolve bounded, multi-stage Rust method-result chains across files. Each
  stage records its receiver call-result evidence, exact outer nominal return
  types are selected ahead of nested generic arguments, and incomplete
  project-wide evidence preserves the prior source-proven binding. Missing,
  ambiguous, cyclic, raw-pointer, and over-depth chains fail closed. Trait
  default dispatch is accepted only through a unique source-proven
  implementation. Rust structural facts advance to adapter version 13.

- Resolve Rust calls chained from a same-file, source-proven method result by
  preserving the exact outer nominal result type. Generic results such as
  `ThreadPoolBuilder<CustomSpawn<F>>` now resolve the next member against
  `ThreadPoolBuilder`, replacing malformed inferred placeholders with exact
  call edges; unknown, ambiguous, or non-local result/member evidence still
  fails closed. Rust structural facts advance to adapter version 12.

- Resolve Rust calls chained from a source-proven associated-function result,
  including uniquely aliased receiver owners such as
  `super::DrainGuard::new(...).par_drain(...)`. Concrete implementation
  methods returning `Self` now publish exact return evidence, mixed `::` and
  `.` call syntax is parsed structurally, and ambiguous return or member sets
  fail closed. Rust structural facts advance to adapter version 11.

- Fail closed for Rust `self.method(...)` calls when the indexed receiver has
  multiple repository-local trait method declarations and available evidence
  cannot select one. Such calls no longer become fabricated external or
  deferred method placeholders; a unique local method and a genuinely
  external inherent method remain resolvable. Rust structural facts advance
  to adapter version 10 so cached evidence refreshes.

- Correct the diagnostic Graphify comparator when Graphify projects a Rust
  return-type reference onto its callable's declaration line while Compass
  preserves the exact returned-symbol occurrence. Dominance requires exact
  source and target identities, a return-type context, and one Compass
  `returns` fact; unrelated references and competing returns still fail
  closed.

- Preserve every source-valid call target behind mutually exclusive Rust
  `#[cfg(unix)]` and `#[cfg(windows)]` reexports, including a lexical fallback
  for other platforms. Ordinary duplicate reexports and feature flags that can
  overlap still fail closed. Rust structural facts advance to adapter version
  9 so cached evidence refreshes.

- Publish Rust blanket trait implementations from the exact impl-scoped
  generic parameter declaration. The trait occurrence remains source-anchored;
  competing wildcard imports and parser-recovered implementation headers fail
  closed. Code Graph v1 accepts this Rust-specific `parameter -> trait`
  implementation shape while retaining the closed endpoint matrix for other
  languages. Rust structural facts advance to adapter version 8 so cached
  evidence refreshes.

- Publish source-anchored Rust references from an implementer declaration
  proven in the current source evidence to every non-primitive nested type
  used as a trait implementation argument. The occurrence remains in the
  implementation's lexical scope, so imported types and scoped implementation
  parameters resolve exactly while competing wildcard imports remain unresolved. Rust
  structural facts advance to adapter version 7 so cached evidence refreshes.

- Treat a Rust wildcard target in another crate as repository-local only when
  that exact file, module, or enum declaration is present in the indexed
  source. This lets multi-crate workspaces resolve unique grouped reexports and
  local calls across sibling crate globs while unknown external globs still
  make the search fail closed. It also removes misleading placeholders such as
  `rayon::prelude::Vec` that assigned standard-prelude names to an unrelated
  explicit wildcard.

- Resolve Rust `Self::Type` through a complete source-proven supertrait
  hierarchy and the associated-type realization owned by the exact receiver
  declaration. Parent-module glob imports, including private imports exposed
  through `use super::*`, participate only when their bounded local search has
  one compatible target. Competing traits, repeated local receiver names,
  external branches, and incomplete searches fail closed. Rust structural
  facts advance to adapter version 6 so cached evidence refreshes.

- Resolve an unqualified Rust symbol across multiple visible glob imports when
  their bounded, repository-local union contains exactly one compatible
  declaration. Competing declarations, excessive glob/search sets, and
  unproven external lowercase symbols remain unresolved instead of selecting
  an arbitrary import.

- Preserve Rust associated types as trait- or implementation-scoped type
  aliases and resolve `Self::Type` returns to the exact lexical declaration.
  Each associated alias separately references its concrete realization;
  duplicate declarations remain unresolved. Rust structural facts advance to
  adapter version 5 so cached evidence refreshes.

- Resolve repository-local Rust imports and reexports to the unique semantic
  module when file and module realizations share the same qualified identity.
  Resolution still fails closed when another declaration kind participates or
  more than one semantic module remains. The Graphify comparison now recognizes
  an occurrence-matched semantic module as a stronger realization of its
  physical source file.

- Publish Rust type, lifetime, and const generic parameters as distinct,
  owner-scoped parameter nodes. Exact field, signature, return, and bound uses
  resolve to the lexical parameter without leaking across declarations or
  implementation scopes. Rust structural facts advance to adapter version 4
  so cached evidence refreshes.

- Preserve Rust declarations nested inside function bodies and resolve their
  source-anchored lexical calls. Repository-local wildcard imports now resolve
  lowercase calls after lexical and module candidates, while unproven external
  wildcard calls continue to fail closed. Rust structural facts advance to
  adapter version 3 so cached evidence refreshes.

- Preserve exact TypeScript and TSX extraction for valid `in`/`out` generic
  variance modifiers even though the pinned grammar reports those tokens as
  recoverable errors. Compass reparses only parser-identified variance tokens
  under type-parameter lists, preserving byte/line anchors, mapped-type `in`,
  identifiers named `out`, and genuine malformed-input diagnostics. TypeScript
  runtime variables now also receive identities distinct from same-named type
  and interface declarations, so constructor calls resolve to the value
  namespace and ambiguous runtime bindings fail closed. Extraction semantics
  advance to version 8 so cached TypeScript facts refresh.

- Resolve Python `self` and `cls` calls through source-proven inheritance,
  including a later direct base when every preceding base is a known leaf and
  class members that directly alias an earlier module-level callable. Rebound
  and static-method receiver names and overwritten aliases continue to fail
  closed. When a source-defined concrete descendant proves a different runtime
  target, or unknown earlier ancestry makes a later direct member possible,
  publish every bounded hierarchy-proven alternative as `INFERRED` without
  weakening the exact target. A descendant with external ancestry may retain
  a direct first-base member as possible dispatch, while unrelated same-name
  members, fully known inconsistent C3 hierarchies, ambiguous members, and
  bound overflows remain unpublished.

- Resolve calls through source-proven inheritance when a Python function-local
  class is used as the qualified receiver, while failing closed after a local
  rebinding of that class name.

- Harden Python value-reference resolution so argument, collection, assignment,
  and return references cannot fabricate function calls. Function-valued uses
  remain references unless future evidence proves an invocation contract.
- Preserve source-anchored Python wildcard imports and package re-exports, and
  resolve uniquely proven repository-local symbols through bounded wildcard
  facade chains. Multiple wildcard sources remain ambiguous and fail closed.
- Classify Python call syntax from resolved declaration kinds rather than name
  capitalization, so lowercase classes resolve as constructions, uppercase
  functions remain calls, and unresolved names do not invent class identity.
- Preserve statically proven module-level Python callables created with
  `functools.partial`, including package re-exports, while dynamic, shadowed,
  conditional, and ambiguous factories fail closed. Python structural facts
  advance to adapter version 6 so cached evidence is refreshed.
- Publish uniquely bound, unconditional Python module variables as exact
  declarations and connect direct constructor initializers to one proven local
  class. Imports and value references can resolve those variables, while
  competing, conditional, deleted, shadowed, and ambiguous bindings fail
  closed. Explicit receiver calls no longer fall back to a same-named
  unqualified import. Python structural facts advance to adapter version 7.
- Resolve zero-argument Python `super()` calls through complete source-backed
  C3 hierarchies instead of requiring the method on the immediate base.
  Unknown hierarchy prefixes may expose a later member only as an explicitly
  inferred possible dispatch; ambiguous members still fail closed. Python
  structural facts advance to adapter version 8.
- Preserve source-proven recursive Python calls, including distinct occurrence
  anchors for repeated recursion. Parameters, assignment targets, closure
  bindings, and unknown receivers cannot fall through to a same-named
  declaration; `global` and `nonlocal` directives retain Python lexical
  semantics. Entity deduplication retains only recursive call loops that were
  already self-edges before rewiring; non-call loops and loops created by
  merging distinct endpoints remain suppressed. Python structural facts
  advance to adapter version 9.

- Split framework extraction into focused Spring, Express, Axum, Next.js, and
  Vite adapters. Project evidence now indexes framework configuration files,
  aliases, plugins, and file-route roots; Vite configuration nodes and a
  reusable exact-route qualification API are available to graph consumers.
  Extraction semantics advance to version 7 so cached source facts refresh.
- Add focused Fastify, Hono, and Remix adapters. Fastify and Hono reuse the
  bounded TypeScript route primitives for hooks, method arrays, mounts, and
  literal route objects; Remix publishes nested file routes with `PAGE`,
  `LOADER`, and `ACTION` operations and dependency/configuration activation.
- Preserve JavaScript and TypeScript workspace import and re-export resolution
  across fully cached edit/restore builds, and improve canonical graph JSON
  publication parallelism for medium and large structural graphs without
  changing serialized bytes.
- Bound default parallel AST extraction to eight workers to reduce parser
  working-set multiplication on large repositories. Go multi-result call
  attribution now preserves indexed return types through ranges and closures;
  Rust generic trait-bound receivers resolve to their source trait methods,
  while ambiguous bounds remain unresolved. Universal resolver publication
  also avoids a duplicate declaration-slot index. Python bound-method calls
  now carry source-proven `self`/`cls` receiver dispatch, while static,
  rebound, and shadowed receivers fail closed.

## 0.3.2 - 2026-08-03

- Harden graph correctness and make the default query path use canonical
  `graph.json` results, with clearer diagnostics and deterministic validation
  across graph builds and code queries.
- Make structural builds the fast default by making Program IR opt-in and
  reducing large-graph publication, cache, and query overhead without changing
  graph facts or output contracts.
- Add broad framework-aware route extraction and resolution coverage across
  Python, TypeScript, Go, Rust, Swift, Java, PHP, Ruby, C#, and Astro, with
  explicit owner-mismatch handling and documented route semantics.
- Refresh public product and installation documentation, including the VS Code
  Marketplace install path and interactive graph/history guidance.

- Make structural graph builds the fast default: `init`, `update`, `extract`, and
  `watch` now omit Program IR unless `--program` or `--program-artifact` is
  selected. Keep `--no-program` accepted for compatibility with existing
  automation.
- Make `extract --code-only` scope structural extraction to code inputs as well
  as skipping semantic-provider work; the diagnostic file inventory remains
  available without adding document nodes to the graph.
- Reduce large-graph build and query overhead without changing graph facts or
  output contracts: transfer publication buffers across the v1 boundary, avoid
  resealing already-atomic artifacts, and derive query-index identity from a
  streaming artifact digest instead of reserializing the complete graph.
- Document the supported framework-route matrix and harden common route forms:
  named Django, Flask, and FastAPI path arguments; React Router `Component`
  elements; ASP.NET absolute action templates; Drupal multi-method routing YAML;
  and documented Drupal hook implementations. Route resolution now fails closed
  on explicit owner mismatches, preserves opaque Express callbacks, composes
  FastAPI/Flask registration prefixes, binds file-based endpoint exports,
  recognizes NestJS gateways, applies Rails namespaces/Laravel resource
  modifiers/ASP.NET action routes, and gates Spring mappings to controller
  owners. Native route composition now covers Go chi/gorilla prefixes, Axum and
  Actix nested builders, multiline Rust attributes, and Vapor grouped/`on`
  registrations; the release qualification manifest exercises 27 route flows.
  Extraction semantics and the AST cache namespace advance to version 6
  so existing projects refresh these facts once instead of retaining stale
  route graphs.

## 0.3.1 - 2026-08-03

- Make versioned graph comparisons meaning-aware: source-coordinate shifts,
  clustering/layout metadata, and anchor-derived edge identities no longer
  appear as graph-wide structural changes. Historical queries now read only
  sealed graph roots plus document metadata on a cold cache miss, while exact
  record diffs remain lossless and profile compatibility remains mandatory.
- Replace sticky graph-edge tooltips in the shared viewer and VS Code with a
  theme-aware relationship card that shows direction, confidence, evidence,
  and source location, with explicit cleanup on edge, canvas, drag, and zoom
  transitions.
- Make `graph.json` the default graph build and typed-query engine. SQLite
  snapshot publication is now explicit with `--store sqlite`, and querying it
  is explicit with `--engine store`.
- Make opt-in Compass Store publication and queries practical on large graphs:
  batch immutable writes without duplicate reads, remove the legacy full-graph
  database payload, compress projected trees, keep one shared SQLite database
  with generation references, retain and collect two bounded generations, and
  execute typed queries directly through immutable indexes. Canonical
  `graph.json` hashing is streamed during atomic publication, timing reports
  transaction/object/byte/GC metrics, and JSON/store search now shares bounded
  candidate ordering and underscore tokenization. Django qualification reduced
  a fresh SQLite build from 11.4× JSON to 1.24× internal wall time while store
  search became 20.7× faster with byte-identical results.
- Make VS Code editor graph actions reliable and focused: project typed graph
  source anchors into cursor-based call resolution, consolidate duplicate
  context submenus, resolve symbols without an initial prompt, and render only
  the nodes and relationships returned by context queries. The extension also
  adapts typed caller/callee query results when the selected Compass 0.3.0 CLI
  cannot consume those source anchors through `call-graph`, and now rejects
  Compass CLI releases below 0.3.0 instead of accumulating legacy fallbacks.
- Qualify the local `compass-store` release contract: versioned store and
  graph-snapshot envelopes, namespace-first addressing, SQLite backup/restore,
  `compass store status|validate|backup|restore`, explicit rebuild tooling, and
  canonical JSON/typed-query/CompassQL differential evidence. `graph.json`
  remains permanent; redb is a library-only adapter and PostgreSQL/DynamoDB
  remain deferred.
- Optionally publish a validated shared `.compass-store/compass-store.sqlite3`
  namespace/partition/key snapshot and a typed generation `store.ref` beside
  `graph.json` with `--store sqlite`. Typed code-query commands can use the store when
  explicitly selected and the selector agrees, preserve deterministic
  JSON-equivalent results, and support explicit `--engine json|store`
  selection; `graph.json` remains a complete compatible engine.
- Restore sub-10-second cold builds for the pinned 3,105-file Django
  qualification corpus by batching and compressing portable AST cache
  publication, parallelizing independent resolver and graph-normalization
  work, and avoiding redundant evidence, index, and serialization allocations
  without changing valid graph facts or Program output. Partial-graph omission
  diagnostics now use portable content-addressed edge identities instead of
  transient raw positions.
- Restore complete definition ranges for source navigation on universally
  extracted functions, methods, and containers while preserving exact
  identifier anchors in provenance. Ambiguous or non-containing scopes fall
  back to the exact declaration, and graph publication semantics advance to
  version 3 so existing outputs rebuild under the corrected range policy.
- Give the VS Code Architecture Flow a bounded, horizontally scrollable system
  canvas with roomier lanes, non-overlapping controls, drag-to-pan navigation,
  resilient empty states, and a route-table alternative.
- Make `compass install` setup more seamless: recognize host directories and
  instruction files during agent detection, show and preflight every dry-run
  destination, provide host-specific activation actions, reject ineffective
  strict-mode or unsupported scope combinations, bootstrap missing graphs on
  first broad use, and replace the Codex no-op hook with a bounded graph-first
  search guard.
- Preserve relationship source anchors in the shared graph viewer, show their
  file and line on edge hover, and let VS Code open the exact call or wiring
  site when a located edge is double-clicked. Prepared graph overviews advance
  to `compass.graph-overview/2` so older cached projections refresh safely.
- Make Markdown extraction source-driven and structurally parsed with pinned
  block and inline grammars. Publish bounded frontmatter metadata, headings,
  nested blocks, tables, code fences, reference definitions, exact link sites,
  and explicit ambiguous-fragment evidence. Include raw frontmatter in Markdown
  file hashes so metadata-only edits refresh graph and semantic cache entries;
  recognize `.markdown` files as documents. Extraction semantics advance to
  version 5 so old structural facts cannot be reused.
- Add a pinned structural HTML adapter for `.html`/`.htm`, shared entity-aware
  normalization for URL ingestion, semantic landmarks/tables/links, bounded
  metadata and malformed-input diagnostics, and explicit MDX/Quarto/footnote
  evidence.

## 0.2.1 - 2026-08-01

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
- Restore source ranges and named communities in natural-language query output
  when reading typed `compass.graph/1` nodes and relationships.
- Prefer source-backed declarations over unresolved call placeholders in
  natural-language queries, show placeholder wiring and relationship sites,
  and make `explain` report ambiguous labels before accepting an exact node ID.

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
