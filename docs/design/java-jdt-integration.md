# Managed JDT integration

This document defines how Compass can add compiler-grade Java evidence through
Eclipse JDT while preserving the native Tree-sitter code graph as the default
and fallback.

## Decision summary

Compass will not embed a JVM or the full JDT language server into the Rust
process. Shared installation, runtime, protocol, cache, history, and CLI
lifecycle behavior follows
[Managed language analyzers](managed-language-analyzers.md); this document owns
the Java-specific provider and semantic policy. The integration has two
optional provider modes:

1. A small Compass-owned JVM bridge built on JDT Core performs bounded batch
   analysis for `update`, `extract`, history, and CI.
2. A managed JDT LS process may later provide incremental project state for
   `watch` and editor-oriented workflows.

Both providers emit versioned, provider-neutral Program evidence. They do not
publish graph records directly. Compass joins their facts to exact
Tree-sitter evidence, preserves provider provenance and uncertainty, and
publishes only validated relationships.

Normal structural commands remain native and offline:

```text
unconfigured Compass
  -> statically linked Tree-sitter Java parser
  -> Java universal evidence
  -> structural graph

configured JDT provider
  -> the same structural graph
  + bounded JDT evidence
  -> exact evidence join
  -> enriched graph
```

Installing JDT or a managed Java runtime is always an explicit operation.
Ordinary graph construction never downloads tools, resolves remote
dependencies, or executes Maven or Gradle merely because Java files exist.

## Context

### Available now

Java is a version-3 `Qualifying` universal evidence pipeline. Its Tree-sitter
producer emits
source-backed declarations, scopes, packages, imports, annotations,
inheritance, type references, method and constructor candidates, receiver
spelling, argument counts, ownership, and external-reference evidence.

Compass also accepts verified offline SCIP indexes through
`--program-artifact`. Fresh Java definitions and references can disambiguate a
Tree-sitter-emitted `Calls` or `Constructs` occurrence when both sides match
exact repository-relative byte ranges. Stale evidence, non-call references,
provider conflicts, ambiguous definitions, and external-only targets do not
retarget the graph.

### Remaining problem

Tree-sitter proves syntax and structural context but does not perform Java type
attribution. It cannot, by itself, reliably select among all overloads, apply
generic substitutions, infer expression receiver types, or distinguish the
statically selected declaration from possible runtime implementations.

Requiring users to generate and pass a SCIP file improves accuracy but leaves
tool discovery, installation, classpath construction, freshness, cache
identity, and failure handling outside Compass. A first-class JDT integration
must bring those concerns under one explicit and inspectable CLI workflow.

### Why two JDT modes

JDT Core exposes batch ASTs and resolved bindings without requiring the full
language-server lifecycle. It is the smaller and more deterministic boundary
for cold graph construction.

JDT LS adds project import, persistent workspace state, document updates,
call hierarchy, type hierarchy, and other interactive services. It also adds
an OSGi runtime, workspace directories, initialization state, more substantial
resource use, and additional Maven/Gradle behavior. It is therefore a later
watch-mode provider, not the first batch implementation.

## Goals

- Keep `compass update` and graph queries functional without Java, JDT, a
  network, or an installed semantic tool.
- Let a user install and configure Java semantic analysis entirely through the
  `compass` executable.
- Resolve exact Java symbols, overloads, receiver types, method references,
  lambda targets, generic substitutions, overrides, and implementation facts.
- Preserve exact source anchors, relationship direction, multiplicity,
  provider identity, coverage, and ambiguity.
- Make tool, runtime, classpath, source, configuration, and provider versions
  part of deterministic cache and history identity.
- Bound downloads, archive expansion, subprocess duration, memory, output,
  facts, diagnostics, paths, and concurrency.
- Support macOS, Linux, and Windows without shell command construction.
- Permit offline, reproducible CI and historical materialization after an
  explicit setup step.

## Non-goals

- Replacing the Tree-sitter Java adapter or making JDT required for structural
  graph construction.
- Embedding JVM bytecode or JDT objects inside the Rust process.
- Automatically downloading a tool during `update`, `extract`, `watch`, or a
  historical build.
- Automatically executing repository-controlled Maven or Gradle logic.
- Treating a recovered JDT binding as exact evidence.
- Selecting one runtime implementation for a virtual call when several remain
  possible.
- Publishing JDT AST nodes as a second, competing Java graph.
- Adding network access to historical materialization.
- Marking Java `Qualified` solely because a provider exists.

## User experience

### One-time project setup

The primary workflow is:

```text
compass analyzer setup java --provider jdt-core
compass update
```

