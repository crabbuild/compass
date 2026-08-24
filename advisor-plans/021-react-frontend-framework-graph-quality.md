# Plan 021: Make React frontend framework graphs enterprise-ready

> **Executor instructions**: Follow this plan phase by phase. Run every
> verification command and confirm the expected result before moving on. If a
> STOP condition occurs, stop and report it; do not improvise. This is a
> multi-PR program, not one oversized patch. Keep each phase independently
> reviewable and leave the production graph coherent after every merge. When
> the program is complete, update this plan and its row in
> `advisor-plans/README.md`.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 4e12ca92..HEAD -- \
>   COMPATIBILITY.md CHANGELOG.md MIGRATION.md SECURITY.md PERFORMANCE.md \
>   crates/compass-model crates/compass-files crates/compass-languages crates/compass-resolve \
>   crates/compass-graph crates/compass-core crates/compass-query crates/compass-cypher \
>   crates/compass-output crates/compass-cli crates/compass-mcp \
>   crates/compass-history crates/compass-semantic-diff \
>   packages/compass-viewer editors/vscode tests/viewer \
>   crates/compass-output/assets/viewer fixtures tests/qualification \
>   benchmarks/performance scripts docs .github/workflows/compass-ci.yml
> ```
>
> If an in-scope file changed, compare this plan's “Current state” with the
> live code. Update paths and tests mechanically when ownership is unchanged.
> STOP when a public contract, evidence boundary, or ownership assumption no
> longer holds.

## Status

- **Priority**: P1
- **Effort**: XXL (ten phases; ship as multiple PRs)
- **Risk**: HIGH
- **Depends on**: Plan 013's TypeScript/JavaScript production hard cut; the
  final release claim should consume Plan 005 or an equivalent
  exact-production qualification gate
- **Category**: migration, tests, dx, direction
- **Planned at**: commit `4e12ca92`, 2026-08-22
- **Plan audit**: first completed against live commit `de41139b` on 2026-08-22
  and superseded by the fifth-pass execution audit against `20cf959e` plus the
  current worktree on 2026-08-23; `Planned at` remains the drift-check
  baseline `4e12ca92`. The audits cover strict schemas, pack ownership,
  AST/config seams, cache invalidation, downstream consumers, security,
  performance, and qualification. Intervening commits must still pass the
  drift check before execution.

## Live implementation audit (2026-08-22)

The first vertical slice has been reviewed against every contract boundary in
this plan. The following items are implemented and covered by focused tests:

- `Renders`, typed render details, frontend roles, endpoint validation, graph
  normalization, query/discovery enumeration, inbound impact policy, and
  viewer Zod/label/appearance contracts;
- package/runtime-gated `react-ui` detection over the borrowed TypeScript/TSX
  tree, including self-closing JSX, fragments, exact JSX references,
  occurrence-preserving render projection, conservative hook closure, and
  exported client/server directive roles;
- deterministic framework-pack registry identity in the build profile, so a
  framework activation/descriptor/limit change cannot reuse stale extraction
  facts;
- downstream policy is explicit for the provisional edge: callers/callees and
  callflow remain call-only, code-query impact and default affected traversal
  include inbound `renders`, the dependency lens exposes it, semantic/history
  deltas retain it generically, and dependency-cycle topology intentionally
  excludes it as UI topology;
- negative non-React activation, exact anchor, repeated-occurrence,
  directive-scope, hook false-positive, impact, serialization, viewer, and
  cold/repeated-resolution tests.

The audit also confirms that the plan still has no implicit “done” path. The
remaining release blockers are explicit and tracked by their phases: the
parser-backed shared syntax/config view, typed route-stage migration and
hierarchy, Next/TanStack/React Router/Remix/Vite packs, expanded resource
limits and per-pack cache semantics, the evidence-sufficiency matrix and
versioned expectation schema, task-context/CLI/MCP versioning, pinned
independent qualification corpora and oracle, performance baselines, and the
exact production release gate. Existing Next/Vite/route behavior is therefore
not counted as covered by the React slice, and the plan/index row remains
`TODO` until Phases 0–9 pass.

The audit found one unrelated pre-existing graph test mismatch in
`crates/compass-graph/tests/markdown_identity.rs` (duplicate Markdown heading
URI expectation); it is not changed or used as frontend evidence. It must be
resolved or explicitly dispositioned before the repository-wide baseline can
be called green.

### Follow-up audit dispositions

The follow-up audit also closed the concrete gaps found while exercising the
vertical slice: strict render-kind decoding now rejects unknown values,
source-scoped node collision remapping updates UI-role references, the legacy
affected traversal includes inbound `renders`, the dependency lens exposes
`renders`, and the compatibility/migration/changelog notes describe the
pre-release enum widening. React fact volume now uses the descriptor's checked
pack budget instead of a silent detector-local truncation. Model, language,
output, resolver, query, core
determinism, semantic-diff, history, product-boundary, and fixture
qualification checks were rerun; the fixture gate produced byte-stable output.
These checks validate the slice only. They do not waive the remaining
framework, route, task-context, independent-corpus, performance, or release
gates listed above.

### Audit gap register (2026-08-22)

The second-pass audit checked the plan against the live closed-enum consumers,
package/config resolution boundaries, qualification math, safety limits,
parallel extraction behavior, and generated-client surfaces. Four plan gaps
were found and closed below; none changes the product scope or permits a
release claim early:

| Audit area | Gap found | Disposition |
|---|---|---|
| Closed consumers | The minimum inventory named the principal crates but did not name the query-contract model, task-context CLI/MCP owners, VS Code transport, viewer fixture generator, or qualification graph builder. | Step 1.1 now names those files and requires an explicit policy/test for each; Phase 8 and Phase 9 repeat the transport and qualification checks. |
| Corpus floor | “2,000 total” plus “400 per stable family” was ambiguous because the support matrix has more than five stable rows. | The quality budget now defines the floor as `max(2,000, 400 × N_stable_families)` and records the current `N_stable_families` explicitly. |
| Resolution and limits | Workspace/package-manager variants, compiler-option inheritance, regex/resource bounds, all-collection budgets, and detector-version invalidation were implied rather than enumerated. | Phase 0.5, Step 2.3, and Step 2.6 now list the supported/unsupported resolution forms, explicit per-pack version bumps, and named limits, including bounded regex handling. |
| Operational determinism | Cold/warm tests did not require equivalent output across worker counts or interrupted/resumed extraction. | The shared quality budget, cross-phase tests, and Phase 9 gate now compare single/default/max worker runs and interruption recovery, with no partial-success escape. |

The plan still intentionally excludes React Native/Expo, Storybook semantics,
runtime portals/hydration, and dynamic configuration; those exclusions are now
explicit in Scope and the evidence matrix rather than silently counted as
recall gaps. The phase ledger and plan index remain `TODO` until the full
release gates pass; the implemented React slice is not a completed phase.

### Third-pass implementation audit (2026-08-22 worktree)

The live worktree now contains additional bounded implementation slices. They
close extraction-level gaps but do not change the release status:

| Area | Closed in this pass | Still required before phase exit |
|---|---|---|
| Shared syntax and identity | Borrowed TypeScript syntax view, static-value limits, import-alias identity, config-object recovery, per-pack semantics versions, and accumulator validation | Package-manager/workspace precedence, `tsconfig` inheritance, generated-file policy, and cache reopen/invalidation qualification |
| React render graph | Exact `jsx`, `create_element`, `root`, and statically linked `lazy` projections; repeated anchors and non-React negatives | Dynamic/Next factory policy, cross-file ambiguity fixtures, graph/query/task-context snapshots, and corpus recall evidence |
| Route packs | Next App/Pages stages and hierarchy, React Router ownership and loader/action stages, TanStack Router/Start aliases and server roles | Version matrix, generated-tree precedence, Remix universal cutover, route hierarchy qualification, and old-cache compatibility |
| Vite | AST config aliases/order, shadowed-factory negative, and bounded `import.meta.glob` file-set facts | File-set consumer/matching contract, plugin call identity, nested config/package precedence, and Vite hard-cut qualification |
| Generic facts | Typed relation/file-set facts, strict validation, and source-file relation lookup | Public relation resolver contract tests, unsupported/ambiguous diagnostics, and downstream graph publication expectations |
| Qualification | Focused universal-pack regression fixtures now exist for React, Next, TanStack, React Router, and Vite | Versioned expectation schema/loader, independent oracle, pinned corpus manifest, `qualify_react_frontend_graph.sh`, Wilson/anchor/multiplicity metrics, worker/interruption rows, and external artifact publication |
| Agent workflow | Existing impact/render policy remains covered | Versioned FrameworkContext task-context section, CLI/MCP/VS Code strict transport contracts, query aliases/examples, trust-boundary tests, and docs example runner |

Consequently, the “no gaps” audit result is **not yet a release-ready plan**:
the remaining rows are concrete gates, not assumptions. The phase ledger and
`advisor-plans/README.md` row must remain `TODO` until those rows have checked-in
tests and the exact production qualification command passes twice with a
byte-stable artifact. The unrelated Markdown identity mismatch recorded above
also remains a repository-baseline disposition item.

### Fourth-pass audit: verification findings and newly explicit residuals (2026-08-22)

The focused pack/resolver checks found and fixed two correctness holes rather
than silently accepting them: aliased Vite `defineConfig` calls now match the
actual imported local binding, and a truncated generic framework-relation
candidate set can no longer publish as an exact edge (the diagnostic records
the truncation). The corresponding Vite regression and resolver unit tests are
checked in.

The same run also exposed residuals that must stay on the execution ledger:

| Finding | Why it remains a gate | Required disposition |
|---|---|---|
| Established source/config/template packs still use an implicit semantics version of `1` in the registry digest | Detector changes outside universal descriptors can reuse cached facts without a reviewed per-pack bump | Add an explicit nonzero version field to every runtime pack and reject stale cache entries |
| `TypeScriptSyntax::is_incomplete` does not yet reject every node overlapping a parser recovery region, and static-value/regex budgets are helper constants rather than descriptor-observed limits | Recovery-overlapping configuration or route syntax can be mistaken for complete evidence, and named `FrameworkLimits` are not all enforced before allocation | Add recovery-range tests, descriptor-driven syntax budgets, retained-literal accounting, regex complexity checks, and typed limit diagnostics |
| Next/Remix file-route endpoint discovery still uses bounded source regexes and convention-wide file anchors; Next/Remix version and root precedence are not qualified | This violates the parser-backed exact-anchor contract for exported handlers and can drift across framework majors | Replace with the shared AST view or mark the form incomplete, then qualify version/root precedence and old-cache behavior |
| TanStack currently recognizes code-based factories but not the planned file-route/generated-tree policy; Vite plugin identity is still name/substring based, and Vite glob facts have no `compass-files` matcher/publication consumer | The advertised route/config relationships are not yet project-wide or independently attributable | Implement bounded file-route/generated-tree handling, exact plugin import/call identity, and file-set matching/invalidation before promotion |
| Generic role/configuration/file-set facts are validated and cached, but only roles/domains and generic relations have resolver publication paths; configuration and file-set facts do not yet reach graph/query consumers | Agents can observe a raw fact without a typed downstream answer or an explicit unsupported state | Add central semantic publication, consumer contracts, and negative/ambiguous diagnostics for each fact family |
| The existing `typescript_routes` integration suite currently has three failures after the typed-stage/React Router cutover (legacy page-stage expectation, loader compatibility field, and component-target ordering) | A frontend migration cannot claim baseline safety while established route behavior is red, even when the deltas are intentional | Either preserve the old public behavior through a compatibility projection or update the contract/tests with an explicit migration note and parity disposition |
| Build verification emitted a tree-sitter language-pack parser-source download warning | Release/fixture qualification must remain offline and cannot depend on an uncached network fetch | Qualify from pre-provisioned parser artifacts and add an offline/no-network check to the Phase 9 gate |
| The production fixture gate now reaches the independent oracle and stops on the new `renders` edge kind; its vocabulary/manifest still only describe the pre-React edge contract, and its route-stage enum still only accepts `handler`/`middleware` | A qualification oracle that cannot parse a published edge or typed route stage cannot prove the React/React Router contract; changing the producer without changing the independent expectation schema leaves a false sense of coverage | Version the qualification expectation schema, add `renders`/render detail and typed route-stage expectations with exact producer/origin rules, add the nearest-package activation fixture, and rerun clean/warm/rebuild/checkout/lifecycle byte comparisons |
| React Router’s new required activation pack was initially absent from the fixture graph because `fixtures/code-graph/routes/typescript/react-router.tsx` had no nearest package marker; adding the marker exposes the stale oracle rather than proving parity | Activation is intentionally package-scoped, so a corpus without an owning manifest tests a disabled pack and can silently undercount framework recall | Keep the package-local dependency fixture, assert enabled/disabled sibling packages, and include activation changes in cache invalidation and semantic-manifest coverage |
| Route-hierarchy publication first emitted `compass.languages.unknown` provenance and required an independent-oracle route-to-route `contains` allowance; the edge is now explicitly convention-owned, but this path is not yet covered by the frontend qualification corpus | Hierarchy is a graph edge with its own provenance and endpoint policy; an unowned or unqualified edge is not safe for agent traversal | Add a hierarchy flow/negative expectation covering parent selection, ambiguity, route-to-route containment, producer/origin/rule, and no unknown-producer fallback |

These findings do not add new product scope; they make the already implied
ownership, recovery, cache, downstream-consumer, and offline requirements
executable. The plan is now audited against both the implementation and the
negative verification path. It still must not be marked `DONE`, and the
Markdown identity mismatch remains a separate repository-baseline failure.

Historical fourth-pass verification ledger (superseded by the fifth-pass audit
below): `cargo fmt --all -- --check`, the complete
`compass-languages` test suite, focused Next/TanStack/Vite/React Router packs,
focused resolver route/frontend tests, the Python oracle unit suite, the
product-boundary script, and `git diff --check` pass. The complete
`compass-resolve` suite is still red in three existing
`typescript_routes.rs` cases (generic file-route stage compatibility, the
legacy React Router loader field, and component-target ordering). The complete
fixture gate compiles and passes its scale/determinism lifecycle checks, with
each production update reporting a bounded omission summary of five nodes and
eight edges, but it currently stops in the independent oracle on the
unversioned `renders` edge vocabulary/typed-stage contract; clippy/workspace-
wide tests, JS contracts, and the plan-specific frontend qualification command
have not been run to a release conclusion. These are evidence states, not
waived requirements.

### Fifth-pass live execution audit (2026-08-23; superseded by the sixth-pass release audit)

This pass supersedes the red/unknown verification states recorded in the
2026-08-22 ledger above. It re-audited the phase ledger and all four residual tables against the
live worktree at `20cf959e` plus its uncommitted implementation changes. It
also ran the production paths rather than treating focused unit tests as a
release substitute. The earlier `typescript_routes` and Markdown concerns are
now dispositioned: the complete resolver suite and
`crates/compass-graph/tests/markdown_identity.rs` both pass. The inference
policy was additionally corrected after the first workspace run exposed a
real low-vs-max graph mismatch; the special-case was removed and the existing
coherence regression now passes.

Closed implementation/contract gaps from the fourth pass:

| Area | Evidence now present |
|---|---|
| Pack/cache identity | Every runtime framework descriptor has an explicit non-zero version; the pack registry test, framework semantics digest, cache-profile digest, and stale-version checks pass. |
| Shared syntax and limits | The borrowed TypeScript/TSX syntax view is recovery-aware and bounded; static values, regex/call/fact budgets, file-set validation, and typed limit diagnostics are covered by language tests. |
| Framework extraction | React renders/roles, Next App/Pages stages, React Router/Remix handlers, TanStack code/file routes and generated-tree negative, and Vite AST config/plugin/glob facts have focused positive/negative tests and resolver publication. |
| Generic facts and consumers | Typed relation/configuration/file-set facts reach graph publication; route hierarchy carries convention provenance; query/impact/lenses, semantic diff, task context, CLI/MCP, VS Code, and viewer schemas have strict tests. |
| Qualification wiring | The independent oracle recognizes `renders`, typed route stages, route hierarchy, configuration, file-set imports, negative activation, and rejects any positive partial-publication diagnostic. The existing code-graph gate invokes the frontend gate. |
| Repository gates | Workspace clippy, workspace library/binary tests, task-context integration, CompassQL/OpenCypher TCKs, JS typecheck/unit/Playwright (87/87), viewer asset determinism, product boundary, Python oracle, broad code-graph lifecycle, and the dedicated production frontend fixture gate pass. |

The exact frontend fixture result is deterministic across its repeated
production update: 112 nodes, 128 edges, 12 framework nodes, 8 routes, 2
route-hierarchy edges, 4 renders, 2 configuration fields, 1 file-set resource,
6 file-set imports, and 4 negative-corpus nodes. The broad code-graph gate also
passes all lifecycle byte comparisons and the independent summary. Its known
mixed-language fixture still reports bounded omissions (4 nodes/4 edges); that
is an accepted diagnostic in that broader gate, not evidence that a positive
frontend corpus may be partial—the dedicated frontend oracle fails on any such
diagnostic.

The following are the only remaining gaps found by this audit. They are
release-evidence gaps, not unassigned design holes, and each has an owner and
acceptance condition in Phase 0.5/9:

| Remaining gap | Concrete evidence | Required closure before `DONE` |
|---|---|---|
| Pinned representative corpus | `tests/qualification/react-frontend-repositories.toml` is explicitly `mode = "fixtures-only"`; it contains no immutable repository revisions, checksums, licenses, or reviewed external expectations. `/Volumes/Workspace/Github` has no pinned React-family qualification set. | Add read-only immutable corpora covering the seven stable target families (React, Next App, Next Pages, React Router, Remix, TanStack Router, Vite), keep TanStack Start separately labelled, and validate mutable refs/checksums/licenses/scope. |
| Independent precision/recall metrics | `scripts/qualify_react_frontend_graph.py` currently validates schema, endpoint/provenance vocabulary, selected counts, hierarchy, renders, config, file-set imports, and negatives; it does not match every expectation by identity/range/direction/role/provenance or compute precision, recall, ambiguity, anchor, multiplicity, or Wilson bounds. | Implement the versioned expectation matcher and machine-readable result artifact; fail on skipped capabilities, zero denominators, fabricated targets, truncation-as-success, or thresholds below the quality budget. |
| Pinned-mode production command | `scripts/qualify_react_frontend_graph.sh` accepts only `--fixtures-only`; its banner mentions pinned mode but no pinned runner, established/candidate comparison, external artifact path, binary digest, or read-only checkout guard exists. | Add pinned mode and exact-release-binary identity, external result publication, network/process checks, source-tree immutability checks, and register it with the exact-production release gate. |
| Performance baseline | `PERFORMANCE.md` has no Plan 021 frontend baseline/result. Existing scale tests prove bounded behavior but do not record the required cold, unchanged-warm, semantic edit, manifest/config edit, restore, alternate-checkout, duration, and peak-RSS rows. | Freeze the Phase 0.5 release baseline with revision, binary digest, machine profile, corpus manifest, and explicit 10%/20% decisions. |
| Worker and interruption evidence | Fixture qualification repeats a default-worker `--force` build; broad lifecycle checks cover delete/rename/restore, but no frontend corpus rows compare one/default/max workers or cancellation/interruption cleanup and resume. | Add one/default/max byte comparisons and an interrupted run that leaves no publishable partial artifact and resumes to the uncanceled digest. |
| Package/config precedence matrix | Current project evidence is package-local and bounded, but the release corpus does not exercise npm/yarn/pnpm workspaces, nested/mixed versions, lockfile invalidation, `tsconfig` `extends`/references/`jsxImportSource`, aliases, or dependency-section policy. | Add the Phase 0.5 matrix and prove affected-package-only re-extraction plus fail-closed unsupported forms and cache invalidation. |
| Capability-level promotion | The fixture expectations cover one sentinel per family and the positive checker only requires aggregate minimums; Next App vs Pages, React Router vs Remix, TanStack Start, plugin identity, route hierarchy ambiguity, dynamic/unsupported forms, and multiplicity are not independently measured on pinned data. | Stratify reviewed records by framework/capability/declaration/negative/ambiguous state and keep support claims provisional until every advertised capability meets its own threshold. |

No additional hidden ownership, schema, parser, resolver, consumer, safety, or
determinism gap was found. The correct audit result is therefore “implementation
slice closed; release qualification still open,” not a premature promotion:
the phase ledger and `advisor-plans/README.md` row remain `TODO` until the
pinned corpus, metric/artifact, performance, worker/interruption, and
package/config evidence above are checked in and the exact production gate
passes twice. The reference documentation intentionally continues to describe
the non-React packs as provisional and the matrix as a release target.

Verification ledger for this fifth pass: `cargo fmt --all -- --check`,
`cargo clippy --workspace --lib --bins --locked -- -D warnings`,
`cargo test --workspace --lib --bins --locked --quiet`, the complete resolver,
language, graph, query, history, output, CLI, semantic-diff, task-context,
CompassQL, and OpenCypher gates, `npm run typecheck:js`, `npm run test:js`,
`node scripts/check_viewer_assets.mjs`, `sh scripts/check_product_boundary.sh`,
the Python oracle tests, `git diff --check`,
`./scripts/qualify_react_frontend_graph.sh --fixtures-only`, and
`./scripts/qualify_code_graph_v1.sh --fixtures-only` all exit 0. The standard
macOS linker emits its existing `__eh_frame section too large` warning; it does
not change any test or qualification result.

### Sixth-pass release audit (2026-08-23)

The pinned qualification wiring is now exercised end to end against the
immutable seven-corpus manifest, so the earlier fifth-pass table must not be
read as the current state. The audit used the exact release binary
`d5516d3f92e9b17ef4b45998bc0496e363f2d64e2b50f7691746719080968d10` and the
source-only TypeScript 5.9.3 oracle. The result artifact is outside the
checkout at
`/Volumes/Workspace/crabbuild-target/compass-021-react-frontend/qualification/react-frontend-audit10/react-frontend-pinned-result.json`.

Evidence now present:

- All seven pinned checkouts were at their full commit SHA, clean, license
  checksummed, and unchanged after extraction. The projection and subprocess
  harness now re-checks bounds after config closure and applies a 30-minute
  command limit.
- The audit produced 227,696 oracle observations and 13,236 scored
  relationship/role facts: 13,236/13,236 matched, precision 1.0, recall 1.0,
  Wilson lower bound 0.9997098561, and zero fabricated targets.
- Every advertised capability met the explicit per-capability 99% precision /
  95% recall gate. Unresolved route declarations remain visible as explicit
  unresolved records (five Next App records and 28 React Router records); they
  are not silently selected or counted as exact targets.
- Cold, warm, semantic-edit, manifest-edit, restore, and alternate-worker rows
  were recorded for every corpus. One/default/max-worker graph digests were
  byte-identical, and the interrupted Next run returned SIGINT without a
  publishable pointer; its clean resume matched the uncanceled digest.
- The independent scorer now matches capability facts one-to-one, preserving
  duplicate occurrence/multiplicity accounting. A regression test proves one
  graph edge cannot satisfy two identical oracle occurrences.

Residual release gates are narrower but still real:

| Remaining gate | Current audit disposition | Required closure |
|---|---|---|
| Approved performance baseline | The audit result is intentionally `candidate-not-compared`; no checked-in Plan 021 baseline exists. The shell gate now rejects normal pinned mode without `--baseline`, and baseline rows must carry schema `compass.react-frontend-performance-baseline/1`, the same manifest digest, and positive measurements. | Freeze and review a baseline in `PERFORMANCE.md`/the qualification volume, then run the normal pinned command with `--baseline` and retain the comparison artifact. |
| Reviewed external expectation provenance | The compiler oracle is independent and source-derived, while the manifest now pins commits, licenses, and scopes. It is not yet a checked-in, per-capability reviewed expectation ledger for the external corpora. | Add a reviewed expectation-policy/record digest (including explicit unresolved/unsupported and ambiguous cases) and require it in promotion review; do not infer review from a passing generated oracle. |
| Package/config precedence matrix | The seven corpora exercise real projects, but not the full promised npm/yarn/pnpm workspace, nested-version, `tsconfig` inheritance/reference, `jsxImportSource`, alias, lockfile, and affected-package invalidation matrix. | Add the Phase 0.5 matrix with expected fail-closed outcomes and cache-scope assertions, then run it with the same production binary. |
| Promotion scope | TanStack Start remains separately labelled and is not part of the seven stable rows. Dynamic/opaque React factories remain intentionally unsupported or unresolved where the contract says so. | Keep those claims provisional until their own reviewed fixtures and thresholds pass; never promote them on aggregate results from another family. |

The current implementation and qualification gates therefore have no hidden
parser, ownership, graph-consumer, determinism, or safety hole found by this
audit, but Plan 021 is still **not release-complete**. Keep every phase and
the `advisor-plans/README.md` row `TODO` until the four residual gates above
are closed and the exact pinned production command passes with an approved
baseline (and the fixture gate remains byte-stable on a second run).

### Seventh-pass closure audit (2026-08-24)

This pass closes the four residual gates identified by the sixth-pass audit;
the historical sixth-pass text above is retained as the earlier checkpoint.

- The reviewed expectation policy is checked in at
  `tests/qualification/react-frontend-expectation-policy.json`, is bound to
  the manifest digest `6de700895d36ecb7b38f3964878936ad397d419a113d55ed2fbdc6586f971e6b`,
  and carries ledger digest
  `13af10ea1bf9ab86895809f3332be25de6aa976cf56590e5c577545a57a4418d`.
  Every external oracle record has a reviewed exact, unresolved, ambiguous,
  or unsupported disposition; promotion does not infer truth from a generated
  production graph.
- `scripts/qualify_react_frontend_matrix.py` and
  `tests/qualification/react-frontend-precedence-matrix.json` exercise npm,
  Yarn, and pnpm package scope, lockfile invalidation, TypeScript `extends` and
  project references, `jsxImportSource`, aliases, and affected-package cache
  boundaries. The production release binary passes all eight matrix cases,
  including exact restoration and stable sibling output.
- Project evidence is built before verified-output fast paths and includes the
  bounded manifest/config/lockfile and TypeScript-config closure digest. A
  package/config edit therefore cannot reuse a stale verified graph, while
  unrelated package scopes remain reusable.
- React client directives are module-wide boundaries: private component
  declarations and `export { Component }` forms receive client roles, while
  `use server` remains export-scoped. Lazy/component render endpoints accept
  the variable-to-function form with exact source anchors and multiplicity.
- The final pinned run reports 227,696 independent oracle records and 13,236
  scored facts with 13,236/13,236 matches, precision 1.0, recall 1.0, Wilson
  lower bound 0.9997098561, zero fabricated targets, and zero unsafe paths.
  The exact result path
  (`/Volumes/Workspace/crabbuild-target/compass-021-react-frontend/qualification/plan021-evidence/react-frontend-pinned-result-final.json`),
  result digest (`ab7eb4c5961ac9f8cae0c7aae9fb155404e864873bde0da2519ef6d32d515082`),
  release binary digest, 42-row performance comparison, and interruption/resume
  evidence are recorded in the external qualification artifact named in
  `PERFORMANCE.md`.
- TanStack Router is qualified by its own capability rows. TanStack Start
  remains explicitly pre-stable; dynamic or opaque factories remain
  unsupported/unresolved and are not promoted from aggregate results.
- The Graphify fixture comparison remains diagnostic: Compass uses the
  source-grounded oracle and typed directed graph contract; Graphify output is
  not a runtime dependency, fallback, or promotion oracle.

The final exact pinned command and both fixture runs pass. The checked-in
high-water baseline records the shared-volume jitter and preserves the 1.10×
budget; no threshold was widened to make a noisy run pass. The remaining
repository-wide baseline exception is the unrelated pre-existing
`missing_dotnet_references_are_external_and_do_not_abort` publication
resilience test, which fails before and after this change and is not used as
frontend evidence. The phase ledger and plan index can be promoted after the
post-push CI run confirms the same checked-in contracts.

## Why this matters

Compass already extracts useful TypeScript/JavaScript and JSX evidence, but
its frontend framework layer is uneven: generic JSX is represented only as
references, Next.js is primarily inferred from a few file paths, Vite config
uses text and regular-expression matching, and TanStack Router/Start has no
dedicated pack. The resulting graph can answer language questions but cannot
reliably answer the daily questions an enterprise coding agent needs:

1. Which component renders this component, and from which JSX occurrence?
2. Which URL, layout, loader, action, error boundary, or server handler owns
   this code?
3. Does this symbol execute on the browser, server, build tool, or across an
   explicit client/server boundary?
4. Which tests, routes, and upstream components are affected by a change?
5. Which Vite alias, plugin, or eager glob controls discovery and bundling?

This program adds those answers without executing user JavaScript, requiring
Node.js, downloading grammars, contacting framework services, or inventing
meaning when resolution is ambiguous. The result is a deterministic,
provenance-preserving graph that agents can safely use for navigation, impact
analysis, review, and change planning.

## Outcome and support contract

The shipped support matrix after this plan is:

| Surface | Target support | Required graph evidence |
|---|---|---|
| React | Qualifying | components, custom hooks, exact render occurrences, root render entry points |
| Next.js App Router | Qualifying | route hierarchy, route groups, slots, intercepting routes, layouts/templates/boundaries, route handlers, client/server directives, server functions |
| Next.js Pages Router | Qualifying | pages, API routes, `_app`, `_document`, `_error`, dynamic/catch-all segments |
| React Router framework/data mode | Qualifying | route tree, components, loaders, actions, middleware/error boundaries where statically declared |
| Remix | Qualifying | file/config routes, route modules, loaders, actions, error boundaries, and established flat-route parity |
| TanStack Router | Qualifying | file- and code-based route trees, parentage, path/id, loaders and route components |
| TanStack Start | Qualifying, separately labelled pre-stable | server routes/functions and client/server boundary evidence supported by pinned fixtures |
| Vite | Qualifying | config roots, ordered aliases, plugins, and bounded literal `import.meta.glob` expansion |

“Qualifying” is not a marketing synonym for complete. Documentation must list
the exact supported declaration forms, unsupported dynamic forms, limits, and
the versioned evidence producer. Promotion beyond Qualifying requires the
quality gate in Phase 9 and a separate maintainer decision.

## Current state

### Existing architecture to preserve

- `crates/compass-model/src/code_graph.rs` owns graph records, `NodeRole`,
  `EdgeKind`, route stages, validation, and the serialized
  `compass.graph/1` contract.
- `crates/compass-languages/src/lib.rs` and `Cargo.toml` define the language
  crate boundary and dependencies.
- `crates/compass-files` owns source discovery, ignore/scope policy, path
  containment, manifests, and atomic/cache I/O. Vite glob expansion must reuse
  that boundary rather than walking the filesystem from a language pack.
- `crates/compass-languages/src/evidence/typescript.rs` emits native
  TypeScript/JavaScript syntax evidence. JSX tags already emit exact
  `CandidateRelation::References` occurrences with `context = "jsx"`; JSX
  props, spreads, and child expressions have distinct contexts. Preserve
  these language facts even when adding framework relations.
- `crates/compass-languages/src/evidence_pipeline.rs` and
  `typescript_universal_evidence.rs` register TypeScript and JavaScript as
  hard-cut universal pipelines. TSX is classified as TypeScript. Do not
  restore a legacy fallback.
- `crates/compass-languages/src/frameworks/pack.rs` owns framework pack
  identity, detection, capabilities, and emitted raw framework facts.
- `crates/compass-languages/src/frameworks/model.rs` owns the serialized raw
  framework fact variants and `FrameworkLimits`. `RawRouteFact` currently has
  one `handler_reference`, one route-wide anchor, and a string vector of
  `middleware_references`; it cannot faithfully represent independently
  anchored layout, loader, action, boundary, or per-method stages.
- `crates/compass-languages/src/frameworks/mod.rs` still registers
  `typescript-web`, `nextjs-routes`, `remix-routes`, and `vite-config` as
  established source adapters. This plan migrates each affected pack once,
  atomically, after candidate parity.
- `crates/compass-languages/src/frameworks/next.rs` delegates to
  `file_routes::detect_next`. `file_routes.rs` recognizes basic App Router
  `page`/`route` files and Pages Router pages/API routes, but does not model
  layouts, templates, loading/error/not-found boundaries, route groups,
  parallel slots, intercepting routes, or server directives.
- `crates/compass-languages/src/frameworks/typescript.rs` contains the current
  React Router/Remix-style route extraction and its nearest tests.
- `crates/compass-languages/src/frameworks/vite.rs` currently uses
  `body.contains(...)` and regular expressions for plugins and aliases and
  attaches broad whole-file anchors. It is the behavior to freeze, replace,
  and then delete—not a pattern to extend.
- `crates/compass-languages/src/project_evidence.rs::parse_vite_configuration`
  and `parse_next_configuration` run before per-file framework detection and
  currently use `contains`/regex helpers. Vite aliases are collapsed into a
  `BTreeMap<String, String>`, which loses array order, regex-vs-string kind,
  duplicate entries, anchors, and completeness. Replacing only `vite.rs`
  would leave heuristic project resolution in production.
- There is no generic React component framework pack to hard-cut. JSX
  references are language evidence. `typescript-web` currently owns React
  Router alongside Angular, Nest, and Vue; removing that entire pack would be
  an unrelated regression. `nextjs-routes`, `remix-routes`, and `vite-config`
  are the established pack IDs that can be migrated in place.
- `crates/compass-languages/src/frameworks/mod.rs::UniversalDetectionContext`
  already exposes source bytes, the prepared tree-sitter `root`, project
  evidence, and `SemanticEvidenceBatch`. The missing piece is a shared,
  bounded TypeScript/JavaScript AST helper with consistent recovery,
  completeness, and literal rules; packs must use that existing one-parse seam
  rather than adding ad-hoc walkers or source-string matching.
- `crates/compass-resolve` owns project-wide target selection, ambiguity, and
  identity. `crates/compass-resolve/tests/typescript_routes.rs` has the closest
  end-to-end Next/Vite framework test.
- `crates/compass-graph` owns deduplication and publication. Framework facts
  must pass through the same validation, provenance, and deterministic sort
  boundaries as other graph evidence.
- `crates/compass-core/src/task_context.rs` builds agent-facing context
  sections under strict schema `compass.task-context/1`. It currently has no
  dedicated framework context section, and adding a new enum value can break
  old strict readers unless the schema is versioned or the feature is exposed
  through a compatible typed extension.
- `crates/compass-query` owns impact, traversal, natural-query discovery, and
  CompassQL execution. Its default impact relation set includes structural
  relations such as calls, references, imports, and embeds. A render edge must
  be integrated deliberately; it must not be reclassified as a call.
- `packages/compass-viewer/src/contracts/codeQuery.ts` contains closed Zod
  lists for graph edge kinds and node roles. Viewer source must change before
  regenerating `crates/compass-output/assets/viewer/{graph.js,viewer.css,manifest.json}`.
- `crates/compass-semantic-diff`, `crates/compass-history`, graph inference,
  query discovery/ranking, CLI/MCP contracts, and the viewer are downstream
  policy consumers of new graph values even where Rust exhaustiveness does not
  force a compile error.
- `crates/compass-core/src/pipeline.rs` serializes and compacts
  `RawFrameworkFact`, normalizes its paths, fingerprints fact-neutral output,
  and reuses it from the AST cache. New variants and changed pack semantics
  must update all exhaustive matches and cache identity together.

### Load-bearing current excerpts

Confirm these shapes during the drift check; line numbers are from planned-at
commit `4e12ca92`.

`crates/compass-model/src/code_graph.rs:189` has a closed semantic-role enum
ending at `Generated`, and `:208` has a closed `EdgeKind` ending at `MapsTo`:

```rust
pub enum NodeRole {
    Controller,
    // ...
    Generated,
}

