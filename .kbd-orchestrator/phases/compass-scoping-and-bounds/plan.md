# Plan — compass-scoping-and-bounds

**Stage:** Plan (re-run 2026-08-28; supersedes 2026-08-09 plan — Waves 1–2 carried
forward unchanged, Waves 3–8 added for the expanded scope in `goals.md`
§"Expanded scope (2026-08-28, user decision)")
**Backend:** OpenSpec (`openspec/` present, `openspec_available: true`)
**Evolver cycle:** none
**Changes:** 20 — 0 complete (6 original + 14 expanded)
**Mode assumption:** fork-local implementation (assessment §2 reading (a)).
Upstream submission follows `docs/future/upstream-contribution-plan.md` as exit
work after waves complete; it is not a numbered change because its schedule
belongs to upstream maintainers.

Reads `assessment.md` (2026-08-28 re-run), `analysis.md`, `spec.md`,
`library-candidates.json`, `handoffs/{analyze,spec,assess}.md`,
`docs/future/*.md`.

---

## Settled architecture decision (precondition to all implementation)

The phase's open architecture question — *partitioning/sharding vs a larger
number* — is **settled** (assessment §1 "The phase's architecture question —
settled verdict"; decision log 2026-08-09 entries "Sharding rejected",
"Partitioning already exists", "Read-path allocation identified as the true
constraint"): **neither.** Sharding is rejected (multi-machine technique;
eventual consistency violates the determinism invariant). A larger default is
rejected (explicit non-goal; AGENTS.md bounded work). Record-class partitioning
already ships in `compass-history::PartitionedGraph` (`cand-001`); the fix is
eliminating the monolithic read-path allocation (C-001, `cand-003`) and reusing
the existing partitioning (C-005). No implementation work in any wave begins
under an unsettled architecture question.

## Ordering rationale

```
Wave 1 (parallel)          Wave 2 (after C-001)      Wave 3 (MCP 2026)
├── C-001  read_snapshot   ├── C-004  env override   ├── C-007  rmcp 3.1.4 upgrade
├── C-002  error message   └── C-005  compass-       ├── C-008  stateless HTTP
├── C-003  size estimate           partition         ├── C-009  result envelope
└── C-006  vendor/ policy                            └── C-010  conformance+interop

Wave 4 (decision gates)    Wave 5 (Surreal graph)    Wave 6 (skills+CLI)
├── C-011  BSL decision    ├── C-014  projection     ├── C-016  skills split
├── C-012  persistent          crate [HARD GATE:     └── C-017  compass agent CLI
│          probes              C-011+C-012+C-013]
└── C-013  corpora +       └── C-015  equivalence
           budgets                 corpus

Wave 7 (distribution)      Wave 8 (optional)
├── C-018  inventory +     └── C-020  store adapter  [CONDITIONAL]
│          generators
└── C-019  install tests
```

- **Waves 1–2 first, unchanged.** The original defects are small, shippable, and
  C-002/C-004 settle the error-text contract **before** Wave 3's result-envelope
  work touches the same surfaces — otherwise that contract is rewritten twice
  (assessment §8, carried WARNING).
- **Wave 3 before Wave 4.** MCP migration is urgent (the shipped session-based
  contract is legacy per spec 2026-07-28), self-contained, and independent of
  the SurrealDB license gate.
- **Wave 4 before any Surreal code merges.** Cheap disqualifiers first: if the
  BSL decision or persistent probes fail, Waves 5 and 8 are cancelled with zero
  stranded code (assessment §7, §8).
- **Wave 6 has no dependency on Waves 4–5** and may run in parallel with them if
  execution capacity allows; it is ordered after for ledger clarity only.
- **Wave 7 after Waves 3 and 6:** packages distribute the migrated MCP contract
  and the split skills; generating them earlier packages contracts that are
  about to change.
- **Wave 8 last and conditional** — the research doc's own rule: no Store
  adapter unless an independent user problem justifies it.

**Apply C-001 first** — with the verify-first protocol from adversarial review
r2 (assessment §10): before absorbing the uncommitted diff, re-run
`git diff crates/compass-store/src/lib.rs` and
`cargo check -p compass-store --locked`; if absent or broken, C-001 reverts to
plain pending and is implemented from spec.

---

## Wave 1

