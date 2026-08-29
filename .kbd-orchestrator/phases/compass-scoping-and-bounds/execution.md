EXECUTION: compass-scoping-and-bounds
Project: Compass
Date: 2026-08-28
Last updated: 2026-08-29
Selected backend: openspec
Dispatched to: SELF (Codex, through kbd-apply)
Backend rationale: The repository already uses OpenSpec, the phase requires traceable compatibility and decision records, and the KBD-owned apply wrapper preserves canonical task/change progress and hooks.
Backend entrypoint: the installed `kbd-apply` skill resolved through the active
Codex skill root; create and complete each named OpenSpec change, but drive every
implementation task through `kbd-apply begin-task/end-task`.
OpenSpec available: YES (CLI 1.10.0)
Source plan: .kbd-orchestrator/phases/compass-scoping-and-bounds/plan.md

EXECUTION SCOPE

- C-001 / stream-snapshot-read: Stream current snapshot reads and remove payload-sized production allocations.
- C-002 / actionable-snapshot-limit-error: Render a recoverable publication-limit failure.
- C-003 / preflight-graph-size-estimate: Detect predictable oversized graphs before full extraction.
- C-004 / honor-max-graph-bytes-on-publication: Apply the opt-in graph-byte override consistently after streaming is safe.
- C-005 / extract-compass-partition: Extract history-independent partition records and canonical helpers.
- C-006 / vendor-skip-policy: Record and test the default vendor discovery policy.
- C-007 / rmcp-3-migration: Pin and migrate to rmcp 3.1.4 with stdio parity.
- C-008 / mcp-stateless-http: Implement the MCP 2026-07-28 stateless HTTP contract.
- C-009 / mcp-result-envelope: Add versioned typed result envelopes to core navigation tools.
- C-010 / mcp-conformance-interop: Gate MCP migration on conformance and named-client interoperability.
- C-011 / surrealdb-license-decision: Prepare the artifact-profile decision and record user/legal sign-off.
- C-012 / surreal-persistent-probes: Run disposable persistent SurrealKV/RocksDB qualification probes.
- C-013 / qualification-corpora-baselines: Version deterministic corpora, baselines, and pre-ratified budgets.
- C-014 / surreal-graph-projection: Add an optional, generation-atomic Surreal graph projection.
- C-015 / surreal-dual-engine-equivalence: Prove bounded semantic equivalence across native and Surreal engines.
- C-016 / focused-agent-skills: Add six focused skills without changing the umbrella contract.
- C-017 / compass-agent-cli: Add the compatibility-safe compass agent namespace.
- C-018 / distribution-inventory-generators: Generate deterministic Codex, Claude, and OpenCode packages.
- C-019 / harness-install-validation: Test installed package lifecycle and ownership boundaries.
- C-020 / surreal-store-adapter: Conditionally implement a Surreal Store adapter only for a separately recorded user problem.

DISPATCH CONTRACTS

Every automated change uses this common contract: SELF executes the exact OpenSpec task surface through kbd-apply; `.kbd-orchestrator/phases/compass-scoping-and-bounds/progress.json` is the canonical projection; implementation completion is marked independently of evidence/certification/publication; QA, adversarial diff review, OpenSpec verification, spec sync, and archive follow in that order.

