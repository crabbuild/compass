# Plan — compass-scoping-and-bounds

**Stage:** Plan
**Date:** 2026-08-09
**Backend:** OpenSpec (`openspec/` present, `openspec_available: true`)
**Evolver cycle:** none
**Changes:** 6 — 0 complete

Reads `assessment.md`, `analysis.md`, `spec.md`, `library-candidates.json`,
`handoffs/{analyze,spec}.md`.

---

## Ordering rationale

Wave 1 is everything with no dependency. Wave 2 is gated on C-001, because until the read
path stops materializing the payload, both dependents are unsafe or premature.

```
Wave 1 (parallel)          Wave 2 (after C-001)
├── C-001  read_snapshot   ├── C-004  env override   [HARD GATE]
├── C-002  error message   └── C-005  compass-partition
├── C-003  size estimate
└── C-006  vendor/ policy
```

**Apply C-001 first.** It is the only Wave 1 item that unblocks others, and the audit
showed it is smaller than originally scoped.

---

## Wave 1

### C-001 — Stop materializing the snapshot payload in `read_snapshot`

- **OpenSpec:** `/opsx:new stream-snapshot-read`
- **Owner:** `compass-store` · **Agent:** claude · **Risk:** medium
- **Library:** `cand-003` (build — no external library; verdict `adopt`)
- **Compatibility-sensitive:** yes (`read_snapshot` is `pub`)

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
- [ ] Oversized build reports before full extraction completes
- [ ] Estimate deterministic for equivalent inputs
- [ ] Limit outcome stays distinct from an empty result
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
- [ ] C-002's message updated to advertise it, in the same change or immediately after

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

## Blocked before execution

### Environment — blocks all verification

`/Volumes/Workspace` is **not mounted** (re-checked at plan time). Every compiling command
requires `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main`. Per AGENTS.md,
falling back to a local `target/` is forbidden — stop and report instead. **No change can be
verified until that volume is available.**

### Decision — blocks C-005 adoption (not extraction)

Identity/namespacing for shared record keys. History fingerprints are meaning-affecting and
published realizations immutable. Before any current-graph consumer adopts the shared keys,
either namespace per path or prove non-collision with a test.

---

## Unmeasured, carried forward

1. Payload composition — blocks C-003 calibration.
2. Whether `--exclude` on `universal-agent-runtime` lands under 2 GiB. With 4,827 markdown
   files under `crates/` it may not. Flagged since assessment; still unverified.

---

## Adversarial review

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
- Every "done when" box is unverifiable while `/Volumes/Workspace` is unmounted. This plan
  is executable only after that is resolved.