`analyzer setup java` performs an explicit, transactional setup for the current
project. In an interactive terminal it reports the selected provider, Java
runtime, download size, installation location, project configuration change,
and whether build-tool execution or network dependency resolution is enabled.
It obtains confirmation before any download unless `--yes` is present.

Automation uses explicit options:

```text
compass analyzer setup java \
  --provider jdt-core \
  --tool-version locked \
  --java-home /opt/jdk-21 \
  --non-interactive
```

The command is idempotent. Repeating the same request validates and reuses the
installed tool instead of downloading or rewriting it.

### Daily commands

Once the project is configured, existing commands use the pinned provider:

```text
compass update
compass extract --code-only
compass watch
```

Per-command overrides remain available:

```text
compass update --analyzer java=structural
compass update --analyzer java=jdt-core
compass update --analyzer java=scip --program-artifact index.scip
```

`structural` is the explicit native-only mode. `jdt-core` requests the batch
provider. `scip` requires one or more explicit artifacts. A future `jdt-ls`
mode is limited to supported long-running workflows.

### Inspection and lifecycle

```text
compass analyzer status java [--format text|json]
compass analyzer doctor java [--format text|json]
compass analyzer update java [--yes]
compass analyzer remove java [--project|--tool|--all]
```

`status` is read-only and reports configuration plus installed identities.
`doctor` validates runtime compatibility, tool manifests, checksums,
classpath inputs, filesystem permissions, protocol negotiation, and a bounded
synthetic analysis. It does not repair or download anything.

`remove --project` removes only project selection. `remove --tool` removes an
unreferenced managed installation after resolving its exact path and checking
leases. Neither command deletes build-tool dependency caches or user-managed
JDKs.

### Failure behavior

The provider selection changes the failure contract:

| State | Behavior |
| --- | --- |
| No provider configured | Build the structural graph without a JDT warning |
| Provider explicitly disabled | Build the structural graph and record the native-only profile |
| Configured provider ready | Run it and publish verified enrichment |
| Configured provider missing or incompatible | Fail with a typed diagnostic and exact setup/repair command |
| Provider returns partial evidence | Preserve coverage reasons; publish only when the command permits partial results |
| Provider times out or exceeds a limit | Report a limit failure, never an empty successful analysis |
| Provider conflicts with structural or another provider | Preserve the structural result and explicit conflict evidence |

Compass must not silently fall back from a configured JDT profile to a
different graph realization. Users can explicitly choose structural mode or
`--allow-partial` where that command already permits partial publication.

## Configuration

Project configuration selects meaning-affecting behavior:

```toml
[analyzers.java]
provider = "jdt-core"
tool_version = "locked"
runtime = "managed"
language_level = "project"
classpath_mode = "project"
allow_build_tool = false
allow_dependency_network = false
max_heap_bytes = 2147483648
timeout_seconds = 600
```

The exact public keys are finalized with the CLI implementation and added to
the configuration reference. Configuration precedence is:

1. command-line option;
2. project Compass configuration;
3. user Compass configuration;
4. environment override where documented;
5. built-in default.

Paths may be configured, but persisted history profiles store normalized tool
identities and digests rather than machine-specific absolute paths or secret
values.

## Architecture

### Component flow

```text
compass-cli
  parse `java` commands and build options
        |
        v
compass-core
  select provider, construct build context, sequence publication
        |
        +-------------------------------+
        |                               |
        v                               v
compass-java                     compass-languages
  tool/runtime discovery           Tree-sitter Java evidence
  managed installation                    |
  classpath description                   |
  bounded process runner                  |
        |                                  |
        v                                  |
Compass JDT bridge                         |
  JDT Core AST + bindings                  |
        |                                  |
        v                                  |
compass-program <--------------------------+
  ProjectAnalyzer -> EvidenceBatch
  validation, merge, coverage, provenance
        |
        v
compass-resolve
  exact-anchor join and conservative dispatch
        |
        v
compass-graph -> graph.json + program.json + history
```

### Ownership

| Responsibility | Owner |
| --- | --- |
| Project commands, help, streams, JSON result mapping | `compass-cli` |
| Transactional setup/update/remove and build sequencing | `compass-core` |
| JDT installation, runtime discovery, project description, process invocation | New focused `compass-java` crate |
| Provider-neutral analyzer contract and evidence normalization | `compass-program` |
| Java AST facts and exact source occurrences | `compass-languages` |
| Cross-file identity, exact joins, hierarchy and dispatch resolution | `compass-resolve` |
| Graph contract and new normalized relationship kinds | `compass-model` and `compass-graph` |
| Tool/archive cache primitives and atomic publication | `compass-files` |
| JDT Core adapter | `integrations/compass-jdt-bridge` |
| Release asset construction, checksums, licenses, SBOM | release workflow and packaging scripts |

