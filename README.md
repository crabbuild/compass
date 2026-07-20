# Trail

Trail is the native Rust implementation of Graphify. It maps source code and
structured project files into a traversable knowledge graph without embeddings,
a vector database, Python, runtime grammar downloads, or separately installed
native libraries. Code remains fully local and deterministic; semantic formats
use the selected model provider.

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
trail graph extract
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
trail graph cache-check
trail graph merge-chunks
trail graph merge-semantic
trail graph provider
trail graph save-result
```

The deterministic language registry is checked against every extension handled
by Python, and all Tree-sitter grammars are statically linked into the binary.
Graph edges retain their `EXTRACTED`, `INFERRED`, or `AMBIGUOUS` provenance.

`trail graph extract` now combines local AST facts with native semantic
extraction for documents, papers, PDFs, office files, and images. It supports
built-in and trusted custom providers, standard/deep cache namespaces, adaptive
chunk recovery, root-confined image loading, bounded pure-Rust PDF/DOCX/XLSX
ingestion, and guarded incremental replacement. Use `--code-only` to guarantee
that no model is invoked. Integrations that have not reached compatibility are
rejected explicitly instead of being accepted silently.

Native Whisper inference, verified model and URL artifact acquisition, and
bounded audio/video decoding—including AVI—are implemented internally; their
public commands remain hidden until the corresponding Python command workflows
pass strict compatibility tests.

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
It separately fuzzes untrusted semantic fragments and AVI/audio containers
before they can reach graph construction or transcription.

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
