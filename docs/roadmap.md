# Compass roadmap

Compass is inspired by Graphify, implemented natively in Rust, and expected to
evolve independently. This roadmap separates shipped/current behavior from
committed design work and longer-term ideas.

> **Who this page is for:** users evaluating project direction, integrators
> planning adoption, and contributors choosing work.
>
> **You will learn:** what exists in the current repository, what has a
> committed design or implementation plan, and which ideas are explicitly
> aspirational.
>
> **Prerequisites:** none.
>
> **Reading time:** 8–10 minutes.

## How to read status

- **Available now** means the behavior exists in the current repository and is
  backed by source, help, tests, or release evidence. A feature can be available
  in the repository before every target has a published release artifact.
- **Planned** means a committed design or implementation plan describes the
  work. A plan is not proof that implementation is complete.
- **Aspirational** means the idea is a possible direction, not a commitment.
  There is no promised release, compatibility guarantee, or delivery date.

The authoritative release surface remains the installed binary's help,
[compatibility ledger](../COMPATIBILITY.md), and release notes.

## Available now

### Native structural knowledge graphs

Compass discovers, parses, resolves, builds, clusters, analyzes, and publishes
project graphs natively:

```bash
compass update .
compass query "authentication flow"
compass explain TokenVerifier
compass path ApiHandler TokenVerifier
compass affected TokenVerifier --depth 3
```

The normal code path does not require Python, embeddings, a vector database,
runtime parser downloads, or model credentials.

Evidence:

- public command registry in `compass-cli`;
- deterministic pipeline crates and tests;
- [compatibility ledger](../COMPATIBILITY.md);
- [performance qualification](../PERFORMANCE.md).

### CompassQL 1

Compass includes a native, deterministic, read-only subset of openCypher with:

- parsing, semantic analysis, planning, optimization, and bounded execution;
- exact patterns, joins, optional matching, expressions, aggregation, bounded
  paths, shortest paths, and unions;
- parameters;
- table, versioned JSON, and JSONL;
- explain/profile;
- time, row, path, expansion, and memory limits;
- checked TCK/support evidence.

Evidence:

- [CompassQL 1](COMPASSQL.md);
- [CompassQL support matrix](COMPASSQL_SUPPORT.md);
- `compass-cypher` and `compass-query`;
- CLI/TCK/differential tests.

### Immutable versioned graph history

Compass can build, query, compare, export, prefer, and garbage-collect complete
graph realizations for exact Git commits:

```bash
compass history enable --code-only
compass history build HEAD
compass query "authentication" --at HEAD~20
compass diff HEAD~1 HEAD --topology-only
```

The SQLite-backed Prolly store supports immutable realizations, extraction
fingerprints, structural sharing, protected offline worktrees, durable jobs and
leases, validation, and canonical reconstruction.

Evidence:

- `compass-history`;
- history CLI and qualification scripts;
- [Versioned history guide](guides/versioned-history.md);
- [storage design](design/storage-and-history.md).

### Native assistant integration

The current repository includes a native `compass install`/`uninstall`
implementation with global/project scope, platform-specific destinations,
embedded skill/reference assets, idempotence checks, and managed strict mode
where supported.

```bash
compass install --project --platform codex
```

Evidence:

- `compass-cli` installer source and generated assets;
- installer filesystem-tree tests;
- [Assistant setup](guides/assistant-setup.md).

The exact release containing the latest installer asset bundle should be
confirmed on the releases page.

### Optional semantic and integration surfaces

The current workspace includes native support for:

- provider-backed semantic extraction and community labeling;
- PDF/Office/media processing and native Whisper internals;
- Cargo, PostgreSQL, Google Workspace, and SCIP knowledge;
- MCP stdio/HTTP serving;
- Neo4j and FalkorDB export;
- bounded remote ingestion;
- GitHub PR dashboard/impact workflows;
- cross-project graph registry;
- reflection/session-memory overlays;
- multiple human and machine export formats.

Each surface has its own network, credential, platform, and completeness
boundary. “Available” does not mean enabled by default.

### Community, licensing, security, and support

Compass is dual-licensed under MIT or Apache-2.0 and includes:

- contribution guidance;
- Code of Conduct;
- security policy;
- support routes;
- third-party notices;
- release and distribution workflows.

## Planned

