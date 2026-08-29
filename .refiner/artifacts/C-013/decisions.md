# Refinement decisions — C-013

- Kept the research thresholds unchanged and recorded their source hashes before
  considering any future Surreal projection result.
- Used streaming graph emission and metadata-only full logical iteration so the
  million-node profile is reproducible without checking in a multi-gigabyte graph.
- Pinned full node-record and edge-record digests rather than relying only on a
  small sample or generator source hash.
- Made the retained benchmark manifest closed: every retained file below
  `benchmarks/qualification/` other than the manifest itself must be listed.
- Required exactly 30 balanced tasks and verified all expected evidence through
  the bounded raw-traversal denominator on the exact medium graph.
- Preserved process-level current-engine measurements as host-specific evidence,
  not universal performance claims or future-engine verdicts.
- Kept generated graphs, caches, logs, and binaries disposable under `target/`.
- Kept KBD-required `.refiner/artifacts/C-013` receipts as project QA evidence;
  these are distinct from generated product data and local `.compass/` state.
