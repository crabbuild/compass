## Why

Current SQLite snapshot validation reconstructs the entire canonical graph payload even when callers only need metadata or integrity verification. On multi-gigabyte repositories this creates an avoidable payload-sized allocation at the same bounded limit that is intended to make failure safe.

## What Changes

- Add a manifest-only snapshot read that validates catalog metadata without touching payload chunks.
- Add a chunk-streaming snapshot read that verifies bounded length and digest while delivering one stored chunk at a time.
- Reimplement production validation and reference generation on the streaming path so both preserve corruption detection without retaining payload bytes.
- Retain the existing full-payload `read_snapshot` API as an explicit compatibility path, implemented on top of streaming.
- Add round-trip, reopen, corruption/interruption, and publication-atomicity regression coverage.
- Document the allocation contract and record the release-visible change.

## Capabilities

### New Capabilities

- `snapshot-streaming`: Bounded manifest-only and chunk-streaming access to current SQLite graph snapshots while preserving full-read compatibility and integrity semantics.

### Modified Capabilities

None.

## Impact

The change is owned by `crates/compass-store` and adds two public methods without removing or changing the existing `read_snapshot` signature. Production validation and snapshot-reference call sites stop allocating proportional to `payload_bytes`; serialized snapshot formats, stable identifiers, and dependency graphs remain unchanged. `CHANGELOG.md` records the additive API and memory-safety improvement; no user migration is required.
