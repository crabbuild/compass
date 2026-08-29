# Refinement log — C-010

## Iteration 1 — 2026-08-29

- Verified the latest published MCP revision through official Context7-backed
  Model Context Protocol documentation.
- Reframed the OpenSpec proposal, design, and normative scenarios around latest-
  protocol conformance while retaining exact named-client evidence.
- Updated the public implementation guide and KBD plan, execution gate, and
  decision log to distinguish an older-client incompatibility from a Compass
  conformance failure.
- Re-ran strict OpenSpec validation, native stdio conformance, the pinned official
  HTTP conformance suite, the product-boundary check, formatting, workspace
  Clippy, and workspace library/binary tests successfully.
- Syntax-validated the persisted artifact manifest and constraint record and
  checked that every declared source and distribution artifact exists and is
  non-empty.

Overall after deterministic refinement: PASS.

## Iteration 2 — review-packet correction

- The initial isolated review packet omitted the CLI, migration, and changelog
  files that implement and document the session-timeout deprecation path.
- Verified both the unit and real-binary regressions for warning text and exit
  taxonomy, plus the existing `MIGRATION.md` and `CHANGELOG.md` entries.
- Expanded the declared C-010 review surface so the reviewer receives those
  implementation and documentation hunks instead of inferring their absence.

Overall after refinement iteration 2: PASS; fresh-context re-review required.

## Iteration 3 — complete-surface review findings

- Removed a machine-specific absolute KBD skill path from the repository-owned
  execution handoff.
- Split stdio and HTTP into separately named CI conformance steps.
- Added exact ordered tool and resource inventory assertions to the native stdio
  conformance test and removed a stale nonexistent-fixture reference.
- Corrected the verification receipt to disclose the one baselined header-runner
  invalid-argument check precisely.
- Added spawned-server liveness checking to prevent a fixed-port readiness false
  positive and bounded every official runner subprocess to a configurable 1–1800
  second interval using a portable Python subprocess boundary.

Overall after refinement iteration 3: PASS; fresh-context re-review required.

## Iteration 4 — PASS-review follow-up

- Disclosed that the reference runner's identifier-only expected-failure format
  cannot distinguish its invalid-argument cause from a future header defect.
- Added a native regression that directly pins optional-whitespace normalization.
- Relabeled the execution ledger as a historical dispatch snapshot with the
  canonical live-progress pointer.
- Corrected the Codex reproduction wording and the HTTP-only runner's final
  success message.

Overall after refinement iteration 4: PASS.

## Iteration 5 — final-review hardening

- Made whitespace-only normalized MCP headers disappear so downstream validation
  treats them exactly like missing required headers.
- Moved HTTP Compass startup/readiness inside the interop harness cleanup scope
  and redirected server stderr to a bounded temporary-file lifecycle instead of
  an undrained pipe.
- Replaced optimization-sensitive Python assertions for subprocess pipes with
  explicit diagnostics.
- Disclosed identifier-only failure matching for every reference-runner baseline,
  and separated those unadvertised diagnostic gaps from native product claims.

Overall after refinement iteration 5: PASS.

## Isolated review before final hardening — 2026-08-29

- Rebuilt the isolated review packet with every declared source artifact,
  including untracked additions, through a temporary Git index.
- Fresh-context adversarial review returned PASS with zero critical findings,
  three warnings, and two suggestions.
- The remaining observations are non-blocking: two concern C-008-owned context
  outside this packet, and one recommends a future output bound for a local
  qualification subprocess.

That review pass was superseded by a later full-packet review after spec sync.

## Iteration 6 — protocol and receipt proof

- Configured the stdio client leg to use only the 2026-07-28 discovery
  lifecycle and assert the negotiated peer version before inventory or
  invocation checks.
- Made the Codex harness require an explicit expected version, verify the
  binary's exact `codex --version` output, and emit version, discovery,
  invocation, and overall PASS fields in its receipt.
- Added a successful end-to-end HTTP `graph_stats` request with optional
  whitespace around `Mcp-Name`.
- Cause-gated the reference runner's whitespace-header baseline to the reviewed
  HTTP 400 / JSON-RPC `-32602` invalid-argument result, so the identifier cannot
  mask a header mismatch.
- Re-ran focused Rust tests, the official HTTP runner, and strict OpenSpec
  validation successfully.

Overall after refinement iteration 6: PASS; fresh-context re-review required.

## Iteration 7 — latest-only production stdio

- A path-contaminated review nevertheless surfaced one valid policy gap:
  production stdio advertised legacy protocol revisions even though its test
  client selected 2026-07-28.
- Restricted both production transports to 2026-07-28 and made the removed
  `initialize` lifecycle fail explicitly on stdio as it already did on HTTP.
- Added a regression that proves a 2025-11-25 stdio initialize is rejected.
- Updated compatibility, migration, changelog, implementation, OpenSpec, and
  refinement records to disclose the intentional stdio cutover.
- Corrected packet assembly to use the OpenSpec change name, allowing the
  reviewer to consume `files.txt` instead of the repository's cumulative dirty
  diff.

Overall after refinement iteration 7: PASS; fresh-context re-review required.

## Iteration 8 — clean scoped final review

- Rebuilt the packet against the `mcp-conformance-interop` declared file set
  through a temporary Git index, excluding unrelated dirty-tree files from the
  diff under review.
- The external fresh-context judge completed through the REST gateway and
  returned PASS with zero critical findings, three warnings, and one suggestion.
- The strict sycophancy-correction gate passed with a score of 0.0.
- The remaining observations are non-blocking and recorded in the durable
  findings artifact: cumulative hunks inside shared compatibility documents,
  parity risk in the defensive host-header diagnostic helper, the prerequisite
  rmcp pin living in C-007, and a suggested explicit `curl` preflight for the
  qualification script.

Overall after refinement iteration 8: PASS; ready to archive.