- C-001 -> SELF/OpenSpec. Entry: stream-snapshot-read. Model class: medium. Concrete model: current Codex gpt-5.6-sol session (the configured medium.local route is null, so this session supplies the required frontier fallback). Model rationale: public API compatibility plus corruption and publication-atomicity coverage in one crate.
- C-002 -> SELF/OpenSpec. Entry: actionable-snapshot-limit-error. Model class: medium. Concrete model: current Codex gpt-5.6-sol session. Model rationale: one domain/presentation boundary and a public CLI contract.
- C-003 -> SELF/OpenSpec. Entry: preflight-graph-size-estimate. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: calibrated performance design spanning discovery and publication with boundedness constraints.
- C-004 -> SELF/OpenSpec. Entry: honor-max-graph-bytes-on-publication. Model class: medium. Concrete model: current Codex gpt-5.6-sol session. Model rationale: coordinated environment-limit behavior across graph and store boundaries.
- C-005 -> SELF/OpenSpec. Entry: extract-compass-partition. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: a new public crate, canonical encoding, immutable history, and transitive dependency constraints.
- C-006 -> SELF/OpenSpec. Entry: vendor-skip-policy. Model class: small. Concrete model: Qwen3.5-9B-Q8_0 per project registry, executed in this Codex session. Model rationale: bounded discovery-policy decision with direct tests.
- C-007 -> SELF/OpenSpec. Entry: rmcp-3-migration. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: high-risk SDK migration, dependency policy, and protocol compatibility.
- C-008 -> SELF/OpenSpec. Entry: mcp-stateless-http. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: transport/security/CLI migration across multiple public boundaries.
- C-009 -> SELF/OpenSpec. Entry: mcp-result-envelope. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: versioned machine contract spanning every core navigation result property.
- C-010 -> SELF/OpenSpec. Entry: mcp-conformance-interop. Model class: medium. Concrete model: current Codex gpt-5.6-sol session. Model rationale: focused test/CI surface with external-client evidence.
- C-011 -> SELF drafting plus MANUAL user/legal approval. Entry: surrealdb-license-decision. Model class: frontier for the artifact draft; decision authority: user/legal. Concrete model: current Codex gpt-5.6-sol session for research and drafting only. Model rationale: high-consequence licensing/release decision that implementation cannot make.
- C-012 -> SELF/OpenSpec. Entry: surreal-persistent-probes. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: disposable multi-engine durability and resource qualification with kill/recovery evidence.
- C-013 -> SELF/OpenSpec. Entry: qualification-corpora-baselines. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: cross-surface deterministic fixture design and anti-bias budget ratification.
- C-014 -> SELF/OpenSpec. Entry: surreal-graph-projection. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: new optional persistence abstraction and transactionally published projection.
- C-015 -> SELF/OpenSpec. Entry: surreal-dual-engine-equivalence. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: dual-engine routing and exact semantic equivalence at scale.
- C-016 -> SELF/OpenSpec. Entry: focused-agent-skills. Model class: medium. Concrete model: current Codex gpt-5.6-sol session. Model rationale: additive installer artifacts and trigger-discrimination tests across one CLI asset boundary.
- C-017 -> SELF/OpenSpec. Entry: compass-agent-cli. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: six public subcommands, compatibility aliasing, validation, and documentation.
- C-018 -> SELF/OpenSpec. Entry: distribution-inventory-generators. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: deterministic cross-harness package generation spanning Rust and TypeScript.
- C-019 -> SELF/OpenSpec. Entry: harness-install-validation. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: external harness lifecycle tests, ownership safety, and CI integration.
- C-020 -> SELF/OpenSpec only if its condition precedent is recorded. Entry: surreal-store-adapter. Model class: frontier. Concrete model: current Codex gpt-5.6-sol session. Model rationale: optional async database adapter behind an unchanged synchronous contract.

APPROVAL GATES

- C-001 implementation complete before C-004 or C-005 begins.
- C-005 stops if compass-partition would depend on prolly-map, prolly-store-sqlite, compass-ir, or compass-analysis.
- C-010 must prove the latest published MCP revision through both conformance legs and record exact Codex CLI, Claude Code, and OpenCode outcomes; an older-client incompatibility does not require a legacy server fallback (user decision, 2026-08-29).
- C-011 was accepted by the project user on 2026-08-29 for every named artifact profile under the pinned 3.2.4 BSL conditions, passed mandatory pre-archive review, and is published in the dated archive; with C-012 and C-013 complete, the Wave 5 gate is satisfied.
- C-013 budget ratification must predate any Wave 5 Surreal measurement.
- C-020 is cancelled unless a distinct user problem is recorded by the end of Wave 7.