The CLI remains thin. Tool management is not implemented in the Java adapter,
and the JVM bridge does not know the Compass graph schema.

## Dependency and distribution model

### Rust workspace

JDT is not a Cargo dependency. The initial implementation should reuse the
workspace's existing bounded HTTP, SHA-256, archive, temporary-directory, and
subprocess dependencies where their contracts are sufficient. Shared managed
tool primitives should be extracted from the existing Compass upgrade path
rather than copied into the CLI.

The proposed `compass-java` crate depends on domain contracts, not CLI types.
It exposes injected download and process-runner traits so tests never contact
real services or execute real project builds.

### JVM bridge

The bridge is a separate, reproducibly built Java artifact with pinned direct
and transitive dependencies. Its primary dependency is
`org.eclipse.jdt:org.eclipse.jdt.core`. The bridge uses JDT DOM AST batch APIs,
resolved bindings, and an explicit classpath/sourcepath environment.

The bridge release contains:

```text
compass-jdt-bridge-<version>.zip
  compass-jdt-bridge.jar
  lib/*.jar
  manifest.json
  LICENSES/
  sbom.spdx.json
```

A shaded JAR is acceptable only if reproducible builds, duplicate resource
handling, service metadata, Eclipse license notices, and dependency audits are
verified. An explicit `lib/` directory is easier to inspect and update.

### JDT LS distribution

JDT LS is installed as its official product distribution plus a
Compass-owned adapter manifest. Compass does not reconstruct the OSGi product
from individual Maven dependencies. Each workspace receives a unique JDT LS
data directory; installed product files remain immutable and shared.

### Java runtime

Runtime selection follows this order:

1. explicit command option;
2. project configuration;
3. user configuration;
4. `JAVA_HOME`;
5. `java` on `PATH`;
6. explicitly installed Compass-managed runtime.

Compass invokes `<java-home>/bin/java` directly with separate arguments and
validates its vendor, architecture, major version, executable path, and
reported capabilities. It does not construct a shell command.

A managed runtime is an optional platform-specific asset installed only by an
explicit `compass analyzer setup java --managed-runtime` request. Compass
never removes or updates a user-managed runtime.

## Managed tool store

The store uses the existing Compass user-data location, with platform path
selection owned by one shared helper:

```text
<compass-data>/
  tools/
    java/
      jdt-core/<tool-version>/
        compass-jdt-bridge.jar
        lib/
        manifest.json
      jdt-ls/<tool-version>/
        product/
        manifest.json
      runtimes/<runtime-id>/
        runtime/
        manifest.json
  leases/
  staging/
```

Each installation manifest uses a versioned machine contract:

```json
{
  "schema": "compass.managed-tool/1",
  "tool": "jdt-core",
  "tool_version": "<pinned>",
  "bridge_protocol": "compass.java-analysis/1",
  "minimum_java_major": 21,
  "archive_sha256": "<digest>",
  "installed_files_sha256": "<canonical digest>",
  "source": "official-compass-release",
  "license": "EPL-2.0"
}
```

Unknown major schemas fail explicitly. Installations are immutable. Updating
creates and validates a new directory before changing project or user
selection. A failed update leaves the previously selected installation ready.

### Installation transaction

```text
resolve exact release metadata
  -> validate HTTPS endpoint and redirects
  -> download with byte and duration limits into unique staging
  -> verify expected asset name and SHA-256
  -> inspect archive paths, members, sizes, and expansion ratio
  -> extract only accepted regular files
  -> validate manifest, licenses, protocol, and synthetic invocation
  -> fsync and atomically publish immutable installation
  -> atomically update selection
```

Concurrent setup uses an installation lease keyed by tool and version. A
second process waits for a bounded duration and then validates the installation
created by the lease holder. It does not reuse partial staging state.

## Project and classpath model

### Project description

The Rust side produces a deterministic project description before invoking
JDT:

```text
repository-relative source roots
repository-relative generated-source roots when explicitly included
module boundaries and module dependencies
classpath entries with stable file identities
Java language/release level
source encodings
compiler options relevant to name and type resolution
excluded and unsupported inputs
```

The bridge does not crawl the repository or run a build tool. It consumes the
validated description supplied by Compass.

### Classpath modes

Classpath discovery is phased and explicit:

1. `explicit`: user-provided source roots and classpath entries.
2. `metadata`: existing `.classpath` or supported static project metadata.
3. `project`: deterministic, non-executing parsing of supported Maven and
   Gradle files plus already-present dependency artifacts.
