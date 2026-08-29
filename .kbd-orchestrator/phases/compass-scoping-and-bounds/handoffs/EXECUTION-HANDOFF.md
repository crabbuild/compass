# Execution handoff — compass-scoping-and-bounds

**To:** gpt-5.6-sol via Codex CLI 0.146.0
**From:** Claude Code (planning session, 2026-08-28)
**Repo:** repository root
**Stage reached:** plan complete, adversarial-vetted (PASS). Execute is yours.

Read this file, then `plan.md` in this directory. Everything you need is in the
repo — nothing depends on the planning session's chat history.

---

## 1. What this phase is

Compass failed to publish a 6.8 GB monorepo: the canonical graph exceeded the
2 GiB `MAX_GRAPH_BYTES` limit after burning 331.90 s. The phase makes that
failure graceful and recoverable **without** weakening the bounded-work
invariant. On 2026-08-28 the user expanded scope to also implement the
`docs/future/` program (MCP 2026, SurrealDB evaluation, Agent Skills, harness
distribution) in this same phase.

**Settled architecture decision — do not relitigate.** The open question was
"partitioning/sharding, or a larger limit?" The answer is **neither**:

- Sharding is rejected — multi-machine technique; eventual consistency violates
  Compass's determinism invariant (AGENTS.md).
- A larger default is rejected — explicit non-goal; AGENTS.md requires bounded
  work and a limit error distinct from an empty result.
- Record-class partitioning **already ships** in `compass-history::PartitionedGraph`;
  it is simply unused by the current-graph path.
- The real constraint is the **monolithic read-path allocation**. That is C-001.

Provenance: `assessment.md` §1, `decision-log.md` entries dated 2026-08-09,
`library-candidates.json` (`cand-001` adopt, `cand-002` reject, `cand-003` adopt).

## 2. Authoritative documents, in reading order

| File | What it gives you |
| --- | --- |
| `plan.md` (this dir) | **The work.** 20 changes, 8 waves, per-change acceptance criteria, hard gates |
| `goals.md` (this dir) | G1–G4 original + E1–E7 expanded scope (user decision, dated) |
| `assessment.md` (this dir) | Gap report, verified external facts w/ source URLs, blockers |
| `spec.md`, `analysis.md` (this dir) | C-001 consumer audit; sharding/partitioning analysis |
| `decision-log.md` (this dir) | Every decision + provenance. Append to it; never rewrite |
| `AGENTS.md` (repo root) | **Authoritative operating guide.** Product invariants, crate ownership, Rust conventions, completion checklist |
| `CLAUDE.md` (repo root) | Build/test commands, architecture tour, compatibility surfaces |
| `COMPATIBILITY.md` | What counts as an incompatible user-visible change and what it requires |
| `docs/future/*.md` (3 files) | Source material for waves 3–8. **Read directly** — the assessment only summarizes them |

## 3. State as of handoff (verified this session, not assumed)

```
branch:        docs/claude-md
ledger:        0/20 implementation, status plan_ready, wave 1
waypoint:      .kbd-orchestrator/current-waypoint.json
typed runtime: legacy mode; stage entry committed locally (revision 3)
               control plane at 127.0.0.1:7892 unreachable — expected, not an error
working tree:  1 tracked modification (crates/compass-store/src/lib.rs) + 47 untracked
               harness/generated paths (.claude/, .codex/, .agents/, .prometheus/, compass-out/)
```

### C-001 verify-first check: **PASSED — already done for you**

`plan.md` tells you to verify the uncommitted C-001 diff before absorbing it.
That check ran at handoff time. Results:

```
$ git diff --stat crates/compass-store/src/lib.rs
 crates/compass-store/src/lib.rs | 169 ++++++++++++---------------
 1 file changed, 152 insertions(+), 17 deletions(-)

$ grep -n "pub fn read_snapshot" crates/compass-store/src/lib.rs
 837:  pub fn read_snapshot_manifest(&self) -> Result<SnapshotManifest, StoreError>
 856:  pub fn read_snapshot_chunks<F>(&self, mut consume: F) -> Result<SnapshotManifest, StoreError>
 901:  pub fn read_snapshot(&self) -> Result<(SnapshotManifest, Vec<u8>), StoreError>

$ cargo check -p compass-store --locked
 Finished `dev` profile in 27.95s          # clean
```

