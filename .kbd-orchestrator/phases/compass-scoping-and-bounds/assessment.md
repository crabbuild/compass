# Assessment — compass-scoping-and-bounds (re-run, expanded scope)

**Phase:** compass-scoping-and-bounds
**Date:** 2026-08-28 (supersedes 2026-08-09 assessment; original findings incorporated by reference below)
**Trigger:** user directive to absorb implementation of the `docs/future/` program
(SurrealDB, MCP 2026, Agent Skills, harness distribution) into this phase so all of
it completes here. The expanded scope is recorded in `goals.md` §"Expanded scope
(2026-08-28, user decision)" — this assessment assesses against that updated
goals artifact.
**Research:** external facts validated 2026-08-28 via Firecrawl search; codebase
facts verified by direct source inspection at the current working tree.

---

## 1. Verdict

**The expanded phase is implementable. Nothing in the codebase blocks it. Two
blockers are non-code and cannot be closed by this phase's own work: the SurrealDB
BSL license/release decision, and upstream maintainer agreement if the goal is
landing these changes upstream rather than on this fork.**

The 2026-08-09 verdict stands unchanged for the original scope: scoping already
exists and works; the real defects are the read-path allocation, the error message,
the missing fail-fast estimate, and the override inconsistency (C-001…C-006).

### The phase's architecture question — settled verdict

`goals.md` requires settling: *"is the right answer graph partitioning/sharding
rather than a larger number?"* **Verdict: neither. The answer is eliminating the
monolithic read-path allocation; partitioning already exists; sharding and a
larger limit are both rejected.** Basis (2026-08-09 analyze stage, decision log
D-entries, re-confirmed by source inspection this session):

1. **Sharding rejected** — a multi-machine technique; Compass is local-first
   single-process, and eventual consistency violates the determinism invariant
   (AGENTS.md).
2. **A larger number rejected** — explicit phase non-goal; AGENTS.md requires
   bounded work and a limit error as a distinct outcome.
3. **Record-class partitioning already ships** —
   `compass-history::PartitionedGraph` (`crates/compass-history/src/artifacts.rs`)
   implements partition()/into_partition()/reconstruct(); the gap is that the
   current-graph path doesn't use it, which C-005 (extraction into
   `compass-partition`) addresses.
4. **The true constraint is the read path** — the 2 GiB failure originates in
   `compass-graph/src/snapshot.rs` (`digest_json` over one monolithic
   `CanonicalGraphDocument`) compounded by `read_snapshot` reconstructing the
   whole payload in memory; C-001 splits that API so production callers never
   materialize it.

What remains for implementation is exactly C-001…C-006 — no new architecture
decision is open in the original scope.

### Change inventory referenced throughout (from plan.md, statuses from progress.json)

| ID | Title | Status |
| --- | --- | --- |
| C-001 | Stop materializing snapshot payload in `read_snapshot` (API split: manifest-only + chunk-streaming reads) | pending (code drafted, uncommitted) |
| C-002 | Make snapshot limit error actionable (name the remedy; omit override until C-004) | pending |
| C-003 | Pre-flight graph size estimate (fail fast before 331 s builds) | pending |
| C-004 | Honor `COMPASS_MAX_GRAPH_BYTES` on the publication path | pending — HARD gate on C-001 |
| C-005 | Extract `compass-partition` shared crate from `compass-history::PartitionedGraph` | pending — gated on C-001 |
| C-006 | Decide `vendor/` default-skip policy | pending |

---

## 2. Scope directive and its recorded conflict

Both `docs/future/` planning notes explicitly advise **against** doing this:

> "Do not append the ecosystem work to that phase merely because `read_snapshot`
> may later help a remote adapter. Finish, pause, or formally supersede the
> current phase first." — `upstream-contribution-plan.md`

> "Run `/kbd-new-phase` scoped to Proposal A only." — `SURREAL_DB.md`

The user directive of 2026-08-28 overrides this and is recorded here as a **user
decision** (same category as the 2026-08-09 PartitionedGraph-extraction decision).
Consequences accepted by that decision:

- The phase grows from 6 changes / 4 goals to an estimated **20+ changes / 10
  goals** spanning storage, query, transport, security, licensing, CLI, and
  packaging contracts.
- The research doc's own staging estimates total **14–27 weeks** (Phases −1
  through 5) on top of the ~1–2 weeks remaining in the original scope.