4. `build-tool`: bounded invocation of a configured Maven or Gradle executable.

`build-tool` requires `allow_build_tool = true`. Network dependency resolution
requires the separate `allow_dependency_network = true`. These permissions are
visible in text and JSON status and enter the build profile.

Executing a build tool is high trust: repository build scripts and plugins may
run arbitrary code, access credentials, create files, spawn processes, or use
the network. The first implementation must not enable it merely to improve
classpath completeness.

### Classpath identity

The build-context digest includes, in canonical order:

- provider and bridge versions;
- Java runtime vendor, major version, architecture, and stable installation
  identity;
- language level and relevant compiler options;
- source-root and module descriptions;
- hashes of meaning-affecting Maven, Gradle, wrapper, lock, version-catalog,
  and IDE metadata files;
- normalized classpath order;
- bounded content digest for each classpath artifact;
- generated-source policy;
- build-tool and network permission policy.

Equivalent inputs produce the same digest independent of filesystem discovery
order. Missing or unreadable classpath entries produce typed coverage reasons,
not silently shortened identity.

## Provider protocol

### Transport

Batch analysis uses request and response files in a unique staging directory.
The command line contains only fixed flags and validated paths:

```text
java
  -Xms64m
  -Xmx<bounded heap>
  -Dfile.encoding=UTF-8
  -jar compass-jdt-bridge.jar
  analyze
  --request <request.json>
  --output <response.frames>
```

The response is a sequence of length-prefixed records so one oversized record
can be rejected without reading an unbounded document. Stdout is reserved for
the protocol handshake or remains empty; bounded stderr carries diagnostics.

### Request contract

Illustrative version-1 request:

```json
{
  "schema": "compass.java-analysis-request/1",
  "provider": "jdt-core",
  "repository_digest": "sha256:...",
  "build_context_digest": "sha256:...",
  "language_level": "21",
  "modules": [
    {
      "id": "app",
      "source_roots": ["src/main/java"],
      "sources": [
        {
          "path": "src/main/java/example/Service.java",
          "sha256": "sha256:...",
          "encoding": "UTF-8"
        }
      ],
      "classpath": [
        {
          "path": "<validated local path>",
          "identity": "sha256:..."
        }
      ]
    }
  ],
  "limits": {
    "max_facts": 5000000,
    "max_diagnostics": 10000,
    "max_response_bytes": 1073741824
  }
}
```

Absolute classpath paths are invocation inputs but never enter published graph
or Program artifacts. Source paths in evidence are normalized and
repository-relative.

### Response contract

The bridge emits provider facts, not graph nodes or edges. Fact families
include:

```text
definition             resolved declaration identity and exact name anchor
reference              exact target identity and semantic role
call_resolution        statically selected callable at an AST-proven call
receiver_type          attributed receiver expression type
override               method overrides exact superclass method
method_implementation  method implements exact interface declaration
type_hierarchy         direct superclass or interface identity
lambda_target          functional-interface method identity
method_reference       referenced method or constructor identity
diagnostic             bounded problem or incompleteness reason
```

Every fact carries:

- provider ID and version;
- repository-relative source path when source-backed;
- exact half-open byte range when source-backed;
- stable compiler symbol identity;
- exact, recovered, partial, ambiguous, or failed status;
- evidence IDs supporting derived facts;
- module and build-context identity.

JDT character offsets are converted to UTF-8 byte offsets by the bridge using
the exact source bytes represented by the request digest. The Rust validator
rechecks bounds and the anchored source digest.

### Protocol negotiation

Before analysis, Compass requests a bounded handshake containing:

```text
bridge version
supported request/response major versions
JDT Core version
minimum and active Java versions
supported language levels
supported fact families
hard implementation limits
```

No compatible major version is a setup or configuration error. Compass never
parses human-readable `--version` prose to determine protocol compatibility.

## Evidence and graph projection

### Program provider

The JDT runner implements `compass_program::ProjectAnalyzer`. Its descriptor
has `ProviderKind::Project`, repository scope, an input digest covering the
source inventory, and a configuration digest covering the complete build
context.

The normalized `EvidenceBatch` is validated before merging with syntax and
artifact providers. Conflicting exact targets remain multiple targets with
partial `provider_conflict` coverage. Recovered bindings remain partial and
cannot overwrite exact evidence.

### Exact AST join

JDT evidence may strengthen a Java graph call only when:

1. the Java adapter emitted a `Calls` or `Constructs` candidate;
2. the candidate occurrence has the same normalized source path and exact
   byte range as the compiler reference;
3. the compiler target maps to one exact local declaration anchor or one
   validated external classpath identity;
