# Compass Technical-Moat Roadmap

**Status:** Approved roadmap design

**Date:** 2026-07-23

**Horizon:** 24 months

**Primary strategy:** Evidence ladder

## Purpose

Compass will deepen its technical moat by becoming an evidence engine for
program behavior and software change. The roadmap builds progressively stronger
evidence:

1. Interprocedural program understanding
2. Runtime evidence overlays
3. Semantic change and impact intelligence
4. Cross-repository and cross-system graphs
5. Graph-grounded agent planning and verification

Each layer consumes the evidence produced by the layers below it. Compass will
first qualify the approach on large monorepositories, then extend the same
evidence and query model to multi-repository organizations.

The roadmap builds on Compass's existing local graph extraction, provenance,
immutable graph history, semantic diff, CompassQL, PR-intelligence design, MCP,
and assistant integration. It does not replace those foundations with a hosted
graph database or make an agent the source of truth.

## Strategic decision

Three roadmap shapes were considered:

1. An evidence ladder that deepens program analysis before federation and agent
   workflows.
2. A universal engineering graph that prioritizes source breadth.
3. An agent-first product that adds analysis only when an agent workflow needs
   it.

Compass will use the evidence ladder. This approach compounds the product's
existing versioning, provenance, query, and change-analysis advantages. It also
produces standalone value at each milestone. The accepted trade-off is that
deep analysis will initially support fewer languages than Compass's structural
extractor.

## Product principles

- Prefer precise, reproducible evidence over broad unsupported claims.
- Preserve static facts, observations, deterministic conclusions, hypotheses,
  and agent assertions as different record classes.
- Attach witness paths, provenance, revisions, and analysis versions to every
  derived conclusion.
- Represent ambiguity and incomplete coverage explicitly.
- Keep static analysis and primary graph operations local-first.
- Treat Tree-sitter as the deterministic syntax baseline, not as a substitute
  for compiler semantics.
- Merge complementary syntax, index, compiler, and binary evidence through one
  provider-neutral Program IR.
- Ingest offline runtime artifacts before adding read-only connectors.
- Optimize deep analysis for a large monorepository before adding distributed
  graph federation.
- Reuse immutable summaries and Prolly subtrees across graph realizations.
- Expose stable evidence through CompassQL and typed APIs.
- Keep coding agents replaceable consumers of Compass evidence.

## Scope

The roadmap includes:

- Higher-precision type, call, control-flow, and data-flow analysis
- Behavior and effect summaries
- Contract extraction and compatibility analysis
- Offline runtime, test, coverage, and profile evidence
- Behavioral diff, impact analysis, and test selection
- Cross-repository contract and ownership graphs
- Cross-system links to APIs, schemas, infrastructure, documentation, and
  telemetry
- Agent-facing task scope, context, planning, and verification APIs
- Incremental and federated analysis suitable for large engineering systems

The roadmap does not include:

- Direct telemetry collection by Compass
- A hosted graph database as the primary architecture
- Opaque learned risk scores
- Treating model-generated relationships as deterministic program facts
- Broad connector accumulation before analysis depth is proven
- Full language-depth parity before the initial deep-analysis languages qualify
- A Compass-owned autonomous code-editing agent

## Layer 1: interprocedural program understanding

### Goal

Move Compass from a graph of extracted entities and relationships to an
executable model of how behavior and information propagate through a program.

### Capabilities

- A typed, provider-neutral Program IR for functions, methods, values, types,
  call sites, branches, reads, writes, exceptions, asynchronous boundaries,
  and effects
- Higher-precision call resolution for interfaces, traits, virtual dispatch,
  callbacks, closures, dependency injection, reflection hints, and framework
  routing
- Control-flow graphs covering branches, loops, early exits, exceptions,
  asynchronous suspension, and concurrency boundaries
- Interprocedural definition-use and data-flow analysis across parameters,
  returns, fields, collections, and common serialization boundaries
- Immutable function summaries covering inputs, outputs, mutations, side
  effects, calls, errors, resources, and trust-boundary behavior
- Contract extraction for APIs, database constraints, configuration,
  invariants, preconditions, postconditions, and errors
- Security primitives for sources, sinks, sanitizers, permission checks,
  secrets, injection paths, and validation