- The phase name no longer describes its content; the waypoint's wave counter
  becomes the real unit of progress and verify/reflect must run **per wave**, not
  per phase, or drift will accumulate unchecked.

**Ambiguity the plan stage must resolve:** "get all of it done in this phase"
can mean (a) implement locally on this fork, or (b) land upstream via the PR
sequence in `upstream-contribution-plan.md`. These differ by months and by
dependencies outside our control (maintainer response). This assessment assumes
**(a) local implementation on the fork**, with upstream submission tracked as
separate exit work following the contribution plan. If (b) is intended, PR-0
(maintainer decision record) becomes the phase's first hard gate and the
schedule is not ours to control.

---

## 3. State drift since 2026-08-09

| Item | 2026-08-09 | Now (2026-08-28) |
| --- | --- | --- |
| C-001 | pending, no code | **substantially implemented but uncommitted**: `crates/compass-store/src/lib.rs` (+152/−17) adds `read_snapshot_manifest()` + streaming `read_snapshot_chunks()`, exactly the API split the spec-stage decision describes. No OpenSpec change exists; ledger still says `pending`; work sits on branch `docs/claude-md`. Evidence (this session): `git diff --stat crates/compass-store/src/lib.rs` → `169 ++++…----, 152 insertions(+), 17 deletions(-)`; `git branch` → `docs/claude-md`; diff hunks show the two new methods and the revised `MAX_GRAPH_BYTES` doc comment. |
| C-002…C-006 | pending | pending, unchanged |
| Ledger | 0/6, `plan_ready` | 0/6, `plan_ready` (accurate for formalized work; stale w.r.t. the uncommitted C-001 code) |
| Binary vs source | — | installed `compass` 0.3.6, source manifest 0.3.7 |
| docs/future | did not exist | 3 notes (1,294 lines): research report, SurrealDB proposal, upstream contribution plan |

Binary/source evidence (this session): `compass --version` → `compass 0.3.6`;
`Cargo.toml [workspace.package] version = "0.3.7"`. MCP tool list evidence: the
15 tool names in §5 are the `mcp__compass__*` tools exposed by the live server
connected to this session.

**First required action regardless of plan shape:** reconcile C-001 — create its
OpenSpec change, absorb the existing diff, add tests, commit on an appropriately
named branch. Until then every ledger read is misleading.

---

## 4. External facts (validated 2026-08-28, Firecrawl)

| Claim in research doc | Verified status | Source |
| --- | --- | --- |
| `rmcp` 3.1.4 is current | **Confirmed** — released 2026-08-20; implements stable MCP `2026-07-28`; a "Migrating to 3.x?" guide exists in the SDK README | <https://docs.rs/rmcp/latest/rmcp/> ("rmcp-3.1.4 … 20 August 2026"); <https://github.com/modelcontextprotocol/rust-sdk> ("This SDK implements the stable MCP `2026-07-28` specification") |
| MCP `2026-07-28` is the latest spec | **Confirmed** — spec site marks it "(latest)"; `initialize`/`initialized` and `Mcp-Session-Id` retired (SEP-2575, SEP-2567); stateless `Mcp-Method`/`Mcp-Name` headers replace the handshake | <https://modelcontextprotocol.io/specification/2026-07-28> ("Version 2026-07-28 (latest)"); <https://blog.modelcontextprotocol.io/posts/2026-07-28/> ("officially retired the `initialize`/`initialized` exchange along with the `Mcp-Session-Id` header") |
| SurrealDB 3.2.4 is latest stable | **Confirmed** — "LATEST STABLE 3.2 · Latest patch 3.2.4 · Aug 2, 2026"; 3.3.0-beta.1 exists (2026-08-13, pre-release — do not target) | <https://surrealdb.com/releases> |
| SurrealDB BSL 1.1 licensing | FAQ page current; the research doc's reading (embed/redistribute allowed, DBaaS restricted, Apache-2.0 after change window) was not re-litigated here — the **formal license decision remains open** and is a phase gate | <https://surrealdb.com/license> |

Implication of the session-model removal: `compass-mcp`'s HTTP transport tests
`Mcp-Session-Id` and `--session-timeout` semantics that the current spec has
deleted. The migration is not optional modernization; the shipped contract is now
a legacy protocol.

## 5. Codebase verification (direct inspection)

