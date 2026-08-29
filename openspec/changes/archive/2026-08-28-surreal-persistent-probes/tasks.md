## 1. Reproducible Probe Surface

- [x] 1.1 Add versioned deterministic input and expected-output vectors covering parallel directed relations, stable IDs, provenance, confidence, generations, and pagination; verify vector schema and canonical bytes
- [x] 1.2 Create a disposable non-workspace SurrealDB 3.2.4 runner with feature-isolated SurrealKV and RocksDB builds; verify the temporary manifests never modify Compass manifests or lockfile

## 2. Persistent Engine Qualification

- [x] 2.1 Run the complete persistent workload and dirty-shutdown recovery probe on SurrealKV; record semantic and recovery pass/fail evidence plus measurements
- [x] 2.2 Run the identical workload and dirty-shutdown recovery probe on RocksDB; record semantic and recovery pass/fail evidence plus measurements

## 3. Disposition and Completion

- [x] 3.1 Compare engine outputs, capture the exact pinned license evidence, delete all spike code/build outputs, and record the Wave 5 pass/fail disposition while verifying the Compass dependency graph remains clean
- [x] 3.2 Validate and refine the retained artifacts, perform mandatory adversarial review, synchronize any applicable documentation, and archive the OpenSpec change
