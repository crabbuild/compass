# Changelog

## Unreleased

- Improve natural code discovery on independently reviewed Python and
  TypeScript libraries. Ranking now recognizes common operation vocabulary,
  compound protocol owners, container roles, and decoder-chain intent without
  weakening bounded recall or negative-query handling. TypeScript framework
  compatibility propagation is limited to declared handlers and middleware so
  ordinary referenced callables retain stable module-qualified identities.
  Add pinned HTTPX and NestJS low-inference relevance oracles for repeatable
  cross-tool qualification.

## 0.3.11 - 2026-08-13

- Expand route extraction across popular framework families: add Angular Router
  typed and provider-backed route configs, Echo and Fiber receiver-aware Go
  routes, ASP.NET Core Minimal APIs and nested route groups, and deterministic
  cross-module Express, Fastify, and Hono router mounts. Ambiguous imported
  routers remain local rather than receiving an invented prefix. Advance
  extraction semantics to v11 so affected cached facts rebuild.

- Complete Axum route composition across local router variables, `nest` and
  `merge` chains, cross-module router factories, state-wrapped handlers, and
  ordered `layer` or `route_layer` middleware with import-aware targets.
  Ambiguous or cyclic Rust router targets now remain uncomposed instead of
  inventing an endpoint. The Routes and handlers HTML lens now uses a
  hierarchical layout and minimally qualified labels for same-named handlers.

- Reduce `GRAPH_REPORT.md` token use from opaque graph identities. Markdown now
  uses unique source anchors instead of redundant duplicate-label node IDs,
  compacts unavoidable long IDs to a bounded prefix/suffix plus deterministic
  fingerprint, emits portable short commands only once, and leaves oversized
  exact argv in `orientation.json`. Machine-readable IDs and argv are unchanged.

- Improve default-low code-graph fidelity and natural task discovery from a
  five-language real-repository qualification. Python now preserves exact
  named-import provenance, singleton/local-initializer dispatch, multiline
  receiver parameters, and callable aliases without low-mode placeholder
  leakage. Rust reexports retain exact namespace precedence; Go closure return
  types remain references instead of becoming enclosing-function contracts;
  Java `build` package paths remain source; and TypeScript callable properties
  are valid exact call targets. Natural queries now rank source-backed
  functions and methods by complete predicate/subject evidence while retaining
  genuine ambiguity. Tighten performance-harness timing parsing and diagnostics
  for these qualification runs.

- Stop Markdown pipe-table containers from overwhelming community names and
  `GRAPH_REPORT.md`. Mixed communities now prefer meaningful headings or
  symbols over higher-degree table parser nodes; table-only communities use
  deterministic source-anchored `Table (path:line)` labels and are counted as
  omitted from the architecture-focused report directory while remaining
  queryable in the graph. Advance graph publication semantics to v4 so existing
  build state is regenerated with the corrected analysis.

- Make `GRAPH_REPORT.md` a more complete, label-first architecture entry map.
  The compact Agent Orientation still highlights the leading communities, while
  the bounded Community Directory now retains up to 4,096 ranked non-empty
  communities, including thin communities. The top 32 receive full boundary
  evidence and up to twelve high-connectivity entry points; every remaining
  retained community receives a compact ranked index row with its best anchored
  entry point. Numeric community scopes remain only where they are useful for
  copyable `compass query --scope community:<id>` follow-up.
  Raise the full report to 256,000 characters, the compact orientation to
  16,000 characters, and the shared orientation/MCP resource envelope to 4 MiB.

- Add bounded `compass.task-context/1` packets through `compass context` and
  MCP `task_context`, with exact target resolution, digest-verified source,
  provenance, linked reflection memory, deterministic omissions, work
  accounting, and result digests. Add an independent
  `compass.pr-readiness/1` envelope through `compass review --readiness` and
  MCP `pr_readiness`, preserving the canonical PR report digest while
  summarizing signature/body changes, impact, static test evidence, advisory
  documentation drift, and bounded local ownership.