### C-001 — Stop materializing the snapshot payload in `read_snapshot`

- **OpenSpec:** `/opsx:new stream-snapshot-read`
- **Owner:** `compass-store` · **Agent:** claude · **Risk:** medium
- **Library:** `cand-003` (build — no external library; verdict `adopt`)
- **Compatibility-sensitive:** yes (`read_snapshot` is `pub`)
- **Verify-first (r2 finding): ✅ PASSED 2026-08-28 at handoff.** Diff present
  (+152/−17); `read_snapshot_manifest` (`lib.rs:837`) and `read_snapshot_chunks`
  (`lib.rs:856`) exist alongside the retained `read_snapshot` (`lib.rs:901`);
  `cargo check -p compass-store --locked` finished clean in 27.95 s. Treat the
  diff as a **starting draft**: still to do is reimplementing `validate_snapshot`
  and `snapshot_reference` on the manifest reader (verify at the call sites),
  plus all tests and changelog work below.

Add `read_snapshot_manifest()`; reimplement `validate_snapshot` (`lib.rs:935`) and
`snapshot_reference` (`lib.rs:1184`) on it. Replace the
`Vec::with_capacity(payload_bytes)` concatenation (`lib.rs:838-853`) with a streaming
chunk form. Keep the per-chunk running total and `MAX_GRAPH_BYTES` check.

**Done when**
- [ ] No production path allocates proportional to `payload_bytes`
- [ ] `validate_snapshot` / `snapshot_reference` return identical results for valid **and**
      corrupt snapshots
- [ ] Round-trip, reopen, corruption/interruption, publication-atomicity tests
- [ ] `lib.rs:44-51` doc comment satisfied or corrected — not left as is
- [ ] `CHANGELOG.md`; `MIGRATION.md` only if the public signature changes

### C-002 — Make the snapshot limit error actionable

- **OpenSpec:** `/opsx:new actionable-snapshot-limit-error`
- **Owner:** `compass-graph` (message), `compass-cli` (presentation) · **Agent:** claude
- **Risk:** low · **Compatibility-sensitive:** yes (error text)

Follow the precedent at `compass-core/src/diagnostics.rs:453`. Name `--exclude` /
`.compassignore`. **Do not mention `COMPASS_MAX_GRAPH_BYTES` until C-004 ships.**

**Done when**
- [ ] Error names at least one concrete next action
- [ ] CLI test under `crates/compass-cli/tests/` asserts rendered text and exit code
- [ ] No override advertised that does not exist on this path

### C-003 — Pre-flight graph size estimate

- **OpenSpec:** `/opsx:new preflight-graph-size-estimate`
- **Owner:** `compass-core` · **Agent:** claude · **Risk:** medium

The reported failure burned 331.90 s then discarded everything. Estimate before publication.

**Done when**
- [ ] First task records a baseline: full-failure wall time and payload
      composition on an oversized fixture (the 331.90 s case or a synthetic
      equivalent)
- [ ] Regression test: on that fixture, the limit error is reported during
      discovery/estimation — before per-file extraction of the corpus completes —
      and time-to-error is asserted under a recorded fraction of the baseline
      (target ≤ 20%; the exact number is fixed by the baseline task and pinned
      in the test, not invented here)
- [ ] Estimate deterministic for equivalent inputs
- [ ] Limit outcome stays distinct from an empty result, and the error carries
      the C-002 remediation text (recoverability, G3)
- [ ] Estimation itself bounded

> **Measure first.** Payload composition is unmeasured — which node/edge classes dominate
> the 2 GiB is unknown. Start by measuring against a real oversized repo; do not invent a
> heuristic.

### C-006 — Decide `vendor/` default-skip policy

- **OpenSpec:** `/opsx:new vendor-skip-policy`
- **Owner:** `compass-files` · **Agent:** claude · **Risk:** low
- **Compatibility-sensitive:** yes (discovery scope)

`vendor/` is absent from `SKIP_DIRS` (`detect.rs:61-102`). Go vendor dirs hold real source,
and `vendor/compass-tree-sitter-language-pack` is a workspace member of this repo.

**Done when**
- [ ] Deliberate documented decision: skip / skip-unless-workspace-member / leave
- [ ] Test pins the chosen behavior
- [ ] Compass's own `vendor/` workspace member still discovered if that is the decision

