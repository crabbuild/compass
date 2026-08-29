# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Read AGENTS.md first

`AGENTS.md` is the authoritative operating guide for this repository and applies
to the whole tree. It covers product invariants, crate ownership routing, Rust
conventions, test/fixture policy, public-contract rules, and the completion
checklist. Read it before editing. This file only adds the mechanics AGENTS.md
does not spell out.

## Immutable implementation order

Complete the entire active phase's production implementation before compiling,
linting compiled code, or testing. Do not use incremental `cargo check`,
per-crate builds, filtered tests, or per-change green gates. Tests run only after
the phase fully covers its specification, and only full integration tests count;
never add, run, or cite unit tests as correctness evidence. This rule overrides
workflow/skill defaults that request per-task or per-change compilation and QA.

After implementation, one process owns the verification wave. Run the complete
workspace integration target set with
`cargo test --workspace --test '*' --locked`; Cargo's `--tests`, `--lib`,
`--bins`, and default selections include unit-test harnesses and are prohibited
here. Run complete feature-gated integration target sets afterward where the
phase requires them.

## Build artifacts

Compass build artifacts are large. `.cargo/config.toml` and the root profiles
place artifacts in the checkout-local `target/`, cap compilation at four jobs,
disable incremental artifact growth, retain only line-table debug information
for workspace code, and omit dependency debuginfo. Never point
`CARGO_TARGET_DIR` at a shared repository/worktree cache. Run only one Compass
Cargo command at a time on the host.

If `sccache` is installed, a phase-level build may opt in with
`RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 SCCACHE_CACHE_SIZE=5G`. Do not make
the wrapper unconditional: it is not portable when absent, and its Rust cache
does not support incremental crates. Use `--profile debugging` only when the
post-implementation integration wave proves that a debugger is necessary.

Note a related Makefile wrinkle: several targets (`install`, `dist`,
`release-check`) look up binaries through a literal `target/` path after Cargo
finishes. If you have redirected `CARGO_TARGET_DIR`, prefer direct `cargo`
commands for verification, and inspect any packaging/install target before
running it so it does not silently trigger a second build.

## Commands

Rust (Edition 2024, pinned toolchain 1.97.1, always `--locked`):

```bash
# Run only after every production-code task in the active phase is complete.
cargo test --workspace --test '*' --locked                  # integration targets only
cargo test -p <affected-package> --test '*' --all-features --locked
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --locked -- -D warnings
```

Surface-specific gates (run the ones matching what changed):

```bash
sh scripts/check_product_boundary.sh                # no Graphify references
cargo test -p compass-cypher --test tck --locked    # CompassQL grammar
cargo test -p compass-query --test opencypher_tck --locked
python3 scripts/check_compassql_support.py          # pinned support evidence
./scripts/qualify_code_graph_v1.sh --fixtures-only  # code-graph publication gate
```

JavaScript / viewer / VS Code (npm workspaces: `apps/*`, `packages/*`,
`editors/*`, `tests/*`):

```bash
npm ci
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs   # generated viewer assets match source
```

`make help` lists wrappers. `make test`, `make test-all`, `make test-release`,
and `make ci-fast` select integration targets only and remain phase-end commands;
they are never an incremental implementation loop. `make watch` is deliberately
disabled. `make lint`, `make test-js`, and `make qualify-code-graph-v1` remain
post-implementation surface gates.

The build policy follows Cargo's official build-performance, profile,
integration-target selection, and cache guidance:

- <https://doc.rust-lang.org/cargo/guide/build-performance.html>
- <https://doc.rust-lang.org/cargo/reference/profiles.html>
- <https://doc.rust-lang.org/cargo/commands/cargo-test.html>
- <https://doc.rust-lang.org/cargo/reference/build-cache.html>
- <https://github.com/mozilla/sccache/blob/main/docs/Rust.md>

CI (`.github/workflows/compass-ci.yml`) additionally runs isolated native
integration tests, crate-archive publishing checks, `cargo install` launch
verification, query-relevance qualification, dependency audit/policy, and a
native platform matrix. Consult it before touching those surfaces.

## Architecture

One Rust workspace of ~31 `compass-*` crates plus a vendored tree-sitter
language pack. The shipped artifact is package `compass-cli`, binary `compass`,
entry `crates/compass-cli/src/bin/compass.rs`.

The core value proposition: source code and project artifacts become a typed,
directed multigraph with provenance, published as one atomic, deterministic
snapshot — no Python, embeddings, vector DB, model credentials, or runtime
parser downloads on the structural path.

### The build pipeline

`compass update` / `extract` / `watch` parse into `compass-core::BuildOptions` +
a `BuildPurpose`, then flow one direction through the layers:

```text
compass-files       walk + classify + ignore policy, cache/manifest, atomic writes
      |             (incremental reuse decided here, via fingerprints)
      v
compass-languages   per-file syntax facts only -> Extraction {nodes, edges,
      |             hyperedges, raw calls}; statically linked parsers
      v
compass-resolve     merge Extractions; resolve cross-file imports, calls,
      |             members, re-exports, aliases, stable IDs
      v
compass-graph       dedup, communities/clustering, god nodes, cycles, topology
      |
      v
compass-model       GraphDocument / Graph / QueryIndex — the public contract
      |
      v
compass-core        sequences the above into one transactional workflow
```

The critical boundary: **per-file extractors emit evidence; they never resolve
targets that need project-wide facts.** That lives in `compass-resolve`.
Similarly, anything that depends on graph topology rather than source syntax
belongs in `compass-graph`, not the extractor.

