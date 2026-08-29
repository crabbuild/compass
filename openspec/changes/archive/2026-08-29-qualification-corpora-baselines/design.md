## Context

The research and phase plan precommit exact scale profiles and comparison budgets.
C-013 converts them into deterministic inputs and a recorded decision before Wave
5 uses any Surreal projection measurement as acceptance evidence.

## Goals / Non-Goals

**Goals:**

- Generate exact 100,000/250,000 and 1,000,000/2,500,000 typed graph profiles
  without unbounded in-memory construction.
- Preserve deterministic identities, direction, parallel edges, source evidence,
  ordering, samples, and digests.
- Compare graph-aware tools with a bounded raw-traversal baseline on 30 fixed tasks.
- Reuse the shipped umbrella/focused-skill trigger corpus while freezing explicit
  compatibility subsets.
- Record host, binary, graph, command, sample, p95, and peak-RSS evidence.

**Non-Goals:**

- Ship a new product benchmark command or add a runtime dependency.
- Generate and commit the multi-gigabyte large graph.
- Treat a single-host baseline as a universal performance claim.
- Adjust thresholds after observing a future Surreal projection result.

## Decisions

### Streaming fixture generation

The generator writes one canonical JSON document incrementally. It holds only
bounded record templates and digest state, not the complete node/edge set. A
metadata-only mode exercises the complete ID/edge schedule and digest derivation
without producing a large artifact.

### Deterministic graph topology

Nodes have stable ordinal identities. Every non-root node has a chain `calls`
edge, remaining edges are deterministic forward chords, and a fixed parallel edge
is retained. This makes callers/callees, depth-3 impact/path, multiplicity, and
bounded pagination observable without relying on storage iteration order.

### Bounded raw traversal

The raw baseline scans JSON incrementally into bounded indexes, enforces explicit
node, edge, depth, result, byte, and time ceilings, and reports limit failures as
errors rather than empty results. It provides a comparison denominator, not an
alternate Compass implementation.

### Budget provenance

The ratified table is copied byte-for-meaning from the research document. The
phase plan realization was pinned after the 2026-08-28T12:45:00Z verify-first
handoff entry and before C-013 ratification; its authoritative SHA-256 is
`46e37804d513cf32ce8e7d008816642dffbb9d3b7b60fd3f2e82a482cd398ebf`.
No earlier filesystem timestamp is used as decision provenance.
The research budget section SHA-256 is
`d2f805da25a4904bb7137a0c13c1ab5a813a25c10875d24b2d81887e8298c744`.
C-012 resource values are descriptive and excluded from this decision basis.

## Risks / Trade-offs

- Full large generation is expensive, so ordinary tests use metadata-only large
  validation plus a small materialized profile.
- Process-start baselines include graph loading and cache state; every sample set
  records which condition was measured.
- Host metrics vary, so comparisons use ratios on the same runner and retain raw
  samples rather than publishing universal claims.
