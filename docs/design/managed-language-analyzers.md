# Managed language analyzers

This document defines a shared architecture for improving Compass code-graph
quality with optional compiler, indexer, and language-server evidence across
Python, Rust, Go, TypeScript, JavaScript, and Java.

## Decision summary

Compass will add one managed-analyzer framework with language-owned semantic
policies. It will not add an unrelated installer, subprocess protocol, cache,
or graph projection for every language.

The shared framework owns:

```text
tool discovery and verified installation
runtime discovery
project/build-context description
bounded process and protocol execution
provider manifests and coverage
Program evidence normalization
cache and history identity
CLI lifecycle and diagnostics
```

Each language owns:

```text
which tool evidence is meaningful
which Tree-sitter occurrence must exist
how symbols and source anchors match
which facts may strengthen graph relationships
how dynamic dispatch and ambiguity are represented
which project inputs affect completeness
which qualification gates authorize support claims
```

The pipeline remains evidence-first:

```text
                       optional managed analyzer
                       compiler | indexer | LSP
                                  |
                                  v
source -> Tree-sitter -> structural evidence    Program evidence
              |                   |                 |
              |                   +--------+--------+
              |                            v
              +-----------------> exact policy join
                                           |
                                           v
                              graph.json + program.json
```

An analyzer does not publish graph nodes or edges directly. Its facts are
untrusted inputs to `compass-program`; language policy in `compass-resolve`
decides whether exact structural evidence supports projection.

## Current baseline

### Hard-cut universal languages

Python, Go, Rust, and Java are version-1 hard-cut universal adapters. They emit
`SemanticEvidenceBatch` facts and use shared resolution and projection. Their
replaced direct graph paths are not available as fallbacks.

| Language | Structural strengths | Important semantic boundary |
| --- | --- | --- |
| Python | imports, re-exports, decorators, C3 hierarchy dispatch, runtime ownership | dynamic types, generated members, callable objects, monkey patching |
| Go | packages, receivers, embeddings, calls, type references | implicit interfaces, promoted methods, generics, build tags |
| Rust | namespaces, traits, impl ownership, macros, re-exports | trait selection, autoderef, associated items, macro hygiene, `cfg` |
| Java | packages, overload signatures, receiver spelling, inheritance | compiler attribution, generics, overload selection, virtual dispatch |

Candidate or hard-cut status does not mean compiler-grade completeness. It
states which publication architecture is active and which qualification has
been completed.

### TypeScript and JavaScript universal candidates

TypeScript, TSX, JavaScript, and JSX now use the registered universal candidate
route. TypeScript and JavaScript retain distinct adapter identities while
sharing one bounded source-grounded ECMAScript emitter; TSX is the TypeScript
parser dialect. Their replaced direct graph publisher is not a fallback.
Candidate status means the universal route is active while the complete
qualification matrix and any compiler-backed enrichment remain future work.

Compiler enrichment must not create a permanent third graph route beside the
candidate path. Until a compiler provider is independently bounded and
qualified, compiler facts remain Program-only.

### Program and SCIP support

Compass already has a provider-neutral `ProjectAnalyzer` contract and an
official SCIP artifact decoder. Offline SCIP facts enter Program evidence with
provider descriptors, capability coverage, source freshness, resource limits,
and conflict-preserving merge behavior.

The graph projection is intentionally narrower: current compiler projection
only strengthens exact Java calls. This design generalizes the fact boundary
while retaining language-specific projection policies.

### Qualification gap

Current source-grounded qualification recommends independent source-oracle
providers for Go, TypeScript/JavaScript, Java, and Rust and a stratified,
independently adjudicated production audit. Managed analyzers improve the graph
but do not qualify themselves. A graph produced by a compiler cannot use the
same compiler as its only correctness oracle.

## Goals

- Preserve native, local-first structural extraction for every supported
  language when no analyzer is configured.
- Give users one explicit `compass analyzer` lifecycle for setup, status,
  doctor, updates, and removal.
- Reuse the Program evidence contract across artifact, batch, and interactive
  providers.
- Improve exact definitions, references, imports, calls, receiver types,
  implementations, overrides, generated identities, and external symbols.
- Preserve language-specific meaning rather than forcing one dispatch
  algorithm onto incompatible type systems.
- Make meaning-affecting source, tool, runtime, build, dependency, feature,
  target, and permission inputs part of cache and history identity.
- Keep network, build-tool execution, generated code, plugins, macros, and
  project loading behind explicit trust decisions.
