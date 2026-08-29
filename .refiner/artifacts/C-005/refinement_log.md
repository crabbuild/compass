# Refinement log — C-005

## Iteration 1 — 2026-08-28

- Extracted the storage-neutral partition container, canonical JSON encoder,
  and typed-key constructors into `compass-partition`.
- Preserved Prolly's segment encoding byte-for-byte with local golden vectors,
  without bringing Prolly or history's IR/analysis dependencies into the crate.
- Kept `compass-history` public helpers source-compatible and converted the new
  typed `PartitionError` at the history boundary.
- Verified unchanged history canonical, diff, round-trip, publication, reopen,
  maintenance, and SQLite contract suites.

Focused tests, affected Clippy, dependency gates, workspace formatting/Clippy/
lib-bin tests, package metadata, packaging, product boundary, strict OpenSpec,
diff checks, and graph refresh pass. The installed artifact-refiner package
lacks its canonical controllers, schemas, and validator agent, so the documented
deterministic fallback was used.

Overall: PASS.

## Iteration 2 — 2026-08-28

The first fresh-context review blocked on an absent verification record. Added
the exact command/results record, reran focused QA, and rebuilt the packet. The
second review passed with two warnings: the direct internal dependency was moved
to the plan-required workspace declaration, and cumulative changelog provenance
from completed C-001 through C-004 was acknowledged and retained.