**So: the diff exists, has the right shape, and compiles.** Treat it as a
starting draft, not finished work. What it still needs (from C-001's criteria):

- `validate_snapshot` (~`lib.rs:935`) and `snapshot_reference` (~`lib.rs:1184`)
  reimplemented on `read_snapshot_manifest` so **no production path** allocates
  proportional to `payload_bytes`. Verify by reading the call sites — the draft
  may not have done this.
- Tests: round-trip, reopen, corruption/interruption, publication-atomicity.
- Equivalence: `validate_snapshot`/`snapshot_reference` must return identical
  results for valid **and corrupt** snapshots.
- `CHANGELOG.md`; `MIGRATION.md` only if the public signature changes.

If any of that is missing, finish it inside the C-001 change. Do not commit the
draft as-is.

## 4. How to run the work

Backend is **OpenSpec** (`openspec/` present, v1.10.0, skills in `.agents/skills/`).
Codex uses skills rather than slash commands, so per change:

```bash
$openspec-new-change "<change-id>"     # e.g. stream-snapshot-read
# implement tasks
$openspec-verify-change "<change-id>"
$openspec-archive-change "<change-id>"
```

`plan.md` names the exact change-id for each of C-001…C-020.

**Start here:** C-001, change-id `stream-snapshot-read`.

### Rust gates (from CLAUDE.md — run before calling any change done)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --locked -- -D warnings
cargo test --workspace --lib --bins --locked
```

Narrower loop while iterating: `cargo test -p <crate> --locked`.
Toolchain is pinned 1.97.1; always `--locked`.

### JS gates (only for C-018/C-019, the OpenCode plugin)

```bash
npm ci && npm run typecheck:js && npm run test:js
```

## 5. Rules that will fail review if broken

From `AGENTS.md` and the workspace lints — these are enforced, not advisory:

- **No `unsafe`. No `unwrap()`, `expect()`, or `panic!`.** Return typed errors
  with actionable context.
- **Determinism is correctness.** `BTreeMap`/`BTreeSet` or explicit sorting at
  contract boundaries. Never rely on hash iteration or filesystem order.
- **Never resolve ambiguity by picking the first/most convenient candidate.**
  An explicit unresolved/ambiguous result beats invented meaning.
- **Every traversal bounded** — files, graphs, archives, network, queries,
  subprocess output. A limit error is a distinct outcome from an empty result.
- **Reuse existing helpers** — bounded readers, subprocess helpers, path
  containment, atomic writes. Do not write weaker local variants.
- **Subprocess args passed separately.** Never build shell strings.
- **Workspace deps at root `Cargo.toml`**, referenced as `{name}.workspace = true`.
- **Layer discipline:** per-file extractors emit evidence and never resolve
  project-wide facts (that is `compass-resolve`); topology-dependent logic lives
  in `compass-graph`; `compass-cli`/`compass-mcp` stay thin.
- **Compatibility-sensitive changes** (CLI args/help/exit codes, env vars, graph
  JSON, CompassQL, MCP schemas, output files, history formats, stable IDs) need
  all four: native regression coverage, updated reference docs, a `MIGRATION.md`
  note when users must act, a `CHANGELOG.md` entry when release-visible.
- **Published history realizations are immutable.** Never rewrite in place.
- **Never hand-edit** `progress.json`, `current-waypoint.json`, or event
  counters. Use the typed surface (`prometheus kbd ...`) or the OpenSpec skills.

## 6. Wave order and the gates that bind it

```
Wave 1  C-001 → then C-002, C-003, C-006          (C-001 first: it unblocks Wave 2)
Wave 2  C-004, C-005                               HARD GATE: C-001 merged
Wave 3  C-007, C-008, C-009, C-010                 (MCP 2026 — urgent, independent)
Wave 4  C-011, C-012, C-013                        (decision gates + measurement)
Wave 5  C-014, C-015                               HARD GATE: C-011+C-012+C-013
Wave 6  C-016, C-017                               (parallelizable with 4–5)
Wave 7  C-018, C-019                               HARD GATE: C-010
Wave 8  C-020                                      CONDITIONAL — see below
```

**Gates you must not cross early:**

1. **C-004 before C-001 is merged** — the specific error this plan exists to
   prevent. An env override before the read path streams lets a user request a
   multi-gigabyte contiguous `Vec`: strictly worse than today's clean error.
2. **Any SurrealDB code (C-014, C-015, C-020) before Wave 4 completes.** C-011
   is a **license decision the user/legal must make** — you cannot close it by
   writing code. If it is rejected, Waves 5 and 8 are cancelled and the fallback
   is the existing SQLite/redb/`graph.json` stack.
3. **C-005's dependency rule is stop-and-report, not a preference.**
   `compass-partition` must not depend on `prolly-map`, `prolly-store-sqlite`,
   `compass-ir`, or `compass-analysis`. If that cannot hold, **stop and report** —
   it means the shared-crate framing is wrong and the decision needs revisiting.
4. **C-010 interop is a merge gate.** If Codex/Claude Code/OpenCode do not
   interoperate, Wave 3 does not merge merely because the Rust compiles.
5. **C-020 is conditional.** It ships only if a user problem it independently
   solves is recorded. If none is by the time Wave 7 ends, drop it — the phase
   closes at 19 changes.

## 7. Branch hygiene

Current work sits on `docs/claude-md` alongside unrelated tracked modifications
and 47 untracked harness paths. Per `docs/future/upstream-contribution-plan.md`:
**create a clean branch or worktree per change**, and keep orchestration state
(`.kbd-orchestrator/**`, `.prometheus/**`) in separate commits from
implementation, so the eventual upstream slicing is not archaeology. Do not run
destructive cleanup on this checkout.

## 8. Known-open items (do not treat as settled)

1. **Payload composition is unmeasured** — which node/edge classes dominate the
   2 GiB is unknown. C-003's first task is measurement; do not invent a heuristic.
2. **Whether `--exclude` alone lands `universal-agent-runtime` under 2 GiB** is
   unverified. With 4,827 markdown files under `crates/` it may not.
3. **rmcp 2.2.0 → 3.1.4 transitive-dependency delta** is unmeasured; measuring it
   gates C-007's merge.
4. **The numeric qualification budgets** in the research doc are **unratified**.
   C-013 must ratify (or amend) them by recorded decision **before** any Surreal
   measurements are visible — ratifying after seeing results is motivated reasoning.
5. **Mode assumption:** the plan assumes **fork-local** implementation. Upstream
   PR submission is exit work following the contribution plan. If the user wants
   upstream-first, PR-0 (maintainer decision record) becomes the first gate and
   the schedule is not ours to control — confirm before proceeding on that reading.

## 9. Adversarial review record

Both `assessment.md` and `plan.md` were vetted by an isolated cross-model judge
(`kbd-judge` ≠ producer, `cross_model_check: verified-distinct`). Raw findings:
`review/assess/findings*.json`, `review/plan/findings*.json`.

- **Plan:** round 1 BLOCK (1 CRITICAL — settled-architecture decision missing;
  fixed) → round 2 **PASS**. All r2 WARNINGs fixed in-place.
- **Assessment:** round 2 accepted with 1 CRITICAL unresolved — "C-001 is
  implemented" was unverifiable *from the review packet* because artifact-mode
  packets carry no git state. **That finding is now closed**: the check ran at
  handoff (§3) and passed.
- Sycophancy screen on the assessment: 0.0, clean.

Tooling gap worth knowing: `check-findings-sycophancy.sh` degraded on every run
(`sycophancy.sh lib not found`); the screen was run via the sycophancy-correction
MCP tool instead. Artifact packets omitting git state is an upstream skill-pack
gap, not a repo defect.

## 10. Compass MCP is available to you

`.codex/config.toml` wires the Compass MCP server (stdio, graph at
`compass-out/graph.json`). Use it as the first navigation layer before broad
source searches: `search_symbols`, `get_callers`, `get_callees`, `get_impact`,
`explore_code`, `query_graph`, `shortest_path`. Run `compass update .` after code
changes. Verify important graph conclusions in the cited source — treat missing
paths and inferred edges as uncertain evidence, not proof.

Note the graph may be stale for `crates/compass-store/src/lib.rs` (the C-001
draft postdates the last build).
