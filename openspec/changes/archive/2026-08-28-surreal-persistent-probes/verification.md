# Verification: SurrealDB persistent-engine probes

## Result

PASS for C-012's disposable persistent-engine semantic, recovery, measurement,
license-capture, and cleanup contract. This does not approve the BSL profile or
ratify performance budgets; C-011 and C-013 remain separate gates.

## Requirement evidence

- `input-v1.json` and `expected-v1.json` define 2,051 directed relations with
  stable logical IDs, a same-endpoint parallel pair, a reverse edge, exact
  provenance/confidence, two generations, and 17 deterministic pages.
- Feature-isolated SurrealDB 3.2.4 builds selected SurrealKV without RocksDB and
  RocksDB without SurrealKV, using distinct non-workspace target directories.
- Both persistent engines emitted byte-identical canonical semantic summaries.
  Candidate writes left `g-0001` active until the final activation pointer
  changed to `g-0002`.
- For each engine, the writer was terminated after one durable candidate batch
  and before activation. Reopen preserved the complete active `g-0001` result.
- Clean build, executable size, dependency, workload, cold-start, and RSS values
  are retained as descriptive host baselines, not performance claims.
- The retained BSL file is byte-identical to the official `v3.2.4` license and
  its pinned SHA-256 is verified.
- The disposable Cargo project, targets, databases, and raw logs were deleted.
  Compass manifests and `Cargo.lock` stayed byte-identical and contain no
  SurrealDB package.

## Verification performed

- Strict JSON/schema assertions for every retained vector and result: passed.
- Manifest SHA-256 verification for every retained file: passed.
- Independent ordered-ID SHA-256 derivation: matched both engine results.
- Independently expanded all 2,051 relations from the retained rule; canonical
  compact JSON SHA-256 matched the pinned expected value
  `749489773e9e0329852a9e025222722af46f68bb1cfb3f4c647e17cdff9b2261`.
- Retained post-crash count and ordered-ID digest for both engines: matched the
  complete active `g-0001` expectation.
- Canonical `.semantic` comparison across SurrealKV/RocksDB: byte-identical.
- Official tagged license byte comparison: identical; SHA-256
  `98a94ac615f88370865016487b436fa404560910bd329794ed7502277a94b805`.
- Pre/post Compass manifest and `Cargo.lock` checksum ledgers: identical. Each
  ledger contains 33 sorted manifest hashes plus the lockfile hash and has
  SHA-256 `a98ca39857724b88a12a306b6647299ff93fed69f4c881bee54fddc230b45e6c`.
- Workspace dependency search for `surrealdb`/`surrealdb-core`: no match.
- `openspec validate surreal-persistent-probes --strict`: passed.
- `git diff --check` for the change surface: passed.
- `scripts/check_product_boundary.sh`: passed.
- `compass update .`: passed with 713 files, 121,261 nodes, 285,681
  edges, 3,389 communities, 68 omitted invalid edges, and zero identity
  collisions.

## Qualification status

Deterministic artifact refinement uses the repository's established fallback
record because the installed artifact-refiner adapter lacks its referenced
canonical controllers and schema assets. Mandatory adversarial review and the
final graph refresh are recorded before archive.

The isolated `k3` review passed with no critical findings. Its four warnings
were resolved by retaining the exact scale-edge rule, independent expected
digests, post-crash count/digest evidence, and the manifest/lock checksum-ledger
digest. Its link suggestion was verified against the existing C-011 decision
record at `docs/future/surrealdb-license-decision.md`.

The workspace Rust clippy/test baseline was already green immediately before
C-012 and remains applicable because this change adds no Rust source,
dependency, manifest, lockfile, or shipped product behavior.
