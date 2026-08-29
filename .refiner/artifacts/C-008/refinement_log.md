# Refinement log — C-008

## Iteration 1 — 2026-08-28

- Made Streamable HTTP stateless and advertised only MCP 2026-07-28 on HTTP.
- Required exact `Mcp-Protocol-Version`, `Mcp-Method`, and rmcp per-request
  metadata before handler dispatch.
- Removed the stateful SSE-to-JSON adapter and all HTTP session keep-alive use.
- Preserved stdio protocol compatibility through a transport-specific protocol
  profile.
- Proved raw-wire discovery and `tools/list` work independently without
  `Mcp-Session-Id`, while authentication and host validation remain enforced.
- Rejected legacy or incomplete HTTP requests with bounded typed JSON-RPC
  errors, including the old initialize/session path.
- Kept `--stateless` as a compatibility spelling and deprecated
  `--session-timeout` through 0.4.x with removal in 0.5.0.
- Added unit and executable-level regressions for warning text and unchanged
  usage/runtime exit codes.

Focused tests, all-target Clippy, workspace formatting/Clippy/lib-bin tests,
product contract, product boundary, strict OpenSpec, diff checks, and graph
refresh pass. The installed artifact-refiner package lacks its canonical
controllers, schemas, and validator agent, so the documented deterministic
fallback was used.

Overall: PASS.

## Iteration 2 — 2026-08-28

- Repaired the first adversarial review's two major findings.
- Restored the explicit 64 MiB HTTP response boundary with a streaming body
  limiter. Known oversized bodies return the distinct typed JSON-RPC error
  `-32023`; unknown-length bodies remain streaming but fail when the bound is
  crossed.
- Expanded the raw-wire regression to assert typed failures for absent and
  incomplete request metadata, a mismatched `Mcp-Method`, and the explicitly
  rejected current-version `initialize` path.
- Proved that a valid current request carrying an obsolete `Mcp-Session-Id`
  succeeds independently and receives no session identifier.
- Re-ran all focused checks, workspace formatting/Clippy/lib-bin tests, product
  contract, product boundary, strict OpenSpec validation, diff checks, and the
  Compass graph refresh.

Overall after adversarial repair: PASS.

## Iteration 3 — 2026-08-28

- Moved allowed-host enforcement into the outer authenticated gate before any
  protocol diagnostic can return, while retaining rmcp's inner defense in
  depth.
- Added authenticated raw-wire cases proving an untrusted host receives HTTP
  403 even when the protocol version is missing or obsolete.
- Replaced the inferred streaming-limit check with an unknown-length,
  multi-chunk channel body. The test observes the first in-bound chunk and an
  explicit `LengthLimitError` instead of the overflowing chunk.
- Corrected the isolated review baseline so C-007's `sha2` discovery-golden
  dependency does not appear as C-008 scope.
- Re-ran focused tests and Clippy, the full native baseline, product gates,
  strict OpenSpec validation, diff checks, and graph refresh.

Overall after second adversarial repair: PASS.

## Iteration 4 — 2026-08-28

- Made the accepted rmcp host-equivalence and obsolete-session semantics
  explicit in source comments, tests, OpenSpec, and migration guidance.
- Added invalid-auth/host, missing Host, malformed Host, and portless-allowlist
  raw-wire coverage.
- Scoped the `http-body-util` channel feature to the test dependency and removed
  the final Unreleased changelog ambiguity.
- The fourth fresh-context packet review returned zero findings across all
  checked classes; the strict anti-theater gate accepted its detailed evidence
  trail with score 0.0.
- The final focused and workspace/product baselines, strict OpenSpec validation,
  diff check, and graph refresh all pass.

Final overall result: PASS.