- Bound downloads, archives, processes, protocol records, facts, diagnostics,
  memory, duration, concurrency, and retained state.
- Support reproducible offline CI and historical materialization after an
  explicit preparation step.
- Qualify each provider profile and capability independently before expanding
  support claims.
- Give future languages the same extension path after their structural
  evidence, project model, analyzer policy, and qualification are complete.

## Non-goals

- Replacing Tree-sitter with compiler syntax trees.
- Claiming that the existence of a Tree-sitter grammar alone provides
  compiler-grade semantics or support for every language.
- Requiring Python, Node.js, Go, Rust, Java, or a language server for ordinary
  structural graph construction.
- Automatically downloading tools or dependencies during normal graph,
  query, watch, or history commands.
- Automatically executing repository-controlled build scripts, plugins,
  procedural macros, annotation processors, or package lifecycle scripts.
- Treating analyzer availability or a diagnostic-free project as proof of
  complete graph semantics.
- Turning recovered, unknown, dynamic, or ambiguous facts into exact edges.
- Selecting the first, nearest, or repository-unique terminal name when
  language semantics do not prove it.
- Representing all possible runtime dispatch targets as ordinary `calls`.
- Publishing machine-specific tool paths, dependency-cache paths,
  credentials, or private registry URLs into graph artifacts.

## User experience

### Shared lifecycle

```text
compass analyzer setup python --provider scip-python
compass analyzer setup rust --provider rust-analyzer
compass analyzer setup go --provider go-types
compass analyzer setup typescript --provider typescript
compass analyzer setup javascript --provider typescript
compass analyzer setup java --provider jdt-core
```

JavaScript and TypeScript may share one installed tool and runtime while
retaining separate project policies and coverage. Setup is idempotent and
transactional.

```text
compass analyzer list [--format text|json]
compass analyzer status [LANGUAGE] [--format text|json]
compass analyzer doctor LANGUAGE [--format text|json]
compass analyzer update [LANGUAGE] [--yes]
compass analyzer remove LANGUAGE [--project|--tool|--all]
```

`status` and `list` are read-only. `doctor` performs local validation and a
bounded synthetic protocol check but never repairs, downloads, executes a
project build, or resolves dependencies from the network.

### Daily operation

Once configured, users continue to run:

```text
compass update
compass extract --code-only
compass watch
```

Per-command overrides use one repeatable option:

```text
compass update --analyzer python=structural
compass update --analyzer rust=rust-analyzer
compass update --analyzer go=go-types
compass update --analyzer typescript=scip
```

The exact public grammar is finalized in Phase 0. Language-specific aliases
may exist for discoverability, but they map to the same domain service and
machine contract.

### Explicit side effects

Setup reports before mutation:

```text
language and provider
tool and runtime versions
download sources and maximum bytes
installation and project-configuration locations
build-tool, generated-code, plugin, and network permissions
```

Interactive setup asks for confirmation before downloads. Non-interactive
setup requires `--yes`, a locked local bundle, or an already valid
installation. Existing graph commands never prompt.

### Provider state and failure

Durable installation/configuration states are distinct from analysis results:

```text
unconfigured | ready | missing_tool | missing_runtime
incompatible_protocol | corrupt_installation
incomplete_project_context | disabled

complete | partial | ambiguous | failed | limit_exceeded | cancelled
```

| State | Build behavior |
| --- | --- |
| No analyzer configured | Build the structural graph without analyzer warnings |
| Analyzer explicitly disabled | Build and record a structural-only profile |
| Configured analyzer ready | Run it or reuse its validated cache |
| Configured analyzer unavailable | Fail explicitly; do not substitute structural-only meaning |
| Partial analyzer evidence | Publish only under the command's partial policy and retain reasons |
| Analyzer timeout or limit | Report a limit result, never an empty successful analysis |
| Analyzer/AST mismatch | Retain structural evidence and record unmatched provider evidence |
| Exact providers disagree | Preserve conflict; do not choose a preferred target |

## Configuration model

Illustrative project configuration:

