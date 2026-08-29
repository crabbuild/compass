---
type: SessionRecord
id: karpathy-session-1ecfe43654c6f90b
title: Karpathy session 1ecfe43654c6
tags:
- karpathy
- session-learning
sources:
- session:626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
timestamp: 2026-08-09T12:55:13.508858+00:00
created_at: 2026-08-09T12:55:13.508858+00:00
updated_at: 2026-08-09T12:55:13.508858+00:00
revision: 0
---

## Delta

SurrealDB 3.2.4 confirmed. Here's my honest read.

## The architecture already anticipates this

`compass-store` is documented as **"Backend-neutral, namespace-scoped storage"** with a `Store` trait, and it explicitly says: *"Other backends can implement `Store` without importing Compass graph records or query code"* and *"A remote adapter may put a bounded blocking boundary around its client."* `compass-store-redb` (869 lines) already proves a second backend works. So a `compass-store-surrealdb` adapter is a **supported extension point**, not a fight with the design.

## But the framing needs splitting — the two halves have opposite answers

**Storage backend: plausible, and cleanly so.** SurrealDB embedded (SurrealKV/RocksDB) with a remote option maps onto the existing trait. Your "embedded with an option for remote" instinct matches what the crate doc already contemplates.

**Vector support: this is the part I'd push back on.** It's not a missing feature — it's a *deliberate exclusion*, stated identically in `AGENTS.md:65` and `README.md:89`:

> Structural extraction and graph queries must continue to work without Python, model credentials, **embeddings, a vector database**, runtime grammar downloads, or Graphify.

That's a product invariant, and it's the differentiator in the README's own positioning against Graphify. Vector search isn't absent because nobody got to it; it's absent because deterministic structural evidence with provenance is the claim. Grepping confirms no embedding code exists anywhere.

Adding vectors *as an optional layer* wouldn't violate the letter of the invariant. But it changes what Compass is, and that's your call to make explicitly rather than as a side effect of picking a storage backend.

## Where it doesn't help

For the problem in front of us — a 2 GiB canonical payload — SurrealDB wouldn't fix it. The failure is `digest_json` capping serialization of one monolithic `CanonicalGraphDocument`, plus a read path that materialized the whole thing. That's Compass's own encoding, above any storage layer. Swapping SQLite for SurrealDB leaves it exactly where it is.

There's also real cost: SurrealDB pulls a large dependency tree into a product whose stated identity is "one native Rust executable," and the `Store` trait is **synchronous** while SurrealDB is async — you'd need that bounded blocking boundary the docs mention, which is doable but not free.

## My recommendation

Don't fold this into the current phase. C-001 is half-implemented (two compile errors away from tests running), and the sequencing gate on C-004 still holds regardless of backend.

If you want to pursue it, I'd scope it as its own phase with the two questions separated:

1. **`compass-store-surrealdb` adapter** — evaluated against `compass-store-redb` as the precedent. Bounded, reversible, fits the design.
2. **Vector/embedding support** — a product-direction decision that needs `AGENTS.md` and `README.md` amended first, since both currently promise its absence.

Want me to finish C-001 first, or park it and run `/kbd-new-phase` on the SurrealDB question now? I'd finish C-001 — it's close, and it's the actual fix for the failure that started this.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
- Captured: 2026-08-09T12:55:13.042640Z
- Project: /Users/gqadonis/Projects/references/compass

## Changed Paths

- .prometheus/knowledge/wiki/index.md
- .prometheus/knowledge/wiki/log.md
- crates/compass-store/src/lib.rs
- .prometheus/knowledge/wiki/karpathy-session-e0ee5520d0620fb6.md