- Framework intelligence packs for routers, object-relational mappers, queues,
  dependency injection, test frameworks, and build systems
- Reproducible witness paths for every high-level conclusion

### Language tiers

Compass will distinguish three support tiers:

- **Deep:** call, control-flow, data-flow, effect, contract, and framework
  analysis
- **Resolved:** type and higher-precision call resolution without complete data
  flow
- **Structural:** the existing extracted graph and relationships

Rust and TypeScript/JavaScript are the first deep-tier languages. Rust
dogfoods the analysis on Compass itself. TypeScript/JavaScript exercises large
application monorepositories and dynamic framework behavior. Python,
Java/Kotlin, and Go follow through explicit evidence providers. A language may
reach the resolved tier from an offline semantic index before Compass ships a
native deep analyzer for it.

### Program-evidence provider model

Program IR is a normalized evidence model, not the output of one universal
parser. Compass composes four provider classes:

1. **Syntax providers** analyze individual source files without a build. The
   first implementation reuses Compass's Tree-sitter parsers for stable spans,
   declarations, lexical operations, and conservative source-level control
   flow. Syntax providers maximize breadth and always remain a fallback.
2. **Artifact providers** ingest repository-scoped files generated outside
   Compass. Official SCIP indexes are the first semantic artifact. Compiler
   indexes, language-specific analysis exports, LLVM bitcode, JVM class files,
   and comparable offline artifacts may follow.
3. **Project analyzers** use a language's compiler or project model when deeper
   precision justifies the toolchain cost. Candidate integrations include the
   TypeScript compiler API, Go SSA, Roslyn, Clang, and a stable Rust compiler or
   rust-analyzer interface.
4. **Read-only live providers** query language servers or external systems only
   after the offline contracts are mature. Their observations are overlays and
   never become an undeclared prerequisite for a reproducible build.

The initial foundation implements Tree-sitter syntax evidence for Rust and the
TypeScript family plus optional official SCIP ingestion. It does not invoke an
indexer, compiler, language server, network service, or model. Projects supply
offline artifacts explicitly or place a supported conventional artifact in the
repository. Because raw SCIP does not carry source-content digests, its
freshness is declared unverified unless an optional Compass companion manifest
binds the index digest and document paths to exact source digests.

Providers emit normalized evidence batches rather than final IR. Every batch
declares provider identity, kind, version, scope, input digest, configuration
digest, and capability coverage. Evidence is merged deterministically using
stable source anchors and semantic symbol identities:

- syntax evidence owns source structure, lexical operations, and source spans;
- semantic indexes and compiler providers may resolve identities, references,
  types, roles, implementations, and call targets;
- deeper providers may add control-flow, data-flow, effect, and contract facts;
- no provider silently overwrites contradictory evidence;
- conflicts remain explicit evidence and reduce only the affected capability's
  coverage.

Authority is capability-specific, not a global provider ranking. For example,
SCIP may be authoritative for a call target while Tree-sitter remains
authoritative for the call's exact source span. Facts retain all supporting
evidence IDs so users and downstream analyses can explain how a conclusion was
formed.

### Incremental model

Editing a symbol invalidates its behavior summary and only the dependent
summaries affected by changed facts. Versioned summaries live alongside graph
realizations so historical analysis and semantic diff can reuse unchanged
results.

Provider caches have different scopes. Syntax evidence is keyed by logical
path, source digest, schema, and syntax-provider version. Artifact evidence is
keyed by artifact digest and decoder version and is normalized into document
shards. Project evidence is keyed by repository snapshot, build-context digest,
and analyzer version. The final merge fingerprint includes the ordered provider
manifest, so changing an offline index can refresh resolutions without forcing
source parsing.

### Qualification

- Measure call-target precision and recall against curated and real-repository
  fixtures.
- Keep incremental analysis proportional to the affected dependency cone rather
  than repository size.
- Mechanically validate every reported witness path.
- Represent unsupported or ambiguous behavior explicitly.
- Make control flow, data flow, effects, contracts, and confidence queryable
  through CompassQL.

## Layer 2: runtime evidence overlays

### Goal

Connect statically possible behavior with observed execution without
overwriting or weakening the static model.

### Capabilities

- Ingest OpenTelemetry/OTLP exports and map spans to services, endpoints,
  queues, database operations, and code symbols.