```toml
[analyzers.python]
provider = "scip-python"
environment = "project"
use_library_code_for_types = false
allow_dependency_network = false

[analyzers.rust]
provider = "rust-analyzer"
features = "project"
targets = ["host"]
allow_build_scripts = false
allow_proc_macros = false
allow_dependency_network = false

[analyzers.go]
provider = "go-types"
workspace = "project"
build_tags = []
allow_cgo = false
allow_module_network = false

[analyzers.typescript]
provider = "typescript"
projects = "references"
dependency_mode = "existing"
allow_package_scripts = false
allow_dependency_network = false

[analyzers.javascript]
provider = "typescript"
allow_js = true
check_js = "project"

[analyzers.java]
provider = "jdt-core"
classpath_mode = "project"
allow_build_tool = false
allow_dependency_network = false
```

These keys are design examples, not shipped configuration. Public keys require
typed parsing, validation, fingerprints, compatibility review, reference
documentation, and CLI tests.

Precedence is command option, project configuration, user configuration,
documented environment override, then the built-in structural default. Paths
may be configured, but published artifacts retain normalized identities and
digests rather than machine-specific absolute paths.

## Architecture

### Shared component flow

```text
compass-cli
  analyzer lifecycle and build overrides
        |
        v
compass-core
  select profiles, build project contexts, sequence coherent publication
        |
        +----------------------+---------------------+
        |                      |                     |
        v                      v                     v
compass-analyzers       compass-languages     focused project describers
  managed tools          Tree-sitter facts     Python | Cargo | Go | Node | Java
  runtimes                      |                     |
  bounded runner                |                     |
        |                       |                     |
        v                       |                     |
external analyzers              |                     |
  SCIP | batch bridge | LSP     |                     |
        +-----------------------+---------------------+
                                v
                         compass-program
                  normalize, validate, merge, cover
                                |
                                v
                         compass-resolve
                       language policy joins
                                |
                                v
                         compass-graph
                                |
                                v
                  graph.json | program.json | history
```

### Ownership

| Responsibility | Owner |
| --- | --- |
| Analyzer parsing, help, streams, JSON outcomes | `compass-cli` |
| Profile selection, parallelism, cancellation, publication | `compass-core` |
| Tool/runtime installation, protocol runner, shared manifests | New `compass-analyzers` |
| File/cache/lease/archive primitives | `compass-files` |
| Evidence, coverage, merge, artifact decoding | `compass-program` |
| Structural facts and exact AST occurrences | `compass-languages` |
| Language-owned analyzer projection policy | `compass-resolve` |
| Relationship contracts and validation | `compass-model` and `compass-graph` |
| Immutable profile/realization identity | `compass-history` |
| Tool-specific adapters | `integrations/compass-<tool>-bridge` or focused native crate |

`compass-analyzers` contains no language resolver. `compass-languages` does not
download or run tools. Bridges know Program protocols, not graph JSON.

### Provider classes

| Class | Examples | Primary use |
| --- | --- | --- |
| Artifact | SCIP index | reproducible offline batch evidence |
| Batch project analyzer | Go types, TypeScript Compiler API, JDT Core | cold/update/history |
| Interactive project analyzer | Pyright, rust-analyzer, gopls, tsserver, JDT LS | watch incrementality |

All normalize into `EvidenceBatch`. Provider class affects lifecycle,
freshness, cache, and cancellation; it does not make evidence exact by itself.

## Managed tools and project context

### Immutable tool store

```text
<compass-data>/analyzers/
  tools/<tool>/<version>/manifest.json
  tools/<tool>/<version>/bin-or-runtime/
  runtimes/<runtime>/<version>/<platform>/
  staging/
  leases/
  workspaces/
```

One tool may serve several languages. Installations are immutable, updates
validate a new installation before selection changes, and failed updates leave
the prior version ready.

The managed manifest records a schema, tool/version, protocol, languages,
runtime requirement, archive and installed-file digests, source, licenses, and
SBOM. Unknown major schemas fail explicitly.

### Runtime resolution

Every runtime follows the same shape:

```text
explicit command path
  -> project configuration
  -> user configuration
  -> standard environment variable
  -> executable on PATH
  -> explicitly installed managed runtime
```

Language variables such as `VIRTUAL_ENV`, `GOROOT`, `RUSTUP_TOOLCHAIN`, and
`JAVA_HOME` never outrank explicit Compass configuration. Compass validates
runtime version, architecture, path, and capabilities and invokes it without a
shell.

### Project description

Analyzers receive a validated description rather than unrestricted discovery:

```text
repository and source inventory digest
module/package/crate/project boundaries
source roots and exact source digests
dependency artifacts and identities
target/runtime/language versions
feature, build-tag, compiler, and resolver options
generated-source policy
execution and network permissions
resource limits
```

