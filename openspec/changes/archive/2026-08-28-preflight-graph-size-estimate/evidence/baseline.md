# C-003 baseline evidence

Measured 2026-08-28 with the current `compass-cli` debug binary.

## Real checkout attempt

- input: read-only external `universal-agent-runtime` qualification checkout
- current disk footprint: 16 GB
- result: worker stack overflow before graph publication
- elapsed: 73.82 s real, 71.50 s user, 18.86 s system

This checkout no longer reproduces the isolated 2 GiB publication failure, so
it was not used for estimator calibration.

## Deterministic synthetic equivalent

- sources: 2,000 Rust files, one deterministic function per file
- source bytes: 112,893
- canonical graph bytes: 6,843,452
- graph: 4,000 nodes, 2,000 edges
- elapsed: 1.61 s real
- composition: graph metadata/inventory 2,065,352 bytes; nodes 3,122,253
  bytes; edges 1,655,788 bytes
- observed expansion: 60.62 canonical bytes per source byte
- implementation calibration: 60x
- regression error ceiling: 322 ms (20% of 1.61 s)
