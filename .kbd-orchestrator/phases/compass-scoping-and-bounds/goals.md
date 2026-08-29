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

## Expanded scope (2026-08-28, user decision)

The phase scope was expanded by user directive on 2026-08-28 to include
implementing the `docs/future/` program in this phase. This supersedes the
notes' own advice to run it as a separate phase; the conflict is recorded in
`assessment.md` §2.

- **E1** — Migrate `compass-mcp` to MCP 2026-07-28 on `rmcp` 3.1.4: stateless
  discovery, typed result envelope, deterministic tool ordering, session-flag
  deprecation, stdio+HTTP conformance, named-harness interop.
- **E2** — Decision gates before any SurrealDB code merges: BSL 1.1
  license/release acceptance for the exact artifact profile; persistent
  SurrealKV + RocksDB throwaway probes (parallel directed relations, stable IDs,
  provenance, generation-scoped reads, kill-during-write recovery).
- **E3** — `compass-graphdb-surreal` graph projection adapter: schemafull
  nodes/relations, generation activation, bounded native reads, dual-engine
  semantic equivalence.
- **E4** — `compass-store-surreal` Store adapter (optional, last; only if an
  independent user problem justifies it).
- **E5** — Skills split (6 focused skills, umbrella preserved) + `compass agent`
  CLI namespace (list/install/doctor/export/validate/mcp-config).
- **E6** — Native harness packages (Codex `.codex-plugin`, Claude
  `.claude-plugin` + marketplace, OpenCode plugin/npm) generated from one
  canonical inventory.
- **E7** — Measurement prerequisites: golden answers, scale fixtures, 30-task
  suite, baselines; numeric budgets ratified before measurements are visible.

Original non-goals stand: no scoping feature, no raising the 2 GiB default.
Additional non-goals from the notes: no Prometheus runtime dependency, no
default-path SurrealDB, no raw write-capable SurrealQL tool by default.
