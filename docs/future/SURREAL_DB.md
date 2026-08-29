# Proposal (future evaluation) — SurrealDB as a Compass store backend

> **Status:** unevaluated idea, tabled for later. Nothing here is decided,
> scheduled, or endorsed. Recorded on 2026-08-09 so the question is not lost.
>
> This document deliberately separates two proposals that are easy to conflate.
> They have different risk profiles and, on current evidence, different answers.

---

## The idea

Add a SurrealDB backend (embedded by default, with a remote option) for Compass
graph storage. SurrealDB 3.2.4 is the current release as of this writing; it is
written in Rust, embeds via SurrealKV/RocksDB, speaks a graph data model
natively, and ships vector-index support.

---

## Proposal A — SurrealDB as a `Store` backend adapter

**Assessment: architecturally supported. Worth a real evaluation.**

`compass-store` is already designed for this. Its own crate documentation states:

> Backend-neutral, namespace-scoped storage for Compass current graph snapshots.
> […] Other backends can implement [`Store`] without importing Compass graph
> records or query code.
>
> The v1 trait is synchronous and runtime-neutral. **A remote adapter may put a
> bounded blocking boundary around its client**, while a future async facade can
> preserve these same request, ordering, and error semantics without forcing an
> executor into this contract crate.
>
> — `crates/compass-store/src/lib.rs:3-14`

The portable address is `(namespace, partition, key)`. `compass-store-redb`
(~869 lines) already demonstrates a second backend against this trait, so a
`compass-store-surrealdb` crate would follow an established precedent rather
than requiring new architecture.

The "embedded by default, remote optional" shape matches what that doc comment
already contemplates for remote adapters.

### Open questions for a real evaluation

1. **Sync/async boundary.** The `Store` trait is synchronous; SurrealDB's Rust
   SDK is async. The bounded blocking boundary the docs describe is achievable
   but is real work, and it must not smuggle an executor into the contract crate.
2. **Dependency weight.** Compass's stated identity is "one native Rust
   executable" with "no Python or Graphify dependency" (README). SurrealDB pulls
   a substantial tree. Measure the effect on binary size, build time, and
   `cargo deny` policy before committing.
3. **Determinism.** AGENTS.md requires deterministic identities, ordering, and
   canonical encoding at contract boundaries. Any backend must preserve ordered
   scans and stable iteration; this needs proving, not assuming.
4. **What problem does it solve?** Enumerate the concrete wins over SQLite +
   redb. "It is a graph database" is not by itself a benefit when Compass's graph
   semantics live in `compass-model`/`compass-graph`, above the storage layer.
5. **Qualification.** `compass-store-qualification` exists; a new backend should
   pass the same gates as SQLite before being offered.

### What it does *not* solve

The 2 GiB publication failure that prompted this discussion
(`compass-scoping-and-bounds`) is **not** a storage-backend problem. It
originates in `compass-graph/src/snapshot.rs:3138`, where `digest_json` caps
serialization of one monolithic `CanonicalGraphDocument`, compounded by a read
path that reconstructed the whole payload in memory. Both are Compass's own
encoding, above any store adapter. Swapping SQLite for SurrealDB would leave
that failure exactly where it is.

---

## Proposal B — Vector / embedding support

**Assessment: this is not a missing feature. It is a documented product
exclusion, and adopting it is a product-direction decision — not a side effect
of choosing a storage backend.**

Two places state the same invariant:

> Compass is a native, local-first Rust product. Structural extraction and graph
> queries must continue to work without Python, model credentials, **embeddings,
> a vector database**, runtime grammar downloads, or Graphify.
>
> — `AGENTS.md`, Product invariants

> Structural extraction and graph queries run locally. They do not require
> Python, **embeddings, a vector database**, model credentials, or runtime parser
> downloads.
>
> — `README.md`

A repository-wide search finds no embedding or vector-search code. Its absence
is deliberate, and it is part of how the README differentiates Compass from
Graphify: deterministic structural evidence with provenance, not similarity
scores.

Adding vectors as a strictly optional layer would not violate the letter of the
invariant — the invariant says structural extraction must work *without* them.
But it changes what Compass claims to be, and it would need:

1. an explicit decision to amend `AGENTS.md` and `README.md`, which currently
   promise the absence;
2. a clear account of how similarity results coexist with the "evidence over
   implication" rule (`.impeccable.md`) and the requirement to prefer an explicit
   unresolved/ambiguous result over invented meaning;
3. determinism guarantees, since approximate nearest-neighbour search is
   typically not deterministic across index builds — this may be the hardest
   constraint to satisfy;
4. a story for where embeddings come from without reintroducing the model
   credentials the product invariant excludes.

**Recommendation: evaluate Proposal A on its own merits first. Do not let a
storage-backend decision quietly import a product-direction change.**

---

## Suggested evaluation path, if pursued

1. Run `/kbd-new-phase` scoped to Proposal A only.
2. Benchmark against SQLite and redb using `compass-store-qualification`.
3. Measure dependency weight, binary size, and build time; check `deny.toml`
   policy compliance.
4. Prove determinism of ordered scans and snapshot publication.
5. Treat Proposal B as a separate phase requiring a documented product decision
   before any code.

---

## Provenance

Raised during the `compass-scoping-and-bounds` phase while implementing C-001
(`read_snapshot` API split). Tabled by the user to avoid derailing that change.
The architectural facts above were verified by source inspection at the time of
writing; the SurrealDB version was current per `cargo search`. Nothing here has
been benchmarked or prototyped.
