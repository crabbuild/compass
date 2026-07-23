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

- A typed semantic intermediate representation for functions, methods, values,
  types, call sites, branches, reads, writes, exceptions, asynchronous
  boundaries, and effects
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
Java/Kotlin, and Go follow through explicit language adapters.

### Incremental model

Editing a symbol invalidates its behavior summary and only the dependent
summaries affected by changed facts. Versioned summaries live alongside graph
realizations so historical analysis and semantic diff can reuse unchanged
results.

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

- **Semantic IR:** language-neutral program behavior
- **Language adapters:** language- and framework-specific translation
- **Summary engine:** immutable summaries and dependency invalidation
- **Evidence store:** runtime, test, coverage, and profile overlays
- **Impact engine:** semantic comparison and reverse-dependency traversal
- **Contract registry:** normalized system interfaces
- **Federation catalog:** global identities, snapshots, scope, and freshness
- **Agent evidence service:** context, planning, delta, and claim operations
- **CompassQL mapping:** read-only access to stable evidence types

The primary flow is:

```text
Source and manifests
        |
        v
Language and framework adapters
        |
        v
Typed semantic IR
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
- Language adapters do not perform organization-level impact analysis.
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

## Delivery sequence

| Horizon | Primary milestone | User-visible outcome |
| --- | --- | --- |
| 0-3 months | Analysis foundation | Versioned semantic IR, summary store, benchmark corpus, and language-adapter contract |
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

Every result declares one state:

- **Complete:** every required evidence source and analysis pass succeeded
  within the declared scope.
- **Partial:** useful evidence exists, but named languages, repositories,
  artifacts, or passes are missing.
- **Indeterminate:** available evidence cannot establish or reject the claim.
- **Failed:** corruption, incompatible schemas, invalid artifacts, or execution
  errors prevented a valid result.

Partial results identify exactly what is missing and which conclusions may be
affected. Timeouts and resource limits return bounded partial results when a
valid subset exists. They never silently reduce precision or claim complete
coverage.

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
