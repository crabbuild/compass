# Verification — SurrealDB 3.2.4 license decision

Date: 2026-08-29

## Decision evidence

- The project user explicitly wrote `C-011: ACCEPT` in the active Codex task.
- The pre-existing design defines `accept` as approval of every named profile
  under its stated conditions.
- The sign-off records the authority role, date, exact message provenance,
  closed profile scope, and pinned-version conditions without adding a waiver.

## Prerequisite certification evidence

C-012 is archived at
`openspec/changes/archive/2026-08-28-surreal-persistent-probes/`; every one of its
six tasks is checked complete. Its seven blocking refinement constraints are
PASS. The retained records have these SHA-256 identities:

- `.refiner/artifacts/C-012/constraints.json`:
  `e7ba948a92b389b7e90d687c797c642ee2529dff8be0627b5f0c434075a4cf40`
- `.refiner/artifacts/C-012/dist/refinement-receipt.md`:
  `36ee14e3a792f387384597051aa79e2f4b5f982b108d6e36bbfc771e7932d70d`

C-013 is archived at
`openspec/changes/archive/2026-08-29-qualification-corpora-baselines/`; every one
of its seven tasks is checked complete. Its eight blocking refinement
constraints are PASS. The retained records have these SHA-256 identities:

- `.refiner/artifacts/C-013/constraints.json`:
  `017a1d20a7a4f3c0015dfb5665e950b6da1cc62aa9441a2a25da8bfbcf85e05b`
- `.refiner/artifacts/C-013/dist/refinement-receipt.md`:
  `8e871f316594dd753995aa83816203132952c7fe85b5f04e4d33e0e9616002e3`
- `docs/future/qualification-corpora-baselines.md`:
  `5944c9b2b77d04f8b3b6282cb2925451054fe20e9451ff0a6f54c89a3990b6e1`

The C-013 decision document records that its complete budget table was ratified
unchanged before Wave 5 measurements and retains the exact plan and research-
section hashes used as provenance.

The current bytes of
`.kbd-orchestrator/phases/compass-scoping-and-bounds/plan.md` have SHA-256
`46e37804d513cf32ce8e7d008816642dffbb9d3b7b60fd3f2e82a482cd398ebf`,
exactly preserving that immutable pre-measurement planning snapshot. Live gate
resolution is recorded in the decision log, execution contract, and KBD progress
rather than mutating the pinned plan.

## License capture

The repository contains the captured tagged license bytes at
`scripts/fixtures/surreal-persistent-probes/SURREALDB-3.2.4-LICENSE.txt`. Their
SHA-256 is
`98a94ac615f88370865016487b436fa404560910bd329794ed7502277a94b805`,
which matches the entry in `manifest-v1.json`. The manifest itself has SHA-256
`81cb3080e87f9e780492946a37b8c92c5108d96c107aa1445b112a95832ed67d`.
On 2026-08-29 a bounded raw fetch from the pinned upstream URL produced the same
SHA-256 and a byte-for-byte `cmp` against the retained fixture passed.

## Gate disposition

C-012 and C-013 are certified complete. C-011 acceptance is recorded and its
mandatory corrected pre-archive fresh-context review passed with zero critical
findings, three warnings, and one suggestion; that round's strict anti-sycophancy
gate passed with score 0.0. This change is published at
`openspec/changes/archive/2026-08-29-surrealdb-license-decision/`, so C-014/C-015
may proceed. C-020 remains independently conditional. No SurrealDB dependency
exists in the Compass workspace. A separate post-publication audit caught and
caused correction of a mutated C-013 plan precommitment. The final archived-state
audit passed with zero critical findings and three warnings; its strict
anti-sycophancy gate passed. The exact findings remain retained in the KBD review
record.

## Validation evidence

- `openspec validate surrealdb-license-decision --strict`: PASS before archive
- `openspec validate --all --strict`: PASS after archive (12 passed, 0 failed)
- deterministic C-011/C-012/C-013 manifest, source/dist-path, PASS-constraint,
  fixture-checksum, and pinned-plan-hash checks: PASS
- `git diff --check`: PASS
