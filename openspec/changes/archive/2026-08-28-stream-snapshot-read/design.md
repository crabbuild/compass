## Context

See `proposal.md` for motivation and `specs/snapshot-streaming/spec.md` for the behavior contract. The SQLite snapshot format stores a validated catalog manifest and bounded immutable chunks. The existing `read_snapshot` reconstructs all chunks into one `Vec`; its only production consumers discard those bytes. The query engine already uses indexed graph snapshot readers and is outside this change.

The current serialized format, digest algorithm, snapshot identifiers, 2 GiB default bound, synchronous store contract, and public `read_snapshot` signature are compatibility constraints.

## Goals / Non-Goals

**Goals:**

- Make metadata access constant-sized and integrity validation chunk-bounded.
- Preserve full-read compatibility for tests and any external consumers.
- Preserve corruption detection for both validation and reference generation.
- Keep every chunk and accumulated byte count bounded and deterministic.

**Non-Goals:**

- Changing the snapshot serialization or active-selector publication protocol.
- Raising or making the maximum graph size configurable; that is C-004.
- Partitioning the current graph or changing query-engine read paths.
- Introducing async I/O or a new dependency.

## Decisions

### Separate metadata, streaming, and collecting APIs

`read_snapshot_manifest` reads and validates only the active catalog value. `read_snapshot_chunks` captures that manifest once, visits exact chunk keys in index order, maintains a running byte count and SHA-256 digest, and calls a fallible consumer for each chunk. The existing `read_snapshot` collects those callbacks into a `Vec` and remains the only API that intentionally materializes the payload.

Alternative considered: change `read_snapshot` to return an iterator. Rejected because it would break the public signature and force migration despite no production consumer needing the payload.

### Preserve reference integrity with streaming validation

`validate_snapshot` and `snapshot_reference` use the streaming verifier and retain the returned manifest. A manifest-only reference implementation would be cheaper but would change corrupt-snapshot behavior by accepting missing or modified chunks, violating the KBD acceptance criterion.

Alternative considered: read the manifest, then separately call validation. Rejected because two active-selector reads can observe different generations and do redundant work. One streaming call pins the identity being verified.

### Keep the stored-chunk and aggregate guards

Chunks remain bounded by the existing value-size limit. Before selecting a chunk BLOB, the SQLite read path requires the `blob` storage class, probes `length(value)`, and rejects rows above `CHUNK_BYTES`; the bounded follow-up query repeats the type and length predicates so a changed row cannot be materialized above the limit. SQLite can determine a BLOB's byte length without reading its complete contents. A defensive Rust byte-length check preserves the contract across adapter conversion behavior. The stream then uses saturating accumulation, rejects totals above the graph bound, compares the final length to the manifest, and compares the final digest. It never trusts `payload_bytes` as an allocation request.

Alternative considered: preallocate from `payload_bytes` in the compatibility collector. Rejected because a corrupt manifest could again drive a multi-gigabyte allocation before any chunk evidence is read.

The compatibility collector instead preallocates only after `read_snapshot_manifest` validates the bounded manifest, repeats the `MAX_GRAPH_BYTES` check at the allocation site, then streams that same captured manifest so allocation sizing and chunk identity cannot race across active-selector generations.

## Risks / Trade-offs

- [Public callback API can be misused by a consumer that retains every chunk] -> Document that streaming bounds the store's allocation, while consumer-owned retention remains the consumer's responsibility.
- [Chunks are delivered before terminal length and digest validation] -> Document them as provisional and require irreversible consumers to stage work until successful return or independently verify before commit.
- [Reference generation still reads all chunks to preserve prior corruption semantics] -> Accept the I/O cost; the defect is payload-sized memory, not the bounded verification work.
- [A test that checks only manifest success could miss payload corruption] -> Keep separate corrupt-manifest, missing-chunk, bad-length/digest, and reference-rejection scenarios.
- [Writable-store concurrency could advance the active selector] -> Capture one manifest per streaming operation and address immutable chunks by its snapshot ID.

## Migration Plan

Land the additive APIs and switch internal production callers in one change. The existing signature and serialized formats remain unchanged, so rollback is a source-level revert and users have no migration action.
