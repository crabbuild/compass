# Trail compatibility ledger

Trail is a native Rust implementation of Graphify with a strict compatibility
adapter. The compatibility executable is `graphify`; the primary product
interface is `trail graph`.

## Frozen oracle

- Python baseline: Graphify `v0.9.20`
- Baseline commit: `edec9eabeceeae6aa2375eddb3835efa1a32c0a3`
- Oracle runtime: the repository's pinned Python environment
- Native implementation root: `rust/`

There are no committed changes to `graphify/` between the frozen baseline and
the current implementation checkpoint. A future Python behavior change must be
added below before it can be called compatible.

## Compatibility contract

For every command exposed by the `graphify` binary, parity includes argument
forms, exit status, stdout and stderr, graph and sidecar schemas, cache and
manifest behavior, deterministic ordering, installed files, and mutation of
existing `graphify-out/` directories. Tests normalize only declared sources of
variability such as temporary roots, elapsed time, and frozen clock values.

The released binaries do not start Python and do not load tree-sitter grammars
at runtime. Python is a development and CI oracle only.

## Certified command families

| Family | Native entry points | Differential evidence |
| --- | --- | --- |
| Build | `update`, `extract`, `watch`, `cluster-only`, `label` | cold, warm, changed, rename/delete, graph, cache, manifest, report, and CLI parity fixtures |
| Query | `query`, `path`, `explain`, `affected`, `tree`, `benchmark` | Python-generated and native graphs, legacy `edges`, stable ranking, traversal, budgets, and output snapshots |
| Graph operations | `export`, `diagnose multigraph`, `merge-graphs`, `merge-driver`, `merge-chunks`, `merge-semantic`, `cache-check` | structure, attributes, ordering, conflict, malformed-input, and round-trip fixtures |
| Service | `serve` and `graphify-mcp` | official MCP client oracle, all tools/resources, stdio and HTTP transport, authentication and limits |
| Project workflows | `global`, `clone`, `add`, `prs`, `hook`, `provider`, `save-result`, `reflect`, `check-update`, `hook-check`, `hook-guard` | command-specific Python oracles and native integration tests |
| Assistant setup | `install`, `uninstall`, and direct platform commands | stdout/stderr plus complete global/project filesystem-tree comparison for every supported platform |

The deterministic registry is checked against every Python code extension.
SCIP, Cargo manifests, PostgreSQL, Google Workspace, documents, PDF, Office,
images, semantic providers, Neo4j, and FalkorDB use native implementations.
Audio/video decoding and Whisper inference are native internals; no additional
public transcription command is exposed because the frozen Python CLI has no
such command.

## Assistant platform matrix

The generic installer covers Claude, Windows, CodeBuddy, Codex, OpenCode, Kilo,
Aider, Copilot, Claw, Droid, Trae, Trae CN, Hermes, Kiro, Pi, Amp, Agents,
Skills, Devin, Antigravity, Gemini, Cursor, and the compatibility aliases Kimi
and Antigravity Windows where accepted by Python. Direct legacy lifecycle
commands are tested for every direct command printed by Python help, including
VS Code.

The installer assets are embedded in the binary. Installation therefore does
not depend on a source checkout, Python package, or network access.

## Platform and distribution matrix

CI tests and release packaging cover:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

Each release archive contains `trail`, `graphify`, and `graphify-mcp`, license
notices, completions, an SPDX SBOM, a SHA-256 checksum, and build-provenance
attestation. Cargo packages are verified independently from workspace builds.

## Evidence commands

From `rust/`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
GRAPHIFY_PYTHON=../.venv/bin/python cargo test --workspace --all-targets --locked
cargo package --workspace --locked --no-verify
cargo deny check
```

Performance qualification is described in `PERFORMANCE.md` and release
automation is in `.github/workflows/rust-ci.yml`, `rust-hardening.yml`,
`rust-release.yml`, and `rust-publish.yml`.

## Post-baseline changes

No Python implementation deltas are pending. Add one row per future delta:

| Python commit | Affected contract | Fixture/evidence | Trail status |
| --- | --- | --- | --- |
| _none_ | — | — | — |

An entry is complete only after the native behavior and a differential
regression fixture land together. An incompatible or intentionally retired
behavior requires explicit approval and a migration note; silence is not an
accepted exception.