Discovery proceeds from explicit configuration, deterministic manifest
parsing, existing artifacts, safe metadata commands, explicitly authorized
build loading, then separately authorized dependency network access.
Unsupported dynamic configuration yields partial coverage rather than implicit
execution.

The build-context digest includes tool/runtime/protocol identity, source and
project inventory, semantic dependency order, language options, features,
tags, targets, generated-code policy, execution/network permissions, and
limits affecting completeness.

## Provider protocol

### Transport and handshake

Batch bridges use versioned request and framed response files in unique
staging. Interactive providers use bounded protocol clients but normalize
through the same validator.

The handshake declares tool/bridge versions, supported protocol majors,
languages, fact families, project requirements, limits, and incremental
document-version support. Human `--version` prose is not a compatibility
contract.

### Fact families

```text
DefinitionIdentity     ReferenceIdentity       CallResolution
ReceiverType           ExpressionType          ImportResolution
ExportResolution       TypeHierarchy           Override
Implementation         GeneratedDefinition     Expansion
ExternalDefinition     Diagnostic
```

Every fact carries provider/configuration identity, language/project identity,
repository-relative path and exact UTF-8 byte range when source-backed, stable
provider symbol, typed payload, evidence status, supporting IDs, and coverage
reasons. Analyzer-native offsets are converted against exact source bytes and
revalidated in Rust.

New facts are added only when meaning, anchors, merge, coverage, bounds, and a
consumer are defined. Opaque provider JSON is not an escape hatch.

## Evidence merge and projection

Program merge remains deterministic and conflict-preserving:

- identical facts deduplicate;
- exact providers agreeing strengthen coverage;
- exact providers disagreeing retain targets plus `provider_conflict`;
- recovered evidence cannot override exact evidence;
- partial contexts cannot publish complete coverage;
- unmatched facts remain inspectable in `program.json`.

An analyzer fact may strengthen a graph relationship only when:

1. an allowed Tree-sitter candidate or occurrence exists;
2. normalized source path and exact range match;
3. source and project context are fresh;
4. target maps uniquely to a local or validated external definition;
5. target kind and ownership satisfy the structural candidate;
6. exact providers at that occurrence agree; and
7. capability coverage permits use.

Facts that fail this contract remain Program evidence and do not invent graph
relationships.

The public graph distinguishes static `calls`, `constructs`, `references`,
module bindings, exact `overrides`/`implements_method`, conservative
`may_dispatch_to`, and generated `expands_to` meaning. New relations require
model, validation, query, output, history, compatibility, and migration review.

## Language policies

### Python

Use verified `scip-python` first. Add Pyright only through a supported stable
interface or pinned Compass bridge.

Compass should pass `scip-python` an explicit, frozen environment manifest so
the indexer does not discover packages by starting `pip` implicitly. Any
fallback environment-discovery process is a separately bounded and reported
project-context operation.

Project identity includes Python version, roots/search paths, environment,
project configuration, typeshed/stubs, editable packages, namespace policy,
and library-code-for-types policy. Compass does not install packages during a
graph build.

Useful evidence includes imported symbols, typed receivers, overloads,
protocols, properties, static/class methods, callable objects, constructors,
typed generated members, callbacks, and returns.

Monkey patching, metaclasses, runtime decorator replacement, dynamic imports,
computed attributes, reflection, and extensions without stubs stay partial.
`Unknown` and `Any` do not authorize terminal-name matching.

**Acceptance criteria**

- Typed facts never change untyped dynamic calls.
- `self`, `cls`, `super`, protocols, properties, descriptors, and callable
  objects have exact and ambiguity tests.
- Stub/source disagreement remains an explicit conflict.
- Environment and stub changes invalidate project context.
- Django and held-out qualification preserve current C3/runtime ownership.

### Rust

Use rust-analyzer SCIP first and a persistent rust-analyzer provider later.
Rust compiler/rustdoc artifacts may supplement public API identity but do not
replace call-site evidence without qualified anchors.

The SCIP command and its output behavior are pinned as a tool capability, not
assumed to be a stable Compass protocol.

Project identity includes manifests, lockfile, workspace, toolchain, features,
targets, `cfg`, dependencies, build-script/proc-macro status, and admitted
generated output.

Useful evidence includes imports/re-exports, inherent versus trait methods,
impls, associated items, generic substitutions, autoderef/autoref, closures,
function pointers, macro expansion/hygiene, and external crates.

