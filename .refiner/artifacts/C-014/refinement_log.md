# Refinement log — C-014

## Iteration 1 — 2026-08-29

- Added the feature-isolated `compass-graphdb-surreal` workspace crate with the
  exact reviewed SurrealDB 3.2.4 dependency and Mem, SurrealKV, and RocksDB
  profiles.
- Implemented deterministic canonical generation planning, schemafull typed
  record families, lossless payload preservation, stable record identities,
  and exhaustive `EdgeKind` mapping.
- Implemented single-transaction candidate staging, exact identity validation,
  immutable manifests, active-generation pointers, idempotent reactivation, and
  injected cancellation.
- Added pure projection tests and real Mem runtime coverage for successful
  round-trip, parallel/reverse/self-loop semantics, idempotence, interruption,
  and later successful activation.
- Added dependency-tree and default-binary closure gates, license policy records,
  integration documentation, and strict OpenSpec artifacts.

Overall after deterministic refinement iteration 1: PASS; bounded-surface audit
and final repository qualification required.

## Iteration 2 — bounded-surface closure

- Found that the first full-generation reader trusted manifest counts but used
  unrestricted set selects, contradicting the change's explicit bounded-result
  requirement.
- Added positive `ProjectionLimits` for nodes, relations, and serialized bytes;
  defaults match the ratified large qualification graph and canonical
  `GraphDocument` reader byte cap.
- Enforced the limits before opening activation transactions and before reads,
  added query-side `LIMIT` clauses to every identity and record set query, and
  required materialized counts and bytes to match the immutable manifest.
- Added a real Mem regression proving an over-limit plan fails before any active
  generation is published, plus pure default/invalid-limit tests and static query
  surface assertions.
- Re-ran all targeted engine profiles and the complete repository baseline. The
  final graph refresh published the current graph while retaining its explicit
  68-edge partial warning.

Overall after deterministic refinement iteration 2: PASS; isolated adversarial
review required.

## Iteration 3 — first adversarial findings

- The first full packet was contaminated by cumulative, already accepted C-004,
  C-005, and C-010 hunks in shared workspace files; its sole critical finding
  concerns review scope rather than C-014 code. The corrected packet isolates
  exact C-014 additions and shared-file evidence without discarding those user
  changes.
- Hardened idempotent reactivation to compare schema/source digests and re-read
  the complete candidate identity sets before moving the active pointer.
- Added composite repository/generation/identity indexes so query result limits
  also have an indexed access path across retained generations.
- Defined `InterruptAfter(0)` as a pre-mutation interruption and added a real Mem
  regression for the zero-mutation outcome.
- Removed a vacuous byte-comparison claim from the feature-isolation script. The
  authoritative proof is the complete default dependency closure; `--binary`
  now builds the default binary from that already verified closure.
- Clarified that the projection's 1 GiB default is the canonical `GraphDocument`
  reader bound, distinct from the separately configurable store publication cap.

Overall after deterministic refinement iteration 3: PASS; fresh scoped
adversarial review required.

## Iteration 4 — PASS-review follow-up

- Preserved the original staging failure and the transaction-cancellation
  failure together in a dedicated typed error instead of masking the actionable
  root cause.
- Added a real Mem activation/read round trip using a repository identity that
  contains SurrealQL metacharacters, proving end-to-end that the identity remains
  bound data.
- Moved the cumulative `docs/README.md` hunk into the same exact shared-file
  evidence boundary as the workspace manifest, lockfile, and changelog; the
  adjacent MCP row remains attributed to its already reviewed C-010 change.

Overall after refinement iteration 4: PASS; final fresh-context review required.

## Iteration 5 — final PASS-review closure

- Corrected the limits documentation: node count, relation count, and serialized
  bytes are independent ceilings, and C-015—not C-014 prose—determines which
  ceiling binds for each ratified corpus.
- Expanded the default dependency-tree exclusion to every `surrealdb-*` package,
  including collections, protocol, strand, and types-derive.
- Restored `check_product_boundary.sh` to its focused Graphify boundary; the
  Surreal isolation gate remains a separately invoked blocking qualification.
- Made the optional binary-build gate accept the portable `compass.exe` artifact
  path as well as `compass`.
- Retained the final external PASS review and its strict anti-sycophancy score;
  the remaining artifact-refiner fallback and dirty-tree packet observations are
  explicitly disclosed non-blocking tooling limitations.

Overall after refinement iteration 5: PASS; ready for final verification and
archive after the repository baseline is re-run.