- Ingest LCOV, JaCoCo, LLVM, Cobertura, and test-framework coverage.
- Ingest folded stacks, `pprof`, and supported profiler exports.
- Record test pass/fail evidence, duration, flakiness, environment class, and
  exact commit or build identity.
- Annotate observed topology, hot paths, allocation pressure, latency, errors,
  retries, and timeouts.
- Detect dead or unobserved critical paths, unexpected runtime dependencies,
  uncovered contracts, and runtime calls missing from static resolution.

Each observation is an immutable overlay identified by repository, commit,
build, environment class, time window, artifact digest, schema version, and
ingestion profile.

Compass preserves these states:

- `POSSIBLE`: established by static analysis
- `OBSERVED`: present in runtime or test evidence
- `CONTRADICTED`: inconsistent with a modeled contract or topology

Absence of observation never proves that a static path is impossible. Runtime
evidence may prioritize findings or strengthen confidence, but it does not
silently remove static paths.

### Delivery order

1. Local files and CI-produced artifact bundles
2. A stable evidence manifest and ingestion plugin interface
3. Read-only OpenTelemetry, CI, profiler, and API-catalog connectors
4. Incremental refresh using provider cursors

Compass will not collect telemetry directly.

### Security

- Apply schema-aware redaction.
- Reject or bound oversized artifacts.
- Exclude raw request bodies by default.
- Preserve provenance for every observation.
- Provide local retention and deletion controls.

The differentiating capability is static-dynamic reconciliation: Compass can
place an observed path inside the larger set of possible behavior, identify the
contracts it crossed, and compare it with the graph for another revision.

## Layer 3: semantic change and impact intelligence

### Goal

Turn historical graph comparison into behavioral change analysis suitable for
pull requests, architecture enforcement, and regression prevention.

### Capabilities

- Compare control flow, data flow, side effects, errors, authorization checks,
  resource access, and concurrency behavior.
- Detect compatibility changes in APIs, schemas, database structures, events,
  configuration, CLI surfaces, serialization, and public types.
- Propagate impact through callers, data dependencies, state, contracts,
  runtime observations, ownership, and repository boundaries.
- Identify the commit that introduced a behavior or dependency.
- Detect invariant regressions such as a formerly universal authorization
  requirement no longer holding.
- Select tests using reachability, coverage, contract relevance, historical
  failures, and changed behavior.
- Identify affected behavior with no known test evidence.
- Connect changed hot paths to profiles and benchmark evidence.
- Detect concurrent pull requests that intersect the same behaviors,
  contracts, data flows, or impact regions.
- Decompose risk into explainable factors rather than one opaque score.

### Evidence categories

- **Proven impact:** an extracted dependency or deterministic contract break
  establishes the connection.
- **Possible impact:** ambiguity, reflection, dynamic dispatch, or incomplete
  coverage leaves multiple outcomes.
- **Observed impact:** runtime or test evidence confirms the connection.
- **Historical association:** earlier changes or failures suggest relevance but
  do not establish causality.

Only proven contract violations, failed required tests, and deterministic
CompassQL policy findings are eligible for blocking behavior. Other categories
remain advisory unless a policy promotes a reproducible condition.

### Primary experience

The flagship command is conceptually:

```text
compass impact BASE HEAD
```

It answers:

1. What behavior changed?
2. Which contracts changed or broke?
3. What may be affected, and through which witness paths?
4. Which tests are required, recommended, or missing?
5. Which owners and concurrent changes intersect the impact cone?
6. Which conclusions are proven, observed, possible, or historical?

Pull-request analysis compares the synthetic merge result with the merge base,
then traverses only invalidated summaries and indexed reverse dependencies.

## Layer 4: cross-repository and cross-system graphs

### Goal

Extend Compass from repository analysis to organization-scale reasoning without
discarding repository ownership, revision identity, or evidence boundaries.

### Capabilities

- Publish immutable graph manifests containing public symbols, contracts,
  dependencies, ownership, and provenance.
- Assign stable global identities to services, packages, APIs, events,
  database objects, infrastructure resources, and repositories.
- Connect producers and consumers of HTTP/gRPC APIs, events, packages,
  database schemas, configuration, CLIs, and file formats.
- Traverse from a changed contract to downstream consumers while reporting
  revision, freshness, and compatibility.