Build scripts and procedural macros execute code and require separate
permissions. Disabled execution produces explicit coverage reasons.

**Acceptance criteria**

- Inherent, trait, blanket, default, and ambiguous selection have positive and
  negative tests.
- Autoderef cannot rebind unrelated same-named methods.
- Macro facts retain invocation/expansion provenance without invented anchors.
- Feature, target, `cfg`, build-script, macro, and dependency changes invalidate
  the documented scope.
- Current Rust call quality does not regress while references,
  implementations, and containment improve on held-out evidence.

### Go

Build a Compass batch bridge over `go/packages` and `go/types`. Add `go/ssa`
plus CHA/RTA only in a later dispatch phase. Use `gopls` for persistent watch
analysis. The bridge emits Compass protocol because this design has not
selected a maintained Go SCIP indexer.

Project identity includes modules, sums, workspaces, vendor/replacements, Go
versions, GOOS/GOARCH, build tags, CGO, dependency artifacts, generated-source
policy, package driver, and module-network permission.

Useful evidence includes packages, imported objects, pointer/value receivers,
promoted methods, method expressions/values, generics, interface satisfaction,
and external definitions.

Interface calls target the selected interface method; concrete matches are
implementations or bounded dispatch candidates, not ordinary calls. Function
values and reflection remain incomplete without proof.

Compass does not inherit an untrusted `GOPACKAGESDRIVER`. A non-default package
driver must be explicitly selected, identified in project context, and run
under the same process and network bounds as the Go toolchain.

**Acceptance criteria**

- Module/workspace/vendor/replacement/test/build-tag identities are
  deterministic.
- Pointer/value method sets and multi-level embedding have exact/ambiguous
  tests.
- Interface satisfaction never becomes a call without call-site evidence.
- Dynamic functions/reflection retain incomplete coverage.
- Network-disabled tests never contact module services.

### TypeScript

TypeScript and TSX use version-1 universal candidate evidence and have their
replaced publication/resolution branches removed. Promotion to a complete
adapter still requires the conformance and corpus gates below; analyzer facts
remain Program-only until a separate provider is qualified.

Use verified `scip-typescript` first. Build a pinned TypeScript Compiler API
bridge for controlled batch analysis and BuilderProgram incrementality.

The Compiler API is not treated as a stable external protocol. The bridge pins
the exact TypeScript release and exposes only Compass's versioned protocol.

Project identity includes TypeScript/Node versions, tsconfig inheritance and
references, compiler/module options, path aliases, package conditions,
package-manager/lock/workspace identities, declarations/generated sources,
and JSX runtime.

Useful evidence includes type/value/namespace symbols, overloads, generics,
structural types, declaration merging, inferred types, imports/exports,
callbacks, properties, JSX, and external declarations.

**Acceptance criteria**

- Universal cutover proves one route and no relation-family regression before
  graph enrichment.
- ESM/CJS interop, NodeNext, bundler, paths, references, merging, overloads,
  JSX, and conditional exports have exact/negative fixtures.
- Compiler upgrades change provider identity and invalidate caches.
- Missing dependencies/declarations yield partial coverage, not terminal-name
  recovery.

### JavaScript

JavaScript/JSX/MJS/CJS use the version-1 JavaScript universal candidate and
share the TypeScript emitter with distinct language policy and identity.
Analyzer graph projection remains disabled until a provider is separately
qualified.

Share the TypeScript tool/runtime but retain distinct JavaScript policy for
`allowJs`, `checkJs`, JSDoc, declarations, CJS/ESM, and package resolution.

Useful evidence includes modules, JSDoc types, classes/constructors,
prototype/object members, callbacks, promise chains, bound functions, JSX,
and imported declarations.

`eval`, computed modules/properties, runtime prototype mutation, proxies,
unmodeled virtual modules, and framework magic remain partial. `.call`,
`.apply`, and `.bind` strengthen calls only with exact original callable and
invocation meaning.

**Acceptance criteria**

- JavaScript and TypeScript coverage remain separate.
- CJS/ESM cycles, re-exports, prototypes, JSDoc overloads, JSX, and package
  conditions have exact/negative fixtures.
- `checkJs = false` cannot claim checked-project type coverage.
- Dynamic/computed forms never fall through to repository-wide matching.

### Java

Java shares this lifecycle, Program protocol, cache, history, and
qualification policy. [Managed JDT integration](java-jdt-integration.md) owns
its JDT Core batch provider, JDT LS watch provider, classpath model, exact
AST/compiler join, and Java-specific acceptance criteria.

