# Refinement log — C-012

## Iteration 1 — 2026-08-28

- Defined a versioned engine-neutral vector with stable IDs, parallel direction,
  provenance, confidence, generations, and bounded pagination.
- Built one disposable SurrealDB 3.2.4 runner with mutually exclusive persistent
  SurrealKV and RocksDB feature profiles.
- Corrected the crash harness so baseline publication happens in a separate
  process; this ensures the storage handle is closed before the killed writer
  opens the database.
- Retained exact per-engine semantic/resource results and official license bytes.
- Verified engine equivalence, checksums, license identity, Compass dependency
  cleanliness, and complete disposal of the spike.

The installed artifact-refiner adapter lacks its referenced canonical
controllers, schemas, and validator. The repository's deterministic fallback
record format was used, and its JSON documents are syntax-validated.

Overall after refinement: PASS.

## Iteration 2 — adversarial findings

- Retained the exact scale-edge expansion rule recovered from the executed
  disposable runner.
- Added independent expected ordered-ID and expanded-relation digests.
- Added per-engine post-crash active generation, relation count, and ordered-ID
  digest evidence.
- Recorded the exact pre/post Compass manifest and lockfile checksum-ledger
  digest.
- Verified the review's link suggestion points to the existing C-011 decision
  record.

The isolated cross-model review verdict remains PASS with no critical finding;
all warnings are resolved in the retained artifacts.

## Final qualification

- All retained JSON and checksum-manifest entries validate.
- The official tagged license comparison, product-boundary gate, OpenSpec
  strict validation, and scoped whitespace check pass.
- Final graph refresh indexed 713 files, 121,261 nodes, 285,681 edges, and
  3,389 communities with zero identity collisions; 68 invalid edges remain
  explicitly omitted by the existing partial-publication boundary.

## Post-archive provenance revalidation — 2026-08-29

- Normalized the OpenSpec source-artifact path to the dated archive location.
- Kept the probe disposition historical to its 2026-08-28 completion date; it
  does not incorporate or depend on C-011's later acceptance outcome.
- Revalidated every source/dist path, every PASS constraint, and every retained
  fixture checksum after the provenance correction.

Post-archive provenance validation: PASS.