pub enum EdgeKind {
    Contains,
    // ...
    MapsTo,
}
```

`crates/compass-languages/src/frameworks/model.rs:51,141` demonstrates the raw
route limitation and closed fact union:

```rust
pub struct RawRouteFact {
    // ...
    pub anchor: RawFrameworkAnchor,
    pub handler_reference: String,
    pub middleware_references: Vec<String>,
    // ...
}

pub enum RawFrameworkFact {
    Route(RawRouteFact),
    Domain(RawDomainFact),
    Annotation(RawFrameworkAnnotationFact),
}
```

`crates/compass-languages/src/frameworks/mod.rs:57` proves universal packs
already share the prepared parse:

```rust
struct UniversalDetectionContext<'source, 'tree> {
    path: &'source Path,
    source: &'source [u8],
    root: Node<'tree>,
    project: Option<&'source ProjectEvidence>,
    evidence: &'source SemanticEvidenceBatch,
}
```

`crates/compass-core/src/task_context.rs:14,81` shows why Phase 8 requires a
schema decision:

```rust
pub const TASK_CONTEXT_SCHEMA: &str = "compass.task-context/1";

pub enum TaskContextSectionKind {
    DeclarationSource,
    ExactCallers,
    ExactCallees,
    ImplementationType,
    RelatedTests,
    TransitiveImpact,
}
```

### Contract and design constraints

Read these files before implementing any phase:

- `AGENTS.md`
- the affected crate's `src/lib.rs`, `Cargo.toml`, and nearest tests
- `COMPATIBILITY.md`
- `docs/implementation/workspace-tour.md`
- `docs/implementation/extending-compass.md`
- `docs/design/principles.md`
- `docs/design/language-architecture.md`
- `docs/reference/universal-semantic-evidence.md`

The load-bearing rules are:

- Per-file extractors emit syntax evidence; project-wide target selection
  belongs in `compass-resolve`.
- Discovery, identities, ordering, encoding, and output must be deterministic.
- Ambiguous targets remain explicit. Never select the first candidate.
- Preserve relationship direction, multiplicity, source anchors, and
  provenance. One syntactic render occurrence is one relationship occurrence.
- All source, graph, archive, query, and subprocess work is bounded. A limit
  error is not an empty result.
- Normal extraction remains native and offline. No Node.js runtime, TypeScript
  compiler, framework CLI, build execution, package install, network service,
  runtime grammar download, or Graphify dependency is allowed.
- Tests use checked-in fixtures or bounded local fakes, never real services or
  credentials.

### Proposed graph vocabulary

Freeze this vocabulary in Phase 0; do not let individual framework phases
invent incompatible synonyms.

1. Add an additive `EdgeKind::Renders` relation, serialized consistently with
   existing edge naming. Direction is renderer/owner → rendered component.
   Each JSX element, fragment-owned component reference, or supported
   `createElement` occurrence gets its own source anchor and provenance.
2. Keep the existing JSX `References` edge. `Renders` is a framework semantic
   projection over the same syntax, not a replacement for language evidence.
   Publish `Renders` only for an exact target. Unresolved or ambiguous render
   facts retain bounded candidates/diagnostics and the underlying language
   reference; they do not create a convenient placeholder render edge.
3. Add typed roles only after the compatibility decision:
   `ui_component`, `hook`, `client_boundary`, `client_component`,
   `server_component`, `server_function`, and `data_loader`. Roles may coexist
   when facts justify them; a client component is still a UI component.
   A React function/class keeps its structural `NodeKind::Function` or
   `NodeKind::Class`; `ui_component` is a semantic role. Do not retype it as
   `NodeKind::Component`, which is already used for synthetic framework/domain
   components such as plugins and bean definitions.
   `client_boundary` means a valid directive boundary. `client_component`
   attaches to the file/module boundary and `client_component` means the
   component is declared in that directly evidenced boundary;
   importing a component below the boundary does not automatically relabel its
   declaration. `server_component` is limited to a directly qualifying
   framework convention and is not a deployment-topology claim.
4. Extend typed route-stage vocabulary only where downstream consumers need
   it: `layout`, `template`, `loading`, `default`, `error_boundary`,
   `not_found`, `data_loader`, and `action`, in addition to
   middleware/handler.
5. Ordinary hook invocation remains `Calls`. “Uses hook” is derived from a
   call whose resolved target has the `hook` role; do not create a redundant
   edge kind.
6. Framework-owned logical route nodes may be synthetic only when their stable
   identity, source provenance, and collision rules are specified and tested.
   Prefer attaching route evidence to declared symbols/files when that
   preserves the framework's actual model.
7. Route hierarchy uses an explicitly validated `Contains` edge from parent
   `NodeKind::Route` to child `NodeKind::Route`; do not overload `RoutesTo`.
   Inherited layouts/boundaries remain attached once to their owning route
   node instead of being copied onto every descendant. Add only the exact
   route→route endpoint allowance, not a blanket “route is a container” rule.
8. Freeze render-owner attribution: ordinary JSX is owned by its innermost
   declared callable; a class component's `render` method normalizes to the
   owning class component; top-level JSX uses the smallest exact declared
   value or file owner; render props/component-valued parameters remain
   unresolved symbolic targets unless project resolution proves a concrete
   declaration. Tests and stories retain their test/file owner so component
   impact can find them.
9. Decide and version typed `RenderEdgeDetails` rather than hiding durable
   semantics in free-form context strings. At minimum distinguish JSX,
   `createElement`, root render, and statically proven lazy indirection while
   keeping the relationship site at the actual render occurrence.
10. Add typed import details for Vite glob projections (mechanism, eager/lazy,
    selected import, query, ordered pattern identity) through the same
    compatibility decision. Do not encode these durable fields into one
    colon-delimited context string.

`compass.graph/1` uses typed serialized enums. Before adding any enum value,
prove whether existing readers tolerate it. If they do not, use the repository
compatibility process and bump the appropriate contract/version; do not call
an incompatible enum addition “additive.”

## Quality budget

All phases share these non-negotiable gates:

- Reviewed synthetic fixtures: 100% expected facts present, zero unexpected
  framework facts.
- Negative fixtures: zero false activations when a package name, JSX-shaped
  string, route-like filename, or config keyword occurs without the required
  syntactic/project evidence.
- Repeated and cold/warm-cache runs: byte-for-byte equivalent normalized graph
  output.
- Source anchors: exact token/expression range when the parser provides it;
  never a whole-file anchor merely because extraction is easier.
- Ambiguity: zero silently chosen ambiguous targets.
- Safety: zero execution of project configs or generated code; all glob and
  route expansion obey explicit count/path/byte limits.
- Final pinned corpus: at least 2,000 independently reviewed relationship and
  role records, at least 400 records per stable framework family, and at least
  100 per advertised relation/capability. The total floor is
  `max(2,000, 400 × N_stable_families)`, where
  `N_stable_families` counts each support-matrix row promoted as stable (the
  current target matrix has seven: React, Next App Router, Next Pages Router,
  React Router, Remix, TanStack Router, and Vite; TanStack Start is separately
  pre-stable). Aggregate observed precision must be at least 99.5%, Wilson
  lower bound at least 99%, and each advertised capability at least 99%
  precision and 95% recall. The capability matcher must preserve one-to-one
  occurrence multiplicity; explicitly unresolved, ambiguous, and unsupported
  records are reported separately and require a reviewed disposition rather
  than disappearing from the artifact or being treated as exact matches.
  There must be zero fabricated targets, unsafe path escapes, or
  nondeterministic results.
- Performance: record cold, unchanged-warm, one-file semantic edit,
  manifest/config edit, restore, and peak RSS using the methodology in
  `PERFORMANCE.md`. Any median latency or peak-RSS regression above 10% against
  the frozen Phase 0 baseline requires an explained maintainer decision; a
  regression above 20% is a failed gate. Correctness thresholds are never
  traded away to meet performance.
- Parallel determinism: the same fixture and pinned corpus must produce
  byte-identical normalized graphs with one worker, the default worker count,
  and the configured maximum; cancellation/interruption followed by a clean
  resume must either publish the complete equivalent artifact or a typed
  failure, never a plausible partial success.

## Commands you will need

Every Cargo command must use the external per-checkout target directory. Run
the preflight in each new shell before the first build:

```bash
test -d /Volumes/Workspace && test -w /Volumes/Workspace
mkdir -p /Volumes/Workspace/crabbuild-target/compass-021-react-frontend
```

Expected: both commands exit 0. If the volume is absent or not writable, STOP;
do not create a local `target/` directory.

| Purpose | Command | Expected on success |
|---|---|---|
| Format | `cargo fmt --all -- --check` | exit 0, no diff |
| Model tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-model --locked` | all pass |
| Language tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --locked` | all pass |
| Resolver tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-resolve --locked` | all pass |
| Query tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-query --locked` | all pass |
| Semantic diff | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-semantic-diff --locked` | all pass |
| History | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-history --locked` | all pass |
| CLI contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-cli --test compass_product --locked` | all pass |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0 |
| Code-graph fixture gate | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend ./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0, deterministic fixture qualification passes |
| Frontend pinned audit evidence | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend ./scripts/qualify_react_frontend_graph.sh --pinned --audit-only --artifact-root /Volumes/Workspace/crabbuild-target/compass-021-react-frontend/qualification/react-frontend-audit` | exit 0; seven immutable corpora, independent scorecards, worker/interruption rows, and an external result artifact |
| Frontend pinned release gate | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend ./scripts/qualify_react_frontend_graph.sh --pinned --baseline /Volumes/Workspace/crabbuild-target/compass-021-react-frontend/qualification/react-frontend-performance-baseline.json --artifact-root /Volumes/Workspace/crabbuild-target/compass-021-react-frontend/qualification/react-frontend-release` | exit 0 only with the approved baseline, all quality/performance thresholds, and exact release binary identity |
| Clippy baseline | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo clippy --workspace --lib --bins --locked -- -D warnings` | exit 0, no warnings |
| Test baseline | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test --workspace --lib --bins --locked` | all pass |
| JS contracts | `npm ci && npm run typecheck:js && npm run test:js` | all exit 0 |
| Generated viewer | `node scripts/build_viewer_assets.mjs && node scripts/check_viewer_assets.mjs` | generated assets match source and check exits 0 |

