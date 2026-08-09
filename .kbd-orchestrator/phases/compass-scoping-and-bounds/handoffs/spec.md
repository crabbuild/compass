# Stage handoff — spec

**Stage:** spec
**Completed:** 2026-08-09T12:21:35Z
**Next stage:** plan (or execute)

## Summary

Six changes specced with a hard sequencing gate. The consumer audit that analyze flagged
as blocking is complete and shrank C-001 substantially: read_snapshot's 8 call sites
include only two production callers, both of which discard the payload with `_`, while
compass-query already reads through GraphSnapshotReader::open_selector. C-001 is an API
split, not a streaming rewrite.

## Hard gates

1. C-004 (COMPASS_MAX_GRAPH_BYTES override) MUST NOT land before C-001. An override
   before the read path stops materializing lets a user request a multi-gigabyte
   contiguous Vec.
2. C-005: compass-partition MUST NOT depend on prolly-map, prolly-store-sqlite,
   compass-ir, or compass-analysis. If that cannot be satisfied, STOP and report —
   it means the shared-crate framing does not hold.
3. C-002 must not advertise the override before C-004 exists.

## Scope honesty

C-005 as directed extracts PartitionedGraph plus key/canonical helpers — a struct and
four functions out of a 2,317-line module. It positions for current-graph partitioned
publication; it does not deliver it. That is a further change requiring the identity
decision below.

## Open questions

1. Identity/namespacing for shared record keys — blocks C-005 adoption (not extraction).
   History fingerprints are meaning-affecting and realizations immutable; shared keys
   must not collide or conflate across paths.
2. Payload composition unmeasured — blocks C-003 calibration.
3. Untested: whether --exclude on universal-agent-runtime lands under 2 GiB.

## Environment blocker

/Volumes/Workspace is NOT mounted. Every compiling command requires
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main. Execution cannot
verify anything until that volume is available.
