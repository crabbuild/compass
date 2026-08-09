# Phase goals — compass-scoping-and-bounds

## Objective

Make Compass fail gracefully and act recoverably when a repository's canonical
graph exceeds the 2 GiB `MAX_GRAPH_BYTES` publication limit, without weakening
the bounded-work invariant that limit exists to enforce.

Triggered by a real failure: `compass init` on a 6.8 GB monorepo
(`universal-agent-runtime`, 9,320 in-scope markdown files, 4,827 under `crates/`)
failed after 331.90s with:

    error: snapshot limit exceeded: canonical graph exceeds the 2147483648-byte limit

Assessment established that scoping is NOT the gap — `SKIP_DIRS`, `.gitignore`
respect, `.compassignore`, and `--include`/`--exclude` all already ship and all
worked correctly. The matched content was legitimately in scope.

## Goals

- **G1** — Resolve the override inconsistency: `COMPASS_MAX_GRAPH_BYTES` is
  honored in `compass-model`, `compass-output`, `compass-global`, and
  `compass-core` (whose errors advertise it), but not on the
  `compass-graph`/`compass-store` publication path that actually fails. Either
  honor it there or document explicitly why it must not be.
- **G2** — Make the limit error actionable: name the remedy, not just the fact.
- **G3** — Fail fast: a doomed build should not consume 331s before reporting.
- **G4** — Decide `vendor/` default-skip policy deliberately (Go vendor dirs and
  Compass's own `vendor/` are legitimate source).

## Open architecture question (this phase's Analyze target)

If a repository of this size genuinely cannot publish under 2 GiB even with
correct exclusions, is the right answer **graph partitioning/sharding** rather
than a larger number? Settle this before writing code.

## Non-goals

- Building a scoping feature (already exists).
- Raising the 2 GiB default. AGENTS.md requires bounded work and states a limit
  error is a distinct outcome from an empty result.
