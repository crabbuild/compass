# Refinement log — C-006

## Iteration 1 — 2026-08-28

- Compared skip, skip-unless-workspace-member, and leave policies against the
  language-neutral discovery boundary.
- Retained current behavior because Go vendor trees are build-relevant source
  and workspace membership cannot describe every supported ecosystem.
- Documented the explicit `.compassignore`, saved-scope, and `--exclude`
  controls for repositories that want vendor source omitted.
- Added one regression covering Rust workspace-member source, non-member Go
  vendor source, deterministic discovery, and watcher parity.

Focused tests, crate tests, crate lib Clippy, workspace formatting/Clippy/
lib-bin tests, product boundary, strict OpenSpec, diff checks, and graph refresh
pass. The installed artifact-refiner package lacks its canonical controllers,
schemas, and validator agent, so the documented deterministic fallback was used.
The attempted all-target crate Clippy run exposed two unrelated pre-existing
test-target lints; neither is in the added regression.

Overall: PASS.

## Iteration 2 — 2026-08-28

The first fresh-context review correctly blocked on two fixture gaps. Made the
declared workspace member structurally valid with its own Cargo manifest and
added vendor-specific assertions for `.compassignore`, command-line-style extra
exclusions, saved project scope, and watcher rejection. Focused QA and review
were rerun after the correction.

## Iteration 3 — 2026-08-28

The second fresh-context review found that discovery eligibility alone did not
prove the proposal's classification/publication claim. Added a `compass-core`
build contract that publishes Rust workspace-member and Go vendor source into
the graph by default and proves `.compassignore`, CLI-style extra excludes, and
saved project scope each remove the matching file inventory and anchored nodes.