- Add the versioned `compass.viewer.workbench/1` export contract and a shared
  navigation shell for offline HTML and VS Code. `compass export html` now
  accepts repeatable code, call, impact, affected, architecture, history, and
  artifact lenses in one self-contained page; strict parsing rejects unknown
  or format-incompatible options. Add relationship, evidence, node-kind, and
  language filters, explicit coverage state, deterministic depth layouts, and
  convenient graph-camera controls for bounded zoom, 100% reset, whole-graph
  and selected-neighborhood fitting, plus independent node and relationship
  labels. Add bounded 1–4-hop selection isolation with incoming/outgoing edge
  filtering, layout spacing, a navigable minimap, and discoverable keyboard
  shortcuts. Resuming a settled nested community graph now reheats its layout
  so pause/resume produces visible physics movement. Consolidate workbench
  filters into the top graph-control rail, allow neighborhood settings to be
  prepared before selection, keep filters scoped to the visible overview or
  community detail, fit newly isolated neighborhoods, and scale layout reheating
  so motion remains visible on fitted repository graphs. Keep filter and graph
  settings panels mutually exclusive and viewport-bounded, provide recoverable
  empty-filter states, and discard selections that no longer exist in the
  filtered graph. Add the `workbench-json` machine export while preserving
  plain `compass.viewer.graph/1` JSON compatibility.

- Make PHP extraction and type resolution fail closed for colliding exact or
  case-folded symbols instead of selecting a hash-iteration winner. Qualified
  duplicate types now prefer one unique same-file declaration and otherwise
  remain unresolved. Advance extraction semantics to v9 so affected cached
  facts rebuild automatically.

- Add canonical `compass.pr_intelligence.report/1` review analysis over exact
  immutable target, PR-head, and synthetic-merge realizations, with stable
  `cmpprv1` findings, explicit completeness, a versioned advisory integer
  rubric, and independently versioned deterministic gates. Expose the same
  report through `compass review`, JSON/text/Markdown/SARIF projections, and
  MCP `review_pull_request`. Add a checksum-pinned reusable GitHub Action with
  read-only analysis, evidence artifact and job summary, fork-safe bounded
  sticky-comment delivery, and `fail-on: none|deterministic`; advisory risk
  never blocks merging.

- Qualify PR dependency risk with typed topology evidence. Ordinary dependency
  changes no longer count as cross-boundary impact without differing community
  identities, and changed edges receive a cycle factor only when a bounded,
  directed strongly connected component proves cycle participation. Semantic
  diff derived caches advance to engine version 2.

- Keep `GRAPH_REPORT.md` community evidence labels concise while making them
  deterministic and unique within a graph. Repeated hub names now gain compact
  source or wiring-site context, with the graph-local community ID used only
  as a final tie-breaker; unique names remain unchanged.

- Add deterministic `--inference-level low|medium|high|max` controls to
  structural graph builds. Lower levels retain exact or source-backed evidence
  and can admit explicitly qualified external relationships without the full
  deferred-receiver expansion. Hard-cut the build default to evidence-first
  `low`; users can opt into the former complete behavior with
  `--inference-level max`. Filtered builds prune unreferenced inferred
  placeholders and bind the selected level into build-profile/cache identity.
  Historical schema-1 build profiles that omitted the field remain readable
  as `max`, while new low profiles record the level explicitly and trigger a
  coherent rebuild.

- Make the performance harness select and record an inference level, and use
  symmetric community clustering for explicit Compass/Graphify comparisons.
  Add a pinned delta-rs diagnostic covering build latency and memory, graph
  integrity and provenance, natural and exact query recall, query latency, and
  adversarial no-answer behavior.