---

## Wave 2 — gated on C-001

### C-004 — Honor `COMPASS_MAX_GRAPH_BYTES` on the publication path

- **OpenSpec:** `/opsx:new honor-max-graph-bytes-on-publication`
- **Owner:** `compass-graph`, `compass-store` · **Agent:** claude · **Risk:** medium
- **Depends on:** C-001 — **HARD GATE**

> **Do not start before C-001 is merged.** An override before the read path streams lets a
> user request a multi-gigabyte contiguous `Vec` — strictly worse than today's clean error.

Resolve the inconsistency: honored in `compass-model` (`graph.rs:370`), `compass-output`
(`json.rs:414`), `compass-global` (`lib.rs:370`), `compass-core` (`diagnostics.rs:713`);
not in `compass-graph/snapshot.rs` or `compass-store`.

**Done when**
- [ ] Consistent with the four crates that honor it, or a documented exception
- [ ] Default stays 2 GiB (no default increase — AGENTS.md bounded work)
- [ ] Override explicit and opt-in
- [ ] C-002's message updated to advertise the override **within this change** —
      C-004 is not done while the error text omits it (closes the coupling
      WARNING carried since 2026-08-09; no separate follow-up change exists)

### C-005 — Extract `compass-partition` shared crate

- **OpenSpec:** `/opsx:new extract-compass-partition`
- **Owner:** new crate `compass-partition` · **Agent:** claude · **Risk:** medium
- **Depends on:** C-001 · **Library:** `cand-001` (adopt — in-repo, `compass-history`)

**Moves:** `PartitionedGraph` (`artifacts.rs:76-84`), `node_key`/`edge_key`/`hyperedge_key`
(`keys.rs:10,20,46`), `canonical_json_bytes` (`canonical.rs:9`).

**Does not move:** `into_partition` and all history-coupled logic — `CompletionEvidence`,
`authoritative_sidecars`, `AnalysisBundle`, `compass_ir::ProgramBundle`, `HistoryError`,
`prolly::{KeyBuilder, VersionedValue}`.

> **Blocking dependency rule.** `compass-partition` must NOT depend on `prolly-map`,
> `prolly-store-sqlite`, `compass-ir`, or `compass-analysis`. `compass-graph` depends on
> none of these today. **If this cannot be satisfied, STOP and report** — it means the
> shared-crate framing does not hold and the decision needs revisiting.

**Done when**
- [ ] `compass-history` behavior byte-identical; existing canonical-encoding, round-trip,
      and diff tests pass **unchanged**
- [ ] Workspace member added to root `Cargo.toml`; deps via `{name}.workspace = true`
- [ ] Workspace lint policy applies (no `unsafe`, no `unwrap`/`expect`/`panic`)
- [ ] No new transitive dependency reaches `compass-graph`
- [ ] Own error type; `compass-history` converts at its boundary

**Scope note (carried from spec):** this extracts a struct and four helpers from a
2,317-line module. It positions for current-graph partitioned publication; it does not
deliver it. That is a further change requiring the identity decision below.

---

## Wave 3 — MCP 2026 migration (E1)

### C-007 — Upgrade `rmcp` 2.2.0 → 3.1.4 with stdio parity

- **OpenSpec:** `/opsx:new rmcp-3-migration`
- **Owner:** workspace `Cargo.toml`, `compass-mcp` · **Agent:** claude · **Risk:** high
- **Compatibility-sensitive:** yes (MCP schemas, CLI flags)

Dedicated dependency-migration change on its own branch. Document the 2.2.0→3.1.4
API and transitive-dependency delta; confirm license, `deny.toml`, unsafe-code
policy; port the server to the 3.x API with the **stdio** transport passing first.
No SurrealDB work, no new tools, no envelope redesign in this change.

**Done when**
- [ ] Workspace pins `rmcp = "=3.1.4"` (the `=` operator — a bare `"3.1.4"` is a
      caret requirement Cargo may float to 3.x.y; the pin must be exact) in root
      `Cargo.toml`, members via `{name}.workspace = true`
- [ ] API/transitive delta documented in the change's design doc
- [ ] `cargo deny` and workspace lint gates pass
- [ ] All existing stdio-transport tests pass (adapted to the 3.x API where the
      old API is gone, with each adaptation noted)
