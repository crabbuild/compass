# Refinement log — C-017

## Iteration 1 — 2026-08-28

- Added the six-command namespace, deterministic registry inventory, native MCP
  rendering, exact install delegation, offline doctor, atomic bundle export,
  and bounded validation.
- Added end-to-end evidence for help, schemas, install compatibility, export,
  validation, and healthy/stale/corrupt doctor states.
- Passed focused format, all-target/all-feature Clippy, CLI and installer tests,
  product gates, strict OpenSpec, workspace Clippy/tests, and graph refresh.

## Iteration 2 — 2026-08-28

- Corrected export JSON to return the exact versioned bundle manifest rather
  than a summary that reused the manifest schema identifier.
- Strengthened managed skill verification to compare every installed file and
  version marker with the current embedded seven-skill collection after first
  verifying ownership checksums.
- Confirmed focused unit tests remain green after refinement.

## Iteration 3 — 2026-08-28

- Hardened manifest ownership checks against forged self-consistent manifests,
  matched embedded assets by explicit prefix, and centralized the MCP protocol
  compatibility predicate.
- Made doctor snapshot-aware and scope-aware, distinguished source detection
  from manifest load failures, and added strict duplicate, conflict, and empty
  option diagnostics.
- Added global validation budgets, opened-file identity checks, nested secret
  detection, transport/config cross-checks, exact Compass MCP entry validation,
  and rejection of export-only platform filters during validation.
- Reworked export staging around a same-filesystem random temporary directory,
  preserving empty-directory permissions and retaining rollback diagnostics.
- Strengthened installer compatibility coverage to compare manifest bytes after
  replacing only the canonical root token.
- Passed the final focused and workspace Rust baselines, product boundary,
  strict OpenSpec validation, diff hygiene, graph refresh, and isolated K3
  adversarial review. The review result was PASS (0 critical, 2 warnings, 3
  suggestions), and every remaining non-blocking finding was resolved.

The installed artifact-refiner adapter lacks its referenced canonical
controllers, schemas, and validator. The repository's established deterministic
fallback record format was used and its JSON documents are syntax-validated.

Overall after refinement: PASS.
