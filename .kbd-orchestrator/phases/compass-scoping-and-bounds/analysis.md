# Analysis — compass-scoping-and-bounds

**Stage:** Analyze (between Assess and Spec)
**Mode:** stack-specified (Rust workspace, Edition 2024 — no stack discovery needed)
**Date:** 2026-08-09
**Question carried from assessment:** *If a repository genuinely cannot publish under
2 GiB even with correct exclusions, is the right answer graph partitioning/sharding
rather than a larger number?*

**Verdict: No — do not shard. Partitioning already exists and is unused on this path.
The 2 GiB cap is a policy ceiling, not a capacity limit, and one read-path
implementation detail undermines the invariant it claims to uphold.**

---

## The decisive finding

`compass-store/src/lib.rs:44-51` documents the 2 GiB cap with this rationale:

> "The local store is the bounded large-graph path and therefore accepts up to 2 GiB
> **while serving records through indexed scans instead of materializing the whole
> document**."

That claim does not hold on the read path. `load_active_snapshot`
(`compass-store/src/lib.rs:838-853`) does exactly what the comment says it avoids:

```rust
let capacity = usize::try_from(manifest.payload_bytes).unwrap_or(MAX_GRAPH_BYTES);
let mut bytes = Vec::with_capacity(capacity);
for index in 0..manifest.chunk_count {
    ...
    bytes.extend_from_slice(&chunk.value);
```

It pre-allocates up to the full payload size and concatenates every chunk into one
contiguous `Vec<u8>`. Chunking (`CHUNK_BYTES` = 255 KiB, `compass-store/src/lib.rs:60`)
bounds each *stored value*, not the reconstructed document. **A 2 GiB snapshot means a
2 GiB allocation at read time.**

Meanwhile the write path is genuinely streaming. `DigestWriter`
(`compass-graph/src/snapshot.rs:3100-3126`) implements `io::Write`, hashing incrementally
and counting bytes; `serde_json::to_writer` never builds the payload in memory:

```rust
fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
    let next = self.bytes.saturating_add(buffer.len());
    if next > self.maximum { self.exceeded = true; return Err(...); }
    self.hasher.update(buffer);
    self.bytes = next;
    Ok(buffer.len())
}
```

**So the cap is asymmetric with reality:** the write path could stream any size; the
read path allocates the whole thing. The number 2 GiB is not protecting the writer —
it is protecting the reader, but the doc comment claims the opposite. Whatever else
this phase does, that inconsistency must be resolved, because it is the actual
justification for the ceiling.

---

## Partitioning already exists — and is unused here

`compass-history` has a complete partitioned graph representation
(`compass-history/src/artifacts.rs:76-84`):

```rust
pub struct PartitionedGraph {
    pub nodes: Vec<(Vec<u8>, Vec<u8>)>,
    pub edges: Vec<(Vec<u8>, Vec<u8>)>,
    pub hyperedges: Vec<(Vec<u8>, Vec<u8>)>,
    pub analysis: Vec<(Vec<u8>, Vec<u8>)>,
    pub metadata: Vec<(Vec<u8>, Vec<u8>)>,
    pub program_facts: Vec<(Vec<u8>, Vec<u8>)>,
    pub program_summaries: Vec<(Vec<u8>, Vec<u8>)>,
}
```

with `partition()`, `into_partition()`, and `reconstruct()`
(`artifacts.rs:168-181`, `341-345`). It splits a graph by *record class* and keys each
record individually — precisely the shape a bounded publication path needs.

A grep across all crates for `PartitionedGraph` / `into_partition` outside
`compass-history` returns **nothing**. The current-graph publication path
(`compass-graph::snapshot` → `compass-store`) does not use it. Compass therefore
already owns the mechanism the assessment proposed building, applied to historical
realizations but not to `compass init`.

**This reframes the question.** It is not "should we build partitioning?" but "why
does the current-graph path serialize one monolithic `CanonicalGraphDocument`
(`snapshot.rs:2124-2131`) when a record-partitioned path already exists next door?"

---

## Research: sharding vs partitioning (firecrawl, tiers 1–4)

Tier 1 (`gh`-equivalent code search) and internal inspection answered the primary
question, so tier 4 was used only to validate the architectural direction. Budget:
well under `max_queries_per_tier` (8) and `max_minutes` (20).

The literature is consistent and points *away* from sharding for this problem:

1. **Sharding is for multi-machine scaling — not applicable.** PuppyGraph: sharding's
   goal "is no longer organizing data on one machine but scaling past the limits of any
   one machine, in storage, in write throughput, and in total query capacity."
   Compass is explicitly a **local-first single-process product** (AGENTS.md product
   invariants). There is no second machine. Sharding solves a problem Compass does not
   have and would import distributed-query costs for nothing.

2. **Partitioning is the correct tool at a fixed ceiling.** Same source: "Partitioning
   lowers the cost of operating under a fixed ceiling and is nearly free operationally;
   sharding raises the ceiling itself but introduces a class of distributed-query costs."
   Compass wants exactly the former — it *wants* the ceiling to stay.

3. **Graph partitioning is uniquely hard and should be avoided when possible.**
   PuppyGraph: "the inherent interconnectivity of graph data complicates the division of
   the graph across machines while minimizing the edges that span partitions...
   especially problematic for nodes with numerous relationships." Compass's existing
   `PartitionedGraph` sidesteps this entirely by partitioning **by record class, not by
   graph topology** — no cut edges, no cross-partition traversal penalty. That is the
   right design and it is already written.

4. **Azure Architecture Center:** "Prefer many small shards over few large ones," and
   use the pattern only when "total data volume exceeds the storage capacity of a single
   database instance, and no vertical scaling option addresses the shortfall." Neither
   condition holds — a 2 GiB graph fits comfortably on disk; the constraint is a
   self-imposed in-memory reconstruction.

5. **Citus** notes sharding a graph is "perhaps the most unique approach," yielding
   "multiple copies for your data sharded in different ways, eventual consistency...
   and some application logic you have to map to your sharding strategy." Eventual
   consistency is flatly incompatible with Compass's determinism invariant.

**Conclusion from research:** sharding is rejected on architectural grounds. Streaming
the read path is the intervention that matches both the literature and Compass's
constraints.

---

## Candidate approaches evaluated

### A — Stream the read path (recommended)

Replace the `Vec::with_capacity(payload_bytes)` concatenation in
`load_active_snapshot` with a chunk-streaming reader, so reconstruction never
materializes the full document. This makes the store's doc comment *true*.

- **Pros:** removes the actual memory constraint; no new abstraction; no contract or
  format change; the cap becomes a genuine policy bound rather than an allocation
  guard; unblocks any decision about the ceiling on honest terms.
- **Cons:** every consumer of `load_active_snapshot` must accept a streaming/iterator
  interface or the reconstruction must be pushed down — needs a consumer audit.
- **Owner:** `compass-store`, with `compass-graph` adjusting to the new read shape.
- **Risk:** medium — touches a public boundary. Requires round-trip, reopen, and
  corruption/interruption coverage per AGENTS.md history/storage test policy.

### B — Route current-graph publication through `PartitionedGraph` (recommended, sequenced after A)

Reuse `compass-history`'s existing record-class partitioning for the current-graph
path instead of one canonical JSON document.

- **Pros:** the mechanism exists and is already tested for historical realizations;
  partitions by record class so no edge-cut problem; naturally bounded per record.
- **Cons:** `compass-history` currently owns it — moving or sharing it is an ownership
  decision (AGENTS.md routes graph publication to `compass-graph`, immutable history to
  `compass-history`). The two paths have different identity/fingerprint semantics that
  must not be conflated.
- **Owner:** requires an explicit ownership call before implementation.
- **Risk:** medium-high — do not attempt before A; the ownership question is real.

### C — Honor `COMPASS_MAX_GRAPH_BYTES` on the publication path (assessment G1)

- **Pros:** small, resolves the documented inconsistency (4 crates honor it, 2 do not),
  immediately unblocks the reported failure.
- **Cons:** **unsafe until A lands.** Raising the cap today raises a real allocation.
  Granting an override that lets a user request a 4 GiB contiguous `Vec` is a worse
  failure than the current clean error.
- **Sequencing:** must follow A. This is the single most important ordering constraint
  in this analysis.

### D — Shard the graph across stores — **rejected**

Rejected on all five research points above, and on AGENTS.md grounds: local-first
single-process product, determinism required, no eventual consistency permitted.

### E — Raise the default cap — **rejected**

AGENTS.md: work must remain bounded and "a limit error is not an empty result."
Raising the default weakens an intentional invariant without addressing the read-path
allocation. Rejected in assessment; re-affirmed here.

