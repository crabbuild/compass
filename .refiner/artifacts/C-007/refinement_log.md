# Refinement log — C-007

## Iteration 1 — 2026-08-28

- Captured a passing rmcp 2.2 discovery golden before changing the dependency.
- Resolved and pinned rmcp exactly at 3.1.4.
- Adapted only the three compile-time API deltas: MRTR tool/resource response
  enums and the legacy-session configuration method rename.
- Preserved the stdio service lifecycle and current HTTP session selection.
- Added local and protocol-level `server/discover` golden comparisons.
- Measured the exact transitive delta and retained Apache-2.0 compatibility.
- Updated vulnerable `h2 0.4.15` and yanked `chacha20 0.10.1` to compatible
  patch releases so the repository dependency policy passes without exceptions.

Focused tests, all-target Clippy, workspace formatting/Clippy/lib-bin tests,
product contract, product boundary, cargo-deny, strict OpenSpec, diff checks,
and graph refresh pass. The installed artifact-refiner package lacks its
canonical controllers, schemas, and validator agent, so the documented
deterministic fallback was used.

Overall: PASS.