When a phase changes CompassQL grammar/support, also run:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-cypher --test tck --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-query --test opencypher_tck --locked
python3 scripts/check_compassql_support.py
```

Expected: all exit 0. Add a plan-specific qualification command in Phase 9;
that command becomes the authoritative final gate.

## Suggested executor toolkit

- Use the `diagnose` skill, if available, when a precision/recall regression
  cannot be reduced to one fixture.
- Use official framework documentation as the semantic source of truth. Pin
  the documentation/framework versions represented by qualification fixtures.
- Primary semantic references at planning time:
  - React rules and hooks: <https://react.dev/reference/rules> and
    <https://react.dev/reference/react/hooks>
  - Next.js App Router and directives:
    <https://nextjs.org/docs/app>,
    <https://nextjs.org/docs/app/api-reference/directives/use-client>, and
    <https://nextjs.org/docs/app/api-reference/directives/use-server>
  - Next.js route conventions:
    <https://nextjs.org/docs/app/api-reference/file-conventions/route>,
    <https://nextjs.org/docs/app/api-reference/file-conventions/parallel-routes>,
    and
    <https://nextjs.org/docs/app/api-reference/file-conventions/intercepting-routes>
  - TanStack Router route trees, file routing, and loading:
    <https://tanstack.com/router/latest/docs/routing/route-trees>,
    <https://tanstack.com/router/latest/docs/routing/file-based-routing>, and
    <https://tanstack.com/router/latest/docs/guide/data-loading>
  - TanStack Start server routes/functions:
    <https://tanstack.com/start/latest/docs/framework/react/guide/server-routes>
    and
    <https://tanstack.com/start/latest/docs/framework/react/guide/server-functions>
  - React Router framework routes and route modules:
    <https://reactrouter.com/start/framework/routing> and
    <https://reactrouter.com/start/framework/route-module>
  - Vite aliases and glob imports:
    <https://vite.dev/config/shared-options.html#resolve-alias> and
    <https://vite.dev/guide/features.html#glob-import>
- Use external public repositories only under
  `/Volumes/Workspace/Github/<owner>/<repository>`, and treat existing
  checkouts as read-only. Never clone qualification repositories into this
  checkout or `/tmp`.

## Scope

**In scope** (only as required by the phase being executed):

- `crates/compass-model/` — typed roles, relationships, route stages, schemas
- `crates/compass-files/` — bounded, ignored-aware file-set evaluation used by
  literal Vite glob facts
- `crates/compass-languages/` — React/framework evidence and pack migration
- `crates/compass-resolve/` — project semantics, identity, ambiguity
- `crates/compass-graph/` — validation, deduplication, publication
- `crates/compass-core/` — orchestration and framework task context
- `crates/compass-query/` and `crates/compass-cypher/` — traversal, impact,
  natural and structured query support
- `crates/compass-history/` and `crates/compass-semantic-diff/` — compatibility,
  fingerprints, and relationship-change semantics for new typed values
- `crates/compass-output/`, `crates/compass-cli/`, and `crates/compass-mcp/` —
  contract-preserving presentation and agent-facing access
- `packages/compass-viewer/`, `editors/vscode/`, `tests/viewer/`, and generated
  `crates/compass-output/assets/viewer/` — closed TypeScript contracts and
  compatibility rendering only; no visual redesign
- `fixtures/`, `tests/qualification/`, `benchmarks/performance/`, and `scripts/`
  — independent expectations, corpus manifests, measurements, and gates
- `docs/`, `COMPATIBILITY.md`, `CHANGELOG.md`, `MIGRATION.md`, `SECURITY.md`,
  and `PERFORMANCE.md` when required
- `.github/workflows/compass-ci.yml` — wire the offline fixture gate and, when
  Plan 005 is available, the exact-production release gate

**Out of scope**:

- A viewer redesign or new frontend UI. Update rendering only if the existing
  viewer must recognize new typed graph values.
- Vue, Nuxt, Svelte, SvelteKit, Astro, Angular, Solid, Qwik, or non-React
  framework expansion. Their existing behavior must not regress.
- TanStack Query, Table, Form, Store, and Virtual, plus Storybook and frontend
  styling/data-flow ecosystems. In this plan “TanStack” means Router and Start;
  the other packages need separate graph semantics and qualification rather
  than being silently implied.
- Vitest, Jest, Playwright, Cypress, and Testing Library framework semantics.
  Their source still receives ordinary language/test evidence; Vite ownership
  must not silently turn a test-runner config or test helper into application
  route, render, or runtime-boundary evidence.
- React Router's experimental RSC APIs and any unpinned prerelease form. Keep
  them as ordinary language evidence until a separate maturity decision.
- React Native/Expo and platform-specific JSX/runtime semantics. This is a
  web-framework graph plan; a future native plan must define its own package
  activation, platform boundaries, and qualification corpus.
- React runtime portals/hydration, `cloneElement`/`Children` execution
  semantics, and arbitrary runtime element factories unless Phase 0 adds an
  exact parser fact, target rule, and independent expectation. They remain
  ordinary language evidence or an explicit unsupported/incomplete result.
- Type-level prop flow, runtime state flow, taint analysis, React reconciliation
  modeling, or inferred runtime bundle contents.
- Executing Vite/Next/TanStack configs, running framework CLIs, importing
  arbitrary JavaScript, or requiring Node.js for normal graph extraction.
- A general-purpose package manager, lockfile resolver, or TypeScript compiler
  service. Use existing project semantics and bounded native parsing.
- Treating file names, capitalization, `use` prefixes, or package strings alone
  as proof of semantic meaning.
- Graphify runtime, fixtures, tests, configuration, artifacts, or fallbacks.
- Existing untracked roots such as `qualification/` or `routes/`; the checked-in
  qualification owner is `tests/qualification/` and framework fixtures belong
  under `fixtures/code-graph/`.

## Git workflow

- Use branch prefix `codex/`, for example `codex/021-react-graph-phase-1`.
- Land one phase or one independently green vertical slice per PR. Do not mix
  unrelated cleanup with semantic changes.
- Match the repository's conventional commit style, for example
  `feat(graph): add occurrence-preserving render relations`.
- Do not push, publish a release, or open a PR unless the operator requests it.
- At every handoff, record exact commands run and checks omitted.

## Phase delivery ledger

Each row is a separate review unit. Update this ledger as phases land; do not
mark the plan-level index row `DONE` until Phase 9 passes.

| Phase | Deliverable | Depends on | Status |
|---:|---|---|---|
| 0 | support contract, evidence matrix, baselines | Plan 013 hard cut | DONE |
| 1 | graph/route vocabulary and downstream compatibility | 0 | DONE |
| 2 | parser-backed framework syntax/config/fact/cache substrate | 0, 1 | DONE |
| 3 | React pack and render graph | 2 | DONE |
| 4 | Next.js pack hard cut | 2, 3 | DONE |
| 5 | TanStack Router/Start packs (Start remains pre-stable) | 2, 3 | DONE |
| 6 | React Router/Remix migration | 2, 3 | DONE |
| 7 | Vite hard cut | 2 | DONE |
| 8 | task context, impact, queries, trust boundary | 1, 3–7 | DONE |
| 9 | exact-production release qualification | 0–8 | DONE |

## Per-phase verification commands

When a step says “run the Phase N gate,” it means the exact commands below,
all of which must exit 0. Newly named test targets are created by that phase;
their absence is a failed/incomplete phase, not permission to skip them.

**Phase 0**

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-resolve --test framework_qualification --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --test registry --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-core --test code_graph_v1_determinism --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend ./scripts/qualify_code_graph_v1.sh --fixtures-only
```

