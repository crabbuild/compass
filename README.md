# Compass: local knowledge graphs for your codebase

Compass turns source code and project artifacts into a searchable knowledge graph. Use it to find implementation paths, understand dependencies, estimate change impact, give coding assistants focused context, or export the graph to other tools.

Compass is inspired by [Graphify](https://github.com/Graphify-Labs/graphify),
built natively in Rust, and evolving independently beyond its original
compatibility baseline. Structural extraction and graph queries run locally
without Python, embeddings, a vector database, runtime grammar downloads, or
separately installed native libraries. Semantic extraction is optional and uses
only the model provider you configure.

> **Repository description:** Native, local-first knowledge graph engine for
> code and project artifacts—inspired by Graphify, built in Rust, and evolving
> beyond it.

## Documentation

Start with the path that matches your goal:

| Reader | Start here |
| --- | --- |
| Evaluating Compass | [Getting started](docs/getting-started.md) → [How it works](docs/concepts/how-it-works.md) |
| Using or integrating Compass | [Guides](docs/README.md#complete-a-task) → [Cookbook](docs/cookbook/README.md) → [Reference](docs/README.md#look-up-an-exact-contract) |
| Extending the Rust workspace | [Design principles](docs/design/principles.md) → [Architecture](docs/design/architecture.md) → [Workspace tour](docs/implementation/workspace-tour.md) |

The [documentation hub](docs/README.md) includes comprehensive guides for
versioned history, CompassQL, assistant setup, security, operations,
implementation, troubleshooting, and the status-qualified
[roadmap](docs/roadmap.md). Diagrams use portable ASCII or accessible SVG.

## Command surface and graph history

Only completed commands are exposed. `compass` is the authoritative and only
shipped CLI. It may evolve independently of the Python `graphify` command.

### Current native command surface

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

Run `compass --help` to see commands grouped by workflow. Run
`compass help <command>` or `compass <command> --help` for arguments,
option descriptions, defaults, examples, and related-command tips. Nested help
accepts the full path, such as `compass help history build`.

### Versioned graph history

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
compass history export HEAD --format compass-out --output historical-output
compass history list HEAD --format json
compass history gc
compass history disable
```

For a fully local code graph with no model credentials, select a code-only
history profile explicitly:

```bash
compass history enable --code-only
compass diff HEAD~1 HEAD --topology-only
```

Code-only and semantic realizations have different extraction fingerprints and
are never mixed by a normal diff. Compass does not silently downgrade a
semantic profile when provider credentials are missing.

To qualify history correctness and performance against two commits in a clean
real repository checkout, run:

```bash
scripts/qualify_history_real_repo.sh /path/to/repository OLD NEW
```

The harness builds in an isolated shared clone, checks deterministic and
reverse-symmetric JSON, reopens the SQLite store, verifies topology filtering,
and requires topology-only diff to be at least twice as fast as a full diff.

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
live resources. Don't copy only `history.sqlite` while Compass is running.
Compass creates the resource directory and operational records with owner-only
permissions. Jobs, leases, locks, and protected temporary worktrees are files
beside the database rather than Prolly values.

`history export --format graph-json` reconstructs the canonical graph JSON.
`--format compass-out` also restores authoritative, non-derivable sidecars
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

Scripts and new documentation should use `compass`. Versioned history is
specified, documented, and qualified through `compass`; its commands and help
do not need a matching Python Graphify surface.

Assistant setup is native and self-contained. The generic
`compass install --platform <name>` and project-scoped `--project` forms,
their uninstall counterparts, and their generated file trees are tested in the
Rust workspace.
## How Compass works

Compass parses a project into nodes and directed relationships. It then groups related nodes into communities and writes the results to `compass-out/`.

```text
 Source code        Project files       Optional semantic sources
 .rs .py .ts ...    Cargo, MCP, etc.    docs, PDFs, Office, images
      \                  |                         /
       +-----------------+------------------------+
                         |
                         v
               Parse and resolve locally
                         |
                         v
              +------------------------+
              |   Compass graph        |
              |                        |
              |  nodes --relations-->  |
              |     \__ communities    |
              +------------------------+
                         |
          +--------------+---------------+
          |              |               |
          v              v               v
       CLI queries   graph.html     assistants / MCP
```

Consider a checkout flow. Compass may represent it like this:

```text
 CheckoutHandler
      |
      +--CALLS [EXTRACTED]--> authorizePayment()
      |                              |
      |                              +--USES--> PaymentGateway
      |
      +--CALLS [INFERRED]----> reserveInventory()
```

The graph uses these concepts:

- **Node**: a file, function, class, document section, database object, or another project entity
- **Relationship**: a directed connection such as `CALLS`, `IMPORTS_FROM`, `USES`, or `CONTAINS`
- **Community**: a densely connected group that usually corresponds to a subsystem or feature
- **God node**: a highly connected node that may be important, generic, or too broad to help navigation
- **Provenance**: how Compass determined a relationship: `EXTRACTED` from direct evidence, `INFERRED` from resolution, or `AMBIGUOUS` when more than one interpretation remains

Compass preserves relationship direction, source locations, and provenance in the graph. You can inspect uncertain relationships instead of treating every connection as equally reliable.

## Install Compass on macOS

Install the latest release on Apple Silicon or Intel with one command:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/crabbuild/compass/releases/latest/download/install.sh | sh
```

The installer verifies the release archive's SHA-256 checksum and installs
`compass` to `~/.local/bin`. Set `COMPASS_INSTALL_DIR` to choose another
directory. This first macOS release is unsigned and isn't notarized by Apple.

## Install Compass from source

Building Compass requires Rust 1.97.1 or newer. The repository pins that toolchain in `rust-toolchain.toml`.

```bash
git clone https://github.com/crabbuild/compass.git
cd compass
cargo install --locked --path crates/compass-cli --bin compass
compass --version
```

The installed `compass` binary doesn't require Python. The development parity
suite uses Python only to compare selected behavior with Graphify.

Register the native Compass skill with your coding assistant:

```bash
compass install
```

Use a project-scoped skill when the configuration should travel with the
repository:

```bash
compass install --project --platform codex
```

After the crate is published to crates.io, install it with:

```bash
cargo install --locked compass-cli --bin compass
```

The release workflow publishes prebuilt Intel and Apple Silicon archives on the
[Compass releases page](https://github.com/crabbuild/compass/releases).

## Build your first graph

Run `update` from a project directory. This command performs deterministic structural extraction and doesn't call a model.

```bash
cd your_project_directory
compass update .
```

Compass writes the graph and its supporting artifacts to `compass-out/`:

| Artifact | Use it for |
| --- | --- |
| `graph.json` | Machine-readable nodes, relationships, attributes, and provenance |
| `GRAPH_REPORT.md` | Architecture summary, communities, god nodes, and graph diagnostics |
| `graph.html` | Interactive browser exploration when the graph is within the visualization limit |
| `manifest.json` | Incremental build state |

Open `compass-out/graph.html` in a browser for a visual tour. Start with `GRAPH_REPORT.md` when you need a repository-wide architecture view.

Ask a focused question when you need a smaller working set:

```bash
compass query "where is authentication enforced?"
```

This query searches and traverses the saved graph. It returns relevant nodes and relationships, not a model-generated narrative, and it doesn't access the network.

## Explore the graph with concrete questions

Compass includes focused commands for common code-reading tasks. Each command reads `compass-out/graph.json` by default.

Find the neighborhood related to a concept:

```bash
compass query "payment retry logic"
```

Inspect one symbol and its incoming and outgoing relationships:

```bash
compass explain PaymentGateway
```

Find the shortest known route between two symbols:

```bash
compass path CheckoutHandler PaymentGateway
```

Estimate what may depend on a changed symbol:

```bash
compass affected authorizePayment --depth 3
```

`affected` follows impact-related relationships such as calls, imports, and uses. Treat the result as a review scope, not proof that every returned file must change.

### Run exact structural queries with CompassQL

[CompassQL](docs/COMPASSQL.md) is Compass's deterministic, read-only subset of openCypher. Use it when you need an exact graph pattern, stable automation, parameters, or JSON output.

This query finds callers of `authorizePayment()`:

```bash
compass query --cql \
  "MATCH (caller)-[:CALLS]->(target) \
   WHERE target.label = 'authorizePayment()' \
   RETURN caller.id, target.id \
   LIMIT 20"
```

Use parameters when values come from a script:

```bash
compass query --cql \
  'MATCH (caller)-[:CALLS]->(target) \
   WHERE target.label = $target \
   RETURN caller.id' \
  --param target='authorizePayment()' \
  --format json
```

CompassQL never mutates the graph. See the [CompassQL support matrix](docs/COMPASSQL_SUPPORT.md) for accepted syntax, limits, diagnostics, and unsupported openCypher features.

## Choose structural or semantic extraction

Use `update` for source code and deterministic project structure. Use `extract` when you also need documents, papers, Portable Document Format (PDF) files, Office files, images, or external schemas in the graph.

```text
compass update .
    local structural graph
    no model call

compass extract . --code-only --cargo
    explicit no-model mode
    includes Cargo workspace dependency edges

compass extract docs --backend openai --model your_model_name
    adds semantic facts from supported documents
    sends selected content to the configured provider
```

`--code-only` guarantees that Compass won't invoke a model. Without that flag, semantic extraction uses the selected built-in or trusted custom provider. Configure provider credentials through the provider's environment variables, then run `compass extract --help` for controls such as token budget, concurrency, timeouts, and partial results.

Native integrations can add other graph layers:

```bash
# Add workspace-internal Cargo dependencies.
compass extract . --code-only --cargo

# Read a PostgreSQL schema through a read-only connection.
compass extract . --code-only --postgres 'postgresql://localhost/app_database'

# Export Google Drive shortcuts through the configured gws CLI.
compass extract . --code-only --google-workspace
```

Compass confines image loading to the selected root. PDF, DOCX, and XLSX ingestion also uses bounded native readers.

## Keep the graph current

Compass records file state in `manifest.json`, so later updates can reuse unchanged extraction results.

Run an update after a batch of changes:

```bash
compass update .
```

Use the watcher during active development:

```bash
compass watch .
```

The watcher rebuilds deterministic changes after its debounce interval. When semantic media changes, it writes `compass-out/needs_update` instead of calling a model in the background. Run `compass extract` again when you're ready to refresh that content.

## Connect a coding assistant

Compass can install project-scoped instructions, skills, and hooks for supported coding assistants. For example, install the Codex integration from the project root:

```bash
compass install --platform codex --project
```

The integration tells the assistant when to read the architecture report and when to run a focused graph query. It doesn't upload the graph.

Every platform receives the same canonical `compass` skill and progressive
reference bundle. The core handles graph-first navigation and evidence rules;
on-demand references cover CompassQL, semantic extraction, immutable history,
labeling, hooks, exports, MCP serving, multi-repository workflows, reflections,
diagnostics, and security boundaries. The bundle currently contains 15
progressive-disclosure references. A native build-time guard checks exact
reference coverage and reads the public CLI inventory to ensure every command
has dedicated help and installed skill guidance.

List every supported platform and installation option:

```bash
compass install --help
```

Remove one project integration without deleting the graph:

```bash
compass uninstall --platform codex --project
```

## Serve the graph over MCP

Compass includes a Model Context Protocol (MCP) server for editors and agents. Standard input and output is the default transport:

```bash
compass serve compass-out/graph.json
```

For a network client, start Streamable HTTP and require an API key:

```bash
export GRAPHIFY_API_KEY='your_access_token_here'
compass serve --transport http --api-key "$GRAPHIFY_API_KEY"
```

HTTP mode supports stateful or stateless sessions, bounded request bodies, Domain Name System (DNS) rebinding checks, session expiry, and graceful shutdown. Run `compass serve --help` to configure the host, port, path, and session behavior.

## Export or share the graph

Exports transform the existing graph without rebuilding the project:

```bash
compass export wiki
compass export obsidian
compass export svg
compass export graphml
compass export neo4j
```

Neo4j and FalkorDB exports produce local openCypher by default. Add `--push URI` for bounded live upserts. Compass connects to Neo4j with Bolt and to FalkorDB with Redis Serialization Protocol (RESP), so neither path needs a database software development kit.

Store database passwords in `NEO4J_PASSWORD` or `FALKORDB_PASSWORD`. Compass redacts those values from failures.

## Know when Compass can access the network

The default build and query workflow stays local. Network access occurs only when you select a network-backed feature:

- `compass update`, local queries, CompassQL, traversal, reports, and local exports don't access the network
- `compass extract --code-only` explicitly disables model calls
- Semantic extraction may send selected content to your configured model provider
- `compass export neo4j --push` and `compass export falkordb --push` connect to the target database
- `compass serve --transport http` listens on the host and port you configure
- Google Workspace extraction uses your configured `gws` command

All Tree-sitter grammars are linked into the binary. Compass doesn't download parsers while scanning a repository.

## Find the command you need

Compass exposes completed native commands under one interface:

| Task | Commands |
| --- | --- |
| Build and enrich | `update`, `extract`, `watch`, `cluster-only`, `label` |
| Explore and assess impact | `query`, `path`, `explain`, `affected`, `tree`, `benchmark` |
| Inspect versioned graphs | `history`, `diff`, and query commands with `--at` |
| Export and serve | `export`, `serve` |
| Manage graph data | `diagnose multigraph`, `merge-graphs`, `merge-chunks`, `merge-semantic`, `cache-check` |
| Work across projects | `global`, `clone`, `add`, `prs`, `hook`, `merge-driver` |
| Configure integrations | `install`, `uninstall`, `provider`, `check-update`, `hook-check`, `hook-guard` |
| Save project knowledge | `save-result`, `reflect` |

Run `compass --help` for the current surface or `compass <command> --help` for command syntax.

## Migrate from Graphify

Compass uses `compass-out/` and doesn't read `graphify-out/` or `GRAPHIFY_OUT`.
Rebuild a project with `compass update .` after switching from Graphify. Set
`COMPASS_OUT` when you need a custom output directory.

Use `compass` for new workflows. The release doesn't include compatibility
executables for the Python Graphify command or its MCP entry point.

```text
graphify <command>       -> compass <command>
python -m graphify ...  -> compass ...
```

See [MIGRATION.md](MIGRATION.md) for side-by-side qualification, cutover, and rollback. See [COMPATIBILITY.md](COMPATIBILITY.md) for the frozen Python baseline and differential evidence.

## Join the Compass community

Use Compass's public community channels to ask questions, report problems, propose improvements, and contribute changes.

| Need | Destination |
| --- | --- |
| Usage question or open-ended idea | [GitHub Discussions](https://github.com/crabbuild/compass/discussions) |
| Reproducible bug or actionable feature request | [GitHub Issue chooser](https://github.com/crabbuild/compass/issues/new/choose) |
| Security vulnerability | [GitHub private vulnerability reporting](https://github.com/crabbuild/compass/security/advisories/new) |
| Code or documentation contribution | [GitHub pull requests](https://github.com/crabbuild/compass/pulls) |

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. All project interactions follow the [Compass code of conduct](CODE_OF_CONDUCT.md). [SUPPORT.md](SUPPORT.md) explains support boundaries, and [SECURITY.md](SECURITY.md) explains private vulnerability reporting.

## Build and verify the workspace

These commands run the checks used for local development:

```bash
cargo build --release --locked -p compass-cli --bin compass
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --locked -- -D warnings
cargo test --workspace --lib --bins --locked
cargo test -p compass-cli --test compass_product --locked
sh scripts/test_release_scripts.sh
```

The compatibility suite uses a sibling Graphify checkout as its behavioral oracle. Set `GRAPHIFY_REPO_ROOT` when that checkout lives elsewhere. Set `GRAPHIFY_PYTHON` and `GRAPHIFY_MEDIA_PYTHON` when the test interpreters use non-default paths.

Release qualification also covers native line and region coverage, focused mutation suites, Miri, AddressSanitizer, and fuzz targets for untrusted inputs. Read [PERFORMANCE.md](PERFORMANCE.md) for benchmark methodology and the current baseline.

## Distribution guarantees

Release archives contain `compass`, shell completions, license notices, and a
SHA-256 checksum. The automated workflow also records build-provenance
attestations for each archive.

The release workflow currently builds macOS archives for Intel and Apple
Silicon. Publishing to crates.io uses a separate environment-protected workflow
that validates the release tag before publishing workspace crates in dependency
order.

## License

Compass's original work is available under your choice of the [MIT License](LICENSE-MIT) or [Apache License 2.0](LICENSE-APACHE). The workspace uses the SPDX expression `MIT OR Apache-2.0`.

Third-party components retain their original licenses. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and the license files stored with vendored components.

Unless you explicitly state otherwise, contributions submitted for inclusion in Compass use the same dual license without additional terms or conditions.