- Admit provably disallowed inferred calls after resolver, semantic, entity,
  and endpoint resolution but before node-link materialization, preserving
  duplicate evidence, unresolved and constructible targets, omission
  diagnostics, and the authoritative v1 policy pass. Reuse clustered community
  artifacts for fact-neutral edits, locally
  recluster bounded affected communities for small topology changes, and fall
  back to full Louvain for removals or oversized regions. Reduce immutable-store
  query startup reads, reject unsupported one-term fallback seeds for
  multi-concept discovery, and add a pinned Delta suite with independently
  labeled positive and negative accuracy oracles. Apply the established absent
  composite-identifier no-answer rule before generic term and relationship
  posting hydration, preserving exact ID/name ranking and ambiguity while
  making proven no-answer queries constant-work.

- Improve agent discovery accuracy and latency with deterministic identifier-
  subword postings, exact trusted-call relationship-term postings, bounded
  proof-complete caller recall, distinct supporting-callee evidence, fair
  candidate allocation, persistence-predicate precision within trusted
  relation candidates, a compact source-backed operation-role term index,
  subject-complete action ranking, capacity-aware traversal, selected-subgraph
  edge-ref filtering, bounded batched node and edge hydration, and one pinned
  immutable store reader with a bounded decoded-object cache per request.
  Intersect multi-concept exact-term IDs before hydrating surviving records,
  and make the normal discovery neighborhood a focused 64 nodes and 128 edges
  while retaining 500/1,000 as explicit hard ceilings. Compare the full
  specificity rank before labeling natural-query alternatives as ambiguous,
  while preserving equal-rank and exact-name ambiguity. Admit explicit `path
  from <symbol> to <symbol>` questions when both endpoints are exact symbol
  references, without weakening generic multi-concept no-answer admission.
  Legacy store snapshots remain readable and report incomplete identifier or
  relationship coverage until they are rebuilt; operation queries use the
  existing bounded fallback until the compact role index is available. The
  immutable relationship capability is v2, and the disposable SQLite query
  accelerator now uses internal format v7 and rebuilds automatically.

- Configure one-shot graph builds to use mimalloc without its process-wide
  reserved arena while preserving explicit operator allocator settings. This
  lets freed extraction and resolver pages return to the operating system
  between build stages instead of accumulating in a 1 GiB arena.

- Thread inference admission into universal, generic, and language-member
  resolution so low builds do not materialize deferred receivers, heuristic
  calls, or inferred external placeholders. Preserve exact test roles without
  retaining discarded inferred test edges. After enforcing the original
  evidence limits, compact uniquely paired duplicate `tests` candidates while
  independently resolving their relation-sensitive rules. Published low nodes
  and relationships remain equivalent to the prior authoritative post-filter;
  graph-level coverage and diagnostics now describe admitted records instead
  of inference that was constructed only to be discarded.

- Make streaming portable-AST cache publication encode and atomically write
  one entry at a time for every batch size. Large cold builds no longer retain
  concurrent MessagePack buffers and compression workspaces under an API whose
  documented purpose is bounded residency; the parallel encode-then-publish
  API remains available where callers explicitly accept batch memory.

- Store universal resolver declarations, scopes, bindings, and occurrences in
  deterministic sorted fact tables instead of hash maps that duplicate every
  fact's owned ID as a second key. Move relationship candidates into a private
  interned table while validated per-file batches are drained, retaining
  bounded per-candidate inflation during index construction and projection.
  Compact validated occurrences into a private slot-backed string table and
  retain only the role, spelling, qualifier, context, and exact range consumed
  by resolution.
  Release secondary resolver indexes after every build decision is fixed, and
  consume the legacy clustering/report projection instead of retaining it
  beside a complete typed graph. Borrowed lookup and explicit duplicate-ID
  rejection preserve graph semantics while reducing the transient resolver
  working set without changing the public evidence, cache, or graph schema.

- Add a digest-pinned 500-question, AI-reviewed synthetic relevance matrix
  covering all query classes, execute it in CI with strict ranking, recall,
  intent, structural, no-answer, and work bounds, and keep its generated JSON
  reproducible from reviewed equivalence classes. Complete bounded single-edit
  fuzzy recovery for insertion and substitution typos, avoid mistaking
  `caller`/`callee` symbol names for contradictory intent, and cache at most
  512 immutable fuzzy name lookups per engine.

