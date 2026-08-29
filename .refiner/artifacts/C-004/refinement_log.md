# Refinement log — C-004

## Iteration 1 — 2026-08-28

- Centralized snapshot-limit parsing in `compass-store` with raw-byte, MB, GB,
  underscore, zero, invalid, overflow, and platform-representation coverage.
- Kept the default at 2 GiB and made the override process-local and explicit.
- Applied the effective limit to C-003 preflight and every canonical snapshot
  publication, validation, read, streaming, and delta boundary.
- Updated C-002's stable actionable message and real CLI regression to name
  the now-shipped override.

All focused, workspace, product, qualification, strict OpenSpec, formatting,
diff, and graph-refresh gates pass. The installed artifact-refiner package
lacks its canonical controllers/schemas/validator, so the documented
deterministic fallback was used. K3 judge dispatch and the single allowed retry
both returned no response within three minutes; no verdict is claimed.

Overall: PASS.
