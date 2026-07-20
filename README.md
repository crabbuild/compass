# Trail

Trail is the native Rust implementation of Graphify. It maps source code and
structured project files into a traversable knowledge graph without an LLM,
embeddings, a vector database, Python, runtime grammar downloads, or separately
installed native libraries.

The public command surface is namespaced under `trail graph`:

```bash
trail graph update .
trail graph query "where is authentication enforced?"
trail graph path LoginHandler SessionValidator
trail graph explain SessionValidator
trail graph affected SessionValidator
```

Only completed commands are exposed. The workspace also builds a `graphify`
compatibility executable, but it exposes only commands that pass differential
tests against the Python implementation.

## Current native command surface

```text
trail graph update
trail graph extract --code-only
trail graph watch
trail graph cluster-only
trail graph query
trail graph path
trail graph explain
trail graph affected
trail graph tree
trail graph export
trail graph benchmark
trail graph diagnose multigraph
trail graph merge-graphs
```

The deterministic language registry is checked against every extension handled
by Python, and all Tree-sitter grammars are statically linked into the binary.
Graph edges retain their `EXTRACTED`, `INFERRED`, or `AMBIGUOUS` provenance.

Semantic extraction for prose, PDFs, images, office files, audio, and video is a
later compatibility phase. Until that phase lands, `trail graph update` preserves
an existing semantic layer but never invokes a model or sends data off-machine.
The native semantic boundary now includes validated provider contracts,
adaptive chunk recovery, root-confined image loading, and bounded pure-Rust
PDF/DOCX/XLSX text ingestion. These components are differential-tested but
remain internal until the complete extraction path meets the same compatibility
standard.

## Install from source

Rust 1.97.1 or newer is required to compile Trail:

```bash
cd rust
cargo install --locked --path crates/trail-cli
trail graph --help
```

No Python environment is needed by the installed binaries. Python is used only
by the development parity suite.

After a release is published to crates.io, the registry install is:

```bash
cargo install --locked trail-cli
```

## Build and verify

```bash
cd rust
cargo build --release --locked --bins
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

The scheduled hardening workflow additionally enforces native line coverage,
runs the safe graph model and traversal crates under Miri, executes the native
workspace with AddressSanitizer, and fuzzes hostile graph JSON/query input.
It separately fuzzes untrusted semantic fragments before they can reach graph
construction.

The compatibility tests use the Python checkout at the repository root as the
behavioral oracle. Office/PDF dependencies live in a separate `.venv-media`
environment so installing `lxml` cannot alter unrelated GraphML oracle output.
Set `GRAPHIFY_PYTHON` and `GRAPHIFY_MEDIA_PYTHON` when those interpreters are in
different locations.

Performance methodology, the reproducible qualification harness, and the
current local baseline are documented in [PERFORMANCE.md](PERFORMANCE.md).

## Distribution

`rust-release.yml` builds native archives for Linux, macOS, and Windows on both
x86-64 and ARM64. Every archive contains standalone `trail` and compatibility
`graphify` executables, a SHA-256 checksum, and GitHub build-provenance
attestation. The crate manifests are package-ready for an ordered crates.io
publish. `rust-publish.yml` is a separately approved environment-protected
workflow that validates an exact release tag and confirmation string, then
publishes the crates in dependency order.