The items below have committed design or implementation-plan evidence in the
default branch. They are not listed in the current public command surface
unless explicitly moved to Available now. Ideas that exist only in an
uncommitted workspace or a separate development branch are intentionally not
promoted to committed plans here.

### Reusable CompassQL policy and integration surfaces

Goal: reuse one graph loading, compilation, execution, and output implementation
through CLI/MCP/historical/policy contexts, rather than building parallel
query engines.

Planned areas include:

- reusable architecture-policy evaluator;
- exact historical policy queries;
- “new violation” comparisons;
- MCP integration for structured CompassQL;
- stable policy output and limits.

Evidence:

- [`docs/superpowers/plans/2026-07-22-compassql-integrations.md`](superpowers/plans/2026-07-22-compassql-integrations.md)
- [`docs/superpowers/plans/2026-07-22-compassql-policy.md`](superpowers/plans/2026-07-22-compassql-policy.md)

## Aspirational

These directions have no promised release or compatibility commitment.

### Time-travel architecture explorer

Provide a visual and queryable view that moves across historical realizations:

```text
commit A ---- commit B ---- commit C
   |             |             |
 graph          graph         graph
   \_____ typed topology and policy changes ____/
```

Potential value:

- explain when a dependency appeared;
- watch community boundaries evolve;
- connect policy violations to the introducing commit;
- inspect semantic versus topology change.

Open questions include UI scope, large-history performance, and how to preserve
exact evidence without oversimplifying diffs.

### Multi-repository federation

Build a first-class model for services, shared libraries, schemas, and
deployment dependencies spanning many repositories.

Possible capabilities:

- repository-qualified stable identities;
- cross-repository edges with provenance;
- federated CompassQL;
- version-aligned release graphs;
- organization-wide impact analysis.

The current global graph registry is a useful primitive but not a complete
federation contract.

### Runtime-evidence overlays

Combine static graph structure with optional runtime evidence such as traces,
coverage, or profiles while keeping the sources distinct:

```text
static CALLS edge        structural possibility
runtime observation      observed execution in one environment/run
```

The key design challenge is avoiding a false claim that one runtime sample
proves universal behavior.

### Stable extractor/integration SDK

Offer a documented way to extend languages or external systems without
requiring every integration to live in the core workspace.

Requirements before such an SDK could be stable:

- versioned graph fragment schema;
- explicit provenance and identity rules;
- sandbox/process boundary;
- size/time limits;
- compatibility negotiation;
- deterministic test kit;
- supply-chain policy.

### Rich editor-native views

Provide IDE surfaces for:

- incoming/outgoing relationships at the cursor;
- exact impact previews;
- community and path navigation;
- historical diff annotations;
- policy witnesses;
- provenance/source verification.

Any editor integration should call the same native query contracts rather than
reimplement graph semantics in each extension.

### Broader signed release coverage

Expand reproducible, signed, and platform-native distribution beyond the
current release packaging while retaining:

- checksum/provenance verification;
- dependency-free native behavior;
- exact test matrix;
- update/uninstall safety;
- transparent support status.

### Collaborative graph review artifacts

Create reviewable, immutable bundles that combine:

- graph/policy result;
- exact revision and fingerprint;
- witness paths;
- selected source excerpts/locations;
- human decisions and exemptions;
- machine-verifiable manifest.

This could support architecture review without requiring a hosted proprietary
graph store.

## How an item changes status

```text
Aspirational
  -> approved design/spec
  -> Planned
  -> implementation + contract tests + docs + qualification
  -> Available now
  -> published release availability stated separately
```

A checkbox in a plan is not enough. Moving to Available requires current-state
evidence across the actual public surface.

## Contributing to the roadmap

For a proposed direction, provide:

- user problem;
- current limitation/evidence;
- intended product boundary;
- local/network/credential impact;
- graph identity and provenance effect;
- resource limits and failure behavior;
- compatibility/divergence classification;
- smallest independently useful delivery.

Then follow [Contributing](../CONTRIBUTING.md).

## Related pages

- [Documentation hub](README.md)
- [Compatibility](reference/compatibility.md)
- [Design principles](design/principles.md)
- [Contributing](../CONTRIBUTING.md)

**Next step:** verify an Available item against the installed binary or open one
Planned design and identify the smallest reviewable contribution.