- `Cargo.toml:86` pins `rmcp = "2.2.0"` (server, transport-io,
  transport-streamable-http-server). No `surreal*` dependency anywhere in the
  workspace — the default-footprint gate starts from a clean baseline.
- `compass-graphdb` exists (bounded Neo4j Bolt / FalkorDB RESP export) — the
  architectural precedent the research doc names for a derived-projection
  boundary is real.
- `compass-store-redb` (~869 lines) and `compass-store-qualification` exist —
  second-backend precedent and qualification gates are real, so
  `compass-store-surreal` follows an established pattern rather than new
  architecture.
- `compass-mcp` serves 15 tools + 6 resources (tool list spot-verified against
  the live MCP server this session: `search_symbols`, `get_callers`,
  `get_callees`, `get_impact`, `explore_code`, `get_node`, `query_graph`,
  `get_neighbors`, `get_community`, `god_nodes`, `graph_stats`, `shortest_path`,
  `list_prs`, `get_pr_impact`, `triage_prs`).
- Embedded agent skill + atomic multi-harness installer exist under
  `compass-cli` assets; Rust toolchain 1.97.1 satisfies rmcp 3.x's 1.88 MSRV.

## 6. Gap report against expanded goals

### Original goals (all open, unchanged)

| Goal | Gap | Change |
| --- | --- | --- |
| G1 override inconsistency | open | C-004 (HARD-gated on C-001) |
| G2 actionable limit error | open | C-002 (couples to C-004 error text) |
| G3 fail fast | open | C-003 (needs calibration data) |
| G4 vendor/ policy | open | C-006 |
| — read-path allocation | **code exists, unformalized** | C-001 |
| — partition crate extraction | open | C-005 (gated on C-001) |

### Expanded goals (new, from docs/future)

| Goal | Current state | Gap |
| --- | --- | --- |
| E1 MCP 2026 migration (`rmcp` 3.1.4, stateless discovery, typed results, deterministic tool ordering, session-flag removal) | `rmcp 2.2.0`, session-based HTTP, several text-only legacy tools | entire migration; no technical blocker found; MSRV/toolchain compatible |
| E2 Phase-−1 gates: BSL license decision; persistent SurrealKV + RocksDB probes (parallel edges, provenance, generation reads, kill-during-write) | nothing | probes are code; **license decision is not** — external gate |
| E3 Surreal graph projection (`compass-graphdb-surreal`: schemafull nodes/relations, generation activation, bounded native reads, dual-engine equivalence) | nothing; precedent `compass-graphdb` | entire crate + equivalence corpus |
| E4 Surreal Store adapter (`compass-store-surreal`) | nothing; precedent `compass-store-redb` + qualification | entire crate; sync/async bounded boundary; **optional, last** — research doc rejects Store-first explicitly |
| E5 Skills split + `compass agent` CLI namespace | umbrella skill + installer exist | 6 focused skills, list/install/doctor/export/validate/mcp-config, umbrella preserved as alias |
| E6 Native harness packages (Codex `.codex-plugin`, Claude `.claude-plugin` + marketplace, OpenCode npm/TS plugin) from one `distribution.toml` inventory | installer places skills/adapters; no native packages | inventory + generators + installed-artifact validation |
| E7 Measurement prerequisites (Phase-0 corpora: golden answers, scale fixtures, 30-task suite, baselines) | nothing | must precede E3 acceptance; the research doc's numeric budgets are unratified and must be ratified **before** measurements are visible |

## 7. Blockers code cannot clear

1. **SurrealDB BSL 1.1 legal/release acceptance** for the exact artifact profile
   (E2). Until decided, E3/E4 can be built behind non-default features but cannot
   ship in a release artifact.
2. **Upstream maintainer agreement**, only if reading (b) of the directive holds.
   The contribution plan's checklist (clean worktrees, one problem per PR, no KBD
   state in feature PRs) governs the exit path either way.
3. **Unratified numeric budgets** (E7): the qualification thresholds in the
   research doc are stated as "ratify before measuring." Ratification is a
   decision, not code; the plan stage should schedule it as an explicit gate.

## 8. Risks specific to the merged phase

- **Coupling growth:** C-002/C-004 already couple through error text; the MCP
  migration (E1) touches the same error/result surfaces. Sequencing must keep the
  original six changes ahead of E1's envelope work or the error contract will be
  rewritten twice.