- [ ] Existing tool list, names, and input schemas unchanged (`server/discover`
      parity test against the 2.2.0 golden output)

### C-008 — Stateless HTTP transport per MCP 2026-07-28

- **OpenSpec:** `/opsx:new mcp-stateless-http`
- **Owner:** `compass-mcp`, `compass-cli` (flags) · **Agent:** claude · **Risk:** high
- **Depends on:** C-007
- **Compatibility-sensitive:** yes (CLI flags `--session-timeout`, HTTP contract)

Replace the session model: `server/discover`, stateless request metadata,
`Mcp-Protocol-Version`/`Mcp-Method` headers; retire `Mcp-Session-Id`. Deprecate
`--session-timeout` per COMPATIBILITY.md process (flag accepted with warning for
one minor release, documented removal date). Decide and record whether a legacy
MCP-2025 mode ships; if yes, name its exact cost and removal date; if no, record
that in MIGRATION.md.

**Done when**
- [ ] HTTP transport passes MCP 2026-07-28 conformance (stateless discovery, no
      session handshake)
- [ ] `--session-timeout` deprecation path implemented + tested (warning text,
      exit codes unchanged)
- [ ] Legacy-mode decision recorded either way; if kept, dual contract tests
- [ ] Bearer-token auth, request/response limits, and multi-project selection
      behavior preserved (existing security tests pass)
- [ ] `COMPATIBILITY.md` + `MIGRATION.md` + `CHANGELOG.md` updated

### C-009 — Common result envelope + typed results on core navigation tools

- **OpenSpec:** `/opsx:new mcp-result-envelope`
- **Owner:** `compass-mcp` · **Agent:** claude · **Risk:** medium
- **Depends on:** C-007 (3.x `resultType`/output-schema support)
- **Compatibility-sensitive:** yes (MCP result schemas)

Introduce the versioned envelope (`resultType: "complete"`, separate
`schema: compass.<x>.v1` field, repository/generation/freshness/evidence/
confidence/truncation/warnings) on `search_symbols`, `get_callers`,
`get_callees`, `get_impact` first. Deterministic tool ordering in discovery.
Legacy text-only tools either adopt the envelope or are marked deprecated —
recorded per tool. Do not rename tools in this change.

**Done when**
- [ ] Envelope schema documented and versioned; golden result fixtures for the
      four tools
- [ ] Evidence, direction, multiplicity, ambiguity, bounds, pagination, and
      deterministic ordering preserved (regression tests against pre-envelope
      golden outputs)
- [ ] `resultType` is the MCP discriminator; Compass schema name carried in a
      separate field (research-doc rule)
- [ ] Every remaining tool's envelope/deprecation status recorded in the design doc

### C-010 — Conformance + named-harness interop matrix

- **OpenSpec:** `/opsx:new mcp-conformance-interop`
- **Owner:** `compass-mcp` tests, CI · **Agent:** claude · **Risk:** medium
- **Depends on:** C-008, C-009

Reference conformance on stdio and HTTP; interop against named, pinned versions
of Codex CLI, Claude Code, and OpenCode (record the exact versions tested).
**Merge gate for the wave:** if the clients do not interoperate, Wave 3 does not
merge merely because the Rust compiles (research-doc rule).

**Done when**
- [ ] Conformance suite in CI for both transports
- [ ] Interop matrix recorded with exact client versions and outcomes
- [ ] Failures either fixed or the wave is explicitly held — no partial merge

---

## Wave 4 — decision gates + measurement prerequisites (E2, E7)

> **Purpose:** cheap disqualifiers before any SurrealDB code merges. C-011 and
> C-013 are decision/artifact changes, not code changes; C-012 is throwaway code.

### C-011 — SurrealDB BSL 1.1 license/release decision record

- **OpenSpec:** `/opsx:new surrealdb-license-decision`
- **Owner:** decision record (docs) · **Agent:** claude drafts; **user/legal decides**
- **Risk:** low effort, high consequence · **BLOCKER for Waves 5 & 8**

Draft the exact distributed-artifact profile for decision: BSL 1.1 core,
Additional Use Grant (DBaaS restriction), change date/license, notices, package
registries, plugin archives, downstream redistribution; remote-client-only
fallback packaging if embedded core is rejected. The decision itself is made by
the user (or their legal reviewer) — this change is complete when the decision
is **recorded**, either way.