- Add deterministic, bounded natural-language routing for symbol search,
  callers, callees, impact, and node trails through `compass ask`, clear
  `compass query` intents, and MCP `query_graph`. Generic, contradictory,
  historical, or explicitly traversed queries retain compatibility traversal,
  while ambiguous symbols remain explicit instead of selecting an arbitrary
  candidate.

- Reuse bounded alias, term, typo, and relation-seeded recall when resolving
  structural query operands. Node trails now follow edge direction from source
  to target and report `direction_mismatch` when a route exists only by
  ignoring one or more edge directions.
  Add a versioned profiled library response with intent, recall, ranking, and
  execution timings plus real candidate, posting, expansion, and response-byte
  work counts without changing the deterministic `compass.query/1` response.

- Hard-cut typed symbol search to the deterministic `query-ranker/2`, add a
  23-question executable relevance baseline with reviewed paraphrase,
  production-versus-generated ambiguity, domain, and no-answer cases, and add
  a bounded local redaction/review workflow for growing the corpus from
  approved production query samples without network telemetry or automatic
  judgment generation. Opt-in MCP query logs now cover typed natural queries,
  use a versioned record, and stop at 16 MiB. Search now enforces
  `maxCandidates` as the total recall-pool bound instead of silently inflating
  it to `maxNodes`.

- Make `compass upgrade` discover releases through a bounded, versioned static
  release manifest instead of the unauthenticated GitHub REST API. Corporate
  networks that share an outbound IP no longer consume GitHub's per-IP API
  quota when checking for Compass updates; archive size, digest, target, tag,
  and staged-binary validation remain fail-closed.

- Route JavaScript and TypeScript production extraction through the registered
  universal semantic candidate, including source-backed declarations, imports,
  re-exports, calls, references, heritage, aliases, and framework route
  targets. The candidate is now the same deterministic path used by
  qualification fixtures, with universal provenance and cross-file resolution
  preserved through cached and rebuilt graphs. Preserve the pipeline's exact
  repository-relative identity for universal framework facts so shallow source
  paths are published instead of being mistaken for temporary-directory paths.

## 0.3.10 - 2026-08-12

- Add bounded `compass.task-context/1` packets through `compass context` and
  MCP `task_context`, plus an independent `compass.pr-readiness/1` envelope
  through `compass review --readiness` and MCP `pr_readiness`. Results include
  exact target resolution, digest-verified source, provenance, deterministic
  omissions, work accounting, and stable readiness evidence.

- Improve `GRAPH_REPORT.md` as a label-first architecture entry map with a
  bounded ranked Community Directory, anchored boundary evidence, compact
  labels, deterministic community navigation, and larger report/resource
  envelopes. Nested community layout resume now reheats settled graphs so
  pause/resume produces visible movement.

## 0.3.9 - 2026-08-12

- Add the versioned `compass.viewer.workbench/1` export contract and a shared
  navigation shell for offline HTML and VS Code. `compass export html` now
  supports repeatable code, call, impact, affected, architecture, history, and
  artifact lenses, relationship/evidence/node-kind/language filters,
  deterministic depth layouts, bounded camera controls, 1–4-hop selection
  isolation, minimap navigation, keyboard shortcuts, and the `workbench-json`
  machine export while preserving `compass.viewer.graph/1` compatibility.

## 0.3.8 - 2026-08-11

- Make evidence-first structural inference the default, with explicit
  `--inference-level low|medium|high|max` controls. Low builds preserve
  exact/source-backed relationships, bind the level into cache/build identity,
  and remain deterministic.

- Improve natural and agent discovery with indexed identifier/subword and
  relationship-term recall, bounded caller/operation-role discovery,
  capacity-aware traversal, focused default neighborhoods, and constant-work
  proven no-answer behavior while preserving ambiguity, direction, provenance,
  and bounded contracts.