## External identity and dispatch

External definitions require validated dependency evidence:

```text
Python distribution/module/symbol/stub identity
Rust package source/crate/namespace identity
Go module/replacement/package/object identity
npm package/source/export/declaration identity
Java artifact digest/binary owner/member descriptor
```

They contain artifact provenance and no invented source location. Source
attachments are separate validated evidence. Credentials, private registry
secrets, cache roots, and absolute paths do not enter published identities.

Static selection and runtime possibility remain distinct:

```text
calls             one statically selected declaration
implements_method exact contract implementation
overrides         exact language-defined override
may_dispatch_to   conservative runtime candidate
unknown_dispatch  coverage/diagnostic, not an edge
```

`may_dispatch_to` needs a bounded language algorithm, public contract, query
consumer, and independent qualification. It is never encoded as several
equally exact calls.

## Security, caching, and history

### Independent permissions

```text
download analyzer | download runtime | read dependency artifacts
resolve dependency network | run metadata/build tool
run scripts/plugins/macros/processors | run CGO/native helpers
retain interactive server workspace
```

One permission never implies another. Status and Program coverage show the
effective policy and limitations.

Processes use direct arguments, explicit working directories, allowlisted
environment, bounded time/memory/streams/records/concurrency, process-tree
termination, staged validated output, and atomic publication. Interactive
workspaces are isolated by repository, worktree, provider, and configuration;
document versions prevent stale publication.

### Cache and history identity

Cache keys include provider/protocol, tool/runtime, project context,
source/module inventory, dependencies, permissions, limits, and
normalizer/merge/projection/resolver versions. Caches store validated Program
evidence, not native analyzer objects.

Fine-grained reuse requires versioned language dependency/ABI algorithms; it
is not assumed from body-only changes.

History records exact profiles and remains offline. It may invoke an already
installed compatible batch analyzer but never install/update tools, fetch
dependencies, or substitute providers. Missing pinned inputs make a profile
unrealizable. Published realizations remain immutable.

## Phased implementation

### Phase 0: Contracts and design alignment

**Direction**

- Finalize CLI/configuration, provider classes, handshake, frames, manifests,
  diagnostics, coverage, facts, relations, and cache keys.
- Generalize Java infrastructure without weakening Java policy.
- Add fake artifact, batch, interactive, downloader, runtime, process, and
  project-context providers.
- Capture structural and SCIP baselines for every language.

**Acceptance criteria**

- Every machine contract has a major version and canonical bounded encoding.
- Unknown, duplicate, conflicting, stale, malformed, oversized, and
  path-escaping inputs have negative tests.
- Fakes cover complete, partial, recovered, ambiguous, conflict, timeout,
  cancellation, crash, and limits without external tools.
- Shared and JDT designs use one lifecycle and terminology.
- No public help claims analyzer availability.

### Phase 1: Managed analyzer foundation

**Direction**

- Create `compass-analyzers` with injected installation/runtime/process/lease
  and protocol boundaries.
- Extract safe primitives from existing upgrade behavior.
- Implement `compass analyzer` lifecycle over fixture tools.
- Add project/user selections and build-profile identities without real tools.

**Acceptance criteria**

- Unconfigured commands make no new network/process calls.
- Setup is explicit, idempotent, atomic, and concurrency-safe.
- Checksum, redirects, archive attacks, interruption, permission, and corrupt
  state leave no selected partial installation.
- Runtime/protocol behavior is deterministic cross-platform.
- Removal cannot delete user data, broad roots, or leased tools.
- Text/JSON CLI tests cover every durable state.

### Phase 2: Artifact-first SCIP analyzers

**Direction**

- Add configured runners for `scip-python`, rust-analyzer SCIP,
  `scip-typescript`, and `scip-java`.
- Extend verified manifests to project/tool inputs.
- Normalize complete available SCIP facts into Program evidence.
- Keep non-Java graph projection disabled.

**Acceptance criteria**

- No tool runs/downloads without configuration.
- Equivalent inputs yield byte-identical normalized evidence.
- Source/tool/config/dependency changes invalidate expected scope.
- Stale, raw, malformed, conflicting, and oversized indexes fail closed.
- No non-Java graph relationship changes.
- Pre-generated offline SCIP remains supported.

### Phase 3: TypeScript and JavaScript universal completion

**Direction**