---

## Recommended sequence

| Order | Change | Gap | Owner | Rationale |
| --- | --- | --- | --- | --- |
| 1 | Stream `load_active_snapshot`; correct or satisfy the doc comment | new (A) | `compass-store` | Removes the real constraint; prerequisite for everything else |
| 2 | Make the limit error actionable | G2 | `compass-graph`, `compass-cli` | Cheap, independent, immediately useful |
| 3 | Pre-flight size estimate — fail before 331 s | G3 | `compass-core` | Independent of 1; large UX win |
| 4 | Honor `COMPASS_MAX_GRAPH_BYTES` on publication | G1 | `compass-graph`, `compass-store` | **Only after 1** |
| 5 | Decide `vendor/` skip policy | G4 | `compass-files` | Independent; needs a deliberate call |
| 6 | Evaluate routing current-graph publication through `PartitionedGraph` | — | TBD | Needs the ownership decision first |

Items 2, 3, and 5 are independent of the partitioning question and can proceed in
parallel with 1.

---

## Open questions for Spec/Plan

1. **Ownership (blocks item 6):** `PartitionedGraph` lives in `compass-history`, but
   AGENTS.md routes current-graph publication to `compass-graph`. Does the partitioning
   primitive move to a shared crate, get duplicated with distinct identity semantics, or
   does `compass-graph` depend on `compass-history`? This is an architecture decision,
   not an implementation detail.
2. **Consumer audit (blocks item 1):** who calls `load_active_snapshot`, and can they
   accept a streaming interface? Not yet enumerated.
3. **Unmeasured:** which node/edge classes dominate the 2 GiB payload in the failing
   repo. Without this, item 3's estimator has no calibration data.
4. **Untested:** whether `--exclude` on `universal-agent-runtime` lands under 2 GiB.
   With 4,827 markdown files under `crates/` — undeletable without gutting the graph —
   it may not. Still unverified, as flagged in the assessment.

---

## Adversarial review

Preflight `status: ok` (judge `k3`, critic `MiniMax-M3`, generator `kbd-frontier`;
2 distinct models, judge ≠ producer).

**CRITICAL — raised and incorporated:** the original framing ("should we build
partitioning?") would have produced a duplicate of `PartitionedGraph`. Refuted by
grep — the primitive exists in `compass-history` and simply is not used on this path.
Second CRITICAL: recommending option C (honor the override) *before* A would let a
user request a multi-gigabyte contiguous allocation. Sequencing corrected; C is now
explicitly gated behind A.

**WARNING — carried to Spec:**
- The `compass-store` doc comment is materially inaccurate about its own read path.
  Whether the fix is the code or the comment, do not leave both as they are.
- Item 6 must not start before the ownership question is answered.
- Three of four open questions are unmeasured/untested. Confidence in the *direction*
  is high (it rests on source inspection and convergent literature); confidence in
  *sizing* any of this work is low.

**Sycophancy check:** this analysis rejects the very framing the phase goal proposed
(partitioning as the answer). The honest result is "the mechanism already exists, and
the real defect is a read-path allocation nobody documented." Recorded as such rather
than dressed up as a new feature.

---

## Evidence index

- `crates/compass-store/src/lib.rs:44-51` — cap rationale (claims non-materialization)
- `crates/compass-store/src/lib.rs:838-853` — read path that *does* materialize
- `crates/compass-store/src/lib.rs:39,60` — `MAX_VALUE_BYTES`, `CHUNK_BYTES`
- `crates/compass-graph/src/snapshot.rs:3100-3126` — streaming `DigestWriter`
- `crates/compass-graph/src/snapshot.rs:2124-2131` — monolithic canonical document
- `crates/compass-graph/src/snapshot.rs:39` — `GRAPH_SNAPSHOT_MAX_ITEMS` = 5,000,000
- `crates/compass-history/src/artifacts.rs:76-84,168-181,341-345` — `PartitionedGraph`
- puppygraph.com/learn/sharding-vs-partitioning — partitioning vs sharding at a ceiling
- puppygraph.com/blog/when-to-use-graph-database — graph partitioning difficulty
- learn.microsoft.com/azure/architecture/patterns/sharding — when sharding applies
- citusdata.com/blog/2017/08/28/five-data-models-for-sharding — graph sharding costs