- Reduce resolver/query memory and latency with compact deterministic
  fact/occurrence tables, bounded AST-cache publication, shared readers/object
  caching, targeted index release, and pinned discovery/delta/performance
  qualification.

- Add typed pull-request risk review over immutable target, PR-head, and
  synthetic-merge realizations, with deterministic cmpprv1 findings,
  completeness/advisory risk projections, CLI/MCP surfaces, a fork-safe
  read-only GitHub Action, and cycle-aware dependency topology.

- Harden PHP symbol resolution and graph publication determinism, preserve
  wildcard/import source anchors and directed multigraph evidence, and improve
  community labels/report references.

## 0.3.7 - 2026-08-08

- Improve natural-language query recall and ranking with bounded exact, alias,
  term, typo, and relationship-seeded channels, deterministic intent routing,
  direction-aware node trails, and explicit ambiguity/no-answer diagnostics.
  Add a digest-pinned 500-question relevance qualification matrix and a local,
  bounded feedback workflow for reviewing future query samples.

- Route JavaScript and TypeScript production extraction through the universal
  semantic candidate with source-backed declarations, imports, re-exports,
  calls, references, heritage, aliases, framework routes, and bounded
  cross-file resolution. Preserve repository-relative identities and qualify
  the expanded TypeScript/JavaScript evidence paths.

- Make `compass upgrade` use a bounded, versioned static release manifest with
  exact target, tag, size, and SHA-256 validation, avoiding unauthenticated
  GitHub API rate limits while keeping fail-closed artifact selection.

- Expose profiled natural-query execution through a separate versioned library
  envelope, cap recall and query-log work, and keep the deterministic
  `compass.query/1` response contract unchanged.

## 0.3.6 - 2026-08-06

- Simplify every Compass-owned current-output path beneath `compass-out/`.
  Immutable build snapshots now live under `snapshots/`, the selector is
  `current-snapshot`, the SQLite query index is under
  `store/`, and snapshot-local sidecars use concise names such as
  `build-state.json` and `analysis.json`. This is an unconditional hard cut
  with no old-path detector, compatibility reader, or migrator. History
  realization schema 1,
  store format v1, and `compass/v1` realization roots remain unchanged; the
  output path cutover does not introduce a new serialized schema.

- Let agents size natural `query` and `explain` pages with an explicit token
  budget (2,000 by default) and retrieve every deterministic continuation with
  `--page`. Output now reports exact page/fact ranges and no longer silently
  drops `explain` connections after the first 20. The shipped Compass skill and
  lightweight agent adapters now select budgets deliberately, follow `next`
  pages, and disclose partial results instead of treating page one as complete.

- Let users switch code graphs between automatic, circular, concentric,
  spiral, and community-grouped square-grid layouts. Grid communities form
  aligned mini-grids with consistent gutters; fixed layouts remain
  deterministic and keep physics disabled so large community overviews stay
  responsive.

- Keep the initialization repository tree and selected-path ledger inside
  bounded, independently scrollable panes so large file lists remain usable.

- Keep VS Code activation resilient across fresh and mixed multi-root
  workspaces by distinguishing missing current snapshots from
  invalid/incomplete snapshots and surfacing repository-scoped refresh errors.

## 0.3.5 - 2026-08-06

- Keep very large HTML and VS Code community overviews responsive with a
  deterministic hub-centered layout, inexpensive aggregate edges, indexed
  search and relationship inspection, and bounded standalone community
  details. Large standalone HTML exports no longer duplicate an unbounded
  copy of every community graph inside the page.

- Materialize `graph.json`, `graph.html`, `GRAPH_REPORT.md`, `manifest.json`,
  and optional public build artifacts directly under `compass-out/`. The flat
  paths support low-effort migration of file-based workflows while Compass's
  immutable generations, store references, and cache encoding remain
  independent internal protocols. Root files use atomic replacement, remove
  stale optional outputs, and self-repair after interrupted publication.

## 0.3.4 - 2026-08-05

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