4. all exact providers at that site agree;
5. the target kind satisfies the structural candidate;
6. the provider coverage for the document and capability is usable.

Non-call references cannot become call edges. Compiler facts without matching
AST evidence remain in `program.json` with coverage and diagnostics but do not
invent graph relationships.

### External symbols

Classpath definitions may produce external nodes only from validated class or
JAR evidence. External identities include package, binary owner, member name,
descriptor, artifact identity, and provider provenance. They have no invented
source file or source range.

Source attachment is a separate evidence layer. An attached source JAR may add
anchors only after its artifact identity and source mapping are validated.

### Dispatch semantics

The statically selected declaration and possible runtime implementations are
different meanings:

```text
calls             compiler-selected declared target
overrides         exact method override relationship
implements_method exact interface method implementation
may_dispatch_to   conservative runtime candidate
```

The first release should project exact `calls`, `overrides`, and
`implements_method` facts. `may_dispatch_to` requires a separately designed
and qualified Class Hierarchy Analysis or Rapid Type Analysis pass. Compass
must not retarget a virtual call directly to the first, nearest, or only
currently indexed subclass.

Adding normalized relationship kinds is compatibility-sensitive and requires
model validation, output documentation, query behavior, history round trips,
and migration review.

## Resource and trust boundaries

### Downloads and archives

- Downloads occur only in setup or update commands that explicitly permit
  network access.
- Endpoints, redirects, content length, transferred bytes, duration, archive
  members, extracted bytes, expansion ratio, and paths are bounded.
- Expected SHA-256 digests come from pinned Compass release metadata, not from
  the downloaded archive itself.
- Installations include license notices and an SBOM.
- Unknown files, symbolic links, hard links, devices, absolute paths, parent
  traversal, duplicate paths, and case-folded collisions are rejected.

### Subprocesses

- Arguments are passed separately without a shell.
- Environment inheritance is allowlisted where practical; credentials are
  excluded.
- The working directory is explicit.
- Wall duration, stdout, stderr, response bytes, records, and diagnostics are
  bounded.
- Timeout and cancellation terminate the process tree and wait for cleanup.
- Response publication is staged and atomic.
- Exit success without a complete valid response is a provider failure.

The initial defaults are implementation constants with tests, not promises in
this design. Publicly configurable limits require configuration and command
reference entries.

### Repository-controlled execution

JDT Core parsing of supplied files does not authorize annotation processors,
Maven plugins, Gradle plugins, shell scripts, project launch configurations,
or arbitrary class loading. The bridge treats classpath entries as data and
does not load project classes into the bridge process.

Build-tool classpath discovery is a separate opt-in boundary. JDT LS project
import receives the same explicit trust treatment because importers may invoke
build tooling or resolve remote dependencies.

### Workspace isolation

Batch analysis uses a unique staging directory. JDT LS uses a unique data
directory per repository identity and configuration digest. The data directory
is never shared concurrently between repositories, worktrees, or incompatible
provider versions.

Repository paths and tool paths are canonicalized and containment-checked
before cleanup. Compass never recursively deletes a user project, a user JDK,
or a broad tool-store root.

## Caching and incrementality

### Cache key

The project-analysis cache key includes:

```text
provider descriptor
bridge and protocol versions
repository source inventory digest
build-context digest
module identity
limits that affect completeness
normalizer and merger versions
```

The cache stores normalized validated `EvidenceBatch` fragments, not JDT
object serialization. Unknown major versions and mismatched identities are
cache misses or explicit incompatibilities, never partial reuse.

### Incremental batch analysis

Phase one may analyze complete modules. Later increments may reuse unchanged
module evidence only when the module's sources, upstream ABI fingerprints,
classpath identity, and compiler options are unchanged. A source edit that
changes a public ABI invalidates dependent modules; a body-only edit may avoid
that invalidation only after the ABI fingerprint algorithm is versioned and
qualified.

### Watch mode

JDT LS keeps incremental state for an active worktree. Compass still validates
every returned fact and publishes complete artifact generations atomically.
Events arriving during a build are coalesced under the existing watch policy.
A crashed or stale server is restarted from validated configuration; stale
results cannot publish into a newer generation.

## History and reproducibility

Historical profiles record:

- selected Java semantic mode;
- tool, bridge, protocol, and runtime identities;
- project/classpath policy and digests;
- build-tool and network permissions;
- provider coverage and diagnostics;
- Program normalizer, merger, resolver, and graph versions.

Historical materialization remains offline. It may invoke an already installed
and compatible JDT bridge against the isolated checkout, but it may not install
tools, update tools, run dependency network resolution, or silently substitute
a different provider version. Missing tools or classpath artifacts make that
profile unrealizable and produce a typed error.

