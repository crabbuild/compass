# Design: bounded preflight graph estimate

## Baseline-first calibration

The original incident recorded a 331.90 s late failure on a 6.8 GB checkout.
The available checkout has since grown to 16 GB and currently fails earlier
with a worker stack overflow after 73.82 s, so it cannot isolate snapshot size.
The approved synthetic equivalent uses 2,000 deterministic Rust sources. Its
112,893 source bytes produced 6,843,452 canonical bytes in 1.61 s, an observed
60.62x expansion. The implementation rounds this down to 60x.

The measured canonical composition was:

- graph metadata and file inventory: 2,065,352 bytes (30.2%);
- nodes: 3,122,253 bytes (45.6%);
- edges: 1,655,788 bytes (24.2%).

## Estimator

After discovery and language filtering, Compass performs one metadata lookup
per source. An admitted source contributes `max(512, byte_size * 60)` plus its
root-relative path length. A missing, non-file, or source above
`max_source_bytes` contributes 512 bytes plus its relative path because Compass
publishes it as inventory-only partial coverage rather than parsing it.

All arithmetic saturates. No source content is read and no parser or resolver
runs. Root-relative paths keep equivalent inputs deterministic across checkout
locations. Exceeding the bound returns
`SnapshotError::canonical_graph_too_large`, preserving the typed `Limit`
classification and C-002 remediation.

## Timing contract

The synthetic full-build baseline is 1.61 s. The regression pins a 322 ms
ceiling, exactly 20% of that baseline, for the estimation failure itself.
