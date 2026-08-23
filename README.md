# Compass

<div align="center">
  <p><strong>Understand the shape of a codebase before you change it.</strong></p>
  <p>Compass is a local-first Rust workbench for architecture maps, dependency paths, impact analysis, and Git history.</p>
  <p>
    <a href="docs/getting-started.md">Get started</a> ·
    <a href="docs/README.md">Documentation</a> ·
    <a href="https://marketplace.visualstudio.com/items?itemName=crabbuild.crabbuild-compass-vscode">VS Code extension</a> ·
    <a href="https://github.com/crabbuild/compass/releases">Releases</a>
  </p>
</div>

<p align="center">
  <a href="https://github.com/crabbuild/compass/actions/workflows/compass-ci.yml"><img src="https://github.com/crabbuild/compass/actions/workflows/compass-ci.yml/badge.svg" alt="CI status"></a>
  <a href="https://github.com/crabbuild/compass/releases"><img src="https://img.shields.io/github/v/release/crabbuild/compass?display_name=tag&sort=semver" alt="Latest release"></a>
  <a href="https://github.com/crabbuild/compass/blob/main/LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-4c8bf5" alt="MIT or Apache-2.0 license"></a>
</p>

Compass turns source code and project artifacts into a directed, queryable knowledge graph. It gives people and coding assistants a shared map of symbols, files, communities, relationships, and evidence.

```text
repository → graph snapshot → architecture · paths · impact · history · focused context
```

## See the graph before you open every file

Compass exports a self-contained workbench. Move from a repository map to subsystem flow, call evidence, or bounded dependency neighborhoods without changing snapshots.

<table>
  <tr>
    <td width="50%">
      <img src="docs/assets/screenshots/readme-ripgrep-architecture.png" alt="Compass architecture map of the ripgrep repository with subsystem call direction and source details" width="100%">
      <br><sub><strong>Architecture map · ripgrep</strong><br>Subsystem boundaries, call direction, and source-backed details.</sub>
    </td>
    <td width="50%">
      <img src="docs/assets/screenshots/readme-cli11-code-graph.png" alt="Compass code graph of the CLI11 repository with community filters and minimap" width="100%">
      <br><sub><strong>Repository map · CLI11</strong><br>Large-scale structure with communities, filters, and a minimap.</sub>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <img src="docs/assets/screenshots/readme-ripgrep-call-graph.png" alt="Compass bounded call graph centered on ripgrep SearchWorker search" width="100%">
      <br><sub><strong>Call graph · ripgrep</strong><br>Follow callers, callees, direction, depth, and evidence coverage.</sub>
    </td>
    <td width="50%">
      <img src="docs/assets/screenshots/readme-ripgrep-dependencies.png" alt="Compass dependency lens for the ripgrep repository with community filters" width="100%">
      <br><sub><strong>Dependency lens · ripgrep</strong><br>Inspect bounded relationships while keeping the surrounding context visible.</sub>
    </td>
  </tr>
</table>

