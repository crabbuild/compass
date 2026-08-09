# Spec — compass-scoping-and-bounds

**Stage:** Spec
**Date:** 2026-08-09
**Reads:** `assessment.md`, `analysis.md`, `library-candidates.json`, `handoffs/analyze.md`

---

## Consumer audit result — this changes the plan

The analyze stage flagged "consumer audit of the read path" as blocking. It is now done,
and the result is materially better than assumed.

The function is `SqliteStore::read_snapshot` (`compass-store/src/lib.rs:829`), signature
`-> Result<(SnapshotManifest, Vec<u8>), StoreError>`. **The materialized payload is in the
return type itself.** Eight call sites exist:

| Site | Use of the payload |
| --- | --- |
| `compass-store/src/lib.rs:935` `validate_snapshot` | **discarded** — `.map(\|(manifest, _)\| manifest)` |
| `compass-store/src/lib.rs:1184` `snapshot_reference` | **discarded** — `let (manifest, _)` |
| `compass-store/src/lib.rs:2774,2779,2804,2835` | tests |
| `compass-core/tests/code_graph_v1_determinism.rs:188` | test (asserts `is_err()`) |
| `compass-query/src/graph_engine.rs:211` | calls `begin_read_snapshot`, **not** `read_snapshot` |

**No production consumer uses the returned bytes.** Both non-test callers immediately drop
them with `_`. And `compass-query` — the main read path — already goes through
`GraphSnapshotReader::open_selector` (`graph_engine.rs:223`), an indexed reader that never
materializes the document.

So the 2 GiB contiguous allocation in `read_snapshot` (`lib.rs:838-853`) is incurred **for
a value every production caller throws away.** The store's doc comment
(`lib.rs:44-51`) claiming records are served "through indexed scans instead of
materializing the whole document" is *already true of the path that matters*; only
`read_snapshot` violates it, and nothing in production needs what it returns.

This makes change A far smaller and lower-risk than analyze estimated.

---

## Ordered changes

### C-001 — Stop materializing the snapshot payload in `read_snapshot`

**Gap:** A (analyze). **Owner:** `compass-store`. **Blocks:** C-004.

Split the API so callers that need only the manifest never allocate the payload:

- add `read_snapshot_manifest() -> Result<SnapshotManifest, StoreError>` that reads and
  validates the manifest without touching chunk objects;
- reimplement `validate_snapshot` (`lib.rs:935`) and `snapshot_reference` (`lib.rs:1184`)
  on it;
- keep byte-returning behavior available for tests and any future consumer, but as a
  streaming form — e.g. `read_snapshot_chunks()` yielding chunk slices — rather than one
  `Vec::with_capacity(payload_bytes)` concatenation.

Retain the per-chunk running total and `MAX_GRAPH_BYTES` check so a corrupt manifest still
cannot drive unbounded work. `CHUNK_BYTES` (255 KiB, `lib.rs:60`) already bounds each
stored value.

**Acceptance**
- No production code path allocates proportional to `payload_bytes`.
- `validate_snapshot` and `snapshot_reference` return identical results to today for both
  valid and corrupt snapshots.
- Round-trip, reopen, corruption/interruption, and publication-atomicity coverage per
  AGENTS.md history/storage policy.
- The `lib.rs:44-51` doc comment is either satisfied or corrected — **not left as is**.

**Compatibility:** `read_snapshot` is `pub`. Changing or deprecating it is a public-API
change; if the signature changes, it needs `CHANGELOG.md` and, if consumers must act,
`MIGRATION.md`.

---

### C-002 — Make the snapshot limit error actionable

**Gap:** G2. **Owner:** `compass-graph` (message), `compass-cli` (thin presentation).
**Independent** — may land in parallel with C-001.

Today (`compass-graph/src/snapshot.rs:3138`):

```
canonical graph exceeds the 2147483648-byte limit
```

State the remedy, matching the precedent already set in
`compass-core/src/diagnostics.rs:453`. The message must name: `--exclude` /
`.compassignore` scoping, the `COMPASS_MAX_GRAPH_BYTES` override **only once C-004 has
landed**, and the dominant content class if C-003 has measured it.

**Acceptance**
- Error names at least one concrete next action.
- CLI-level assertion of the rendered text and exit code under
  `crates/compass-cli/tests/`.
- No override is advertised before C-004 ships. Advertising an override that does not
  exist on this path is the defect being fixed — do not reintroduce it inverted.

---

### C-003 — Pre-flight graph size estimate

**Gap:** G3. **Owner:** `compass-core`. **Independent.**

The reported failure consumed 331.90 s and discarded all work. Estimate the canonical
payload size before publication and fail (or warn) early.

**Acceptance**
- A build destined to exceed the cap reports before full extraction completes.
- The estimate is deterministic for equivalent inputs (AGENTS.md).
- A limit outcome remains distinct from an empty result.
- Estimation must not itself become unbounded work.

**Open:** no calibration data yet — which node/edge classes dominate the payload is
unmeasured. Measure during implementation; do not guess a heuristic.

---

### C-004 — Honor `COMPASS_MAX_GRAPH_BYTES` on the publication path

**Gap:** G1. **Owner:** `compass-graph`, `compass-store`. **HARD-BLOCKED BY C-001.**

`COMPASS_MAX_GRAPH_BYTES` is honored in `compass-model` (`graph.rs:370`),
`compass-output` (`json.rs:414`), `compass-global` (`lib.rs:370`), and `compass-core`
(`diagnostics.rs:713`) — and those crates' errors advertise it. `compass-graph/snapshot.rs`
and `compass-store` never read it. Resolve the inconsistency: honor it, or document
explicitly why this path must not.

