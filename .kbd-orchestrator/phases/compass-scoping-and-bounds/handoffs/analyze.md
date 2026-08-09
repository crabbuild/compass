# Stage handoff — analyze

**Stage:** analyze
**Completed:** 2026-08-09T12:03:31Z
**Next stage:** spec

## Summary

Sharding is rejected; partitioning already exists in compass-history
(PartitionedGraph) and is unused by the current-graph publication path. The true
constraint is not the 2 GiB number but load_active_snapshot
(compass-store/src/lib.rs:838-853), which pre-allocates payload_bytes and concatenates
all chunks into one contiguous Vec — directly contradicting the crate's doc comment at
lines 44-51 claiming records are served without materializing the whole document. The
write path already streams via DigestWriter.

## Blocking sequencing constraint

Honoring COMPASS_MAX_GRAPH_BYTES on the publication path (G1) MUST NOT land before the
read path streams. Raising the cap today raises a real allocation; an override would let
a user request a multi-gigabyte contiguous Vec — a worse failure than the current clean
error.

## Open questions for spec

1. Ownership: PartitionedGraph lives in compass-history, but AGENTS.md routes
   current-graph publication to compass-graph. Move to a shared crate, duplicate with
   distinct identity semantics, or add a dependency? Architecture decision — blocks item 6.
2. Consumer audit of load_active_snapshot — not yet enumerated; blocks item 1 sizing.
3. Unmeasured: which node/edge classes dominate the 2 GiB payload.
4. Untested: whether --exclude on universal-agent-runtime lands under 2 GiB. With 4,827
   markdown files under crates/ it may not.

## WARNING findings from adversarial review

- compass-store doc comment is materially inaccurate about its own read path; fix the
  code or the comment, but do not leave both.
- Item 6 must not start before the ownership question is answered.
- Confidence in direction is high (source inspection + convergent literature);
  confidence in sizing is low — 3 of 4 open questions are unmeasured.
