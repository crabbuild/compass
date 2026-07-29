# Task 1 — Framework and Domain Semantics Hardening

## Outcome

Task 1 is implemented in `a5200e3` with the verification-assertion follow-up in
`33ed9ef`. The production graph now retains canonical domain kinds, exposes
confidence-aware resolution, preserves every declared route stage, scopes
TypeScript aliases to their declaring source/module/export, retains conflicting
registration sites, and emits portable typed route-middleware facts.

## Files changed

- `crates/compass-core/tests/code_graph_v1_determinism.rs`
- `crates/compass-graph/src/v1.rs`
- `crates/compass-model/src/code_graph.rs`
- `crates/compass-model/tests/code_graph_v1.rs`
- `crates/compass-resolve/src/frameworks/domain.rs`
- `crates/compass-resolve/src/frameworks/routes.rs`
- `crates/compass-resolve/src/frameworks/typescript.rs`
- `crates/compass-resolve/tests/domain_resolution.rs`
- `crates/compass-resolve/tests/framework_routes.rs`
- `crates/compass-resolve/tests/typescript_routes.rs`

## RED evidence

- The focused framework/domain/TypeScript test build initially failed because
  `RouteStage` had no `reference`, `state`, `candidates`, or optional `target`
  representation.
- `sole_terminal_domain_candidate_remains_ambiguous_and_non_authoritative`
  initially failed with `Exact` instead of `Ambiguous`.
- `production_pipeline_preserves_framework_domain_kinds_and_route_targets`
  initially failed because the canonical `Event` kind was missing.
- The first isolated production-pipeline verification retained both conflicting
  callable targets but exposed an assertion mismatch: graph names were
  `firstHandler()` and `secondHandler()`, while the assertion omitted `()`.
  Commit `33ed9ef` corrected only that assertion.

## GREEN evidence

- `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/private/tmp/compass-task1-target-33ed9ef RUSTC_WRAPPER= cargo test -p compass-core --test code_graph_v1_determinism --locked`
  passed: 2 passed, 0 failed.
- `CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/private/tmp/compass-task1-target-33ed9ef RUSTC_WRAPPER= cargo test -p compass-resolve --tests --locked`
  passed: 58 passed, 0 failed across unit, domain, framework, native,
  PHP/Ruby/JVM, Python, and TypeScript route suites.
- `graphify update .` completed after the code changes.

## Design decisions

- Resolution state is derived from both confidence and candidate cardinality.
  A unique weak terminal-name candidate remains ambiguous; stable qualified or
  same-source evidence may become exact.
- Ordered route-stage details are additive to the wire model and include role,
  position, reference, resolution state, optional target, and candidates.
  Only exact stages publish authoritative `RoutesTo` edges.
- Stage targets and candidates are remapped to normalized stable node IDs, and
  exact stage details agree with their normalized edge targets.
- TypeScript import aliases are keyed by declaring source and local name while
  retaining module and exported-symbol identity.
- Domain facts retain canonical event, message, topic, queue, and job kinds;
  ORM relationships publish only from exact resolutions.
- File-convention route middleware is emitted as a portable typed component
  with the middleware role and convention provenance.
- Registration and relationship deduplication include source anchors, so
  distinct conflicting sites survive while identical facts remain deduplicated.

## Residual concerns

No Task 1 semantic gaps remain in the focused gates. The shared branch contained
independent work outside Task 1; verification was therefore run from a detached
worktree at `33ed9ef`, and unrelated dirty files were not staged or assessed.

## Commits

- Final Task 1 implementation commit: `a5200e3`
- Verification-assertion follow-up: `33ed9ef`