FALLBACK CONDITIONS

- A missing or structurally incomplete OpenSpec change is completed through the OpenSpec artifact workflow before kbd-apply starts.
- A backend/API mismatch that prevents inspectable task progress falls back to native KBD task storage without weakening the plan acceptance criteria.
- Surreal license rejection or persistent-probe failure cancels Waves 5 and 8 and preserves the existing SQLite/redb/graph.json stack.
- Any failed blocking constraint, OpenSpec verification, artifact-refiner check, or CRITICAL adversarial finding blocks certification until fixed and both reviews rerun.

VERIFICATION REQUIREMENTS

- Per owner crate: cargo test -p <crate> --locked and applicable integration/contract gates.
- Every Rust change: cargo fmt --all -- --check; cargo clippy --workspace --lib --bins --locked -- -D warnings; cargo test --workspace --lib --bins --locked.
- Public CLI/product changes: cargo test -p compass-cli --test compass_product --locked and sh scripts/check_product_boundary.sh.
- Code-graph publication changes: ./scripts/qualify_code_graph_v1.sh --fixtures-only.
- JavaScript/package changes: npm ci; npm run typecheck:js; npm run test:js; node scripts/check_viewer_assets.mjs.
- Dependency changes: cargo deny check plus license and transitive dependency evidence.
- Refresh the local Compass graph after code changes and report any failed refresh.

INITIAL DISPATCH LEDGER (historical snapshot)

The statuses below record the 2026-08-28 dispatch starting point. Canonical live
status is `.kbd-orchestrator/phases/compass-scoping-and-bounds/progress.json`;
this execution contract is not rewritten for every task transition.

- [IN_PROGRESS] C-001 — SELF/OpenSpec (verified starting draft)
- [PENDING] C-002 — SELF/OpenSpec
- [PENDING] C-003 — SELF/OpenSpec
- [PENDING] C-004 — SELF/OpenSpec, gated by C-001
- [PENDING] C-005 — SELF/OpenSpec, gated by C-001 and dependency rule
- [PENDING] C-006 — SELF/OpenSpec
- [PENDING] C-007 — SELF/OpenSpec
- [PENDING] C-008 — SELF/OpenSpec, gated by C-007
- [PENDING] C-009 — SELF/OpenSpec, gated by C-007
- [PENDING] C-010 — SELF/OpenSpec, gated by C-008 and C-009
- [PENDING] C-011 — draft by SELF, approval MANUAL
- [PENDING] C-012 — SELF/OpenSpec
- [PENDING] C-013 — SELF/OpenSpec
- [PENDING] C-014 — SELF/OpenSpec, gated by C-011/C-012/C-013
- [PENDING] C-015 — SELF/OpenSpec, gated by C-014
- [PENDING] C-016 — SELF/OpenSpec
- [PENDING] C-017 — SELF/OpenSpec, gated by C-016
- [PENDING] C-018 — SELF/OpenSpec, gated by C-010/C-016/C-017
- [PENDING] C-019 — SELF/OpenSpec, gated by C-018
- [PENDING] C-020 — CONDITIONAL, gated by C-011/C-014 and independent user problem

OUTPUTS

- OpenSpec proposal/design/spec/tasks and archived change for each completed change.
- Per-change `.refiner/artifacts/<change-id>/refinement_log.md`.
- Per-change `.kbd-orchestrator/phases/compass-scoping-and-bounds/review/<change-id>/findings.json`.
- Exact build/test/conformance/measurement evidence called for by plan.md.
- Updated Compass graph plus KBD reflection handoff.

BLOCKERS

- C-020 has no recorded independent user problem at execution start and is therefore conditional, not assumed.

REFLECTION HANDOFF

- Consume execution.md, progress.json, archived OpenSpec changes/specs, per-change refinement logs and adversarial findings, exact verification output, decision-log entries, measurement artifacts, and the refreshed Compass graph.

EXECUTION READY