- Complete the version-1 TypeScript/TSX/JavaScript/JSX candidate conformance
  matrix and corpus qualification.
- Extend structural evidence only when each new relation has direct source
  anchors, resolver coverage, and negative fixtures.
- Remove any remaining candidate-era compatibility branch atomically when its
  replacement is fully qualified.

**Acceptance criteria**

- Each source has one publication route.
- Existing relations, occurrences, frameworks, modules, repeated calls, and
  negatives do not regress.
- TypeScript/JavaScript coverage is distinct.
- ESM/CJS/NodeNext/JSX/declarations/prototypes/recovery are deterministic.
- Replaced production branches are removed.
- Performance/RSS are measured before completion claims.

### Phase 4: Language-owned exact projection

**Direction**

- Generalize projection storage/indexing with one policy module per language.
- Project exact SCIP identities for hard-cut languages.
- Add validated external identities and Go policy fixtures.

**Acceptance criteria**

- Only matching language-approved AST occurrences can change edges.
- Non-call references never become calls.
- Agreement/conflict/recovery/partial cases are deterministic.
- Direction, multiplicity, occurrence, language, provider, origin, and rule
  provenance survive publication.
- Rejected facts remain Program-visible.
- Structural-only digests remain unchanged without analyzers.

### Phase 5: Native batch analyzers

**Direction**

- Implement Go types, TypeScript Compiler API, and JDT Core bridges.
- Add Python/Rust bridges only where stable artifact/protocol facts are
  insufficient.
- Add static project discovery and existing-dependency identity before builds.

**Acceptance criteria**

- Bridges have pinned dependencies, licenses, SBOM, reproducible release
  evidence, handshake, bounds, and cross-platform fixtures.
- No bridge escapes supplied roots or executes project code by default.
- Project descriptions are canonical across ecosystems.
- Missing/disabled inputs become partial coverage.
- Native offsets convert to exact validated UTF-8 anchors.
- Unchanged contexts reuse evidence without runtime startup.

### Phase 6: Controlled builds and dependencies

**Direction**

- Add separately authorized build metadata/execution and network resolution.
- Add policies for scripts, macros, CGO, package scripts, processors, and
  generated sources.
- Fingerprint admitted outputs and permissions.

**Acceptance criteria**

- Nothing executes or downloads without its specific permission.
- Malicious fixtures prove disabled paths do not execute.
- Processes remain bounded and shell-free.
- Permission changes enter cache/history identity.
- Generated facts retain provenance and no invented anchors.
- Secrets and machine cache paths do not publish.

### Phase 7: Interactive analyzers and watch

**Direction**

- Add Pyright, rust-analyzer, gopls, tsserver, and JDT LS after batch policy is
  stable.
- Isolate workspace state and normalize incrementally through Program.
- Preserve watch coalescing and atomic publication.

**Acceptance criteria**

- Lifecycle, progress, cancellation, crash/restart, timeout, and stale response
  paths have bounded tests per server.
- Old document results never publish into new builds.
- Edit/revert returns canonical evidence/graph.
- Unconfigured native watch is unaffected; configured absence fails clearly.
- Server import cannot expand permissions silently.
- Previous coherent output survives failure.

### Phase 8: Hierarchy, dispatch, and generated semantics

**Direction**

- Add exact override/implementation contracts and bounded language dispatch.
- Add generated/expansion relationships only with provenance.
- Add query/impact consumers before public relationships.

**Acceptance criteria**

- Static calls, implementations, overrides, and possible dispatch stay
  distinct.
- Traversal is deterministic/bounded and distinguishes limit from none.
- Every language has positive, negative, ambiguous, and unknown dispatch cases.
- Generated facts never masquerade as hand-written evidence.
- New relationships round-trip through model/query/output/history/diff.

### Phase 9: Independent qualification and promotion

**Direction**

- Qualify structural, artifact, batch, interactive, generated, and dispatch
  profiles separately.
- Use pinned varied real repositories and at least 2,000 independently
  adjudicated facts under the approved statistical gate.
- Measure correctness before performance.

**Acceptance criteria**

- Oracles are independent of evaluated providers.
- Precision confidence and critical-violation gates pass.
- No advertised relation family regresses.
- Equivalent inputs produce stable graph/occurrence/Program/cache/history
  digests.
- Latency, startup, RSS, reuse, graph size, and publication time are reported.
- Cross-platform packages pass setup/doctor/analysis/offline/update/remove and
  corrupt-state recovery.