Published historical realizations remain immutable. Installing a newer JDT
version creates a new possible realization; it never rewrites an old one.

## Phased implementation

Each phase is independently reviewable. Later phases may not weaken an earlier
phase's failure or provenance contract.

### Phase 0: Contracts and test harness

**Direction**

- Finalize the batch request, response-frame, handshake, managed-tool
  manifest, CLI JSON result, diagnostic, and cache-key schemas.
- Extend `ProjectAnalyzer` only where JDT facts cannot be represented without
  losing meaning.
- Add fake downloader, archive, Java runtime, bridge, and process-runner
  implementations.
- Create minimal Java fixtures for overloads, generics, inheritance, method
  references, lambdas, recovered bindings, Unicode, and malformed sources.
- Capture the current Tree-sitter and SCIP graph as the no-regression baseline.

**Primary areas**

```text
compass-program
compass-ir
compass-files
test fixtures
docs/reference schemas after contracts become public
```

**Acceptance criteria**

- Every machine document carries a schema major version and rejects unknown
  majors.
- Canonical request, response, manifest, and cache encodings are deterministic.
- Duplicate, unordered, oversized, path-escaping, malformed, and conflicting
  records have negative tests.
- Fake project providers prove exact, partial, ambiguous, conflict, timeout,
  cancellation, and limit outcomes without Java or network access.
- Native structural and offline SCIP tests remain unchanged and green.
- No public command claims JDT availability.

### Phase 1: Managed tool and runtime lifecycle

**Direction**

- Create the focused `compass-java` provider crate against the shared
  `compass-analyzers` contracts.
- Extract safe shared download/archive/atomic-install primitives from the
  existing upgrade implementation.
- Implement runtime discovery and validation.
- Implement `analyzer setup java`, `status`, `doctor`, `update`, and `remove`
  over a mock or protocol-fixture bridge.
- Add project and user configuration plus immutable installation leases.

**Primary areas**

```text
compass-java
compass-files
compass-core
compass-cli
configuration and command references
release fixture assets
```

**Acceptance criteria**

- Normal `update`, `extract`, query, and history commands perform no new
  network request when Java is unconfigured.
- Setup requires explicit consent for downloads and reports all side effects.
- Setup is idempotent and safe under two concurrent processes.
- Checksum mismatch, redirect violation, archive bomb, path traversal,
  duplicate path, interruption, and unwritable destination leave no published
  installation or changed selection.
- Runtime precedence is deterministic across supported platforms.
- User JDKs and project files are never deleted by tool removal.
- Text and JSON CLI tests cover success, already ready, missing runtime,
  incompatible runtime, offline, corrupt installation, and partial cleanup.
- Tool lifecycle tests use local fixtures or mock servers only.

### Phase 2: JDT Core bridge and explicit-classpath analysis

**Direction**

- Build the reproducible JVM bridge with pinned JDT Core dependencies,
  licenses, and SBOM.
- Implement handshake plus batch analysis using an explicit sourcepath and
  classpath.
- Emit exact definitions, references, call resolutions, receiver types,
  hierarchy, overrides, implementations, lambdas, method references, and
  diagnostics.
- Convert JDT character offsets to source-verified UTF-8 byte anchors.
- Add the bounded Rust process runner and response validator.

**Primary areas**

```text
integrations/compass-jdt-bridge
compass-java
compass-program
release workflows
Java semantic fixtures
```

**Acceptance criteria**

- A fixture with overloaded generic methods resolves the same exact target on
  Linux, macOS, and Windows runners.
- Unicode, CRLF, nested classes, records, anonymous classes, lambdas, and
  method references have exact bounded anchors.
- Recovered bindings are marked partial and cannot appear as exact facts.
- JDT objects and project classes do not cross or load into the Rust process.
- Timeout, cancellation, heap failure, nonzero exit, truncated frames,
  excessive stderr, excessive facts, and response overflow produce distinct
  typed failures.
- Repeating equivalent input produces byte-identical normalized evidence.
- The bridge distribution is reproducible or its remaining nondeterminism is
  documented and excluded from the validated payload digest.

### Phase 3: Program merge and Java graph enrichment

**Direction**

- Implement the JDT `ProjectAnalyzer` adapter.
- Merge JDT and SCIP evidence through the provider-neutral Program boundary.
- Extend exact-anchor projection for local and validated external definitions.
- Add exact `overrides` and `implements_method` relationships if their graph
  contracts are approved in the same phase.
- Preserve the structural graph whenever evidence is partial, ambiguous,
  recovered, stale, unmatched, or conflicting.