These are real Compass exports from public repositories, not product mockups: [BurntSushi/ripgrep](https://github.com/BurntSushi/ripgrep/tree/435f59fc4b43af3ab32f34d53fa34978f393fe52) and [CLIUtils/CLI11](https://github.com/CLIUtils/CLI11/tree/60492bddb50422f32cfa33c1365b96ebee4205ca). Counts and visible symbols reflect those exact revisions and will change as the projects evolve.

## Find the answer at the right scale

| Question | Compass surface |
| --- | --- |
| Where does this subsystem live? | Communities, architecture flow, ownership, and source details |
| What calls this function? | Bounded caller and callee graphs with direction and depth controls |
| What could this change affect? | Reverse dependency paths, affected nodes, and evidence limits |
| Which files belong to this workflow? | Search, explanations, paths, and relationship filters |
| What changed between revisions? | Immutable historical graphs and semantic diffs over exact Git commits |
| How do I share the result? | VS Code, self-contained HTML, JSON, GraphML, SVG, Wiki, Obsidian, Neo4j, and FalkorDB exports |

## Start with one repository

### Install the CLI

On macOS or Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/crabbuild/compass/releases/latest/download/install.sh | sh
```

On Windows:

```powershell
irm https://github.com/crabbuild/compass/releases/latest/download/install.ps1 | iex
```

For an offline install, download the matching archive and `.sha256` file from the [latest release](https://github.com/crabbuild/compass/releases/latest), verify the checksum, extract the archive, and add the directory to `PATH`.

To build from source, use the pinned Rust 1.97.1 or newer toolchain:

```bash
git clone https://github.com/crabbuild/compass.git
cd compass
cargo install --locked --path crates/compass-cli --bin compass
```

Upgrade an installed Compass executable with `compass upgrade`.

### Build the first graph

```bash
cd your-project
compass init
```

`compass init` previews the eligible corpus, saves the project scope in `.compass/config.toml`, and performs the first structural build. Compass writes a coherent snapshot under `compass-out/`.

For a focused monorepo scope, pass include and exclude patterns during initialization:

```bash
compass init . --include src --include 'services/*/src' --exclude '**/generated/**' --yes
```

```text
compass-out/
├── graph.json        machine-readable graph
├── GRAPH_REPORT.md   architecture and community summary
├── orientation.json  bounded orientation for assistants
├── graph.html        interactive visualization when size permits
├── manifest.json     incremental build state
├── snapshots/        retained coherent build snapshots
└── store/            SQLite query index when enabled
```

Keep the graph current while you work:

```bash
compass watch
```

Run `compass watch` in a second terminal. It reuses the saved scope and refreshes the graph as files change.

### Ask a focused question

```bash
compass query "where is authentication enforced?"
compass explain TokenVerifier
compass path ApiHandler TokenVerifier
compass affected TokenVerifier --depth 3
```

These commands read the saved graph locally. Results are bounded, read-only, and tied to graph evidence instead of a model-generated guess.

### Open the workbench

```bash
compass export html
```

Build several perspectives into one self-contained page:

```bash
compass export html \
  --code-graph \
  --architecture-graph \
  --call-graph "<SYMBOL>" \
  --impact-graph "<SYMBOL>"
```

The same workbench is available through the [Compass Codegraph extension for VS Code](https://marketplace.visualstudio.com/items?itemName=crabbuild.crabbuild-compass-vscode). Search for a symbol or file, select a node, inspect its evidence and relationships, then open the exact source location.

## Keep it local and traceable

Compass is built for teams that need useful structure without handing a repository to a remote service.

| Principle | What it means in practice |
| --- | --- |
| Native by default | Structural extraction and graph queries run in one Rust executable. They do not require Python, embeddings, a vector database, model credentials, or runtime parser downloads. |
| Evidence first | Relationships retain direction, source anchors, and provenance. Ambiguous matches remain visible as uncertainty. |
| Bounded by design | Query rows, paths, expansions, memory, and time stay within explicit limits. A limit is reported as a limit, not an empty result. |
| History without rewriting | Historical realizations use exact Git commits and immutable fingerprints. Current and past graphs can be queried, exported, and compared. |

Optional semantic workflows use a configured provider only when you select them. Read [Security and privacy](docs/design/security-and-privacy.md) before processing sensitive repositories.

## Connect your existing tools

### VS Code

Install [Compass Codegraph](https://marketplace.visualstudio.com/items?itemName=crabbuild.crabbuild-compass-vscode), open a repository, and use the Compass sidebar to initialize or open the graph. Cursor-rooted call graphs, architecture flow, queries, and Git evolution use the same snapshot model.

### Coding assistants

Install the portable Agent Skills integration or a host-specific adapter from the repository root:

```bash
compass install
compass install --platform codex
compass install --platform codex --platform claude
compass install --all --dry-run
```

Compass supports Codex, Gemini CLI, OpenCode, Copilot, Claude Code, Kiro, Cline, and generic Agent Skills clients. The integration starts with a bounded orientation or focused query, checks completeness and ambiguity, and opens only the cited source needed for verification.

### Graph exports and MCP

Use `compass export` for machine-readable graph JSON, GraphML, SVG, Wiki, Obsidian, Neo4j, and FalkorDB workflows. Use [`compass serve`](docs/guides/integrating-compass.md) when you want graph resources and tools inside a compatible MCP client.

### Program evidence

Structural graphs are the default. Add the optional language-neutral Program IR when a workflow needs functions, conservative basic blocks, operations, call candidates, capability coverage, or derived summaries:

```bash
compass update . --program
compass program summary
compass program coverage
compass program functions --language rust --name build
```

Compass reads offline SCIP indexes when they already exist. It does not invoke an indexer, compiler, language server, model, or network service to build this artifact.

## Compare history, not memory

```bash
compass history enable --code-only
compass history build main
compass history build HEAD --profile-from main
compass query "authentication" --at HEAD~20
compass diff main HEAD
```

Historical builds use exact Git commits and immutable extraction fingerprints. `compass diff` surfaces likely breaks, behavior changes, affected callers and modules, test evidence, and known limitations in one offline report.

Read the [versioned history guide](docs/guides/versioned-history.md) and [storage design](docs/design/storage-and-history.md) for retention, profiles, and compatibility details.

## Performance

Compass maintains qualified performance baselines while preserving deterministic graph output and correctness. See [Performance qualification](PERFORMANCE.md) for the evidence policy and benchmark commands.

## How it works

![Compass graph construction and query pipeline](docs/assets/diagrams/graph-pipeline.svg)

Compass discovers project files, extracts entities and relationships, resolves cross-file references, analyzes communities, and publishes one consistent snapshot.

```text
function / class / file / document / schema object  → node
CALLS / IMPORTS_FROM / USES / CONTAINS              → relationship
dense group of related nodes                         → community
direct / resolved / uncertain evidence               → provenance
```

The graph keeps relationship direction, source location, and provenance together:

```text
CheckoutHandler
    |
    +-- CALLS [EXTRACTED] --> authorizePayment()
    |                              |
    |                              +-- USES --> PaymentGateway
    |
    +-- CALLS [INFERRED]  --> reserveInventory()
```

Read [How Compass works](docs/concepts/how-it-works.md) for the full pipeline and [Graph model](docs/concepts/graph-model.md) for the data contract.

## Query exact graph patterns with CompassQL

CompassQL is a deterministic, read-only, parameterized, and resource-bounded openCypher subset:

```bash
compass query --cql \
  "MATCH (caller)-[:CALLS]->(target)
   WHERE target.label = 'authorizePayment()'
   RETURN caller.id, target.id
   LIMIT 20" \
  --format json
```

Read the [language contract](docs/COMPASSQL.md) and [support matrix](docs/COMPASSQL_SUPPORT.md) for syntax, limits, and machine-readable output.

## Choose the smallest mode that answers the question

| Command | Network behavior | Best use |
| --- | --- | --- |
| `compass update .` | Local only | Normal source-code graph updates |
| `compass extract . --code-only` | Local only | Explicit no-model extraction |
| `compass extract . --code-only --cargo` | Local only | Code plus Cargo dependency edges |
| `compass extract docs --backend …` | Configured provider | Semantic facts from supported documents and media |

Compass gives explicit control over optional provider boundaries. Structural graph building remains local-first and independent of those providers.

## Project lineage and compatibility

Compass is inspired by and modeled after [Graphify](https://github.com/Graphify-Labs/graphify), which established the workflow of extracting a codebase into a knowledge graph, analyzing communities, and querying focused context.

Compass is now an independent product with its own Rust implementation, commands, configuration, artifacts, and test suite. It does not execute or depend on Graphify.

Read [Compatibility](COMPATIBILITY.md) for Compass-owned contracts and [Migration from Graphify](MIGRATION.md) for the one-time hard cutover.

## Find your next page

| You are… | Start here |
| --- | --- |
| Evaluating Compass | [Getting started](docs/getting-started.md) → [How it works](docs/concepts/how-it-works.md) |
| Using or integrating Compass | [Documentation hub](docs/README.md) → [Cookbook](docs/cookbook/README.md) |
| Extending the Rust workspace | [Architecture](docs/design/architecture.md) → [Workspace tour](docs/implementation/workspace-tour.md) |
| Looking up an interface | [Commands](docs/reference/commands.md) → [Outputs](docs/reference/outputs.md) |
| Tracking future direction | [Roadmap](docs/roadmap.md) |

## Community and contributing

| Need | Destination |
| --- | --- |
| Usage question or idea | [GitHub Discussions](https://github.com/crabbuild/compass/discussions) |
| Bug or actionable feature request | [GitHub Issues](https://github.com/crabbuild/compass/issues/new/choose) |
| Security vulnerability | [Private vulnerability reporting](https://github.com/crabbuild/compass/security/advisories/new) |
| Code or documentation contribution | [Contributing guide](CONTRIBUTING.md) |

Development checks and architecture boundaries live in [CONTRIBUTING.md](CONTRIBUTING.md). Support and disclosure boundaries live in [SUPPORT.md](SUPPORT.md) and [SECURITY.md](SECURITY.md).

## License

Compass's original work is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE). Third-party components retain their original licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
