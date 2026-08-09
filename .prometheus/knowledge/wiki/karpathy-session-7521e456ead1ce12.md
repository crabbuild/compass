---
type: SessionRecord
id: karpathy-session-7521e456ead1ce12
title: Karpathy session 7521e456ead1
tags:
- karpathy
- session-learning
sources:
- session:34d4343f-8edd-46e9-bb53-90b588e5d3ed
timestamp: 2026-08-09T05:36:58.206699+00:00
created_at: 2026-08-09T05:36:58.206699+00:00
updated_at: 2026-08-09T05:36:58.206699+00:00
revision: 0
---

## Delta

Both done.

**Deleted the local sccache** — `~/Library/Caches/Mozilla.sccache` (10 GB) is gone and has not been recreated. Verified safe first: the live sccache server was serving `/Volumes/my-passport/sccache`, and the local dir had zero writes in the preceding 5 minutes even with an active build running.

**Set `SCCACHE_DIR` in `~/.cargo/config.toml`** — added an `[env]` block pinning `SCCACHE_DIR = "/Volumes/my-passport/sccache"` and `SCCACHE_CACHE_SIZE = "40G"`, with a comment explaining the failure mode so it isn't re-litigated later. Config parses cleanly.

I deliberately left off `force = true`: your `.zshrc` exports the identical value so there's no conflict, and staying unforced keeps one-off `SCCACHE_DIR=... cargo build` overrides working. Cargo's `[env]` applies to every invocation, so the non-login shells that previously bypassed the `.zshrc` guard now get the WDD path.

Free space went 367 → 373 GiB. It reads lower than the raw 10 GB because an unrelated Rust build is running concurrently and consuming space as it compiles.

Two caveats worth stating plainly:

- **My verification of the config was static, not functional.** I tried a real build to prove `SCCACHE_DIR` reaches a compiler process, but it hung — it inherited your `rustc-wrapper` and queued behind that concurrent surrealdb build saturating all 10 job slots. I killed and cleaned it up. So I confirmed the file parses with the right values, not that a build observes them end-to-end. Cargo `[env]` semantics are well-defined here, but if you want proof, run any build once the current one finishes and check `sccache --show-stats` still reports the WDD location.
- **`[env]` doesn't cover non-cargo callers.** Anything invoking `sccache` directly — some CMake/C++ paths, for instance — still relies on the shell export. Your `.zshrc` handles login shells; a stray non-login shell could in principle recreate the default dir. Worth a periodic `du -sh ~/Library/Caches/Mozilla.sccache` to catch it early.

There's also a **stale comment** at line 12 of the config saying "warm 12 GB cache on the WDD" — the actual max is 40 GB. I left it alone since it's outside what you asked for, but I can fix it if you want.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 34d4343f-8edd-46e9-bb53-90b588e5d3ed
- Captured: 2026-08-09T05:36:52.482414Z
- Project: /Users/gqadonis/Projects/references/compass

## Changed Paths

- .prometheus/