**Primary areas**

```text
compass-program
compass-core
compass-resolve
compass-model
compass-graph
program and graph CLI tests
```

**Acceptance criteria**

- Exact JDT overload selection retargets only the matching AST call
  occurrence.
- Field reads, type references, imports, and annotation references cannot
  become calls.
- Recursive, constructor, static, instance, inherited, interface-default, and
  `super` calls retain correct direction and occurrence anchors.
- Conflicting JDT and SCIP targets remain explicit conflicts and do not select
  a preferred edge.
- External targets require classpath artifact evidence and contain no invented
  source location.
- `program.json`, `graph.json`, reports, query results, and history preserve
  provider and occurrence provenance.
- Warm structural builds without JDT retain their previous output and
  performance envelope.
- Code Graph v1 fixture qualification and applicable graph contract tests pass.

### Phase 4: Safe project and classpath discovery

**Direction**

- Add deterministic module/source-set discovery for supported Maven and Gradle
  layouts without executing project code.
- Index already-present JAR/class metadata for external identities and direct
  hierarchy.
- Add explicit build-tool runners behind separate execution and network
  permissions.
- Fingerprint build files, wrappers, dependency artifacts, source roots,
  language levels, and generated-source policy.

**Primary areas**

```text
compass-java
compass-core
compass-program
configuration
security documentation
Maven/Gradle fixtures and local fake repositories
```

**Acceptance criteria**

- Multi-module Maven and Gradle fixtures produce deterministic module and
  classpath descriptions without executing their builds.
- Test, main, generated, and excluded source sets cannot leak across configured
  boundaries.
- Missing dependencies produce partial classpath coverage rather than false
  local identities.
- Build-tool invocation never occurs without explicit permission.
- Dependency network access never occurs without its separate permission.
- Fake malicious plugins prove that static discovery does not execute project
  code.
- Build-tool processes have bounded duration/output and no shell construction.
- Equivalent dependency sets in different discovery orders produce the same
  build-context digest.
- Classpath changes invalidate only the documented cache scope.

### Phase 5: Seamless configured builds and history

**Direction**

- Make existing build commands consume the persisted Java configuration.
- Add validated module-level cache reuse and deterministic provider manifests.
- Integrate configured JDT profiles with immutable history materialization.
- Add actionable timing and coverage reporting without exposing absolute paths
  or credentials.

**Primary areas**

```text
compass-core
compass-history
compass-cli
compass-files
commands, configuration, outputs, and operations documentation
```

**Acceptance criteria**

- After `compass analyzer setup java`, `compass update` needs no Java-specific
  flags.
- An unchanged update reuses validated evidence and does not start the JVM.
- Source, ABI, build option, classpath, tool, runtime, and provider changes
  cause the documented invalidations.
- A configured but unavailable provider fails explicitly rather than silently
  changing realization.
- Historical materialization is offline and refuses missing pinned tools or
  classpath inputs.
- Reopening and querying a historical realization preserves exact provider
  identities and coverage.
- No credentials, repository-external absolute paths, or JDT workspace paths
  enter graph or history artifacts.

### Phase 6: JDT LS watch provider

**Direction**

- Add managed official JDT LS product installation.
- Implement bounded initialize/shutdown, workspace isolation, progress,
  cancellation, and restart behavior.
- Convert incremental semantic results into the same Program evidence
  contract; do not create a parallel graph path.
- Make build-tool import and dependency network behavior explicit and
  inspectable.

**Primary areas**

```text
compass-java
compass-core watch orchestration
compass-program
compass-cli
watch fixtures and local protocol doubles
```

**Acceptance criteria**

- Every repository/worktree/configuration uses an isolated data directory.
- Start, initialize, update, cancel, shutdown, crash, restart, and stale-result
  paths have bounded integration tests.
- A source edit publishes no result anchored to an older document version.
- Repeated edit/revert cycles return to the same canonical graph.
- JDT LS absence never prevents native watch mode when the project is not
  configured to require it.
- Project import cannot silently enable build-tool execution or dependency
  network access.
- Watch publication remains atomic and previous coherent artifacts survive a
  server failure.

### Phase 7: Hierarchy dispatch and qualification

**Direction**

- Design and implement exact override indexes plus bounded Class Hierarchy
  Analysis; consider Rapid Type Analysis only as a separately versioned
  refinement.
- Introduce `may_dispatch_to` only with a documented consumer and graph
  contract.
- Qualify structural-only, SCIP, JDT Core, and JDT LS profiles independently.
- Re-evaluate Java `Qualifying` status only after all claimed
  capabilities pass their gates.

**Primary areas**

