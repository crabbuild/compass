# Stage handoff — plan

**Stage:** plan
**Completed:** 2026-08-09T12:27:19Z
**Next stage:** execute

## Summary

6 changes in 2 waves. Wave 1 (C-001, C-002, C-003, C-006) has no dependencies and can run
in parallel; Wave 2 (C-004, C-005) is gated on C-001. Apply **C-001** first — it is the
only Wave 1 item that unblocks others, and the spec-stage consumer audit showed it is an
API split rather than the streaming refactor originally assumed. Backend is OpenSpec.

## Ordering rationale

C-004 must not precede C-001: an override before the read path streams lets a user request
a multi-gigabyte contiguous Vec, strictly worse than today's clean error. C-005 is also
sequenced after C-001 so the extraction happens against an honest read path.

## WARNING findings from adversarial review

- C-003 has testable acceptance criteria but no calibration data. Measuring payload
  composition is its first task, not an afterthought.
- C-002 and C-004 are coupled through the error text: C-002 omits the override, C-004 adds
  it. If C-004 slips, C-002's message stays permanently incomplete — track explicitly.
- C-005's dependency rule (no prolly-map / prolly-store-sqlite / compass-ir /
  compass-analysis) is a stop-and-report gate, not a preference.
- Every acceptance box is unverifiable while /Volumes/Workspace is unmounted.

## Blockers

1. **/Volumes/Workspace NOT mounted** — blocks all verification. Re-checked at plan time.
2. Identity/namespacing for shared record keys — blocks C-005 adoption, not extraction.

## Unmeasured

- Payload composition (blocks C-003 calibration).
- Whether --exclude on universal-agent-runtime lands under 2 GiB.