**Phase 1**

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-model --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-graph --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-output --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-query --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-semantic-diff --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-history --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-cli --test compass_product --locked
npm run typecheck:js
npm run test:js
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
sh scripts/check_product_boundary.sh
```

**Phase 2**

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --test typescript_universal_evidence --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --test registry --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --test package_manifest_coverage --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-resolve --test framework_routes --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-resolve --test framework_qualification --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-core --test code_graph_v1_determinism --locked
```

**Phases 3–7**

```bash
# Phase 3
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --test react_universal_pack --locked
# Phase 4
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --test next_universal_pack --locked
# Phase 5
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --test tanstack_universal_pack --locked
# Phase 6
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --test react_router_universal_pack --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-resolve --test typescript_routes --locked
# Phase 7
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-languages --test vite_universal_pack --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-files --locked
# Shared after each phase
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-resolve --test react_frontend --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-core --test code_graph_v1_determinism --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend ./scripts/qualify_code_graph_v1.sh --fixtures-only
```

**Phase 8**

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-core --test task_context --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-query --test code_impact --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-query --test code_traversal --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-query --test natural_intent --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-mcp --test code_query_tools --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test -p compass-cli --test task_context_cli --locked
npm run typecheck:js
npm run test:js
node scripts/build_viewer_assets.mjs
node scripts/check_viewer_assets.mjs
```

**Phase 9**

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend ./scripts/qualify_react_frontend_graph.sh --fixtures-only
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend ./scripts/qualify_code_graph_v1.sh --fixtures-only
cargo fmt --all -- --check
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo clippy --workspace --lib --bins --locked -- -D warnings
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend cargo test --workspace --lib --bins --locked
sh scripts/check_product_boundary.sh
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
```

## Phase 0: Freeze the semantic contract and independent truth

### Step 0.1: Write the support specification before changing production

Create `docs/reference/react-framework-graph.md`. Define:

- framework activation evidence and non-activation cases;
- stable node roles, edge direction, multiplicity, context, and provenance;
- route identity and display-path rules for every supported router;
- runtime domains (`browser`, `server`, `build`, `shared`, `unknown`) as
  evidence labels, not guessed execution facts;
- precedence when file conventions and explicit declarations coexist;
- exact production pack IDs and migration ownership:
  `react-ui` (new), `nextjs-routes` (migrated in place),
  `react-router-routes` (new extraction owner removed surgically from
  `typescript-web`), `remix-routes` (migrated in place),
  `tanstack-router` (new), `tanstack-start` (new), and `vite-config`
  (migrated in place); `typescript-web` remains for Angular/Nest/Vue;