- Promotion is limited to exact passing capabilities/profiles.

## Cross-phase verification matrix

| Surface | Required evidence |
| --- | --- |
| Contracts | round trip, unknown major, canonical ordering, malformed, duplicate, oversized |
| Installation | mock server, checksums, redirects, archive attacks, interruption, concurrency |
| Runtime | explicit/config/environment/PATH/managed precedence and incompatibility |
| Project context | modules, dependencies, features/tags/targets, missing inputs, digest |
| Process | timeout, cancellation, tree kill, bounded streams, validation, cleanup |
| Program merge | agreement, conflict, recovered, stale, partial, unmatched, order independence |
| Projection | exact join, identity, direction, occurrence, multiplicity, provenance, negatives |
| Dispatch | static versus implementation versus possible target, bounds, unknown |
| Incrementality | cold, unchanged, body/ABI/dependency/tool/config/permission edits |
| History | offline materialization, reopen, immutable realization, missing tool/profile mismatch |
| CLI | help, text/JSON, TTY/non-TTY, idempotence, diagnostics, no partial state |
| Security | no implicit execution/network, no disclosure, malicious project fixtures |
| Qualification | independent held-out oracle, relation gates, determinism, correctness before speed |

## Language readiness matrix

| Milestone | Python | Rust | Go | TypeScript | JavaScript | Java |
| --- | --- | --- | --- | --- | --- | --- |
| Native structural path | Available | Available | Available | Universal candidate | Universal candidate | Available |
| Universal hard cut | Available | Available | Available | Candidate route; qualification ongoing | Candidate route; qualification ongoing | Available |
| Offline SCIP ingestion | Generic | Generic | Generic | Generic | Generic | Calls projected |
| Managed artifact runner | Planned | Planned | Not selected | Planned | Planned | Planned |
| Native batch analyzer | Optional | Optional | Go types | Compiler API | Compiler API | JDT Core |
| Interactive analyzer | Pyright | rust-analyzer | gopls | tsserver | tsserver | JDT LS |
| Exact graph projection | Planned | Planned | Planned | After cutover | After cutover | Local calls available |
| Dispatch qualification | Planned | Planned | Planned | Planned | Limited/planned | Planned |

This matrix describes architecture status, not release scheduling.

## Open questions

1. Which language-specific discovery aliases, if any, delegate to
   `compass analyzer` without creating separate lifecycle implementations.
2. Whether ecosystem project descriptions live in `compass-analyzers` or
   focused crates behind one trait.
3. Which tool distributions Compass may redistribute versus only discover.
4. Which SCIP modes can run without repository build execution.
5. How universal profiles divide TS/TSX/JS/JSX/MJS/CJS mixed projects.
6. Which Program facts or graph relations require new schema majors.
7. Which external dependency identities belong in the graph versus Program.
8. Which metadata commands are safe enough for default use.
9. Whether supported LSP APIs expose sufficient exact facts or need extensions.
10. Which dispatch relationships have consumers justifying public contracts.
11. What measured default limits suit both small and pinned large corpora.

Open questions do not authorize guesses. Resolve each in the phase PR that
establishes its contract and negative tests.

## Completion definition

The integration is complete only when:

- every language retains a supported native structural profile;
- setup, inspection, update, use, and removal share one Compass lifecycle;
- tools, runtimes, builds, generated code, processes, and networks remain
  explicit bounded trust boundaries;
- output flows through Program evidence and language-owned exact joins;
- dynamic, recovered, missing, stale, ambiguous, and conflicting semantics
  remain visible;
- meaning-affecting inputs participate in cache/history identity;
- current, watch, and history workflows publish coherent artifacts;
- TypeScript/JavaScript completion before analyzer facts alter production
  graphs;
- cross-platform correctness, determinism, security, packaging, history, and
  performance gates pass per advertised profile; and
- current behavior and planned phases remain distinguished across all public
  documentation.

## Related pages

- [Managed JDT integration](java-jdt-integration.md)
- [Language architecture](language-architecture.md)
- [System architecture](architecture.md)
- [Security and privacy](security-and-privacy.md)
- [Storage and history](storage-and-history.md)
- [Universal semantic evidence](../reference/universal-semantic-evidence.md)
- [Extending Compass](../implementation/extending-compass.md)
- [Compatibility](../../COMPATIBILITY.md)

**Next step:** complete Phase 0 as a contract-only increment and align the
Java-specific design before adding downloads, processes, or availability
claims.