### Query path

`compass-cypher` (lexer → parser → AST → semantic analysis → logical plan →
optimizer) compiles CompassQL — a bounded, read-only openCypher subset — and
owns `LANGUAGE_VERSION` / `PLANNER_VERSION`, both cache-compatibility inputs.
`compass-query` executes against the graph (text scoring, traversal, impact,
CompassQL execution/cache/profile). Syntax acceptance is a `compass-cypher`
concern; execution behavior is a `compass-query` concern.

### Presentation and interfaces

`compass-output` renders an already-complete graph (Markdown, HTML, JSON, SVG,
GraphML, Cypher, Obsidian, wiki, canvas). `compass-cli` and `compass-mcp` are
deliberately thin: argument parsing, help, exit codes, streams, side effects,
schemas — reusable behavior belongs in the owning domain crate.

### History

`compass-history` stores immutable graph realizations per Git commit over
SQLite/Prolly: canonical encoding, meaning-affecting fingerprints, typed keys,
artifact partitioning, durable job queue, leases, locks, GC, and typed record
diffs. `compass-semantic-diff` compares two published realizations under
evidence gating. **Published realizations are immutable** — never rewrite in
place or silently substitute a different realization/profile.

### Optional layers

`compass-semantic` (provider fragments, prompts, validation — hard caps on
fragment bytes/record counts, endpoint checks), `compass-media` /
`compass-ingest` / `compass-transcribe` / `compass-whisper` (bounded document
and audio extraction), and integrations (`compass-cargo`, `compass-global`,
`compass-google-workspace`, `compass-graphdb`, `compass-postgres`,
`compass-prs`, `compass-reflect`). All of these are optional; structural
extraction and graph queries must keep working without any of them.

### Generated viewer assets

`crates/compass-output/assets/viewer/{graph.js,viewer.css,manifest.json}` are
build outputs of `packages/compass-viewer`. Edit the React source, then:

```bash
node scripts/build_viewer_assets.mjs   # regenerate
node scripts/check_viewer_assets.mjs   # verify they match
```

Never hand-edit the generated files.

## Enforced constraints

Workspace lints forbid `unsafe_code` and deny `clippy::all`, `unwrap_used`,
`expect_used`, and `panic` — return typed errors with actionable context.

Determinism is a correctness property, not a nicety: prefer `BTreeMap`/`BTreeSet`
or explicit sorting at contract boundaries, and never rely on hash iteration or
filesystem order. Never resolve ambiguity by picking the first or most
convenient candidate — an explicit unresolved/ambiguous result beats invented
meaning.

Every traversal of files, graphs, archives, network responses, queries, and
subprocess output must be bounded. A limit error is a distinct outcome from an
empty result; do not collapse the two.

Reuse the existing bounded readers, subprocess helpers, path-containment checks,
endpoint checks, and atomic-write functions rather than writing weaker local
variants. Pass subprocess arguments separately — never construct shell strings.

Add workspace dependencies at the root `Cargo.toml` and reference them as
`{name}.workspace = true`.

## Compatibility-sensitive surfaces

CLI arguments/help/exit codes, environment variables, configuration, graph JSON,
CompassQL, MCP schemas, output files, history formats, and stable IDs. An
incompatible user-visible change needs all four of: native regression coverage,
updated reference documentation, a `MIGRATION.md` note when users must act, and
a `CHANGELOG.md` entry when release-visible. See `COMPATIBILITY.md`.

## Documentation map

- `docs/implementation/workspace-tour.md` — crate-by-crate responsibilities,
  public interfaces, and "change here when" guidance. Use it to route work.
- `docs/implementation/extending-compass.md` — extension checklists.
- `docs/design/language-architecture.md` and
  `docs/reference/universal-semantic-evidence.md` — required reading before
  touching universal adapters, evidence facts, resolution rules, or language
  cutovers.
- `docs/design/principles.md` — the rules behind AGENTS.md.
- `.impeccable.md` — viewer/extension design direction (restrained workbench
  aesthetic, VS Code theme tokens, evidence over implication).
- Docs distinguish shipped behavior from plans; never cite anything under
  `docs/superpowers/` as shipped evidence.

<!-- compass:managed:start -->
## compass

When `compass-out/graph.json` exists, use the Compass knowledge graph as the
first navigation layer. If it is absent and the task needs repository-wide
architecture, dependency, history, or impact evidence, run `compass update .`
once and continue. Skip the build for a narrow task that already identifies the
files to edit or when the user asked not to create generated files.

Rules:

- Run `compass query "<question>"` before broad source searches
- Set `--budget N` to fit available context; when query or explain reports
  `next=N`, repeat the unchanged command with `--page N` and reach `next=none`
  before exhaustive claims
- Use `compass path "<source>" "<target>"` for dependency paths
- Use `compass explain "<concept>"` for one concept and its neighbors
- Use `compass affected "<symbol>"` for change-review scope
- Read `compass-out/GRAPH_REPORT.md` for broad architecture
- Navigate `compass-out/wiki/index.md` when the wiki exists
- Run `compass update .` after code changes unless the user prohibited generated files
- Verify important graph conclusions in the cited source
- Treat missing paths and inferred edges as uncertain evidence, not proof
- Keep explicit `--graph`, `--at`, provider, and output selections unchanged
- Report failed refreshes; an older graph file does not make a failed update current
<!-- compass:managed:end -->