- ambiguity, generated-file, symlink, path-containment, and limit behavior;
- the exact supported declaration forms and explicit unsupported dynamic forms.

Add an ADR if `Renders`, new roles, route stages, or runtime-domain evidence
change a public contract. Update `COMPATIBILITY.md` in the same phase if the
decision affects compatibility policy.

**Verify**:

```bash
rg -n "Renders|multiplicity|ambigu|runtime|limit|unsupported" \
  docs/reference/react-framework-graph.md
```

Expected: every term appears in a normative section; no statement describes a
planned form as already shipped.

### Step 0.2: Add a framework-neutral expectation schema

Extend `crates/compass-resolve/src/frameworks/qualification.rs` rather than
building a competing harness. Add a versioned, serializable expectation format
under `tests/qualification/` that records source file, exact range,
source/target stable identity, role/relation, route metadata, provenance, and
expected ambiguity. The expectation loader must reject unknown major versions,
duplicate IDs, reversed/invalid ranges, out-of-root paths, and unbounded record
counts. Keep the existing Rust `FrameworkQualificationCase` API as a thin
fixture convenience over the same evaluator.

The oracle must be independent of production extractors. It may use reviewed
manifests and separately implemented qualification-only parsers, but it must
not call the product function it is evaluating.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-resolve --test framework_qualification --locked
```

Expected: valid round-trip passes; each corrupt/over-limit fixture fails with a
typed actionable error, and all established route qualification cases pass.

### Step 0.3: Freeze current behavior and negative fixtures

Capture current normalized output for:

- JSX intrinsic vs component tags, member tags, fragments, spread props,
  aliases, namespace imports, default imports, and ambiguous targets;
- basic Next App/Pages routes and Vite aliases/plugins;
- existing React Router/Remix behavior;
- non-framework projects containing misleading strings and filenames.

The baseline is evidence, not a requirement to preserve known false positives.
Classify each delta as preserve, intentional fix, or unsupported.

**Verify**: run each fixture twice and through a warm cache. Expected:
byte-identical normalized outputs across all three runs.

### Step 0.4: Prove the language evidence is sufficient

Create a checked-in evidence-sufficiency matrix in
`docs/reference/react-framework-graph.md`. For every promised declaration form,
name the exact parser node/evidence fact, owner identity, literal fields,
completeness signal, and source range the pack will consume. At minimum cover:

- directive prologues (`use client`/`use server`);
- JSX tags and owners, `createElement`, root render, `lazy`, and
  `next/dynamic` imports;
- statically recoverable calls, call arguments, exports, object/array
  properties, spreads, computed keys, regex/string aliases, and conditional
  branches;
- file-convention evidence whose anchor is a path rather than a token.
- explicitly unsupported web-runtime forms such as portals/hydration,
  `cloneElement`/`Children` execution, arbitrary runtime element factories,
  React Native/Expo platform APIs, and Storybook-only semantics; each must
  have a negative expectation rather than an accidental “not observed” result.

`UniversalDetectionContext` already supplies the prepared syntax root. Phase 2
must standardize one bounded TypeScript/JavaScript helper over that borrowed
root and source, shared by all frontend packs; do not reparse once per pack,
serialize parser node handles, or fall back to source-string/regular-expression
matching. The helper must mark dynamic/computed/spread/conditional regions
incomplete instead of flattening them into false literals.

**Verify**:

```bash
rg -n "directive|JSX|createElement|object|array|spread|computed|incomplete|anchor" \
  docs/reference/react-framework-graph.md
```

Expected: every promised framework form maps to an existing fact or a named
Phase 2 syntax-view addition; no row says “scan source text.”

### Step 0.5: Freeze package-scope and performance baselines

Specify framework activation and config precedence per nearest owning package,
not repository-wide. Cover npm/yarn/pnpm workspaces, nested package manifests,
workspace protocols, mixed framework versions, nested Next/Vite configs,
TypeScript project references, and `jsxImportSource`. Also decide and fixture
the dependency-section policy (`dependencies`, `devDependencies`, optional and
peer dependencies), npm aliases, pnpm catalogs, Yarn Plug'n'Play markers,
conditional `exports`/`imports`, and package-manager lockfile changes. A
lockfile is an invalidation input, not permission to run a package manager.
Resolve TypeScript `extends`, `baseUrl`, `paths`, module-resolution mode,
`jsx`, `allowJs`, and project-reference inheritance as bounded project
evidence; unsupported compiler options remain explicit diagnostics. A root
React dependency must not activate React semantics in an unrelated package
that uses another JSX runtime.

Record cold, warm, semantic-edit, manifest/config-edit, restore, and RSS
baselines using `PERFORMANCE.md`. Include the exact revision, release binary,
machine profile, corpus manifest, and digests.

**Verify**: add focused `ProjectEvidenceIndex` tests in
`crates/compass-languages` and an incremental fixture in
`crates/compass-core/tests/code_graph_v1_determinism.rs`; run both owning-crate
test suites. Expected: only files in the affected package are re-extracted
after a package/config change, restored output matches the original bytes, and
package-manager/compiler-option fixtures resolve or fail closed according to
the matrix, and the baseline artifact validates against its pinned schema.

**Phase exit gate**: run every Phase 0 command in “Per-phase verification
commands.” Expected: all exit 0 before any public vocabulary change begins.

## Phase 1: Add the graph vocabulary as a complete vertical slice

### Step 1.1: Decide and test every affected public contract

In `crates/compass-model/src/code_graph.rs` and its public-contract tests,
determine whether adding enum values to `compass.graph/1` is accepted by every
supported reader. Test JSON round-trips, unknown-value behavior, stable
ordering, GraphML/HTML/JSON rendering, history fingerprints, MCP schemas, and
CompassQL enumeration.

Inventory and assign an explicit policy for every closed consumer before
editing the enum. The minimum inventory is:

- `crates/compass-model/src/code_graph.rs` and `validation.rs`;
- `crates/compass-model/src/query_contract.rs`, `search.rs`, and
  `identity.rs` (closed query/role/edge and stable-ID consumers);
- `crates/compass-graph/src/{v1.rs,inference.rs,snapshot.rs}`;
- `crates/compass-query/src/{code_query.rs,discovery.rs,index.rs,ranking.rs}`;
- `crates/compass-semantic-diff/src/topology.rs` and history publication/diff;
- `crates/compass-store-qualification/src/main.rs` and qualification graph
  builders;
- `crates/compass-cli/src/{help.rs,task_context_commands.rs}`,
  `crates/compass-mcp/src/lib.rs`, and `crates/compass-core/src/task_context.rs`;
- output viewer models, strict CLI/MCP schemas, and task context;
- `packages/compass-viewer/src/contracts/codeQuery.ts`, edge labels, semantic
  appearance, `tests/viewer/fixtures/generate.ts`, VS Code transport/messages
  and consumers, and generated viewer assets (ignored build outputs must be
  regenerated, never hand-edited);
- code-graph coverage manifests, qualification oracles, and support docs.

Freeze a downstream policy matrix: callers/callees exclude `Renders`; impact
includes inbound renderers with a reasoned path; semantic diff reports render
add/remove; history fingerprints it; discovery/query can filter both
directions; viewer labels it; topology/community/ranking either assign a
reviewed weight or explicitly exclude it. Never let a wildcard/default match
silently decide semantics.

If existing readers reject new enum values, follow `COMPATIBILITY.md`: update
the graph contract/version, migration documentation, changelog, fixtures, and
consumer negotiation together. Do not silently weaken deserialization.

**Verify**: run the model, graph, output, query, semantic-diff, history, JS
contract, generated-viewer, CLI contract, and product-boundary commands from
“Commands you will need.” Expected: new compatibility tests and all existing
tests pass, TypeScript Zod schemas accept the new values, and generated assets
match their source.

### Step 1.2: Add `Renders` and frontend roles

Add the vocabulary from “Proposed graph vocabulary” to the model and every
exhaustive consumer. Specify:

- source → target direction;
- allowed endpoint kinds;
- exact occurrence multiplicity;
- stable serialized spelling;
- display label and query alias;
- validation for source anchors/provenance;
- preservation of unknown attributes where the contract permits them.
- the exact endpoint matrix: source is an owning file, callable, class
  component, or existing synthetic component with direct render evidence;
  target is an exactly resolved function/class/component declaration carrying
  `ui_component`, or a source-scoped qualified external component endpoint
  proven by an exact import binding and JSX use. Do not inspect or invent the
  package's internal file. Parameters, import nodes themselves, unqualified
  package guesses, and unresolved placeholders are not silently promoted to
  concrete component targets.

Do not emit production `Renders` edges in this step. First make the contract
and all consumers safe.

**Verify**: run the same Phase 1 command set from Step 1.1. Expected: all pass,
including round-trip, typed render-details, exhaustive-consumer, and invalid-
endpoint tests.

### Step 1.3: Replace the one-handler raw route shape with typed stages

Add a bounded `RawRouteStageFact` in
`crates/compass-languages/src/frameworks/model.rs` with role, independently
anchored reference, optional operation, ordinal, origin/rule, and typed detail.
Migrate `RawRouteFact` from one route-wide `handler_reference` plus
`middleware_references` to an ordered `stages` vector so layout, template,
loading/default/error/not-found, loader, action, middleware/proxy, page, and
per-method route handlers do not borrow the wrong anchor. Update both public
route-stage types and `crates/compass-resolve/src/frameworks/routes.rs` so the
new stages survive raw fact → resolution → graph → query → output round trips.

Treat this serialized raw-fact change as a cache-wire change: either reject old
entries and bump the owning cache/extraction semantics version, or add an
explicit validated migration. Do not deserialize an old handler into a new
stage without preserving its operation, position, and anchor.

Audit `RoutesTo` endpoint validation for inline arrow/function expressions
used as loaders, actions, middleware, and route components. Add only
source-proven callable `Closure` or callable variable/property endpoint
widens required by fixtures, following the existing compatibility precedent
for additive endpoint-matrix changes. Never accept every variable/property as
a handler merely to make inline routes validate.

**Verify**: run `cargo test -p compass-languages --locked`,
`cargo test -p compass-resolve --test framework_routes --locked`, and
`cargo test -p compass-model --locked` with the required target directory.
Expected: one integration fixture containing every stage serializes and
queries back in deterministic route order; old-cache handling is explicit.

### Step 1.4: Define route hierarchy and identity

Freeze separate identities for framework/package scope, router family,
normalized display URL, non-URL structural identity (groups/slots/intercepts),
operation, and source convention. Parent route → child route is a `Contains`
edge with cycle rejection and deterministic sibling order. A page and `GET`
handler at the same display URL must not collide; two parallel slots with the
same display URL must not collapse; route groups must not leak into the URL.

Update `RouteNodeDetails` only through the Phase 1 compatibility decision. Do
not encode hierarchy solely in `declaring_scope` or display strings, and do
not duplicate inherited stages onto descendants.

**Verify**: add public model/graph and resolver tests for same-URL operations,
groups, two parallel slots, an intercept, missing parents, and a cycle.
Expected: identities are distinct where semantics differ, stable across file
order/checkouts, and traversable through validated `Contains` edges.

**Phase exit gate**: run every Phase 1 command. Expected: all Rust/TypeScript
consumers accept the chosen contracts and generated assets are current.

## Phase 2: Build a universal framework evidence substrate

### Step 2.1: Standardize the existing parser-backed frontend syntax view

Create a shared helper module such as
`crates/compass-languages/src/frameworks/typescript_syntax.rs` over the
existing borrowed `UniversalDetectionContext.root` and source bytes. It must
provide exact ranges and explicit completeness for directive prologues,
exports, calls and arguments, JSX ownership, object/array properties,
literal/regex values, spreads, computed keys, and conditional branches. It
must not expose parser node handles beyond the extraction lifetime or persist
arbitrary source literals not selected by a pack.

Make all React/Next/TanStack/React Router/Remix/Vite packs use this helper.
Reject nodes that overlap parser recovery ranges, and join AST shapes to
universal imports/bindings/owners before assigning meaning. Add a truthful
capability only if the descriptor registry needs to state this contract. Bump
the TypeScript/JavaScript producer version and evidence/cache schema only when
serialized evidence changes; framework-only helper semantics are covered by
the pack version/digest in Step 2.3.

**Verify**: add malformed, recovery-overlap, Unicode, multiline, dynamic,
incomplete-spread, and maximum-fact tests in
`crates/compass-languages/tests/typescript_universal_evidence.rs`, then run:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-languages --test typescript_universal_evidence --locked
```

Expected: every syntax view item has an exact valid range, incomplete shapes
remain incomplete, parser recovery cannot be upgraded to framework meaning,
and framework-pack dispatch performs no additional parse.

### Step 2.2: Add generic role and relation facts

Extend the framework pack contract with typed, versioned facts equivalent to:

- `RawFrameworkRoleFact`: exact subject anchor/identity hint, role, context,
  provenance, confidence/evidence class;
- `RawFrameworkRelationFact`: exact source and target evidence, relation kind,
  occurrence anchor, context, provenance, and ambiguity policy.
- `RawFrameworkConfigurationFact`: pack/config identity, typed static field,
  exact anchor, ordinal, completeness, and bounded value selected by the pack;
- `RawFrameworkFileSetFact`: owning declaration/file, ordered literal and
  negative patterns, eager/lazy/import/query options, anchor, package scope,
  and explicit resolution limits. The language pack emits the pattern fact;
  `compass-files`/`compass-resolve` own match evaluation and target selection.

Do not add React-specific variants to a global enum. Reuse the universal
evidence occurrence and candidate identity types where ownership allows it.
The raw fact must be incapable of expressing a resolved target without the
resolver's evidence.

Before adding variants, capture `rg -l 'RawFrameworkFact::' crates | sort` and
classify every exhaustive match: language emitters, engine sorting/limits,
core compaction/digest/path portability, resolver expansion/publication, and
tests. Update each intentionally; do not add wildcard arms that hide a new
fact family from cache normalization or publication.

