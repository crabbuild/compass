# Current waypoint

**Phase:** compass-scoping-and-bounds
**Status:** plan_ready
**Progress:** 0 of 6 changes complete
**Next command:** `/opsx:new stream-snapshot-read` (then `/kbd-execute`)

## First change to apply

**C-001 — Stop materializing the snapshot payload in `read_snapshot`**
Owner: `compass-store`. Wave 1. Unblocks C-004 and C-005.

## Blockers

- Identity/namespacing decision blocks C-005 *adoption* (not extraction).

## Waves

Wave 1 (parallel): C-001, C-002, C-003, C-006
Wave 2 (gated on C-001): C-004 [HARD GATE], C-005

_Updated: 
