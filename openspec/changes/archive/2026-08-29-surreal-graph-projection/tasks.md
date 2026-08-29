## 1. Contract and Dependency Boundary

- [x] 1.1 Add the `compass-graphdb-surreal` workspace crate, exact optional SurrealDB 3.2.4 dependency, engine feature matrix, license disclosure, and feature-off dependency gate; verify the crate builds with default features and `cargo tree` contains no Surreal package for default Compass binaries

## 2. Deterministic Projection Core

- [x] 2.1 Implement validated generation planning, deterministic record keys and ordering, exhaustive typed relation-family mapping, exact payload preservation, and typed errors; verify unit/property-style tests cover invalid inputs, determinism, every edge kind, parallel edges, reverse direction, self-loops, provenance, confidence, and injection-shaped identities

## 3. Generation-Atomic Surreal Runtime

- [x] 3.1 Implement schemafull static statements, parameter-bound Mem/SurrealKV/RocksDB constructors, transactional stage/validate/activate, active-generation reads, and cancellation failure injection; verify Mem integration tests prove successful round-trip and that interrupted writes leave the previous generation visible

## 4. Documentation and Qualification

- [x] 4.1 Update workspace and integration documentation, run targeted/default/feature-enabled tests plus formatting, lint, product-boundary, strict OpenSpec, footprint, and graph-refresh gates; then refine, adversarially review, verify, and archive C-014
