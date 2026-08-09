# Assessment — compass-scoping-and-bounds

**Phase:** compass-scoping-and-bounds (provisional — not yet created via `/kbd-new-phase`)
**Date:** 2026-08-09
**Question:** Should scoping be added as a feature right now?
**Verdict:** **No — scoping already exists and already works. The real defects are elsewhere.**

---

## Correction to the initial diagnosis

An earlier hypothesis in this session claimed the default ignore policy was missing
`node_modules/`, `target/`, and `compass-out/`. **That was wrong.** Direct source
inspection disproves it:

| Claim | Reality | Evidence |
| --- | --- | --- |
| Defaults miss `node_modules`/`target`/`compass-out` | All three are present | `crates/compass-files/src/detect.rs:61-102` (`SKIP_DIRS`) |
| Gitignore not respected | Respected by default | `detect.rs:163` (`gitignore: true`) |
| Scoping absent | `--include`/`--exclude` + interactive prompts exist | `crates/compass-cli/src/init_commands.rs:438-462` |

`SKIP_DIRS` holds ~40 entries and additionally skips dot-directories, `*_venv`,
`*_env`, and `*.egg-info` (`detect.rs:252-268`, `484-500`). `.compassignore` is
supported alongside `.gitignore` (`detect.rs:390-392`).

**Scoping is not the gap. Building the feature would be duplicated work.**

---

## What actually happened

The target repository is a genuinely large, document-dense monorepo. Measured
directly:

```text
9,320  markdown files in scope AFTER all default exclusions
4,827    └── under crates/ alone
1,092    └── openspec/
  780    └── frontend/
```

The reported `Matched: 19666 files (9470 code, 9679 documents, ...)` is therefore
**correct behavior over real content**, not a discovery leak. The exclusions fired;
there is simply that much in-scope material.

The failure is a true capacity limit:

```text
error: snapshot limit exceeded: canonical graph exceeds the 2147483648-byte limit
Compass init failed after 331.90s wall time.
```

Origin: `crates/compass-graph/src/snapshot.rs:3138` (`digest_json`), enforcing
`MAX_GRAPH_BYTES` = 2 GiB from `crates/compass-store/src/lib.rs:51`.

---

## Confirmed gaps

### G1 — The 2 GiB cap has no override on the publication path (blocking UX defect)

`COMPASS_MAX_GRAPH_BYTES` is honored in `compass-model` (`graph.rs:370`),
`compass-output` (`json.rs:414`), `compass-global` (`lib.rs:370`), and
`compass-core` (`diagnostics.rs:713`). Those crates' errors *advertise* the
override:

> `(set COMPASS_MAX_GRAPH_BYTES=<bytes> or COMPASS_MAX_GRAPH_BYTES=<N>GB to raise the limit)`

Neither `compass-graph/src/snapshot.rs` nor `compass-store/src/lib.rs` reads that
variable (verified by grep — zero occurrences in either file). The user hits a
limit that the rest of the product treats as adjustable, with **no override and no
remedy in the message**.

This is an internal inconsistency, not merely a missing feature.

### G2 — The error is unactionable (blocking UX defect)

The message states a fact and stops. It does not name the override, suggest
`--exclude`, mention `.compassignore`, or report which content dominated the graph.
Compare `compass-core/src/diagnostics.rs:453`, which does guide the user.

### G3 — Failure occurs after 331 s, at the end (efficiency defect)

The cap is enforced at serialization time. All extraction, resolution, and analysis
completes first, then publication fails and the work is discarded. No pre-flight
estimate, no progressive warning.

### G4 — `vendor/` is absent from `SKIP_DIRS` (minor, genuine)

`vendor/` (126 MB in the target repo; 441 markdown files) is not skipped. Note this
is deliberate for some ecosystems — Go's `vendor/` holds real dependency source —
so this needs a decision, not a reflex addition. Compass's own tree has a
`vendor/` it legitimately reads.

---

## Research: current practice (firecrawl, 2026-08-09)

Convergent evidence across indexers and formatters:

1. **Respect `.gitignore` by default** — described in a comparative formatter study
   as "the single most important default" and "the single highest-value default"
   (Ruff ✅, Prettier v3+ ✅, dprint ✅, Black ✅; Biome opt-in).
   → **Compass already does this.**

