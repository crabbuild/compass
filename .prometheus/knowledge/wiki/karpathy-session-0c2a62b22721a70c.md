---
type: SessionRecord
id: karpathy-session-0c2a62b22721a70c
title: Karpathy session 0c2a62b22721
tags:
- karpathy
- session-learning
sources:
- session:626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
timestamp: 2026-08-09T11:05:44.269089+00:00
created_at: 2026-08-09T11:05:44.269089+00:00
updated_at: 2026-08-09T11:05:44.269089+00:00
revision: 0
---

## Delta

That confirms the root cause. Here's what's happening.

## Diagnosis

This is **not a bug** — it's a bounded-work invariant firing correctly, but with a bad user experience. Three separate problems:

**1. The scope was never actually narrowed.** Your run reported `Scope: 0 include rule(s), 0 exclude rule(s)` and matched **19,666 files**. The repo is 6.8 GB, and the walk swept in directories that have no business in a code graph:

| Directory | Size |
|---|---|
| `node_modules/` | 1.6 GB |
| `target/` | 821 MB |
| `compass-out/` | 302 MB ← Compass's *own* prior output |
| `vendor/` | 126 MB |

Note the 9,679 "documents" and 508 images — that's overwhelmingly dependency and build-artifact content, not your source.

**2. The 2 GiB cap has no escape hatch on this path.** `MAX_GRAPH_BYTES` (`crates/compass-store/src/lib.rs:51`) is a hard `const`. Elsewhere in the codebase — `compass-model`, `compass-output`, `compass-global`, `compass-core` — the same limit is overridable via `COMPASS_MAX_GRAPH_BYTES`, and the error messages there *tell you so*. But `compass-graph/src/snapshot.rs:3138` and `compass-store` never read that variable. So the message you got is a dead end: no override, no suggested remedy.

**3. It failed after 331 seconds**, at final publication — after all extraction work was done. The limit is checked at serialization time (`digest_json`), not estimated up front.

## Your immediate unblock

`compass init` does support scoping — you just pressed through the prompts. Exclusions are available non-interactively:

```bash
cd /Users/gqadonis/Projects/prometheus/universal-agent-runtime
rm -rf compass-out .compass
compass init --exclude='node_modules/**' --exclude='target/**' \
             --exclude='compass-out/**' --exclude='vendor/**' \
             --exclude='website/**' --exclude='static/**' --exclude='models.dev/**'
```

That should cut the graph by well over an order of magnitude. I haven't run it, so I can't promise it lands under the cap — but the excluded directories are ~3.2 GB of the 6.8 GB, and they're the ones generating the bulk of those 9,679 documents.

## The real fixes for this repo

Three defects worth addressing here, in priority order:

1. **Default ignore policy misses `compass-out/`, `node_modules/`, `target/`, `vendor/`.** Compass ingesting its own output directory is clearly wrong. This belongs in `compass-files::detect`.
2. **The limit error is unactionable.** It should name the override (if one is granted on this path) and point at scoping — matching how `compass-model` and `compass-core` already word theirs.
3. **Failing at 331 s is avoidable.** A pre-publication size estimate could fail fast, or warn during extraction.

Per AGENTS.md, #1 and #3 are `compass-files`/`compass-core` work and #2 spans `compass-graph`/`compass-store`; the CLI message stays thin.

Two things I want your call on before doing anything:

- **Scope:** should I fix all three, or just the ignore-policy default (#1), which alone would have prevented your failure?
- **Whether to raise the cap:** I'd advise against it. It's a deliberate bounded-work invariant, and AGENTS.md is explicit that a limit error is a distinct outcome from an empty result. Better to make the default scope sane than to let a 2 GiB graph through.

Also worth noting: the two commits on `docs/claude-md` and the `.kbd-orchestrator/` files are all still uncommitted/unpushed. This fix is unrelated work — I'd put it on its own branch rather than folding it in.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 626aa7ee-9dcb-4292-9ce9-4e4f58af7dc3
- Captured: 2026-08-09T11:05:43.579156Z
- Project: /Users/gqadonis/Projects/references/compass

## Changed Paths

- .prometheus/knowledge/wiki/index.md
- .prometheus/knowledge/wiki/log.md
- .kbd-orchestrator/
- .prometheus/knowledge/wiki/karpathy-session-6aacc8d765a1b28f.md
- .prometheus/knowledge/wiki/karpathy-session-762f04f1710fc991.md
- .prometheus/knowledge/wiki/karpathy-session-b0a4e7ceb012e58d.md
- .prometheus/knowledge/wiki/karpathy-session-b5d49ca1d46e60a2.md
- .prometheus/knowledge/wiki/karpathy-session-b6f086fb31ed31c5.md
- .prometheus/knowledge/wiki/karpathy-session-f90cdbd5fafd1d1d.md
