# AGENTS.md

This file is the repository-level operating guide for AI coding agents. It
applies to the entire tree unless a more specific `AGENTS.md` exists below the
directory being changed.

## Start here

Before editing:

1. Read the affected crate's `src/lib.rs`, its `Cargo.toml`, and the nearest
   tests.
2. Find the closest existing implementation and follow its ownership boundary.
3. Read `COMPATIBILITY.md` for any public command, format, schema, storage, or
   integration change.
4. Use `docs/implementation/workspace-tour.md` to route work and
   `docs/implementation/extending-compass.md` for extension-specific checklists.
5. Check `git status --short`. Preserve user changes and keep unrelated edits
   out of the patch.

`CONTRIBUTING.md` is the canonical human contribution guide. The design rules
behind this file live in `docs/design/principles.md`.

## Build artifacts and external checkouts

Compass build artifacts are large. Where they live is a per-contributor
environment choice, not a repository rule — this guide does not mandate any
specific path or volume.

- If you redirect Cargo output with `CARGO_TARGET_DIR`, give every repository
  checkout and worktree its own directory. Never share one target directory
  between different repositories or concurrent worktrees: feature sets, build
  scripts, and locks can collide.
- `CARGO_TARGET_DIR` does not persist between shell or tool invocations. Set it
  on each command that can compile (`build`, `check`, `test`, `clippy`, `bench`,
  `doc`, `install`, `package`, or a Make target that invokes one).
- Verify the chosen directory exists and is writable before a long build.
- Run `cargo clean` only with the intended `CARGO_TARGET_DIR` explicitly set,
  and only when the task actually requires reclaiming or invalidating those
  artifacts. Never clean another repository's target directory.
- Treat external repositories used to qualify Compass code graphs as read-only
  inputs. Do not modify, update, reset, or clean an existing checkout unless the
  task explicitly requires it. Keep generated Compass artifacts outside their
  tracked source, or remove only artifacts created by the current task. Do not
  clone qualification repositories into the Compass tree.

Some Makefile targets consume binaries through a literal local `target/` path
after Cargo finishes. If you have redirected `CARGO_TARGET_DIR`, prefer direct
Cargo commands for normal verification, and inspect packaging, install, or
release targets before use so they do not silently trigger a second build.

## Prometheus state ownership

`.prometheus/knowledge/wiki/**` is repository-owned project knowledge and must
remain tracked, including generated session transcripts and their embedded
machine-specific project paths. Treat every other `.prometheus/**` path as local
runtime or tooling state unless a repository rule or explicit user instruction
classifies that path as tracked content. Never discard or exclude the wiki merely
because it lives beneath a hidden tooling directory.

## Product invariants

- Compass is a native, local-first Rust product. Structural extraction and
  graph queries must continue to work without Python, model credentials,
  embeddings, a vector database, runtime grammar downloads, or Graphify.
- Do not introduce Graphify runtime, test, configuration, artifact, or fallback
  dependencies. `scripts/check_product_boundary.sh` enforces this boundary.
- Preserve deterministic discovery, identities, ordering, canonical encoding,
  and output for equivalent inputs. Never resolve ambiguity by selecting the
  first or most convenient candidate.
- Preserve relationship direction, multiplicity, source anchors, and
  provenance. Prefer an explicit unresolved/ambiguous result over invented
  meaning.
- All work over source files, graphs, archives, network responses, queries, and
  subprocess output must remain bounded. A limit error is not an empty result.
- Treat inputs from files, providers, networks, databases, and subprocesses as
  untrusted. Validate before publishing or rendering them.
- Publish coherent artifact sets using the existing validation, staging,
  build-guard, and atomic-write primitives. Do not leave partial output that
  appears successful.
- Keep machine contracts versioned and typed. Unknown major versions must fail
  explicitly; do not make consumers parse human-readable prose.
- Published historical realizations are immutable. Do not rewrite them in
  place or silently substitute a different realization/profile.
- Optional network, credential, and process boundaries must remain explicit.
  Tests must use fixtures or local mocks, never real credentials or services.

## Workspace ownership