2. **Ship hardcoded defaults beyond VCS ignore** — `ck` (`.ckignore`),
   `codegraphcontext` (`.cgcignore`), `understand-anything` (`.understandignore`)
   all bundle `node_modules/`, `target/`, `dist/`, `build/`.
   → **Compass already does this, with a larger list than the examples surveyed.**

3. **Provide a tool-specific ignore file layered over `.gitignore`** — universal
   across the sample (`.ckignore`, `.cursorignore`, `.rgignore`, `.ignore`).
   → **Compass already does this (`.compassignore`).**

4. **Explicit paths override ignore rules.** From the MemSearch discussion
   (zilliztech/memsearch#612, Jul 2026): "If the user explicitly specifies a file
   path to index, that file should still be indexed... since that represents an
   explicit user choice."
   → Worth verifying Compass's behavior; not assessed here.

5. **ripgrep's `--no-ignore-vcs` precedent** (BurntSushi, ripgrep#645): the
   maintainer holds that an explicit paired flag is "the 'right' answer" over
   changing defaults.
   → Supports G1's remedy shape: an *explicit override*, not a raised default.

**Nothing in the research supports building a scoping feature. It supports the
opposite — Compass is already at or above the surveyed baseline on scoping.**

---

## Adversarial review

Applied per `/adversarial-review --mode artifact assess`. Preflight `status: ok`,
judge `k3` ≠ generator `kbd-frontier`, so judge ≠ producer.

**CRITICAL — raised and resolved:** the original framing ("add scoping") would have
shipped a duplicate of `SKIP_DIRS` + `--exclude` + `.compassignore`. Refuted by
source inspection; assessment rewritten. This is the finding that changes the
recommendation, and it came from checking the code rather than trusting the
initial hypothesis.

**WARNING — carried forward:**

- **Do not raise the 2 GiB cap.** `AGENTS.md` requires bounded work and states a
  limit error is a distinct outcome from an empty result. Raising the default
  weakens an intentional invariant. Grant an *explicit, opt-in* override instead.
- **G4 needs a decision, not a reflex.** Skipping `vendor/` breaks Go monorepos and
  conflicts with Compass's own vendored tree.
- **Sycophancy check:** the prior turn's confident "not a bug — three defects"
  framing was partly wrong (the ignore-policy claim). Recorded rather than quietly
  dropped.

**Unverified / open:**

- Whether the suggested `--exclude` workaround actually lands under 2 GiB is
  **untested**. With 4,827 markdown files under `crates/` — a directory that cannot
  be excluded without gutting the graph — it may well still exceed the cap.
- No measurement of which node/edge classes dominate the 2 GiB payload.

---

## Recommendation

**Do not build scoping.** Sequence instead:

| Pri | Gap | Change | Owner crate |
| --- | --- | --- | --- |
| 1 | G1 | Honor `COMPASS_MAX_GRAPH_BYTES` on the snapshot/publication path, or explicitly document why it must not be honored | `compass-graph`, `compass-store` |
| 2 | G2 | Make the limit error actionable — name the override, `--exclude`, `.compassignore`, and the dominant content class | `compass-graph` (message), `compass-cli` (thin) |
| 3 | G3 | Pre-flight size estimate so a doomed build fails early rather than after 331 s | `compass-core` |
| 4 | G4 | Decide `vendor/` policy deliberately | `compass-files` |

G1 and G2 together resolve the reported failure. G3 is the quality-of-life fix.

**Open question for the next stage:** if a repo of this size genuinely cannot
publish under 2 GiB even with sane exclusions, is the correct answer a graph
partitioning/sharding strategy rather than a bigger number? That is an
architecture question, not a scoping one, and it should be settled in `/kbd-analyze`
before any code is written.

---

## Evidence index

- `crates/compass-files/src/detect.rs:61-102` — `SKIP_DIRS`
- `crates/compass-files/src/detect.rs:163` — `gitignore: true`
- `crates/compass-files/src/detect.rs:252-268`, `484-500` — skip enforcement
- `crates/compass-files/src/detect.rs:390-392` — `.compassignore`
- `crates/compass-cli/src/init_commands.rs:365-377`, `438-462` — scoping surface
- `crates/compass-store/src/lib.rs:51` — `MAX_GRAPH_BYTES`
- `crates/compass-graph/src/snapshot.rs:3138` — failure site
- `crates/compass-core/src/diagnostics.rs:453` — actionable-message precedent
