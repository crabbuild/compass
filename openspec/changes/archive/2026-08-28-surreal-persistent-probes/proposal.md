## Why

SurrealDB's in-memory examples cannot establish whether its persistent embedded
engines preserve Compass identities, relations, generation activation, recovery,
ordering, and resource bounds. Disposable SurrealKV and RocksDB probes must
produce that evidence before any production adapter is considered.

## What Changes

- Version deterministic graph and recovery test vectors plus their expected
  canonical results.
- Build an external throwaway Rust probe pinned to SurrealDB 3.2.4 and run the
  same workload against persistent SurrealKV and RocksDB.
- Record semantic, dirty-shutdown, ordering, pagination, dependency, build-time,
  binary-size, cold-start, peak-RSS, and license evidence.
- Delete the spike and its build outputs, retaining only reviewable vectors and
  results; leave no SurrealDB workspace dependency.
- Record an explicit pass/fail gate for later SurrealDB waves.

## Capabilities

### New Capabilities

None. This is a disposable evaluation and evidence change, not shipped product
behavior; `.openspec.yaml` opts out of delta specs.

### Modified Capabilities

None.

## Impact

The retained artifacts live under `scripts/fixtures/surreal-persistent-probes/`
and `docs/future/`. Temporary code and Cargo output remain outside the workspace
and are removed after measurement. A failed required dimension cancels C-014,
C-015, and the Surreal branch of C-020 without changing the current stack.