> **Sequencing gate.** Do not land before C-001. Until the read path stops materializing,
> an override lets a user request a multi-gigabyte contiguous `Vec` — a worse failure than
> today's clean error. This is the single most important ordering constraint in the phase.

**Acceptance**
- Behavior consistent with the four crates that already honor it, or a documented
  exception.
- Default remains 2 GiB. **Not** a default increase — AGENTS.md requires bounded work.
- Override is explicit and opt-in (ripgrep `--no-ignore-vcs` precedent from analyze).

---

### C-005 — Extract `compass-partition` shared crate

**Gap:** ARCH-partitioning. **Owner:** new crate. **Sequenced after C-001.**

Per your direction, `PartitionedGraph` moves to a shared crate. Scope is constrained by
what the type actually depends on.

**What moves** — the record container and its key helpers, all dependency-light:

| Item | Current location |
| --- | --- |
| `PartitionedGraph` | `compass-history/src/artifacts.rs:76-84` |
| `node_key`, `edge_key`, `hyperedge_key` | `compass-history/src/keys.rs:10,20,46` |
| `canonical_json_bytes` | `compass-history/src/canonical.rs:9` |

**What does NOT move** — history-coupled production logic. `into_partition`
(`artifacts.rs:349+`) depends on `CompletionEvidence`, `authoritative_sidecars` /
`TRUSTED_GRAPH_CONTENT`, `AnalysisBundle`, `compass_ir::ProgramBundle`, `HistoryError`, and
`prolly::{KeyBuilder, VersionedValue}`. These are history-realization concerns with no
counterpart on the current-graph path.

**Dependency rule (blocking):** `compass-partition` must NOT depend on `prolly-map`,
`prolly-store-sqlite`, `compass-ir`, or `compass-analysis`. `compass-graph` today depends
on none of these; pulling them onto the current-graph path would contradict the local-first
minimal-dependency posture. If the extraction cannot satisfy this, **stop and report** —
that outcome means the shared-crate framing does not hold and the decision needs revisiting.

`canonical_json_bytes` returns `HistoryError`; the shared crate needs its own error type,
with `compass-history` converting at its boundary.

**Identity semantics (blocking):** history fingerprints are meaning-affecting and published
realizations are immutable (AGENTS.md). Sharing key helpers must not let current-graph and
history produce colliding or conflatable record identities. Either namespace them per path
or prove non-collision with a test. **This must be settled before any current-graph
consumer adopts the shared keys.**

**Acceptance**
- `compass-history` behavior is byte-identical after extraction — same records, same keys,
  same canonical encoding. Verify with existing canonical-encoding, round-trip, and diff
  tests, which must pass unchanged.
- Workspace members updated in root `Cargo.toml`; deps via `{name}.workspace = true`.
- New crate carries the workspace lint policy (no `unsafe`, no `unwrap`/`expect`/`panic`).
- No new dependency reaches `compass-graph` transitively.

**Honest scope note:** this extraction moves a struct and four helpers — roughly 9 lines of
fields plus key/encoding utilities — out of a 2,317-line module. It does **not** by itself
give the current-graph path a partitioned publication format. It positions for that. If the
goal is current-graph partitioned publication, that is a further change requiring the
identity decision above, and it should be specced separately once C-001 has landed and the
read path is honest.

---

### C-006 — Decide `vendor/` default-skip policy

**Gap:** G4. **Owner:** `compass-files`. **Independent.**

`vendor/` is absent from `SKIP_DIRS` (`compass-files/src/detect.rs:61-102`) — 126 MB / 441
markdown files in the failing repo. But Go's `vendor/` holds real dependency source, and
Compass's own tree has a legitimate `vendor/` (`vendor/compass-tree-sitter-language-pack`,
a workspace member). A reflex addition would be wrong.

**Acceptance:** a deliberate, documented decision — skip, skip-unless-workspace-member, or
leave — with a test pinning the chosen behavior.

---

## Execution order

```
C-001 ──┬── C-004   (hard gate: override only after streaming)
        └── C-005   (extraction after read path is honest)

C-002, C-003, C-006  — independent, parallel with C-001
```

---

## Phase-wide constraints

Per `.kbd-orchestrator/constraints.md` and AGENTS.md:

- No `unsafe`; no `unwrap_used` / `expect_used` / `panic`.
- Determinism: `BTreeMap`/`BTreeSet` or explicit sorting at contract boundaries.
- Bounded work: a limit error stays distinct from an empty result.
- Regression test at the lowest useful layer; contract test where user-visible behavior
  changes.
- `read_snapshot` and the limit error text are compatibility-sensitive surfaces.

---

## Open questions still unresolved

1. **Identity/namespacing for shared record keys** (blocks C-005 adoption, not extraction).
2. **Payload composition** — which node/edge classes dominate 2 GiB (blocks C-003
   calibration).
3. **Untested:** whether `--exclude` on `universal-agent-runtime` lands under 2 GiB. With
   4,827 markdown files under `crates/` it may not — flagged since assessment, still
   unverified.

---

## Adversarial review

Preflight `ok`; judge `k3` ≠ generator `kbd-frontier`.

**CRITICAL — incorporated:** analyze assumed change A required a broad streaming refactor
across unknown consumers. The audit refutes that: no production caller uses the bytes, and
`compass-query` already uses an indexed reader. Specced as an API split, not a rewrite.

**WARNING — carried to plan:**
- C-005 as directed yields a small extraction, not current-graph partitioning. Recorded
  plainly so the delivered scope is not mistaken for the larger capability.
- The `compass-partition` dependency rule is a stop-and-report gate, not a preference.
- C-002 must not advertise an override before C-004 exists.
- Three open questions remain unmeasured; confidence in direction is high, in sizing low.
