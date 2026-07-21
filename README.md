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
trail graph serve
trail graph install
trail graph uninstall
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
trail graph reflect
trail graph check-update
trail graph hook-check
trail graph hook-guard
trail graph merge-driver
trail graph global
trail graph clone
trail graph add
trail graph label
trail graph prs
trail graph hook
```

Assistant setup is native and self-contained. The generic
`trail graph install --platform <name>` and project-scoped `--project` forms,
their uninstall counterparts, and every legacy direct platform command exposed
by `graphify` are differential-tested against Python for both terminal output
and the complete installed file tree.

The deterministic language registry is checked against every extension handled
by Python, and all Tree-sitter grammars are statically linked into the binary.
Graph edges retain their `EXTRACTED`, `INFERRED`, or `AMBIGUOUS` provenance.

`trail graph extract` now combines local AST facts with native semantic
extraction for documents, papers, PDFs, office files, and images. It supports
built-in and trusted custom providers, standard/deep cache namespaces, adaptive
chunk recovery, root-confined image loading, bounded pure-Rust PDF/DOCX/XLSX
ingestion, and guarded incremental replacement. Use `--code-only` to guarantee
that no model is invoked; add `--cargo` to include workspace-internal crate
dependency edges from local Cargo manifests, `--postgres DSN` for read-only
schema introspection, or `--google-workspace` to export Drive shortcuts through
the configured `gws` CLI. Integrations that have not reached compatibility are
rejected explicitly instead of being accepted silently.

`trail graph export neo4j` and `trail graph export falkordb` emit compatible
OpenCypher locally. Adding `--push URI` performs native, bounded live upserts:
Neo4j uses Bolt (including verified TLS and explicit self-signed modes), while
FalkorDB uses RESP directly. Passwords can come from `NEO4J_PASSWORD` or
`FALKORDB_PASSWORD` and are redacted from failures; neither path needs Python,
a database SDK, or a native client library.

`trail graph serve` exposes the completed query, graph-inspection, resource,
and PR-impact surface over MCP. Stdio is the default for editor integrations;
`--transport http` enables Streamable HTTP with stateful or stateless
operation, bounded request bodies, DNS-rebinding checks, optional API-key
authentication, session expiry, and graceful shutdown. The same package
installs `graphify-mcp` as a drop-in compatibility entry point:

```bash
trail graph serve graphify-out/graph.json
trail graph serve --transport http --api-key "$GRAPHIFY_API_KEY"
graphify-mcp --graph graphify-out/graph.json
```

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
cargo test --workspace --all-targets --all-features --locked
```

The scheduled hardening workflow additionally enforces native line coverage,
runs focused mutation suites for stable IDs, invalidation, query scoring, graph
guards, and compatibility mappings, runs the safe graph model and traversal
crates under Miri, executes the native workspace with AddressSanitizer, and
fuzzes hostile graph JSON/query input.
It separately fuzzes untrusted semantic fragments and AVI/audio containers
before they can reach graph construction or transcription.

The compatibility tests use the Python checkout at the repository root as the
behavioral oracle. Office/PDF dependencies live in a separate `.venv-media`
environment so installing `lxml` cannot alter unrelated GraphML oracle output.
Set `GRAPHIFY_PYTHON` and `GRAPHIFY_MEDIA_PYTHON` when those interpreters are in
different locations.

Performance methodology, the reproducible qualification harness, and the
current local baseline are documented in [PERFORMANCE.md](PERFORMANCE.md).
The frozen compatibility baseline and evidence map are documented in
[COMPATIBILITY.md](COMPATIBILITY.md). Side-by-side adoption and recovery are
documented in [MIGRATION.md](MIGRATION.md).

## Distribution

`rust-release.yml` builds native archives for Linux, macOS, and Windows on both
x86-64 and ARM64. Every archive contains standalone `trail`, compatibility
`graphify`, and `graphify-mcp` executables, a SHA-256 checksum, and GitHub build-provenance
attestation. The crate manifests are package-ready for an ordered crates.io
publish. `rust-publish.yml` is a separately approved environment-protected
workflow that validates an exact release tag and confirmation string, then
publishes the crates in dependency order.
