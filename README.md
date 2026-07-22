# Compass

Compass is the native Rust implementation of Graphify. It maps source code and
structured project files into a traversable knowledge graph without embeddings,
a vector database, Python, runtime grammar downloads, or separately installed
native libraries. Code remains fully local and deterministic; semantic formats
use the selected model provider.

The public command surface is exposed directly under `compass`:

```bash
compass update .
compass query "where is authentication enforced?"
compass query --cql 'MATCH (f:Function)-[:CALLS]->(a) RETURN f.id, a.id'
compass path LoginHandler SessionValidator
compass explain SessionValidator
compass affected SessionValidator
```

Only completed commands are exposed. `compass` is the authoritative CLI and
may evolve independently of the Python `graphify` command. The workspace also
builds a legacy `graphify` executable for the older command surface where
compatibility remains useful; it does not constrain Compass commands or help.

## Current native command surface

```text
compass update
compass extract
compass watch
compass serve
compass install
compass uninstall
compass cluster-only
compass query
compass query --cql
compass path
compass explain
compass diff
compass history
compass affected
compass tree
compass export
compass benchmark
compass diagnose multigraph
compass merge-graphs
compass cache-check
compass merge-chunks
compass merge-semantic
compass provider
compass save-result
compass reflect
compass check-update
compass hook-check
compass hook-guard
compass merge-driver
compass global
compass clone
compass add
compass label
compass prs
compass hook
```

## Versioned graph history

Compass can materialize a complete immutable graph for an exact Git commit and
keep it in a SQLite-backed Prolly store outside Git history. A realization
contains the AST graph, semantic and inferred edges, hyperedges, community and
analysis data, reconstruction metadata, and authoritative sidecars. History is
opt-in for eager generation; explicit builds and lazy historical queries remain
available while eager generation is disabled.

```bash
compass history enable
compass history build HEAD
compass query "authentication flow" --at HEAD~20
compass diff v1.2.0 HEAD --detailed
compass history export HEAD --format graphify-out --output historical-output
compass history list HEAD --format json
compass history gc
compass history disable
```

`compass history enable` records a repository-wide build profile and installs
managed `post-commit` and `post-merge` hooks. The hooks capture the resulting
commit SHA and durably enqueue work, then return without waiting for extraction.
The worker uses leases and a FIFO queue; a failed job does not prevent later
jobs from running. `disable` is idempotent: it stops eager enqueueing but keeps
the database, jobs, and existing realizations. It does not disable explicit
`build`/`rebuild`, `--at`, or `diff`.

`query`, `path`, and `explain` accept either `--graph PATH` or `--at REV`.
`--at` resolves the revision to an exact commit. If its preferred realization
is missing, Compass synchronously builds it in a detached, offline worktree;
uncommitted files and caller-local `.git/info/exclude` or global-ignore rules do
not enter the build. The committed `.gitignore` still applies. Gitlinks and LFS
pointers are reported as limitations, and checkout filters that could execute
external code are rejected. Historical materialization does not run hooks,
smudge LFS objects, prompt for credentials, fetch from the network, or recurse
into submodules.

Every meaning-affecting input is captured in an extraction fingerprint,
including the build profile, graph and canonical-encoding versions, parser and
analyzer versions, and provider/model configuration. Credentials, machine-local
paths, timings, token counts, and other operational data are excluded. The same
commit may therefore have multiple immutable realizations; one validated,
complete realization is the preferred default. Use `history list`, `show`, and
`prefer` to inspect or select them. An unreadable preferred pointer is never
silently overwritten: recover it only with an explicit
`compass history rebuild REV --replace-corrupt`, which uses an exact
compare-and-swap observation.

All linked worktrees share
`$(git rev-parse --git-common-dir)/compass/history.sqlite`. The pinned
`prolly-store-sqlite` adapter runs SQLite in WAL mode with full synchronous
durability and a busy timeout. The database, WAL, and operational files are
live resources—do not copy only `history.sqlite` while Compass is running.
Compass creates the resource directory and operational records with owner-only
permissions. Jobs, leases, locks, and protected temporary worktrees are files
beside the database rather than Prolly values.

`history export --format graph-json` reconstructs the canonical graph JSON.
`--format graphify-out` also restores authoritative, non-derivable sidecars
verbatim and regenerates reports and HTML only with the renderer versions
recorded in the artifact registry. Export equivalence is semantic and
canonical: insignificant JSON member or record ordering is not part of the
contract, while graph structure, attributes, duplicate id-less hyperedges, and
authoritative bytes are.

Normal `history gc` retains every published realization and removes only
unreachable Prolly nodes plus expired operational records. Pruning alternate
realizations requires `--prune-non-preferred`; it is a dry run until repeated
with `--yes`. Reported bytes and node rows are logical reclamation. The command
does not promise that the SQLite file shrinks or run `VACUUM`.

Text output is intended for people; `--format json` emits stable JSON for the
history commands that support it. Successful queries and no-store read-only
status/list operations exit `0` and do not create `.git/compass`. CLI usage
errors exit `2`; Git, provider, validation, corruption, and storage failures
exit `1`, with diagnostics on stderr. Complete semantic builds require the
selected provider's credentials when a committed input needs model extraction;
a provider failure cannot publish or become preferred.