- **Ledger integrity:** 20+ changes in one phase with an already-drifted ledger
  (C-001) is exactly the condition the upstream plan warns about. Per-wave
  verify + reflect is the mitigation; skipping it forfeits the phase's audit value.
- **Branch hygiene:** current work sits on `docs/claude-md` with unrelated tracked
  modifications. Each wave needs its own branch/worktree from a clean base, per
  the contribution plan, or the eventual upstream slicing becomes archaeology.
- **BSL gate late-failure:** if the license decision fails after E3 is built, the
  projection work strands. Mitigation: E2 (license + probes) must complete before
  any E3 code merges — same "cheap disqualifiers first" order the research doc
  already mandates.

## 9. Recommendation to the plan stage

Re-plan the phase as **waves with per-wave verify/reflect**, preserving the
existing Wave-1/Wave-2 structure as Waves 1–2 and appending:

```text
Wave 1  (existing) C-001 formalization first, then C-002, C-003, C-006
Wave 2  (existing) C-004, C-005                         [HARD GATE: C-001]
Wave 3  E1  MCP 2026 migration (rmcp 3.1.4 branch, conformance, interop)
Wave 4  E2  license decision + persistent probes + E7 corpora/budget ratification
Wave 5  E3  Surreal graph projection + dual-engine equivalence
Wave 6  E5  skills split + compass agent CLI
Wave 7  E6  harness packages (generated from one inventory)
Wave 8  E4  Surreal Store adapter (optional; only if an independent user problem
            justifies it — research doc's own condition)
```

Wave 3 before Wave 4 because MCP migration is self-contained, urgent (shipped
contract is legacy), and independent of the license gate; Waves 5+ are gated on
Wave 4's decisions. One OpenSpec change per PR-shaped unit throughout, per the
contribution plan.

---

## 10. Adversarial review record (2 rounds, isolated cross-model)

Judge: `kbd-judge` via rest-gateway `http://localhost:8181/v1`; producer
`claude-fable-5`; `cross_model_check: verified-distinct` both rounds. Raw
findings: `review/assess/findings.json` (r1), `review/assess/findings-r2.json`.

**Round 1 (BLOCK — 2 CRITICAL, 3 WARNING): both criticals fixed.**
- CRITICAL "expanded scope not in goals" → `goals.md` updated with the
  2026-08-28 user-decision scope section; assessment now assesses against it.
- CRITICAL "architecture question not settled" → §1 now carries the explicit
  verdict (neither sharding nor a larger number; eliminate the read-path
  allocation; partitioning already ships).
- WARNINGs (undefined change IDs, missing evidence, missing source URLs) →
  change inventory table, session evidence quotes, and source URLs added.

**Round 2 (BLOCK — 1 CRITICAL, 3 WARNING): accepted unresolved per the
two-round cap.** The next stage must read these:

1. **CRITICAL (unresolved, disputed):** "C-001 substantially implemented" is
   unverified *from the packet*. Disposition: the claim was verified live this
   session (`git diff --stat` → +152/−17; hunks show `read_snapshot_manifest`
   and `read_snapshot_chunks`), but artifact-mode packets carry no git state, so
   the judge structurally cannot confirm it. Treat as **verify-first**: the
   C-001 formalization change must begin by re-running `git diff` and compiling
   (`cargo check -p compass-store --locked`) before absorbing the diff. If the
   diff is absent or broken, C-001 reverts to plain pending and nothing
   downstream changes.
2. WARNING: codebase assertions beyond packet evidence (branch name, binary
   version, MCP tool list) — same disposition: re-verify at execution time.
3. WARNING: docs/future contents not in packet — the three notes are in-repo at
   `docs/future/`; analyze/plan must read them directly rather than trusting
   this summary.
4. WARNING: expanded-goal gap rows too coarse to plan from (E1/E3/E5/E6 are
   multi-part) — accepted; decomposition into per-change acceptance criteria is
   exactly the plan stage's job and must happen there.

Tooling note for the orchestrator: `check-findings-sycophancy.sh` degraded both
rounds (`sycophancy.sh lib not found — gate skipped`); the sycophancy screen was
run instead via the sycophancy-correction MCP tool (result recorded in the
stage handoff). Artifact-mode packets omitting git state is a
`build-review-packet.sh` gap worth an upstream skill-pack issue.