**Verify**: serialization, validation, bound, sort, duplicate, corrupt-fact,
and incomplete-configuration tests pass in `compass-languages` and
`compass-model`; file-set facts cannot contain resolved filesystem targets at
the per-file extraction boundary.

### Step 2.3: Add explicit framework capabilities and pack versions

Extend framework capabilities for component roles, render relations, runtime
boundaries, route hierarchy, route data stages, build configuration, and
bounded file sets. A
pack advertises a capability only when it emits and qualifies the corresponding
facts. Update manifests and public support reporting atomically.

Add a nonzero framework-pack semantics version to universal descriptors and
established runtime entries. Include the ordered `(pack_id, version,
capabilities, activation rules, limits)` digest in extraction/cache identity.
The version is an explicit per-pack maintainer-controlled value, not a hash of
an opaque function pointer. Changing a detector implementation, syntax-view
contract, activation rule, fact shape, or limit requires a reviewed version
bump and must invalidate affected cached framework facts even when source bytes
and package manifests are unchanged. Registry tests must fail when a pack has
no nonzero version or when a changed fixture is accepted under a stale version.

Extend `FrameworkLimits` with named nonzero bounds for source/config bytes and
AST nodes/depth, retained literal/string bytes, role and relation facts,
diagnostics, route nodes/stages, glob patterns, per-pattern matches, total
file-set edges, alias expansions, and regex pattern length/complexity. Apply
limits before allocation where possible and report observed/maximum values
through the existing typed limit error pattern. Aggregate project limits remain
separate from per-file limits; a limit error never becomes an empty successful
framework result. Regex literals may be retained as evidence but must not be
evaluated for arbitrary filesystem expansion or allowed to trigger unbounded
backtracking.

**Verify**: support matrix tests fail when a pack advertises a capability with
no fixture expectations and pass for the completed candidate packs.

### Step 2.4: Resolve framework facts centrally

Create `crates/compass-resolve/src/frameworks/semantic.rs` and integrate it
through `frameworks::resolve_framework_facts` in
`crates/compass-resolve/src/lib.rs`, where route/domain facts are currently
expanded, resolved, and published. Resolve role and relation facts using exact language
evidence, import/export identity, project aliases, and framework-declared
parentage. Preserve unresolved and ambiguous candidates explicitly. Enforce
the same bounded fan-out and deterministic tie rules as universal relations.

Stage roles, render relations, routes, and domains in a cloned/temporary
extraction, validate the complete set, then publish it coherently. A limit,
invalid endpoint, bad anchor, or resolution error in one new framework family
must not leave a partial graph that appears successful. Preserve the existing
typed diagnostic behavior for unrelated established packs.

**Verify**: tests cover exact, re-exported, aliased, unresolved, ambiguous,
duplicate, cyclic, and over-limit targets. No test expects “first match wins.”

### Step 2.5: Make package activation and incremental invalidation exact

Extend `ProjectEvidenceIndex` and its per-file fingerprint only where Phase
0's matrix proves a gap. Framework packs must receive the nearest owning
package/config scope, resolved JSX runtime, and pack-semantics digest. A
manifest/config/generated-route-tree change invalidates all and only dependent
files; a source-only edit must not re-extract unrelated packages.

Update every exhaustive `RawFrameworkFact` consumer in
`crates/compass-core/src/pipeline.rs`: compaction, digest normalization,
portable path rewriting, partial-extraction clearing, cache serialization, and
tests. New facts with more than one source path must normalize and validate
each path, not only a single common anchor.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-core --test code_graph_v1_determinism --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-languages --test registry --locked
```

Expected: cold/warm/forced/semantic-edit/manifest-edit/config-edit/restore
digests match their declared expectations, stale pack versions miss cache,
and unrelated packages remain cache hits.

### Step 2.6: Replace heuristic frontend project-config indexing

Refactor the project-evidence build into an explicit bounded frontend-config
prepass. Parse recognized Next/Vite/React Router/TanStack configuration files
with the same pinned TypeScript/JavaScript parser and shared AST helper, then
build `ProjectEvidenceIndex` before dependent source resolution. Store a typed
ordered alias rule list with source config, string/regex kind, exact anchor,
ordinal, replacement, and completeness; keep TypeScript/package alias families
separate so their precedence is not flattened.

Delete Vite/Next `contains`, quoted-alias regex, and plugin-name substring
semantics from `project_evidence.rs`. Generic or other-language configuration
parsers may remain only for their existing owners and must not feed the new
frontend packs. Avoid duplicate config parsing by carrying the normalized
prepass facts into the later framework pack; if the language graph still needs
the config AST for declarations, measure and document that bounded second
parse rather than pretending it does not occur.

Version the project-evidence schema/fingerprint and test old-cache rejection.
Bound manifest/config bytes, object/array entries, nesting depth, literal
retention, and diagnostic count before indexing. Define duplicate-key and
package-exports/conditional-branch behavior explicitly. Config parse
failure/incompleteness must remain a diagnostic and cannot be silently
replaced by regex facts; regex values are bounded, preserved as typed evidence,
and never evaluated as an untrusted filesystem matcher.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-languages --test package_manifest_coverage --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-core --test code_graph_v1_determinism --locked
! rg -n "parse_vite_configuration|parse_next_configuration" \
  crates/compass-languages/src/project_evidence.rs
```

Expected: tests pass; the negated search exits 0. A generic text helper may
remain only if every caller is an explicitly out-of-scope non-frontend
configuration owner; ordered aliases and config
incompleteness survive cold/warm/cache round trips.

**Phase exit gate**: run every Phase 2 command. Expected: all exit 0 and no
framework pack reparses or uses text matching to compensate for missing facts.

## Phase 3: Ship the universal React pack

### Step 3.1: Detect React projects from converging evidence

Create `crates/compass-languages/src/frameworks/react.rs` and register the new
universal `react-ui` pack. Activation requires supported syntax
plus project evidence such as a resolved React import/runtime or framework
dependency; a package string or JSX-looking text alone is insufficient.
Support classic and automatic JSX runtimes without assuming the import name
is literally `React`. Respect package-scoped `jsxImportSource`; Preact and
other JSX runtimes are negative cases unless a React-based framework provides
separate exact activation evidence.

**Verify**: positive fixtures for both runtimes activate; negative fixtures
with comments, strings, similarly named local modules, and JSX in a non-React
runtime do not emit React semantic facts.

### Step 3.2: Classify components conservatively

Recognize declarations whose syntax and use provide direct evidence:

- function/arrow/class components returning supported JSX;
- declarations exactly targeted by a JSX element or a framework route
  component field, including components that legitimately return `null`;
- classes with an exact resolved React component base and a declared render
  method;
- exported/default, aliased, namespace-member, `memo`, and `forwardRef`
  wrappers when their callable target is statically recoverable;
- higher-order wrapper chains only while each link remains explicit and
  bounded.
- statically recoverable `React.lazy` and `next/dynamic` component bindings;
  retain lazy indirection provenance and leave computed loaders unresolved.

Do not classify by uppercase naming alone. Do not infer a component from a
type signature without a rendered value. Attach roles to the declared symbol
and preserve wrapper occurrence provenance.

**Verify**: fixtures cover nested/local components, same-name declarations,
anonymous defaults, wrappers, non-components returning strings/objects, and
ambiguous re-exports.

### Step 3.3: Emit occurrence-preserving render relations

Project resolved JSX component references to `Renders` edges. Support member
tags and statically resolved `createElement`/configured JSX factory calls.
Intrinsic DOM/custom-element tags remain language references but do not create
component render targets unless the project defines a supported component
identity for that exact tag form.

An exactly imported external component may produce a qualified external
component endpoint with import and JSX provenance. Namespace/member imports
must retain the selected member. Wildcard or conditional package exports that
do not identify one external symbol remain unresolved.

For conditional, looped, or repeated JSX, keep one edge per syntactic
occurrence; do not estimate runtime cardinality. Keep the existing JSX
`References` facts.

JSX in a test or story-like file follows the owner rules from Phase 0 and can
therefore supply inbound render impact, but this plan does not add Storybook
framework semantics. A JSX component parameter/render prop stays a symbolic
reference unless exact project resolution identifies the passed declaration.

**Verify**: assert edge direction, exact source range, target stable ID,
context, provenance, multiplicity, and stable sort order for every exact form;
unresolved/ambiguous forms retain candidate diagnostics and no `Renders` edge.

### Step 3.4: Add hook and root-entry evidence

Classify built-in hooks by resolved, version-pinned React export identity.
Classify a custom hook only when a declared `use*` callable has an exact call
path to a resolved built-in or already qualified custom hook; a `use*` name
alone is insufficient even inside a React project. Bound/cycle-check the
custom-hook closure. Hook invocations remain `Calls`.

Recognize statically imported `createRoot(...).render(...)`, legacy
`ReactDOM.render`, and supported framework root APIs. Anchor the render edge at
the rendered component expression, not the entire call/file.

**Verify**: aliased imports, namespace imports, shadowing, nested functions,
false `use` prefixes, and ambiguous roots are covered.

### Step 3.5: Qualify and enable the new React pack

