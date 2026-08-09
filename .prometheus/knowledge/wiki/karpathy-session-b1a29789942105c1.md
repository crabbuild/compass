---
type: SessionRecord
id: karpathy-session-b1a29789942105c1
title: Karpathy session b1a297899421
tags:
- karpathy
- session-learning
sources:
- session:626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
timestamp: 2026-08-09T12:31:51.013497+00:00
created_at: 2026-08-09T12:31:51.013497+00:00
updated_at: 2026-08-09T12:31:51.013497+00:00
revision: 0
---

## Delta

It's from `AGENTS.md` lines 24–56, committed in your repo — "Disk and external-checkout policy." It's not something I invented.

The rule says the main disk is capped at 100 GB, so Cargo artifacts must go on a mounted workspace volume, with the literal example path `/Volumes/Workspace/crabbuild-target/compass-main`. And explicitly: *"If `/Volumes/Workspace` is unavailable, stop and report it rather than falling back to a local `target/` directory."*

That's why I kept checking it and kept flagging it as a blocker — the policy tells me to stop rather than build locally.

Currently mounted: `Macintosh HD`, `my-passport`, `Open Design`. No `Workspace`.

So one of these is true, and only you know which:

- **The volume is just detached** — plug it in and the rule works as written.
- **It got renamed** (`my-passport`? something else?) — then `AGENTS.md` has a stale path and should be corrected.
- **The whole policy is obsolete** — maybe disk pressure was resolved and this rule outlived its purpose. Note the wiki transcripts from earlier today mention reclaiming ~10.1 GB and fixing sccache config, so something in this area did change recently.

If it's stale or obsolete, `AGENTS.md` is the fix — and I'd want that corrected before execution, because I baked those paths into `.kbd-orchestrator/project.json` and `constraints.md`, which are now committed. Every build/test/lint command there points at `/Volumes/Workspace`.

Which is it? If you tell me the volume is gone for good, I'll update `AGENTS.md`, `project.json`, and `constraints.md` together so they stop referencing a path that doesn't exist.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
- Captured: 2026-08-09T12:30:02.796796Z
- Project: /Users/gqadonis/Projects/references/compass

## Changed Paths

- .prometheus/knowledge/wiki/index.md
- .prometheus/knowledge/wiki/log.md
- .prometheus/knowledge/wiki/karpathy-session-6ee868a466335f1d.md
- .prometheus/knowledge/wiki/karpathy-session-7da9adb3b7b11504.md
- .prometheus/knowledge/wiki/karpathy-session-badb024212b1a395.md
