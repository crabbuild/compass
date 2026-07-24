# Compass compatibility ledger

Compass is a native Rust implementation of Graphify. The Rust workspace uses a
frozen Python Graphify checkout as a development oracle, but releases ship only
the `compass` executable.

The machine-readable [`compatibility.toml`](compatibility.toml) manifest is the
authoritative compatibility identity. This ledger explains that contract for
humans; `scripts/check_compatibility_manifest.py --check` rejects drift between
the manifest, this document, and CI.

## Frozen oracle

- Python release base: Graphify `v0.9.20` at
  `edec9eabeceeae6aa2375eddb3835efa1a32c0a3`
- Qualified oracle checkpoint:
  `de0806be7c95d97aa7ff40371a235da899d6edb0`
- Oracle runtime: the repository's pinned Python environment
- Native implementation root: the Compass repository root

The qualified checkpoint is one commit after the release base. It adds
deterministic R extraction and its fixture, which the merged R parity suite
requires. The exact checkpoint, rather than the mutable `v8` branch, is the
behavioral oracle. Any future Python behavior change must be added below before
it can be called compatible.

## Upstream lineages

Graphify `origin/main` is tracked as a capability audit, not as the
byte-compatible behavioral oracle:

- audited main commit:
  `91f4d120b630ee35c79bf3c75ccd186870a808f9`;
- main lineage: Graphify v1, package version `0.1.14`;
- common ancestor with the v8 lineage:
  `81a43f028ff1d3fd9a0893318272348a38dad660`.

The main and v8 lines diverged after that ancestor. A main-only capability is
classified in `compatibility.toml` as `compatible`, `superseded`,
`intentional-divergence`, or `not-supported`. Main capability coverage does not
change the frozen oracle's byte, graph, traversal, or CLI contracts. Advancing
either record requires an immutable commit, an audited delta, fixtures, and
updated evidence in the same change.

## Compatibility contract

Selected Compass commands retain differential fixtures for argument forms, exit
status, graph schemas, deterministic ordering, and native extraction behavior.
The Python oracle writes `graphify-out/`; Compass writes `compass-out/`. Runtime
paths and sidecar names are not a compatibility contract.

The frozen Python query renderer has one inherently unstable behavior: nodes
with equal degree are emitted from `set` iteration, so their relative order
changes with Python's hash seed and runtime. Compass does not reproduce that
runtime accident. It orders those ties by stable node ID. Differential query
qualification therefore requires an exact header and exact complete line
multiset; every non-query read command remains byte-compared. Graph artifacts
are likewise compared as ordered-independent node/edge records because the
Python file walk order is platform-dependent, while Compass persists a stable
order. This is the sole approved ordering normalization; node/edge attributes,
multiplicity, ranking, traversal membership, and duplicate output lines must
still match exactly.

The released binaries do not start Python and do not load tree-sitter grammars
at runtime. Python is a development and CI oracle only.

`compass query --cql` is a Compass-native product surface. The frozen Python
oracle has no equivalent flag.

## Certified command families

| Family | Native entry points | Differential evidence |
| --- | --- | --- |
| Build | `update`, `extract`, `watch`, `cluster-only`, `label` | cold, warm, changed, rename/delete, graph, cache, manifest, report, and CLI parity fixtures |
| Query | `query`, `path`, `explain`, `affected`, `tree`, `benchmark` | Python-generated and native graphs, legacy `edges`, stable ranking, traversal, budgets, and output snapshots |
| Graph operations | `export`, `diagnose multigraph`, `merge-graphs`, `merge-driver`, `merge-chunks`, `merge-semantic`, `cache-check` | structure, attributes, ordering, conflict, malformed-input, and round-trip fixtures |
| Service | `serve` | official MCP client oracle, all tools/resources, stdio and HTTP transport, authentication and limits |
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

CI tests cover:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

Release packaging currently covers `x86_64-apple-darwin` and
`aarch64-apple-darwin`. Each archive contains `compass`, license notices, and
completions. A separate file records the SHA-256 checksum, and the automated
workflow records build provenance. Cargo packages are verified independently
from workspace builds.

## Evidence commands

From the Compass repository root, with Graphify checked out as a sibling:

```bash
python3 scripts/check_compatibility_manifest.py --check
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --locked -- -D warnings
cargo test --workspace --lib --bins --locked
cargo test -p compass-cli --test compass_product --locked
sh scripts/test_release_scripts.sh
cargo package --workspace --locked --no-verify
cargo deny check
```

Performance qualification is described in `PERFORMANCE.md` and release
automation is in `.github/workflows/compass-ci.yml`, `compass-hardening.yml`,
`compass-release.yml`, and `compass-publish.yml`. The hardening workflow also runs
the pinned mutation matrix and retains each result as release evidence.

## Post-baseline changes

| Python commit | Affected contract | Fixture/evidence | Compass status |
| --- | --- | --- | --- |
| `de0806be7c95d97aa7ff40371a235da899d6edb0` | deterministic `.r` and `Rscript` extraction | `r_extraction_matches_exactly`, `extensionless_shebang_extraction_matches_exactly` | compatible |

An entry is complete only after the native behavior and a differential
regression fixture land together. An incompatible or intentionally retired
behavior requires explicit approval and a migration note; silence is not an
accepted exception.

Main-line capability dispositions are maintained in `compatibility.toml`.
They are not duplicated in this post-baseline table because they do not advance
the frozen v8 behavioral contract.