There is no established generic React component pack to remove. Register one
`react-ui` owner in the same explicitly `Qualifying`/provisional registry used
by the other staged universal packs, but do not advertise it as a completed
support claim or promote it beyond that state until the independent gates pass.
Preserve TypeScript/JavaScript JSX `References` and do not remove
`typescript-web`, which still owns Angular/Nest/Vue and the pre-Phase-6 React
Router path. Prove that the new semantic projection does not duplicate any
existing graph edge identity.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-languages --test react_universal_pack --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-resolve --test react_frontend --locked
```

Expected: registry tests prove exactly one `react-ui` production pack;
repeated and cold/warm outputs match; JSX references remain; render edges are
not duplicated; product-boundary and fixture gates pass.

**Phase exit gate**: run the Phase 3 and shared Phase 3–7 commands. Expected:
all exit 0.

## Phase 4: Deepen Next.js App and Pages Router evidence

### Step 4.1: Replace path-only route discovery with bounded project semantics

Create a universal Next pack that combines exact file conventions, parsed
exports/directives, configuration roots, and project identity. Normalize route
groups without adding URL segments; preserve parallel slot and intercepting
route identity instead of flattening it. Support dynamic, catch-all, and
optional catch-all segments with deterministic identities.

Pin the supported Next major(s) in fixtures and make version-sensitive
conventions explicit. Include `default` slot fallbacks and the applicable
`middleware`/`proxy` convention for each pinned major. Treat private folders,
metadata/instrumentation files, and unsupported future conventions as explicit
non-route or separately documented evidence, never pages by filename shape.

Treat `src/app`, `app`, `src/pages`, and `pages` precedence as an explicit
project rule. Never walk outside the project root or follow unbounded symlink
cycles.

**Verify**: table-driven fixtures cover groups, nested groups, slots,
intercepts, private folders, colocated non-route files, dynamic segments,
`default`, versioned middleware/proxy, page-vs-route conflicts, root
precedence, Windows separators, non-UTF-8-capable path handling where the test
harness supports it, and path traversal attempts.

### Step 4.2: Model route stages and hierarchy

Emit route evidence for `page`, `layout`, `template`, `loading`, `error`,
`global-error`, `not-found`, and `route` conventions that are supported by the
pinned Next version. Attach each stage to the correct route hierarchy and
declared handler/component. For Pages Router, include pages, API routes,
`_app`, `_document`, `_error`, and supported data functions.

For App Router, include statically declared `generateStaticParams`,
`generateMetadata`, and other pinned route-module exports in typed route
context when their identity is useful, but do not claim they are request
handlers or execute them. HTTP method exports in `route.*` remain separate
operation stages with exact export anchors.

Capture supported static route-segment exports such as deployment runtime,
dynamic rendering, revalidation, cache policy, and preferred-region metadata
only when the pinned Next contract and a literal value make their meaning
exact. Keep deployment runtime metadata distinct from React client/server
component boundaries: for example, an edge/server deployment target does not
by itself make a module a React server component. Computed or version-unknown
values remain typed incomplete/unsupported evidence rather than free-form
attributes.

Do not fabricate a handler when an expected export is absent. Preserve one
route-handler occurrence per supported HTTP method export.

**Verify**: query tests can move URL → hierarchy/stages → declared symbol and
back; missing or ambiguous exports remain explicit. Literal route-segment
metadata round-trips through JSON/history/task context, while computed values
stay incomplete and deployment/runtime labels never change component roles.

### Step 4.3: Model client/server boundaries from directives

Parse top-level directive prologues exactly. Add `client_component` only when
`'use client'` occurs in a valid directive position and the symbol is exported
through that boundary. Add server-function evidence for valid `'use server'`
module/function forms supported by the pinned Next semantics. Do not treat a
string elsewhere in the body as a directive.

Treat `server-only`/`client-only` imports as direct boundary evidence when
resolved to the documented packages. Do not propagate `client_component` to
every transitive import: the directive marks a client boundary, while a
descendant's execution domain remains unknown unless direct framework evidence
supports it. Record boundary and component-runtime semantics separately in the
Phase 0 contract so agents cannot confuse “imported by client code” with “has
its own client directive.”

Server-component classification is conservative: default App Router server
behavior is scoped by project/file convention and is overridden by a valid
client boundary. Record `unknown` rather than guessing through unsupported
dynamic re-export chains.

**Verify**: fixtures cover quote styles, comments/shebangs, invalid directive
placement, nested directives, barrel re-exports, client imports, server
actions, shadowed names, and mixed boundaries.

### Step 4.4: Parse Next config without executing it

Extract only statically recoverable config relevant to graph identity, such as
supported route roots, extensions, base path, and aliases delegated to
existing project semantics. Preserve conditional alternatives as incomplete or
ambiguous rather than choosing the first branch. Record statically literal
redirect/rewrite declarations as configuration evidence only if Phase 0
defines safe route/resource identities; otherwise report them as an explicit
unsupported capability. Reject or label computed/dynamic values as
unsupported; never import or run `next.config.*`.

**Verify**: static ESM/CommonJS/TypeScript config fixtures pass; getters,
function calls, environment-dependent expressions, and malicious side effects
are never executed and produce bounded diagnostics.

### Step 4.5: Perform the Next hard cut

Compare candidate facts with `nextjs-routes`, document intentional deltas,
switch the registry atomically, and remove the established adapter. Do not
dual-publish duplicate routes.

**Verify**: exactly one Next pack produces production facts; all old public
route cases remain covered; new route/boundary fixtures and determinism gates
pass.

Run the new `crates/compass-languages/tests/next_universal_pack.rs` and the
Next-filtered cases in `crates/compass-resolve/tests/react_frontend.rs`, plus
`crates/compass-core/tests/code_graph_v1_determinism.rs`. Expected: the
`nextjs-routes` pack ID is preserved, the established adapter is absent, and
all cold/warm/config-edit/restore assertions pass.

**Phase exit gate**: run the Phase 4 and shared Phase 3–7 commands. Expected:
all exit 0.

## Phase 5: Add TanStack Router and TanStack Start packs

### Step 5.1: Support code-based TanStack route trees

Recognize statically resolved TanStack Router APIs (including supported
`create*Route` forms) from import identity, not callee spelling alone. Extract
route ID/path, parent getter, component, loader, pending/error/not-found
components, and child composition when values are statically recoverable.
Resolve tree parentage centrally and preserve cycles/missing parents as
diagnostics, not invented roots.

Cover the pinned `createRootRoute`, `createRootRouteWithContext`,
`createRoute`, `createFileRoute`, `getParentRoute`, and `addChildren` forms,
plus supported lazy route/component APIs. Route masks, virtual file routes, and
future generator formats are advertised only when separately present in the
evidence matrix and qualification fixtures.

**Verify**: aliased/namespace imports, shadowed functions, reordered options,
spread/merge ambiguity, nested route trees, cycles, and duplicate IDs are
covered.

### Step 5.2: Support file-based TanStack routes

Pin and document the file-route convention being supported. Model root/index,
pathless/layout, dynamic, splat, non-nested, and grouped forms without losing
file identity. Treat configuration tokens such as route directories/prefixes
as static only when parsed exactly.

Generated route-tree files require an explicit policy: consume them only as
bounded generated evidence with provenance and deduplicate them against source
route files, or ignore them with a diagnostic. Never publish duplicate logical
routes from both inputs.

Package-scoped router configuration controls route-token/prefix behavior. A
generated tree from another workspace package must never influence this one.

**Verify**: route identity is stable across source ordering and generated-file
presence; malformed/generated drift is detected.

### Step 5.3: Model loaders, actions, and render boundaries

Attach route components and pending/error/not-found components as typed stages
or role/context evidence according to Phase 0. Loaders remain callable graph
targets with `data_loader` role; their invocations/dependencies retain normal
language call/import relations.

**Verify**: impact tests demonstrate component ↔ route ↔ loader navigation
without conflating render and call edges.

### Step 5.4: Add TanStack Start as a separate capability

Recognize supported server routes and server-function declarations only from
pinned, exact API/import evidence. Keep Start's support claim and quality
metrics separate from stable TanStack Router support because its public API is
pre-stable. Unsupported versions/forms must degrade to language evidence, not
misclassified framework facts.

Where the pinned API exposes validators or middleware as statically named
callables, preserve them as ordered stages with their own anchors. Do not run
validators, middleware builders, or server functions.

**Verify**: versioned fixtures exercise supported and intentionally
unsupported forms; documentation labels the maturity accurately.

### Step 5.5: Enable packs only after qualification

TanStack has no established production pack to preserve. Keep candidate packs
qualification-only until their reviewed fixture and minimum per-capability
precision/recall thresholds pass. Enable each pack with its manifest,
capability report, docs, and registry test in one PR.

**Verify**: before enablement, production discovery reports no TanStack pack;
after enablement, exactly the intended Router/Start pack and version activate.

Run `crates/compass-languages/tests/tanstack_universal_pack.rs`, the TanStack
cases in `crates/compass-resolve/tests/react_frontend.rs`, and the core
determinism test. Expected: `tanstack-router` and `tanstack-start` activate only
in their owning package, generated/source routes deduplicate by explicit
identity, and Start failures cannot promote Router metrics or vice versa.

**Phase exit gate**: run the Phase 5 and shared Phase 3–7 commands. Expected:
all exit 0.

## Phase 6: Migrate React Router and Remix to universal evidence

### Step 6.1: Freeze and port existing route behavior

Inventory every React Router/Remix form in
`crates/compass-languages/src/frameworks/typescript.rs` and its tests. Port the
behavior to universal role/relation facts without changing public route
identity unintentionally. Add data-router and framework-mode forms supported
by pinned official docs, including loaders, actions, lazy route modules, and
error boundaries when statically declared.

Where supported by the pinned React Router or Remix contract, recognize exact
client/server entry modules and documented `.client`/`.server` module
conventions as module-boundary evidence. Scope the convention to its owning
package and framework mode, keep it separate from React RSC semantics, and do
not infer a transitive execution domain for every importer. A matching suffix
outside an activated project is only ordinary language evidence.

Create a dedicated universal `react-router-routes` pack. After parity, remove
only React Router activation/detection from `typescript-web`; keep that pack's
Angular, Nest, and Vue behavior and regression tests. Migrate `remix-routes` in
place so its public pack identity remains stable.

**Verify**: established-vs-candidate comparison has an explained disposition
for every fact; reviewed negative fixtures have zero false activations.
Boundary fixtures cover framework/data/declarative modes, nested workspaces,
client/server entries, suffix lookalikes, barrels, and mixed Remix/React Router
packages without leaking roles across package roots.

### Step 6.2: Perform one atomic registry cut

Remove the established React Router branch and Remix adapter only after
candidate parity. Do not remove the whole `typescript-web` pack, and do not
leave a production fallback or dual publisher.

**Verify**: registry enumeration shows one owner per pack ID; old and new
fixtures pass with byte-stable output.

Run `crates/compass-languages/tests/react_router_universal_pack.rs`, the
React Router/Remix cases in
`crates/compass-resolve/tests/typescript_routes.rs`, and the mixed-framework
cases in `crates/compass-core/tests/code_graph_v1_determinism.rs`. Expected:
React Router facts come only from `react-router-routes`, Remix facts only from
`remix-routes`, and Angular/Nest/Vue established output is unchanged.

**Phase exit gate**: run the Phase 6 and shared Phase 3–7 commands. Expected:
all exit 0.

## Phase 7: Replace Vite regex extraction with parsed evidence

### Step 7.1: Parse Vite config statically

Replace `body.contains`/regex matching with exact syntax evidence for supported
`defineConfig`, plain export, function-export, and array forms. Parse only
literal/static values. Track config anchor and provenance precisely; never run
the config or a plugin.

Use the shared Phase 2 syntax view. For function configs and conditional
branches, publish only common statically proven facts or explicit alternatives
with incompleteness; never assume a command, mode, SSR flag, environment, or
first branch. Pin supported `.js`/`.mjs`/`.cjs`/`.ts`/`.mts`/`.cts` forms to
the Vite versions in qualification.

**Verify**: comments, strings, shadowed `defineConfig`, computed property
names, nested unrelated objects, malicious expressions, and syntax errors do
not produce false facts or side effects.

### Step 7.2: Preserve ordered alias semantics

Extract object and array alias forms with exact `find`/`replacement` anchors,
order, string-vs-regex distinction, and unresolved dynamic values. Feed only
safe literal path aliases into existing project resolution. Do not collapse
ordered entries into a map or interpret arbitrary regex as a filesystem path.

Inventory `root`, `resolve.alias`, `resolve.dedupe`, `resolve.conditions`, and
`resolve.extensions` in the Phase 0 support contract. Only fields with
well-defined native resolution semantics may affect target selection; retain
the rest as bounded configuration evidence or declare them unsupported. Do
not let Vite aliases bypass the established TypeScript/project alias
precedence.

**Verify**: overlapping aliases resolve by documented order, duplicate keys
remain reviewable, regex entries are preserved but not unsafely expanded, and
paths cannot escape configured containment.

### Step 7.3: Extract plugin identity and config dependencies

Resolve plugin factories through imports/re-exports when possible. Emit build
configuration dependency evidence with exact call anchors; never invoke the
plugin. Preserve unresolved/ambiguous factories and repeated plugin instances.

Virtual module IDs and plugin-defined resolution remain unresolved external
configuration evidence unless a checked-in static declaration proves an exact
target. Detect the official React and React-SWC plugin imports without
assuming every plugin whose name contains “react” has React semantics.

**Verify**: alias, namespace, shadow, duplicate, conditional, and array-spread
fixtures cover precision and multiplicity.

### Step 7.4: Add bounded literal `import.meta.glob` evidence

Recognize only the documented literal pattern forms. Resolve relative,
absolute-root, and supported alias patterns within project containment. Apply
explicit maximum pattern, match, path-length, traversal, and total-output
limits. Preserve eager/lazy and supported query/import options as context; do
not claim a runtime call edge to every lazy module. Project each exact matched
module as an `Imports` edge from the owning file/module with the glob literal
as relationship site and typed eager/lazy/options detail; target identity and
inventory provenance distinguish edges that share one pattern occurrence.

The Vite pack emits an anchored raw glob-pattern fact; it does not walk the
filesystem. Evaluate the file set through `compass-files` discovery/ignore and
containment primitives, then resolve/project it collection-wide. Include the
matched file-set fingerprint in incremental state so adding, removing, or
renaming a matching file invalidates the dependent glob without rebuilding
unrelated packages.

**Verify**: positive fixtures cover single/array/negative patterns and options;
negative fixtures cover variables, template expressions, path escapes,
symlink cycles, ignored files, add/delete/rename invalidation, huge matches,
unsupported options, and malformed globs. Limit failures are typed
errors/diagnostics, never empty success.

### Step 7.5: Perform the Vite hard cut

Compare against `vite-config`, switch atomically, and delete the regex adapter
and obsolete dependency if it has no other owner.

**Verify**:

```bash
! rg -n "body\.contains|Regex|regex" crates/compass-languages/src/frameworks/vite.rs
! rg -n "parse_vite_configuration|parse_next_configuration" \
  crates/compass-languages/src/project_evidence.rs
```

Expected: both negated searches exit 0; no extraction-by-text/regex
implementation remains (test names or comments documenting the removal may be
allowed), and no Vite/Next project-evidence caller reaches generic text
helpers. All Vite fixtures and product-boundary checks pass.

Also run `crates/compass-languages/tests/vite_universal_pack.rs`, the Vite
cases in `crates/compass-resolve/tests/react_frontend.rs`, and the core
determinism test. Expected: the `vite-config` pack ID is preserved, no
established adapter remains, config/alias/glob edits invalidate the correct
package only, and no fixture executes JavaScript or a plugin.

**Phase exit gate**: run the Phase 7 and shared Phase 3–7 commands. Expected:
all exit 0.

## Phase 8: Make the evidence useful to agent workflows

### Step 8.1: Version the task-context contract before adding a section

`TaskContextSectionKind` is a closed enum inside strict
`compass.task-context/1`. Decide whether framework context is an opt-in typed
extension compatible with v1 or requires `compass.task-context/2`; test both
old-reader/new-writer and new-reader/old-writer behavior. Update
`TASK_CONTEXT_SCHEMA`, request/response validation, CLI JSON, MCP tool schema,
docs, changelog, and migration note atomically when the major changes.

Do not silently add an enum value to v1 and assume old agents ignore it.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-core --test task_context --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend \
  cargo test -p compass-mcp --test code_query_tools --locked
```

Expected: version negotiation/strict rejection is deterministic and every
supported old/new pairing has an explicit test.

### Step 8.2: Add deterministic framework task context

Add a bounded `FrameworkContext` section in
`crates/compass-core/src/task_context.rs` and thread it through the owning CLI
and MCP contracts. For a focus symbol/file, report the smallest useful set:
framework identity, route and stages, runtime boundary, rendered-by/renders,
loaders/actions, config dependencies, provenance, ambiguity, and truncation.

Update `crates/compass-cli/src/task_context_commands.rs`,
`crates/compass-mcp/src/lib.rs`, and the VS Code transport/query clients as
strict readers: unknown schema versions, roles, edge kinds, pack IDs, or
qualification states must fail closed with a typed diagnostic. The ignored
VS Code `dist/` output and viewer bundle are regenerated from source during
verification; they are not hand-edited or used as a second contract.

Include pack ID/version, qualification state, graph/build identity, coverage
diagnostics, and explicit unsupported/incomplete capabilities. An agent must
be able to distinguish “no relation exists” from “the pack did not support or
finish extracting this form.”

Use typed machine fields; do not force agents to parse prose. Enforce stable
ordering and explicit record/byte limits.

**Verify**: snapshot/contract tests cover empty, normal, ambiguous, truncated,
and untrusted-label cases for CLI JSON and MCP responses.

### Step 8.3: Integrate render relations into impact analysis

Add inbound `Renders` to the appropriate default impact/traversal profiles so
a component change can find upstream renderers/routes. Preserve configurability
and document direction. Do not add a render edge to caller/callee results,
which remain call semantics.

**Verify**: a fixture change to a leaf component reaches its direct renderer,
owning route, and relevant tests with a reasoned path; unrelated components do
not appear; limits and cycles terminate deterministically.

### Step 8.4: Add query vocabulary and examples

Expose stable query aliases/filters for component, hook, client/server,
renders/rendered-by, route stage, loader/action, and framework identity in
natural discovery and CompassQL where the existing architecture supports
them. Update support claims and TCK coverage for any grammar change.

**Verify**: documented queries execute against checked-in fixtures and return
typed deterministic results. `check_compassql_support.py` passes when touched.

### Step 8.5: Harden the agent trust and disclosure boundary

Repository paths, labels, route text, config literals, comments, and source
snippets are untrusted data. Framework context must use typed fields, existing
label sanitization, verified source digests, and bounded strings; it must not
turn repository text into instructions, shell commands, Markdown/HTML control
content, or automatically executable actions. Avoid raw source/config values
unless the existing verified-source contract explicitly includes them.