- Connect code to Terraform, Kubernetes, Helm, CI, queues, databases,
  dashboards, runbooks, and architecture documentation.
- Distinguish code ownership, service ownership, operational responsibility,
  expertise, and review history.
- Execute CompassQL over explicitly selected repository snapshots.
- Enforce organization-wide dependency, layering, security-boundary, and
  contract policies.
- Report which repositories, systems, revisions, languages, and contract types
  are included or missing.

### Federation model

Compass does not merge every repository into one mutable database.

1. Each repository owns immutable graph snapshots and compact public summaries.
2. A local or CI-managed catalog indexes snapshot identities, contracts, and
   cross-repository edges.
3. Federated query planning loads only the required snapshots.
4. Private internals remain local unless a publication profile includes them.

Initial artifact sources include OpenAPI, AsyncAPI, Protobuf, GraphQL, package
manifests, database schemas and migrations, Terraform, Kubernetes, Helm, CI
configuration, SBOMs, lockfiles, architecture decisions, runbooks, and
OpenTelemetry topology exports.

Read-only connectors may later refresh repository catalogs, API registries,
infrastructure state, ownership directories, and observability systems.

Every cross-repository conclusion identifies producer and consumer revisions,
contract version, extraction profile, freshness, and witness path. Missing or
stale evidence reduces declared completeness.

## Layer 5: graph-grounded agent planning and verification

### Goal

Expose Compass evidence as a stable reasoning substrate for coding agents
without making a particular agent or model the source of truth.

### Capabilities

- Translate task intent into relevant behavior, contracts, owners, policies,
  tests, and repositories.
- Assemble the smallest evidence-complete context bundle within a budget.
- Order change plans using dependencies, contracts, migrations, and
  verification obligations.
- Discover invariants, policies, compatibility promises, generated files,
  ownership, and downstream consumers before editing.
- Evaluate a proposed graph delta and identify likely breakage or missing
  migration steps.
- Select tests, policies, contract checks, benchmarks, and runtime comparisons.
- Compare intended and actual behavior after an edit.
- Validate claims such as "all callers were migrated" against graph or
  execution evidence.
- Decompose multi-repository migrations into dependent producer, consumer,
  infrastructure, documentation, and rollout tasks.
- Link decisions and task outcomes to exact versioned graph realizations.

### Typed interface

CLI, MCP, installed Compass skills, IDEs, and third-party agents consume the
same conceptual operations:

```text
scope_task(intent, revision)
build_context(scope, budget)
plan_change(scope, constraints)
predict_impact(proposed_delta)
verification_plan(actual_delta)
verify_claim(claim, evidence)
```

Compass distinguishes facts, deterministic derived conclusions, hypotheses,
and untrusted agent assertions. Agent assertions become evidence only after a
supported validation step.

The target loop is:

```text
understand -> scope -> plan -> change -> compare -> verify
```

## Technical architecture

Compass retains its existing separation between models, language extraction,
graph analysis, query, history, output, CLI, and MCP. New responsibilities
enter as bounded modules rather than accumulating in orchestration or
presentation code.

Conceptual components are:

- **Program IR:** language-neutral program behavior with fact-level provenance
  and capability-specific coverage
- **Provider contracts:** separate file, repository-artifact, and project
  analysis scopes
- **Syntax providers:** deterministic Tree-sitter baseline and fallback
- **Artifact providers:** bounded decoding of official SCIP and later offline
  compiler or binary artifacts
- **Project analyzers:** selected native compiler integrations for deep-tier
  languages
- **Evidence merger:** deterministic identity reconciliation, authority rules,
  conflict preservation, and canonical ordering
- **Framework providers:** language- and framework-specific semantic enrichment
- **Summary engine:** immutable summaries and dependency invalidation
- **Evidence store:** runtime, test, coverage, and profile overlays
- **Impact engine:** semantic comparison and reverse-dependency traversal
- **Contract registry:** normalized system interfaces
- **Federation catalog:** global identities, snapshots, scope, and freshness
- **Agent evidence service:** context, planning, delta, and claim operations
- **CompassQL mapping:** read-only access to stable evidence types

The primary flow is:

```text
Source files ------------> Tree-sitter syntax providers -------+
Offline SCIP/indexes ----> artifact providers -----------------+
Project/build context ---> native project analyzers -----------+
                                                               |
                                                               v
                                      normalized evidence batches
                                                               |
                                                               v
                            deterministic reconciliation and merge
                                                               |
                                                               v
                                     provenance-aware Program IR
        |
        +--> control and data-flow analysis
        +--> contract extraction
        +--> effect and behavior summaries
        |
        v
Immutable graph realization and summary roots
        |
        +<-- runtime, test, coverage, and profile overlays
        |
        v
Semantic diff and impact engine
        |
        +<-- federated contracts and downstream summaries
        |
        v
CLI, CompassQL, MCP, PR reports, and agent evidence API
```

### Boundaries

- Static facts, runtime observations, deterministic conclusions, hypotheses,
  and agent assertions remain distinct.
- Program IR is not LLVM IR and is not required to encode every language in
  one lowest-common-denominator instruction set. It preserves source concepts
  needed for code intelligence and permits capability-specific extensions.
- Tree-sitter providers never claim compiler-resolved types, dispatch, data
  flow, or macro-expanded behavior from syntax alone.
- Artifact providers consume bytes already present on disk; Compass does not
  automatically run third-party indexers in the foundation.
- Absolute checkout roots, SCIP `project_root` values, timestamps, and provider
  input order never affect canonical output.
- Provider implementations do not perform organization-level impact analysis.
- Runtime evidence does not mutate static graph realizations.
- Federation consumes immutable published summaries rather than another
  repository's working tree.
- Agent APIs consume analysis results but cannot insert unverified assertions
  as facts.
- Every derived record carries realization IDs, algorithm and schema versions,
  and witness references.
- Summaries and overlays use versioned Prolly roots so unchanged portions can
  be shared across history.

These are conceptual interfaces first. New Rust crates are justified only by
independent ownership, dependency direction, or compile-time isolation.

### Alternatives rejected

- **Tree-sitter-only generation:** broad and deterministic, but unable to
  provide reliable type checking, overload resolution, macro expansion,
  virtual dispatch, or build-aware semantics.
- **Compiler-native-only generation:** precise, but makes every supported
  language depend on a distinct toolchain and project loader before Compass can
  provide baseline value.
- **LSP-first generation:** useful for later read-only enrichment, but language
  servers are stateful, workspace-dependent, and do not expose a portable
  control-flow or data-flow model.
- **Binary-IR-first generation:** valuable for observed build artifacts, but
  optimized binaries lose source constructs and exclude code that was not
  compiled.

The hybrid provider model is selected because each additional evidence source
improves the same canonical artifact without making that source mandatory.

## Delivery sequence

| Horizon | Primary milestone | User-visible outcome |
| --- | --- | --- |
| 0-3 months | Analysis foundation | Versioned Program IR, provider contracts, Tree-sitter baseline, optional SCIP enrichment, summary store, and benchmark corpus |
| 3-6 months | Deep Rust and TypeScript analysis | High-precision call graphs, control flow, effects, and explainable resolution |
| 6-9 months | Data flow, contracts, runtime artifacts | Data paths, contract graph, and coverage, trace, and profile overlays |
| 9-12 months | Semantic impact intelligence | Behavioral diff, impact cones, test selection, and invariant regression detection |
| 12-15 months | Monorepository maturity | Bounded incremental invalidation, ownership, framework packs, and scale qualification |
| 15-18 months | Repository federation | Published summaries, global identities, and cross-repository contracts and impact |
| 18-21 months | Cross-system intelligence | Infrastructure, database, API, event, documentation, and telemetry reconciliation |
| 21-24 months | Agent evidence API | Scope, context, planning, counterfactual impact, and claim verification |

Three workstreams continue through all milestones:

- Evidence architecture: schema evolution, provenance, confidence, witness
  paths, history, and CompassQL
- Scale architecture: incremental summaries, Prolly reuse, reverse indexes,
  cancellation, memory bounds, and federated planning
- Analysis quality: fixtures, framework models, precision and recall,
  differential tests, and unsupported-behavior accounting

The first vertical slice is:

> For a Rust or TypeScript pull request, explain the behavioral change, show
> its interprocedural impact cone, select relevant tests, and support every
> conclusion with exact witness paths.

## Failure and completeness model