**Done when**
- [ ] Decision document states accept/reject/conditional per artifact profile
- [ ] Signed off by the user (recorded in the decision log with provenance)
- [ ] Rejection path documented (remote-client-only or cancel Waves 5/8)

### C-012 — Persistent SurrealKV + RocksDB throwaway probes

- **OpenSpec:** `/opsx:new surreal-persistent-probes`
- **Owner:** disposable evaluation (not a workspace member) · **Agent:** claude
- **Risk:** medium · **BLOCKER for Wave 5**

Probe persistent engines (not Mem-only — the research doc's own r2 correction):
parallel directed relations with stable IDs; provenance/confidence round-trip;
generation-scoped reads + atomic activation; **kill-during-write recovery**;
deterministic ordering/pagination at scale; dependency/build-time/binary-size/
cold-start/peak-RSS measurements; exact license text capture.

**Done when**
- [ ] All probe dimensions have recorded pass/fail results + test vectors
- [ ] Spike code deleted; results + vectors retained as evaluation artifacts
- [ ] No production dependency left in the workspace (verify `Cargo.lock` clean)
- [ ] Fail → Waves 5/8 cancelled with the fallback recorded (existing stack)

### C-013 — Measurement corpora, baselines, budget ratification

- **OpenSpec:** `/opsx:new qualification-corpora-baselines`
- **Owner:** fixtures + benchmark harness · **Agent:** claude · **Risk:** medium
- **BLOCKER for Wave 5 acceptance**

Create and version: semantic corpus; `qualification-medium` (100k/250k) and
`qualification-large` (1M/2.5M) fixture generators; bounded raw-traversal
baseline; 30-task suite; umbrella-skill invocation corpus; focused-skill
boundary prompts. Record current-engine baselines (binary size, cold start,
query latency p95, peak RSS). **Ratify the numeric budgets** from the research
doc (or amend them by recorded decision) **before** any Surreal measurements
are visible.

**Done when**
- [ ] All corpora/fixtures/harnesses checked in, versioned, deterministic
- [ ] Current-engine baselines recorded and reproducible
- [ ] Budget table ratified by recorded decision, dated prior to Wave 5 runs

---

## Wave 5 — Surreal graph projection (E3) — HARD GATE: C-011 accept + C-012 pass + C-013 done

### C-014 — `compass-graphdb-surreal` projection crate

- **OpenSpec:** `/opsx:new surreal-graph-projection`
- **Owner:** new optional crate `compass-graphdb-surreal` · **Agent:** claude · **Risk:** high
- **Depends on:** C-011, C-012, C-013 — **HARD GATE**; precedent `compass-graphdb`

Schemafull node/relation tables; closed mapping from Compass edge kinds to typed
relation families with required `kind`; generation-keyed records
(`repository_id`, `generation_id`, schema version, stable IDs, provenance,
confidence); transactionally staged projection with `active_generation` pointer
swap after validation. SurrealQL through typed builders only. Non-default Cargo
feature; engines Mem (test) / SurrealKV / RocksDB per C-012 results.

**Done when**
- [ ] Projection derives from one immutable generation; no partial generation
      ever visible (interrupted-write test)
- [ ] Parallel edges, direction, self-loops, provenance, confidence survive
      round-trip (property + corpus tests)
- [ ] No SurrealDB dependency reaches default builds (footprint gate: zero
      Surreal-attributable size delta with feature off)
- [ ] No MCP/CLI presentation logic in the crate

### C-015 — Bounded native reads + dual-engine equivalence

- **OpenSpec:** `/opsx:new surreal-dual-engine-equivalence`
- **Owner:** `compass-graphdb-surreal`, `compass-query` (engine route) · **Agent:** claude
- **Risk:** high · **Depends on:** C-014

Implement `symbol_context`/`impact`/`path`/`subgraph`-class reads through both
engines; equivalence corpus from C-013 plus deterministic scale samples.

**Done when**
- [ ] **Zero semantic mismatches** — identity, direction, multiplicity,
      provenance, bounds, ordering, pagination (a mismatch is a failure, not an
      exception)
- [ ] Budgets from C-013 evaluated and results recorded against the ratified
      thresholds; failures trigger the falsifier protocol (revise or stop)
- [ ] No raw model-authored SurrealQL execution path exposed

---

## Wave 6 — skills split + skill-aware CLI (E5) — parallelizable with Waves 4–5

### C-016 — Focused Agent Skills, umbrella preserved

- **OpenSpec:** `/opsx:new focused-agent-skills`
- **Owner:** `compass-cli` assets · **Agent:** claude · **Risk:** low
- **Compatibility-sensitive:** yes (installed skill files)

Six focused skills (`compass-navigate`, `compass-debug`, `compass-change-impact`,
`compass-architecture`, `compass-index-maintenance`, `compass-mcp-setup`) as
additive artifacts; umbrella `compass` skill unchanged and canonical.

**Done when**
- [ ] Spec-conformant skills (lower-kebab names matching directories; complete
      bundled trees; no absolute paths)
- [ ] Trigger-discrimination tests: umbrella invocation corpus shows zero
      regressions; boundary prompts select correctly (corpus from C-013 if built,
      else created here)
- [ ] Installer places complete trees idempotently (checksum tests)

### C-017 — `compass agent` CLI namespace

- **OpenSpec:** `/opsx:new compass-agent-cli`
- **Owner:** `compass-cli` · **Agent:** claude · **Risk:** medium
- **Depends on:** C-016 · **Compatibility-sensitive:** yes (CLI contract)

`compass agent list|install|doctor|export|validate|mcp-config` with
`compass install` preserved as compatibility alias. Command names pass normal
CLI contract review; `doctor` checks binary/graph/protocol/skill-checksum/MCP
config; `export` deterministic; `validate` rejects absolute paths/credentials.

**Done when**
- [ ] All six subcommands implemented with `--help`, exit codes, tests
- [ ] `compass install` alias behavior byte-compatible (regression test)
- [ ] `doctor`/`validate` negative tests (stale graph, bad checksum, absolute
      path, credential string)
- [ ] Reference docs + `CHANGELOG.md`

---

## Wave 7 — native harness distribution (E6) — after Waves 3 & 6

### C-018 — Canonical distribution inventory + package generators

- **OpenSpec:** `/opsx:new distribution-inventory-generators`
- **Owner:** new distribution surface (crate or `compass-cli` module) · **Agent:** claude
- **Risk:** medium · **Depends on:** C-010 (packages must carry the migrated MCP
  contract — packaging the pre-2026 contract is rework by construction), C-016,
  C-017

One `distribution.toml` inventory → generated Codex package
(`.codex-plugin/plugin.json`, skills, `.mcp.json`; repo marketplace manifest
optional pending a maintainer/distribution decision), Claude package
(`.claude-plugin/plugin.json`, `.mcp.json`, skills) **plus the Claude
marketplace manifest — required, per goals.md E6**, and the OpenCode
TypeScript/npm plugin (thin bridge; no graph logic in TS).

**Done when**
- [ ] Generation deterministic (same inventory → byte-identical packages)
- [ ] No absolute paths, credentials, or machine-specific values (validator test)
- [ ] Real skill directories copied, never symlinked
- [ ] Platform validators pass where they exist (`claude plugin validate`, etc.)
- [ ] OpenCode TS plugin passes the repo JS gates: `npm run typecheck:js` and
      `npm run test:js` cover it (plugin unit tests for tool registration and
      MCP config emission), and it builds under the workspace npm setup

### C-019 — Installed-artifact validation + clean-install tests

- **OpenSpec:** `/opsx:new harness-install-validation`
- **Owner:** CI + tests · **Agent:** claude · **Risk:** medium · **Depends on:** C-018

Clean-machine install/discovery/load/MCP-invocation/upgrade/uninstall per named
harness version; installed-artifact parity (not source templates).

**Done when**
- [ ] Each generated package passes the full lifecycle on a named harness version
      (versions recorded)
- [ ] Upgrade preserves user-authored instructions; uninstall removes managed
      files only (ownership tests)
- [ ] CI runs validation against installed artifacts, not templates — including
      the JS gates (`npm run typecheck:js`, `npm run test:js`) for the installed
      OpenCode plugin artifact

---

## Wave 8 — optional Store adapter (E4) — CONDITIONAL

### C-020 — `compass-store-surreal` Store adapter

- **OpenSpec:** `/opsx:new surreal-store-adapter`
- **Owner:** new optional crate `compass-store-surreal` · **Agent:** claude · **Risk:** high
- **Depends on:** C-011 accept, C-014 shipped
- **Condition precedent:** a recorded, independently stated user problem this
  adapter solves (research-doc rule: do not add it merely because the projection
  exists). **If none is recorded by the time Wave 7 completes, this change is
  dropped and the phase closes at 19 changes.**

Implements the unchanged `Store` contract; documented bounded sync/async runtime
boundary (no executor in `compass-store`, no nested-runtime panics).

**Done when**
- [ ] Passes `compass-store-qualification` **unchanged** (ordered scans, cursors,
      conditional writes, error taxonomy, interruption, reopen, bounded work)
- [ ] Runtime boundary documented + cancellation/timeout tests
- [ ] No executor or Surreal type leaks into `compass-store`

---

## Blocked before execution

### Decision — blocks C-005 adoption (not extraction)

Identity/namespacing for shared record keys. History fingerprints are meaning-affecting and
published realizations immutable. Before any current-graph consumer adopts the shared keys,
either namespace per path or prove non-collision with a test.

### Decision — blocks Wave 5 (carried from assessment §7)

C-011 (license) is a user/legal decision; C-013 budget ratification is a recorded
decision. Neither can be closed by implementation work.

---

## Unmeasured, carried forward

1. Payload composition — blocks C-003 calibration.
2. Whether `--exclude` on `universal-agent-runtime` lands under 2 GiB. With 4,827 markdown
   files under `crates/` it may not. Flagged since assessment; still unverified.
3. rmcp 3.1.4 transitive-dependency delta — measured inside C-007, gates its merge.

---

## Adversarial review

### 2026-08-09 round (Waves 1–2) — carried forward

Preflight `ok`; judge `k3` ≠ generator `kbd-frontier`.

**CRITICAL — none surviving.** The ordering error this stage exists to catch (C-004 before
C-001) was caught at spec and is enforced here by wave structure plus an inline gate on the
change itself.

**WARNING — carried to handoff:**
- C-003's acceptance criteria are testable, but its *implementation* has no calibration
  data. Measurement is the first task, not an afterthought.
- C-002 and C-004 are coupled through the error text: C-002 must omit the override, then
  C-004 must add it. If C-004 slips, C-002's message stays permanently incomplete — track
  it rather than assuming the follow-up happens.
- C-005's dependency rule is a stop-and-report gate, not a preference.
- The earlier "environment blocker" in this plan was withdrawn: it derived from an
  AGENTS.md rule hardcoding one contributor's mount path, which has since been removed.
  Builds run with ordinary Cargo defaults.

### 2026-08-28 round (full re-plan) — isolated cross-model, judge `kbd-judge` ≠ producer `claude-fable-5`

Raw findings: `review/plan/findings.json` (r1), `review/plan/findings-r2.json` (r2).

**Round 1 (BLOCK — 1 CRITICAL, 3 WARNING): all four fixed.**
- CRITICAL "no settled architecture decision point" → "Settled architecture
  decision" section added at the top of this plan, restating the assessment §1
  verdict with provenance.
- WARNING C-004 follow-up ambiguity → the escape hatch removed; C-002's message
  update is now **within** C-004, binding.
- WARNING `"3.1.4"` caret semantics → pin corrected to `=3.1.4`.
- WARNING missing JS verification → `npm run typecheck:js` / `test:js` criteria
  added to C-018 and C-019.

**Round 2 (PASS — 0 CRITICAL, 3 WARNING): all three addressed in-place rather
than carried.**
- C-018 marketplace optionality vs goals.md E6 → Claude marketplace manifest is
  now a required deliverable; only the Codex repo-marketplace stays decision-gated.
- C-003 vagueness → baseline-first protocol with a pinned time-to-error fraction
  and the C-002 remediation-text requirement added to its criteria.
- C-018 missing MCP-migration dependency → hard `Depends on: C-010` recorded.

Residual for the execute stage (carried): sycophancy shell gate degraded both
rounds (`sycophancy.sh lib not found`) — screen run via the MCP tool instead;
artifact-mode packets still omit git state (upstream skill-pack gap).