Update `SECURITY.md` and `docs/design/security-and-privacy.md` if the new
CLI/MCP surface changes disclosure, source-reading, or transport boundaries.
Keep local-only behavior and MCP response-size failure semantics.

**Verify**: extend `crates/compass-core/tests/task_context.rs`, MCP contract
tests, CLI context tests, and viewer escaping tests with control characters,
markup, prompt-like text, oversized values, invalid UTF-8 paths where
supported, and truncation. Expected: output remains typed/escaped/bounded,
digests validate, and no semantic result is silently transport-truncated.

### Step 8.6: Publish day-to-day workflow recipes

Add guide/cookbook documentation for:

- “What renders this component?”
- “What route owns this file?”
- “What crosses the Next client/server boundary?”
- “What loaders/actions and tests are affected?”
- “Which Vite aliases/plugins/globs explain this dependency?”
- “How should an agent treat unresolved, ambiguous, or truncated evidence?”
- “Which pack/version produced this answer, and was that capability
  qualifying, incomplete, or unsupported?”

Separate current behavior from future plans. Include machine-readable CLI/MCP
examples and expected schema versions.

**Verify**: every command in docs runs against a checked-in fixture and is
covered by a doc/example test or a qualification script.

Run the task-context, query impact/traversal/natural-intent, MCP
`code_query_tools`, CLI code-query/context, JS contract, and generated-viewer
commands. Expected: all documented examples match typed snapshots and no
callers/callees response contains `Renders` unless explicitly queried as a
relationship.

**Phase exit gate**: run every Phase 8 command. Expected: all exit 0 and the
selected task-context compatibility behavior matches its documentation.

## Phase 9: Qualify, document, and gate the production claim

### Step 9.1: Build pinned representative corpora

Select immutable revisions representing monorepos and standalone projects for
each stable framework family. Store only repository URL, commit, license,
scope, checksums, and reviewed expectations in Compass; keep checkouts under
`/Volumes/Workspace/Github`. Include generated code, aliases, re-exports,
monorepo boundaries, Windows-style cases, large graphs, and adversarial files.

Create the checked-in manifest at
`tests/qualification/react-frontend-repositories.toml` and reviewed
expectations at
`tests/qualification/react-frontend-expectations.json`. Stratify samples by
framework, capability, declaration form, positive/negative outcome,
resolved/unresolved/ambiguous state, and relationship multiplicity. Count
route identity, target identity, direction, anchor, role, and provenance as
separate correctness dimensions so a partially correct edge does not count as
a true positive.

Do not use a framework's own generated manifest as unquestioned truth. Review
sampled facts against source and independent semantics.

**Verify**: manifest validation rejects mutable refs, checksum drift,
out-of-root paths, duplicate corpus IDs, missing licenses, and excess size.

### Step 9.2: Add a dedicated qualification command

Add the exact production gate `scripts/qualify_react_frontend_graph.sh` with:

- `--fixtures-only` for offline CI;
- pinned-corpus mode for release qualification;
- established/candidate comparison during migrations;
- per-framework and per-capability precision, recall, Wilson bound, ambiguity,
  anchor, multiplicity, determinism, cold/warm-cache, duration, and peak-memory
  reporting;
- cold, unchanged-warm, semantic-edit, manifest/config-edit, restore, and
  alternate-checkout performance rows compared with Phase 0 thresholds;
- one/default/max worker determinism and cancellation/interruption-resume
  rows; a canceled run must leave no publishable partial artifact and a clean
  rerun must match the uncanceled digest;
- a versioned machine-readable result artifact published outside source trees.

The gate must evaluate the exact production registry/configuration. It must
fail on skipped capability classes, zero denominators, truncation disguised as
success, unexpected network/process use, or thresholds below “Quality budget.”
It must run with qualification corpora mounted read-only and fail if a
checkout, source fixture, cache, or generated artifact inside a source tree is
modified. All temporary output and result artifacts belong under the designated
external qualification volume.
Build one release-mode `compass-cli` binary in the mandated external target
directory and use that exact binary for all candidate observations; record its
path, commit, features, and digest so a debug/test-only path cannot qualify the
release claim.

Reuse `scripts/code_graph_v1_oracle.py`, the existing universal-language audit
schema/runner, and `crates/compass-resolve/src/frameworks/qualification.rs`
where their truth boundaries fit. Do not fork a second canonical graph loader,
Wilson implementation, path validator, or production CLI invocation model.

Make `./scripts/qualify_code_graph_v1.sh --fixtures-only` invoke or otherwise
cover the new frontend fixture assertions so the existing product gate cannot
pass while this surface fails. Add the same offline command to
`.github/workflows/compass-ci.yml` if it is not already reached by an existing
job. When Plan 005 exists, register the pinned exact-production gate there;
otherwise document the equivalent release-candidate invocation and keep the
Plan 021 index dependency explicit.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend ./scripts/qualify_react_frontend_graph.sh --fixtures-only
```

Expected: exit 0; every advertised stable framework/capability has a non-zero
sample, all thresholds pass, and two runs produce identical normalized output.

### Step 9.3: Run release gates and update public claims

Run every command in “Commands you will need,” plus the pinned qualification
when its corpora are available. Update the language/framework support matrix,
reference docs, cookbook, `CHANGELOG.md`, and `MIGRATION.md` if users must act.
Document unsupported dynamic cases and exact limits.

Do not promote a framework/capability if another one passed aggregate metrics
on its behalf. TanStack Start remains separately labelled until its own gate
and maturity decision pass.

Run `compass update .` with the exact release candidate after code and docs are
final, as required by the universal-evidence implementation guide. Ensure any
generated Compass artifacts remain outside tracked source and are not added to
the patch.

**Verify**: all gates exit 0; `git diff --check` is clean; `git status --short`
contains only intended files; the published qualification artifact identifies
the exact commit, producer versions, fixture/corpus manifests, and production
configuration.

**Phase exit gate**: run the complete repository baseline, every applicable
surface gate, the frontend fixture qualification twice, and the pinned-corpus
qualification when promotion is requested. Expected: all required checks exit
0, normalized fixture outputs are byte-identical, every advertised capability
passes independently, the production binary is identified exactly, and only
then may the support matrix or Plan 021 index row be promoted.

## Cross-phase test plan

Add tests at the lowest useful layer and public-contract tests for visible
behavior. At minimum, the completed program covers:

- **Activation**: dependency plus syntax, aliases, nearest package/workspace
  scope, mixed versions, `jsxImportSource`, false package strings, non-React
  JSX runtimes, and multiple frameworks in one repository.
- **Identity**: imports, exports, barrels, aliases, namespaces, shadowing,
  anonymous defaults, same-name declarations, symlinks, and monorepo roots.
- **Relations**: direction, occurrence multiplicity, exact range, context,
  provenance, unresolved/ambiguous targets, deduplication, stable ordering.
- **React**: intrinsic/member tags, fragments, conditional/list JSX,
  `memo`, `forwardRef`, `lazy`, `next/dynamic`, factories, component-valued
  parameters, test owners, hooks, and root renders.
- **Next**: App/Pages conventions, groups, slots, intercepts, dynamic segments,
  `default`, versioned middleware/proxy, layouts/templates/boundaries, HTTP
  handlers, directives, route-segment runtime/deployment metadata, re-exports,
  package scope, and static/incomplete config without conflating deployment
  runtime and React component boundaries.
- **TanStack**: file/code trees, parent cycles, generated-tree policy,
  components/loaders/boundaries, server routes/functions, version drift.
- **React Router/Remix**: legacy parity plus data/framework route forms,
  package-scoped client/server entries, and `.client`/`.server` lookalike
  negatives outside an activated framework project.
- **Vite**: static config forms, ordered aliases, plugins, literal globs,
  negative patterns, path containment, limits, and non-execution.
- **Consumers**: JSON/GraphML/HTML, CompassQL, natural discovery, impact,
  strict task-context versioning, MCP, CLI, history/fingerprints, semantic
  diff, TypeScript Zod contracts, generated viewer assets, and cache reopen.
- **Operational**: cold/warm/repeated equivalence, bounded large files and
  graphs, pack-version/manifest/config invalidation, package-local incremental
  work, dependency-section/package-manager/compiler-option matrix,
  cancellation/interruption cleanup, corrupt cache/input handling, equivalent
  output across one/default/max worker counts, Linux/macOS/Windows path
  behavior, latency/RSS thresholds, and untrusted agent-facing text.

Use `crates/compass-resolve/tests/typescript_routes.rs` as the nearest existing
end-to-end framework pattern and the TypeScript universal evidence tests as the
nearest JSX occurrence pattern. Do not encode qualification truth by calling
the production extractor from expected-result generation.

## Done criteria

All must hold:

- [ ] The support specification and any required ADR/compatibility decision
  are merged before new public graph values are emitted.
- [ ] Exactly one production owner exists for `react-ui`, `nextjs-routes`,
  `react-router-routes`, `remix-routes`, `tanstack-router`, `tanstack-start`,
  and `vite-config`; no legacy fallback or dual publish path remains, while
  unrelated `typescript-web` Angular/Nest/Vue behavior is unchanged.
- [ ] The shared frontend syntax view is parser-backed, bounded, incomplete-
  aware, shared by every pack without per-pack reparsing, and has no
  source-string/regex semantic fallback.
- [ ] Raw route stages have independent anchors and survive cache, resolution,
  graph, history, query, and output round trips; route hierarchy has stable,
  collision-free identities and validated route→route containment.
- [ ] Framework pack semantics versions participate in cache identity; source,
  manifest, config, generated-tree, and pack-version edits invalidate exactly
  their dependent package scope.
- [ ] React component/hook roles and occurrence-preserving `Renders` edges have
  exact anchors, stable identity, provenance, ambiguity, and deterministic
  ordering.
- [ ] Next App/Pages route hierarchy, stages, and supported client/server
  boundaries pass positive, negative, ambiguity, and versioned fixtures.
- [ ] TanStack Router and separately labelled TanStack Start pass their own
  capability thresholds before activation/promotion.
- [ ] Vite production extraction contains no text/regex semantic matching,
  never executes configs/plugins, and bounds literal glob expansion.
- [ ] Framework context, impact, CLI, MCP, and query contracts expose the new
  evidence without conflating render and call semantics; strict task-context
  compatibility and untrusted-text tests pass.
- [ ] `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-021-react-frontend ./scripts/qualify_react_frontend_graph.sh --fixtures-only` passes twice
  with byte-identical normalized results.
- [ ] The pinned audit satisfies every threshold in “Quality budget,” with
  zero fabricated targets, unsafe path escapes, or nondeterministic outputs,
  and the performance thresholds have an explicit pass/approved decision.
- [ ] `cargo fmt`, targeted tests, workspace clippy/tests, product boundary,
  code-graph fixtures, CLI contract, and applicable CompassQL gates pass using
  the required external `CARGO_TARGET_DIR`.
- [ ] Reference, guide/cookbook, support matrix, changelog, compatibility, and
  migration docs accurately distinguish shipped, qualifying, unsupported, and
  pre-stable behavior.
- [ ] `git diff --check` passes and no unrelated/user files are modified.
- [ ] Plan 021's row in `advisor-plans/README.md` is `DONE` only after all
  phases and final production qualification complete.

## STOP conditions

Stop and report; do not improvise if:

- `/Volumes/Workspace` is absent or not writable before any Cargo build or
  external-corpus operation.
- Plan 013's TypeScript/JavaScript production hard cut is absent, has been
  reverted, or a legacy fallback is again active.
- Adding roles, relations, or route stages is incompatible with a supported
  reader and no contract migration/version decision has been approved.
- A framework fact requires executing project JavaScript/config, running a
  framework CLI, downloading code, or making Node.js a normal-runtime
  dependency.
- Exact framework semantics require source-text/regex matching, any
  framework-pack-owned reparse, or parser node handles that outlive their
  bounded extraction context; return to Phase 0 and revise the shared helper
  contract over `UniversalDetectionContext.root` instead.
- Pack behavior can change without a deterministic cache-identity change, or a
  package/config edit cannot invalidate dependent files without rebuilding
  unrelated packages.
- Correct resolution would require selecting the first/nearest ambiguous
  candidate, guessing from capitalization/file name/`use` prefix alone, or
  dropping occurrence/provenance data.
- A route/glob/config operation cannot be bounded or contained within the
  project root with existing safe primitives.
- Candidate and established outputs disagree without an independently reviewed
  disposition, or two production packs would publish the same fact.
- Implementing React Router would require deleting or changing unrelated
  Angular/Nest/Vue behavior in `typescript-web`.
- Route group/slot/intercept/operation identities collide, hierarchy requires
  duplicating inherited stages, or the raw fact cannot preserve each stage's
  own anchor.
- The qualification oracle shares the production extractor/resolver logic it
  is meant to evaluate.
- A required change crosses into an out-of-scope framework or a viewer redesign
  rather than a compatibility update for typed values.
- An applicable quality threshold fails twice after a focused diagnosis, or a
  capability has too few independent records to make the advertised claim.
- A performance regression exceeds 20%, or exceeds 10% without an explicit
  reviewed explanation recorded in `PERFORMANCE.md`.
- Existing unrelated work overlaps an in-scope file and cannot be preserved
  cleanly.

## Maintenance notes

- Reviewers should scrutinize evidence boundaries more than framework-name
  coverage. A small exact support surface is more valuable than broad false
  confidence in agent workflows.
- Framework versions and file conventions evolve. Add a pinned fixture and
  independent expectation before extending a supported declaration form.
- `Renders` is occurrence evidence, not React runtime cardinality. Keep that
  distinction in docs, queries, and future execution-flow work.
- Client/server labels express statically evidenced framework boundaries, not
  deployment topology or data-flow guarantees.
- Vite glob output describes statically selected module dependencies and
  eager/lazy context; it must not become an excuse for unbounded filesystem
  discovery.
- Future Vue/Svelte/Astro framework work should reuse the universal role and
  relation substrate, but must have its own support contract and qualification
  rather than inheriting React claims.
- Plan 017 may consume route/render evidence for ranked execution flows after
  this plan, but this plan must not depend on speculative flow inference.
