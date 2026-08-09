---
type: SessionRecord
id: karpathy-session-8cd321cc802fed9d
title: Karpathy session 8cd321cc802f
tags:
- karpathy
- session-learning
sources:
- session:626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
timestamp: 2026-08-09T12:32:13.511112+00:00
created_at: 2026-08-09T12:32:13.511112+00:00
updated_at: 2026-08-09T12:32:13.511112+00:00
revision: 0
---

## Delta

**It was in the original `AGENTS.md`.** Not the skill pack, not me.

- Introduced in commit `dd14b3ce`, *"docs: add AI contributor guidance"*, on **2026-08-01** by **forhappy** — eight days before this session.
- Present at `57b9fe28`, the commit this session started from, with 7 occurrences of `/Volumes/Workspace`.

The only `AGENTS.md` change during this session was the Prometheus state-ownership section — 9 lines, added by you, entirely about `.prometheus/` paths. The disk policy was untouched.

So the chain is: `AGENTS.md` mandated it → I applied it when generating `.kbd-orchestrator/project.json` and `constraints.md` during `/kbd-init` → the KBD `kbd-init` skill's own guidance reinforces it ("Read `AGENTS.md`… for mandatory command wrappers, environment variables, toolchain selectors, and storage paths… Treat a command policy as transitive"). But the *source* of the path is your repo's committed contributor guidance from a week ago.

Which raises the real question: **does that volume actually exist on your machine, or is `dd14b3ce` describing someone else's setup?** `forhappy` isn't the git user configured here (that's Travis James). If this guidance came from a different contributor's environment, the path may never have been valid for you — which would make it a stale rule in `AGENTS.md`, not a missing drive.

If that's the case, the fix is to correct `AGENTS.md` first, then regenerate `project.json` and `constraints.md` from it. Want me to look at what that section should say for your actual setup?

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
- Captured: 2026-08-09T12:32:12.077290Z
- Project: /Users/gqadonis/Projects/references/compass

## Changed Paths

- .prometheus/knowledge/wiki/index.md
- .prometheus/knowledge/wiki/log.md
- .prometheus/knowledge/wiki/karpathy-session-6ee868a466335f1d.md
- .prometheus/knowledge/wiki/karpathy-session-7da9adb3b7b11504.md
- .prometheus/knowledge/wiki/karpathy-session-b1a29789942105c1.md
- .prometheus/knowledge/wiki/karpathy-session-badb024212b1a395.md