The released binary is package `compass-cli`, binary `compass`, with entry point
`crates/compass-cli/src/bin/compass.rs`.

Route changes to the lowest crate that owns the behavior:

| Area | Owner |
| --- | --- |
| Graph records, validation, indexes, public graph contract | `compass-model` |
| File discovery, ignore/scope policy, cache/manifest, atomic I/O | `compass-files` |
| Per-file syntax facts and language registry | `compass-languages` |
| Cross-file imports, calls, members, aliases, and identity | `compass-resolve` |
| Deduplication, graph publication, clustering, topology analysis | `compass-graph` |
| Application build/watch/merge/materialization orchestration | `compass-core` |
| CompassQL lexer, parser, semantics, planner | `compass-cypher` |
| Search, traversal, impact, CompassQL execution | `compass-query` |
| Reports, HTML, JSON, SVG, GraphML, and other renderers | `compass-output` |
| Public arguments, help, streams, exit codes, command side effects | `compass-cli` |
| MCP tools/resources and stdio/HTTP transports | `compass-mcp` |
| Immutable SQLite/Prolly history, fingerprints, leases, GC | `compass-history` |
| Evidence-gated comparison of historical realizations | `compass-semantic-diff` |
| Provider fragments and semantic validation/orchestration | `compass-semantic` |
| Provider-neutral Program IR and analysis | `compass-ir`, `compass-program`, `compass-analysis` |
| Media, ingestion, and transcription boundaries | `compass-media`, `compass-ingest`, `compass-transcribe`, `compass-whisper` |
| Focused external integrations | `compass-cargo`, `compass-global`, `compass-google-workspace`, `compass-graphdb`, `compass-postgres`, `compass-prs`, `compass-reflect` |
| Shared React viewer | `packages/compass-viewer` |
| VS Code integration | `editors/vscode` |
| Browser-level viewer tests | `tests/viewer` |

Keep CLI and MCP layers thin. Reusable behavior belongs in an owning domain
crate; presentation-only transformations belong in `compass-output`. Per-file
extractors emit evidence, while project-wide target selection belongs in
`compass-resolve`.

Language work spans several explicit boundaries:

```text
classification        compass-files + language registry
parser/query wiring   vendor/compass-tree-sitter-language-pack
per-file facts        compass-languages
cross-file resolution compass-resolve
publication evidence  compass-graph + fixtures/qualification
```

Read `docs/design/language-architecture.md` and
`docs/reference/universal-semantic-evidence.md` before changing universal
adapters, evidence facts, resolution rules, or language cutovers. Do not add a
universal adapter profile without direct evidence emission, validation,
resolution, and qualification in the same change.

## Rust conventions

- Use the pinned Rust 1.97.1 toolchain, Edition 2024, and `--locked` for normal
  build/test commands.
- Workspace policy forbids unsafe code and denies Clippy `all`, `unwrap_used`,
  `expect_used`, and `panic`. Return typed errors with actionable context.
- Prefer deterministic collections (`BTreeMap`/`BTreeSet`) or explicit sorting
  at contract boundaries. Do not rely on hash iteration or filesystem order.
- Reuse workspace dependencies from root `Cargo.toml`; add a crate dependency
  with `{name}.workspace = true`. Justify new dependencies and keep
  `Cargo.lock` updated when dependency resolution changes.
- Keep public types and stable serialized values explicit. If meaning changes,
  review schema, language/planner, cache, renderer, or fingerprint versions.
- Preserve unknown graph attributes unless the relevant contract explicitly
  rejects them.
- Use the existing bounded readers, subprocess helpers, path containment,
  endpoint checks, and atomic write functions rather than creating weaker
  local variants.
- Avoid shell command construction for subprocess integrations. Pass arguments
  separately and bound duration plus captured output.
- Keep platform behavior portable across Linux, macOS, and Windows. Do not
  assume UTF-8 paths, Unix separators, or Unix-only process behavior without a
  guarded platform implementation and test.

## Tests and fixtures

Behavior changes require a regression test at the lowest useful layer plus an
interface/contract test when user-visible behavior changes.