```text
compass-resolve
compass-model
compass-graph
compass-query
real-repository qualification harness
performance and compatibility documentation
```

**Acceptance criteria**

- Static `calls` and possible runtime dispatch are never conflated.
- Abstract classes, interfaces, default methods, sealed hierarchies, bridge
  methods, covariant returns, visibility, and ambiguous implementations have
  positive and negative tests.
- Dispatch candidate traversal is deterministically bounded and reports limit
  failure distinctly from no candidates.
- Pinned Java corpus coverage improves over the current candidate baseline
  with no advertised relationship-family regression.
- All Compass samples remain deterministic and eligible under the native
  qualification harness.
- Cold and warm performance, peak RSS, provider startup, cache reuse, and
  Program/graph publication times are reported before any performance claim.
- Linux, macOS, and Windows release packages pass setup, doctor, analysis,
  removal, and offline reuse tests.
- Java is promoted only when the universal qualification policy is satisfied;
  tool availability alone is not promotion evidence.

## Cross-phase verification matrix

| Surface | Required evidence |
| --- | --- |
| Machine contracts | Round trip, unknown major, malformed, duplicate, oversized, canonical ordering |
| Tool installation | Local mock server, checksum, archive attacks, interruption, concurrency, atomic selection |
| Runtime discovery | Explicit/config/environment/PATH/managed precedence on supported platforms |
| Process boundary | Timeout, cancellation, tree kill, bounded streams, exit mapping, cleanup |
| Java semantics | Exact and negative overload, generics, hierarchy, lambda, method reference, recovery cases |
| Projection | Identity, direction, exact occurrence, multiplicity, provenance, ambiguity, conflicts |
| Incrementality | Cold, unchanged, source edit, ABI edit, classpath edit, tool/config edit, delete/rename |
| History | Offline materialization, reopen, immutable realization, missing provider, profile mismatch |
| CLI | Help, text/JSON, TTY/non-TTY, idempotence, no partial state, actionable diagnostics |
| Security | No implicit network/build execution, no secret/path disclosure, malicious repository fixtures |
| Qualification | Fixtures plus pinned real repository, deterministic digests, correctness before performance |

## Open questions

These decisions must be resolved before their corresponding phase starts:

1. Whether the managed bridge and JDT dependencies are published only as
   Compass release assets or also accepted from an explicit user installation.
2. Which minimum Java runtime major the pinned bridge supports across all
   release platforms.
3. Which Java-specific discovery aliases, if any, delegate to the shared
   `compass analyzer` lifecycle without creating a second implementation.
4. Which Maven and Gradle metadata can be interpreted without executing build
   logic and how unsupported dynamic configuration is represented.
5. Whether exact external classpath definitions become graph nodes in Phase 3
   or remain Program-only until class/JAR indexing qualifies in Phase 4.
6. Whether `overrides` and `implements_method` fit the current graph contract
   or require a versioned Code Graph extension.
7. What measured default heap, timeout, output, and fact limits are appropriate
   for small projects and the pinned Spring corpus.
8. Whether JDT LS exposes enough exact batch meaning through supported
   protocols or needs a Compass extension inside the server distribution.

Open questions do not authorize an implementation to guess. Each resolution
belongs in the public contract, an architecture decision record, or the phase
PR that establishes its tests.

## Completion definition

The managed JDT integration is complete only when:

- native Tree-sitter Java builds remain fully supported and unchanged when JDT
  is not configured;
- setup, inspection, daily analysis, update, and removal are available through
  the shared `compass analyzer` CLI;
- every download and process boundary is explicit, bounded, validated, and
  tested without real credentials or services;
- JDT evidence flows through the provider-neutral Program contract and exact
  Java evidence join;
- ambiguity, recovered bindings, missing classpaths, provider conflicts, and
  limits remain visible;
- cache and history identities include every meaning-affecting provider input;
- current, watch, and historical workflows publish coherent artifacts;
- cross-platform release, security, correctness, determinism, and performance
  qualification passes for every advertised provider mode; and
- reference, configuration, security, compatibility, migration, changelog,
  and operations documentation reflect the actually shipped phases.

## Related pages

- [Managed language analyzers](managed-language-analyzers.md)
- [Language architecture](language-architecture.md)
- [System architecture](architecture.md)
- [Security and privacy](security-and-privacy.md)
- [Storage and history](storage-and-history.md)
- [Universal semantic evidence](../reference/universal-semantic-evidence.md)
- [Extending Compass](../implementation/extending-compass.md)
- [Compatibility](../../COMPATIBILITY.md)

**Next step:** complete Phase 0 as a contract-only change before adding tool
downloads, JVM dependencies, or public availability claims.
