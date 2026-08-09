---
type: SessionRecord
id: karpathy-session-199d5165acd78a00
title: Karpathy session 199d5165acd7
tags:
- karpathy
- session-learning
sources:
- session:626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
timestamp: 2026-08-09T10:34:32.153219+00:00
created_at: 2026-08-09T10:34:32.153219+00:00
updated_at: 2026-08-09T10:34:32.153219+00:00
revision: 0
---

## Delta

Created `CLAUDE.md` at the repo root.

Since `AGENTS.md` is already a thorough agent guide, I wrote CLAUDE.md to defer to it rather than duplicate it, and concentrated on what it leaves implicit:

**What's in it:**
- **`CARGO_TARGET_DIR` requirement** — surfaced up top, since it must be set on every compiling invocation and is the easiest thing to get wrong. I also flagged the conflict where some Makefile targets (`install`, `dist`, `release-check`) resolve binaries through a literal `target/` path, which AGENTS.md mentions but doesn't tie to specific targets.
- **Commands** — narrow-loop test/clippy per crate and per integration file, the workspace baseline, the surface-specific gates (product boundary, CompassQL TCK, code-graph qualification), and the npm workspace commands. Noted that `make test` is `--lib --bins` only while `make test-all` needs Python oracle setup.
- **Architecture** — the piece that genuinely requires reading multiple files: the one-directional flow `compass-files → compass-languages → compass-resolve → compass-graph → compass-model`, orchestrated by `compass-core`. I emphasized the boundary that's easiest to violate — per-file extractors emit evidence and never resolve targets needing project-wide facts. Also the `compass-cypher` (syntax) vs `compass-query` (execution) split, immutable history realizations, and the generated viewer assets round-trip.
- **Enforced constraints** — the lint set, determinism as correctness, boundedness (limit error ≠ empty result), and reuse of existing safe primitives.

Two notes on judgment calls: I skipped enumerating individual crates beyond the pipeline ones, since `docs/implementation/workspace-tour.md` already does that well and I pointed at it instead. I also didn't import anything from a Codex or Gemini config — I didn't check for those, as the instruction is to offer rather than read them. If you have an OpenAI Codex or Gemini CLI config you'd like folded in, reply `/import` and it'll scan and list what's importable.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
- Captured: 2026-08-09T10:34:13.627630Z
- Project: /Users/gqadonis/Projects/references/compass

## Changed Paths

- .prometheus/
- CLAUDE.md