Every result declares coverage separately for syntax, symbol identity,
definitions, references, types, call resolution, control flow, data flow,
effects, and contracts. Each capability declares one state:

- **Complete:** every required evidence source and analysis pass succeeded
  within the declared scope.
- **Partial:** useful evidence exists, but named languages, repositories,
  artifacts, or passes are missing.
- **Indeterminate:** available evidence cannot establish or reject the claim.
- **Failed:** corruption, incompatible schemas, invalid artifacts, or execution
  errors prevented a valid result.

Partial results identify exactly what is missing, the responsible provider or
artifact, and which conclusions may be affected. A companion-manifest digest
mismatch can identify a stale SCIP document; Compass excludes that document's
semantic facts without discarding valid syntax evidence. Raw SCIP remains
usable but declares its freshness unverified. Malformed artifacts, incompatible
schemas, unsafe paths, and resource-limit violations are typed failures and
never replace the previous valid `program.json`. Timeouts and resource limits
return bounded partial results only when a validated subset exists. They never
silently reduce precision or claim complete coverage.

## Evaluation

- Language conformance suites cover dispatch, aliasing, data flow, exceptions,
  concurrency, and framework behavior.
- Differential tests compare applicable results with compiler APIs, language
  servers, and established analyzers.
- Mutation tests introduce known behavioral and contract changes and measure
  detection and localization.
- Historical replay compares predicted impact with later fixes, test failures,
  and runtime evidence.
- Witness validation confirms that every reported path exists and follows
  valid relation semantics.
- Determinism tests require identical inputs to produce byte-equivalent
  summaries and canonical findings.
- Incremental results must equal clean full-analysis results.
- Monorepository qualification enforces time, memory, invalidation-cone, and
  storage-growth budgets.
- Federation tests cover stale, missing, conflicting, unauthorized, and
  incompatible snapshots.
- Adversarial tests cover malformed traces, hostile schemas, oversized
  profiles, and sensitive values.

## North-star metrics

- Call-target and data-flow precision and recall by language and framework
- Percentage of findings with mechanically valid witness paths
- Incremental work as a fraction of full analysis
- Contract-break recall and false-positive rate
- Test-selection recall and test-suite reduction
- Impact-cone accuracy against historical evidence
- Cross-repository coverage and freshness
- Agent claim-verification success and unsupported-claim rejection
- Honest partial or indeterminate results versus incorrect confident findings

Release gates are capability- and language-specific. A language may remain in
the structural or resolved tier while another qualifies for deep analysis.
Compass never hides unsupported depth behind one generic supported-language
label.

## Implemented foundation status

The first Program IR foundation described above is now implemented. Native
`update`, `extract`, and `watch` orchestrate content-addressed per-file syntax
analysis, optional offline official SCIP ingestion, deterministic
provenance-aware merge, and derived summaries. The only output artifact is
`program.json`; `.compass_program.json` is neither read nor written.

Rust and the TypeScript/JavaScript family currently have Tree-sitter syntax
providers. Their evidence is deliberately conservative: unsupported compiler
semantics remain partial or indeterminate. Official SCIP protobuf is decoded
in-process with bounded streaming, path validation, freshness reporting, and an
optional digest-binding companion manifest. Compass does not invoke an indexer
or language server. Program schema 2 implements the four-state completeness
contract above while retaining read compatibility with schema 1. Decoded SCIP
documents are cached by immutable artifact digest, and freshness normalization
is invalidated per indexed document rather than per repository.

The read-only `compass program` surface provides summaries, coverage,
function inspection, callers, source-byte call explanations, and CompassQL over
a Program IR graph projection. TypeScript-family functions carry the same graph
node identities as structural extraction when that identity is unambiguous.

History schema 3 stores Program IR facts and summaries in separate
content-addressed roots, while continuing to read schema-2 realizations as
having empty program roots. Full history diff includes program records;
topology-only diff excludes them. GC, validation, backup, export, and
unchanged-output reuse all cover `program.json`.

The executable qualification corpus is in `fixtures/program-ir/`, and
`scripts/qualify_program_ir.sh` checks cold/warm equivalence, incremental
invalidation, artifact freshness and conflicts, checkout-root independence,
history behavior, compatibility output, workspace tests, formatting, and
denied Clippy warnings.