Scripts and new documentation should use `compass`. The separately installed
`graphify` binary is a best-effort legacy entry point for its existing tested
surface, not an alias contract for new Compass features. In particular,
versioned history is specified, documented, and qualified through `compass`;
its commands and help do not need a matching Python Graphify surface.

Assistant setup is native and self-contained. The generic
`compass install --platform <name>` and project-scoped `--project` forms,
their uninstall counterparts, and every legacy direct platform command exposed
by `graphify` are differential-tested against Python for both terminal output
and the complete installed file tree.

The deterministic language registry is checked against every extension handled
by Python, and all Tree-sitter grammars are statically linked into the binary.
Graph edges retain their `EXTRACTED`, `INFERRED`, or `AMBIGUOUS` provenance.

`compass extract` now combines local AST facts with native semantic
extraction for documents, papers, PDFs, office files, and images. It supports
built-in and trusted custom providers, standard/deep cache namespaces, adaptive
chunk recovery, root-confined image loading, bounded pure-Rust PDF/DOCX/XLSX
ingestion, and guarded incremental replacement. Use `--code-only` to guarantee
that no model is invoked; add `--cargo` to include workspace-internal crate
dependency edges from local Cargo manifests, `--postgres DSN` for read-only
schema introspection, or `--google-workspace` to export Drive shortcuts through
the configured `gws` CLI. Integrations that have not reached compatibility are
rejected explicitly instead of being accepted silently.

`compass export neo4j` and `compass export falkordb` emit compatible
OpenCypher locally. Adding `--push URI` performs native, bounded live upserts:
Neo4j uses Bolt (including verified TLS and explicit self-signed modes), while
FalkorDB uses RESP directly. Passwords can come from `NEO4J_PASSWORD` or
`FALKORDB_PASSWORD` and are redacted from failures; neither path needs Python,
a database SDK, or a native client library.

`compass serve` exposes the completed query, graph-inspection, resource,
and PR-impact surface over MCP. Stdio is the default for editor integrations;
`--transport http` enables Streamable HTTP with stateful or stateless
operation, bounded request bodies, DNS-rebinding checks, optional API-key
authentication, session expiry, and graceful shutdown. The same package
installs `graphify-mcp` as a drop-in compatibility entry point:

```bash
compass serve graphify-out/graph.json
compass serve --transport http --api-key "$GRAPHIFY_API_KEY"
graphify-mcp --graph graphify-out/graph.json
```

Native Whisper inference, verified model and URL artifact acquisition, and
bounded audio/video decoding—including AVI—are implemented internally; their
public commands remain hidden until the corresponding Python command workflows
pass strict compatibility tests.

## Install from source

Rust 1.97.1 or newer is required to compile Compass:

```bash
git clone https://github.com/crabbuild/compass.git
cd compass
cargo install --locked --path crates/compass-cli
compass --help
```

No Python environment is needed by the installed binaries. Python is used only
by the development parity suite.

After a release is published to crates.io, the registry install is:

```bash
cargo install --locked compass-cli
```

## Build and verify

```bash
cargo build --release --locked --bins
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

The scheduled hardening workflow additionally enforces native line coverage,
runs focused mutation suites for stable IDs, invalidation, query scoring, graph
guards, and compatibility mappings, runs the safe graph model and traversal
crates under Miri, executes the native workspace with AddressSanitizer, and
fuzzes hostile graph JSON, source code, ignore files, manifests, CLI arguments,
renderers, semantic fragments, and AVI/audio containers before they can reach
graph construction or transcription.

The compatibility tests use a sibling Graphify checkout as the behavioral
oracle. Set `GRAPHIFY_REPO_ROOT` when it is elsewhere. Office/PDF dependencies
live in a separate `.venv-media`
environment so installing `lxml` cannot alter unrelated GraphML oracle output.
Set `GRAPHIFY_PYTHON` and `GRAPHIFY_MEDIA_PYTHON` when those interpreters are in
different locations.

Performance methodology, the reproducible qualification harness, and the
current local baseline are documented in [PERFORMANCE.md](PERFORMANCE.md).
The frozen compatibility baseline and evidence map are documented in
[COMPATIBILITY.md](COMPATIBILITY.md). Side-by-side adoption and recovery are
documented in [MIGRATION.md](MIGRATION.md).

The deterministic structural-query language is documented in
[docs/COMPASSQL.md](docs/COMPASSQL.md), with its exact accepted/rejected surface
in [docs/COMPASSQL_SUPPORT.md](docs/COMPASSQL_SUPPORT.md).

## Distribution

`compass-release.yml` builds native archives for Linux, macOS, and Windows on both
x86-64 and ARM64. Every archive contains standalone `compass`, compatibility
`graphify`, and `graphify-mcp` executables, a SHA-256 checksum, and GitHub build-provenance
attestation. The crate manifests are package-ready for an ordered crates.io
publish. `compass-publish.yml` is a separately approved environment-protected
workflow that validates an exact release tag and confirmation string, then
publishes the crates in dependency order.