- Unit tests normally live beside implementation in `#[cfg(test)]` modules.
- Cross-module and public-contract tests live in `crates/<crate>/tests/`.
- CLI behavior is tested by executing the binary under
  `crates/compass-cli/tests/`; assert streams, exit status, files, and machine
  schema where applicable.
- Language/resolution tests must cover ambiguity and negative cases, not just a
  happy-path symbol match. Assert identity, direction, occurrence/source
  range, provenance, multiplicity, and deterministic ordering.
- History/storage changes need reopen/round-trip, corruption or interruption,
  publication atomicity, diff, and compatibility coverage as applicable.
- Network/provider work uses local mock servers. Subprocess work uses bounded
  fake runners or controlled commands.
- Reuse fixtures under `fixtures/` and keep expected outputs reviewable and
  deterministic. Update snapshots/fixtures only when the semantic change is
  intentional and explain the contract change.

Run the narrowest useful checks while iterating:

```bash
cargo test -p <crate> --locked
cargo test -p <crate> --test <integration_test> --locked
cargo clippy -p <crate> --all-targets --all-features --locked -- -D warnings
```

Before finishing a Rust change, run the repository's native baseline:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --locked -- -D warnings
cargo test --workspace --lib --bins --locked
```

Also run the gates matching the changed surface:

```bash
# Public CLI/product identity
cargo test -p compass-cli --test compass_product --locked
sh scripts/check_product_boundary.sh

# CompassQL grammar, execution, or support claims
cargo test -p compass-cypher --test tck --locked
cargo test -p compass-query --test opencypher_tck --locked
python3 scripts/check_compassql_support.py

# Code-graph publication, languages, resolver, viewer contracts
./scripts/qualify_code_graph_v1.sh --fixtures-only

# JavaScript/viewer/VS Code
npm ci
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
```

`make test`, `make lint`, `make test-js`, and
`make qualify-code-graph-v1` are convenient wrappers. CI also exercises
nextest, packaging/install, dependency policy, integration tests, and native
platform matrices; consult `.github/workflows/compass-ci.yml` when touching
those surfaces. Report any relevant check not run and why.

## Public contracts and documentation

Treat CLI arguments/help/exits, environment variables, configuration, graph
JSON, CompassQL, MCP schemas, output files, history formats, and stable IDs as
compatibility-sensitive.

For an incompatible user-visible change, include all of:

1. native regression coverage;
2. updated command/format/reference documentation;
3. a migration note in `MIGRATION.md` when users must act;
4. a `CHANGELOG.md` entry when release-visible.

Update `SECURITY.md` or the security/privacy design docs when a trust,
credential, network, path, subprocess, or disclosure boundary changes. Update
`PERFORMANCE.md` and run the documented qualification when making performance
claims. Correctness and deterministic equivalence take priority over speed.

Docs distinguish current behavior from plans. Do not describe a design under
`docs/superpowers/` as shipped evidence. Follow the concept/guide/cookbook/
reference organization described in `docs/README.md`.

## Generated and vendored files

- `crates/compass-output/assets/viewer/graph.js`, `viewer.css`, and
  `manifest.json` are generated from `packages/compass-viewer`. Change the
  source, then run `node scripts/build_viewer_assets.mjs`; verify with
  `node scripts/check_viewer_assets.mjs`.
- Do not hand-edit build outputs under ignored `dist/` or `target/` paths.
- Avoid modifying `vendor/` unless the task explicitly requires a vendored
  parser or dependency patch. Preserve licenses, attribution, patch notes, and
  reproducibility. Keep runtime parser behavior statically linked; do not add
  downloads to normal execution.
- Do not commit generated graphs, local `.compass/` state, `compass-out/`,
  credentials, machine-specific paths, or private repository content.

## Completion checklist

Before handing off:

- inspect `git diff` and `git status --short` for unrelated or generated noise;
- confirm the change lives at the correct ownership boundary;
- confirm failure paths, limits, cleanup, determinism, and platform behavior;
- add/update tests and user documentation for changed behavior;
- run targeted checks and the applicable baseline/gates;
- summarize the behavior change, compatibility effect, and exact verification
  performed, including checks not run.
