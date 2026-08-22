# Plan 020: Hard-cut Swift, Dart, Scala, and Groovy to universal evidence

> **Executor instructions**: Deliver this program as ten independently
> reviewable phases. Each phase below repeats its entry context, scope, and
> acceptance criteria so it can be handed to an executor with no conversation
> context. Before changing source, read `AGENTS.md`,
> `docs/design/language-architecture.md`,
> `docs/implementation/universal-evidence.md`,
> `docs/implementation/evidence-resolution-framework-technical-design.md`,
> and `docs/reference/universal-semantic-evidence.md`. Never dual-publish a
> candidate in production: candidate emitters remain test/qualification-only,
> and each language switches its registry, extractor, resolver, cache
> requirements, and framework integration in one atomic hard-cut phase.
>
> **Drift check (run before every phase)**:
> `git diff --stat 88abe4c0..HEAD -- crates/compass-files crates/compass-languages crates/compass-resolve crates/compass-model crates/compass-graph crates/compass-core vendor/compass-tree-sitter-language-pack fixtures/code-graph tests/qualification scripts benchmarks/performance docs COMPATIBILITY.md MIGRATION.md CHANGELOG.md advisor-plans`
> Reconcile changed producer versions, evidence fields, framework pack IDs,
> cache contracts, qualification thresholds, and language-specific paths before
> implementation. If current code contradicts the inventory in this plan, stop
> and update the plan rather than preserving an obsolete path.

## Status

- **Execution status (2026-08-22)**: DONE. The production hard cuts, universal
  registry entries, framework boundaries, deterministic fixture baselines,
  pinned read-only manifests, parser-backed source providers, and fail-closed
  audit harnesses are implemented. SwiftSyntax 603.0.0, Dart Analyzer 8.4.0,
  scala.meta 4.13.10, and Groovy 4.0.27 providers are provisioned outside the
  checkout under the mounted qualification target. The Swift, Dart, Scala,
  and Groovy three-corpus graph audits all pass precision, recall, coverage,
  and diversity gates. Registry state remains version-1 `Qualifying`; a later
  promotion decision is intentionally separate from this delivery.
- **Priority**: P1
- **Effort**: XXL, delivered as ten phases and at least ten PRs
- **Risk**: HIGH
- **Depends on**: no code prerequisite; release automation should consume plan
  005 or an equivalent exact-commit qualification gate
- **Category**: language architecture, correctness, resolution, frameworks,
  performance, tests, documentation
- **Planned at**: commit `88abe4c0`, 2026-08-21

## Why this matters

Compass already recognizes, parses, and publishes some facts for Swift, Dart,
Scala, and Groovy. That is useful established support, but it is not the same
support contract as Python, Go, Rust, Java, Kotlin, Ruby, TypeScript,
JavaScript, PHP, and C#: the four requested languages are absent from
`UniversalEvidenceRegistry`, lack independent quality audits, and still depend
on direct extractors or generic resolver behavior.

This program makes all four languages first-class hard-cut universal evidence
pipelines. “Supported” for this program means one production route per
language, exact and bounded source evidence, conservative project-wide
resolution, deterministic cold/warm/incremental publication, and the full
quality-audit thresholds in `docs/reference/universal-semantic-evidence.md`.
The initial production state is version 1 `Qualifying`, matching Compass's
current language-transition policy. Promotion to `Qualified` is a separate
product decision and must not be inferred merely from registration or a green
fixture suite.

## Extension contract

Files are classified by `compass-files` and the static language registry; the
vendored package supplies pinned parsers only; `compass-languages` emits one
validated, bounded `SemanticEvidenceBatch` per source; `compass-resolve`
performs exact-language, project-aware, fail-closed resolution; framework packs
consume exact normalized evidence; and `compass-graph`/`compass-core` publish a
coherent deterministic graph. Inputs are untrusted source and bounded project
manifests. Outputs preserve declaration identity, direction, multiplicity,
source anchors, ambiguity, diagnostics, and producer provenance. A missing
grammar or invalid evidence is fatal; parser recovery or a spent budget is
explicitly incomplete; competing targets remain unresolved.

## Current state at the planned commit

| Surface | Swift | Dart | Scala | Groovy |
| --- | --- | --- | --- | --- |
| Discovery | `.swift` recognized | `.dart` recognized | `.scala` recognized | `.groovy` and `.gradle` recognized |
| Grammar | pinned static grammar | pinned static grammar, but production extractor does not use it | pinned static grammar | pinned static grammar, but production extractor does not use it |
| Extraction | dedicated AST walker in `src/swift.rs` | source/regex walker in `src/dart.rs` | branches inside generic `engine.rs` | line/regex walker plus Spock branch in `src/groovy.rs` |
| Calls | local edges plus `RawCall`; typed member table | framework-specific edges only; `raw_calls: None` | generic `RawCall` behavior | regex member calls plus `RawCall` |
| Project facts | bounded `Package.swift` dependency scan | no `pubspec.yaml` project evidence | no `build.sbt` project evidence | Gradle dependency scan only |
| Resolution | legacy Swift type table and compatibility pass | no Dart-specific resolver | JVM-family stub rewiring and generic resolution | JVM-family stub rewiring and generic resolution |
| Frameworks | established `vapor-routes` source pack | Flutter/BLoC/Riverpod/navigation rules are embedded in the direct extractor | `play-routes-config` can target Scala handlers | Spock recognition is embedded in the direct extractor |
| Universal registry | absent | absent | absent | absent |
| Qualification corpus | one matrix file plus focused Swift/Vapor and Dart range tests | one matrix file plus focused Dart range tests | one matrix file, type-shape smoke case, and Play handler | one matrix file; no dedicated conformance suite |

Important current boundaries:

- `crates/compass-languages/src/evidence_pipeline.rs` registers C#, Go, Java,
  JavaScript, Kotlin, PHP, Python, Ruby, Rust, and TypeScript only.
- `crates/compass-resolve/src/evidence/languages/policy.rs` has no Swift, Dart,
  Scala, or Groovy policy.
- `crates/compass-resolve/src/members.rs` contains `swift_type_table` handling
  and an observable cross-language Swift compatibility pass. The hard cut must
  remove that behavior; it must not migrate it into universal resolution.
- `crates/compass-resolve/src/lib.rs` groups Scala and Groovy with JVM sources
  for direct stub rewiring. Shared JVM membership is not evidence for a
  cross-language call target.
- `tests/qualification/code-graph-v1-corpus.json` proves recognition for all
  four languages, not semantic completeness.
- The four grammars already exist in
  `vendor/compass-tree-sitter-language-pack/language_definitions.json` and its
  static build set. Parser availability must be reverified, not reimplemented.

## Required semantic decisions

Freeze these decisions in tests and documentation before a hard cut.

### Swift v1

- Canonical identity is module-qualified, not file-stem-qualified. Nested
  types retain lexical ownership. Overload identity includes the Swift base
  name and argument-label sequence; `init`, subscripts, and operators do not
  collapse by terminal spelling.
- `class`, `struct`, `enum`, `actor`, and `protocol` are distinct declaration
  kinds. Protocol conformance is not class inheritance. Extensions attach only
  after exact module/type resolution; competing same-named extension targets
  remain unresolved.
- Imports bind modules. `typealias` is an alias. Attributes and property
  wrappers remain anchored evidence but advertise `Decorators` only if the
  producer emits and audits a truthful relationship family.
- Optional chaining, trailing closures, async/await, generic specialization,
  and call-result chains may emit occurrences; they resolve only from exact
  owner, type, or result evidence.
- Objective-C/C/C++ interoperability never uses native-family terminal-name
  matching. Cross-language endpoints require exact fresh compiler/SCIP facts.

### Dart v1

- Canonical identity uses the Dart library URI and lexical owner. Relative
  libraries, `package:` URIs, `part`, and `part of` must converge on one
  contained library identity without consulting files outside the project.
- Classes, mixins, extensions, extension types, enums, typedefs, top-level
  functions, methods, getters, setters, operators, unnamed constructors,
  named constructors, and factory constructors remain distinct where Dart
  dispatch distinguishes them.
- Imports, deferred imports, prefixes, `show`/`hide`, exports, and aliases are
  explicit bounded bindings. No first matching import wins.
- Named arguments, cascades, null-aware access, generic invocation, and
  extension methods preserve exact occurrences. A `dynamic` receiver or an
  ambiguous extension remains unresolved.
- Flutter, BLoC, Riverpod, and navigation meaning moves out of the language
  extractor into statically registered evidence-backed framework packs.

### Scala v1

- One semantic language identity, `scala`, covers Scala 2 and Scala 3. A
  dialect field is emitted only when bounded project/toolchain evidence proves
  it; syntax guessing must not change declaration identity.
- Packages, package objects, classes, traits, objects, companion pairs, enums,
  case classes/objects, methods, values, variables, type aliases, givens, and
  extension declarations receive distinct source identities. A class and its
  companion object are linked but never collapsed.
- Import selectors, aliases, exclusions, wildcards, Scala 3 exports, overloads,
  multiple parameter lists, named/default arguments, and inheritance/mixins
  are represented explicitly and resolved within bounded candidate sets.
- Implicit conversions, implicit search, contextual `given` selection,
  compiler-synthesized members, macros, and quoted code remain unresolved in
  the structural v1 tier unless exact fresh compiler evidence names both ends.
- Java, Kotlin, and Groovy declarations are never selected from shared JVM
  packages or terminal spelling. Cross-language calls require exact anchored
  compiler/SCIP endpoints.

### Groovy v1

- Canonical identity uses package plus lexical owner. Scripts have a
  source-scoped script owner; their top-level declarations do not become one
  repository-global namespace.
- Classes, interfaces, traits, enums, records, annotations, methods,
  constructors, fields/properties, closures, aliases, and quoted Spock feature
  methods retain exact ranges and distinct identities.
- Static imports, aliases, safe navigation, spread access, closures, named
  arguments, and constructor calls may produce evidence. Runtime metaclass
  mutation, `methodMissing`, `propertyMissing`, dynamic GStrings, categories,
  and nonliteral DSL dispatch never create convenient exact targets.
- `@CompileStatic` or `@TypeChecked` may strengthen source-proven local type
  evidence; it does not authorize using a compiler result that is absent or
  stale.
- Gradle DSL calls remain qualified external/unresolved unless a separately
  versioned framework pack proves their meaning. Spock feature methods may be
  test declarations, but `Tests` is advertised only after relationships to the
  subject under test are independently proven and audited.
- Java/Kotlin/Scala interop follows the same exact-endpoint rule as Scala.

## Qualification truth and corpora

Phase 0 must create immutable manifests with full commit SHAs and inventory
digests for these proposed three-corpus sets. The executor may replace a corpus
only if it is unavailable, unbuildable by its documented pinned toolchain, or
fails the diversity rule; record the replacement and reason in the plan and
qualification documentation before continuing.

| Language | Proposed corpora | Independent qualification-only oracle |
| --- | --- | --- |
| Swift | `apple/swift-nio`, `vapor/vapor`, `apple/swift-collections` | pinned SwiftSyntax parser; optional SourceKit/IndexStore compiler endpoints stay a separate provider profile |
| Dart | `dart-lang/sdk` library subset, `flutter/flutter` packages subset, `rrousselGit/riverpod` | pinned Dart SDK `analyzer` AST helper |
| Scala | `scala/scala3`, `akka/akka`, `playframework/playframework` | pinned scala.meta source parser for constructs plus separately identified SemanticDB compiler endpoints |
| Groovy | `apache/groovy`, `gradle/gradle`, `spockframework/spock` | pinned Groovy compiler `CompilationUnit` AST helper that never executes repository build scripts |

All external repositories live under
`/Volumes/Workspace/Github/<owner>/<repository>`, are treated as read-only,
and are never reset, cleaned, updated, or executed by qualification. Oracle
helpers may parse source and checked-in manifests only. They must record
provider/toolchain versions, complete file inventories, partial files, exact
UTF-8 ranges, canonical inventory digests, and deterministic output. No oracle
may reuse Tree-sitter, Compass graph output, Graphify, or another language's
terminal-name index as truth.

## Commands executors will need

Every compiling Cargo command must use a unique mounted target directory for
the implementation checkout or worktree. Replace `<phase>` with the phase
number and never fall back to a local `target/` directory.

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Target preflight | `test -d /Volumes/Workspace && mkdir -p /Volumes/Workspace/crabbuild-target/compass-language-wave-<phase> && test -w /Volumes/Workspace/crabbuild-target/compass-language-wave-<phase>` | exit 0 |
| Language crate | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-language-wave-<phase> cargo test -p compass-languages --locked` | exit 0 |
| Resolver crate | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-language-wave-<phase> cargo test -p compass-resolve --locked` | exit 0 |
| Core incremental contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-language-wave-<phase> cargo test -p compass-core --test code_graph_v1_determinism --locked` | exit 0 |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0; no Graphify/runtime boundary violations |
| Fixture qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-language-wave-<phase> ./scripts/qualify_code_graph_v1.sh --fixtures-only` | all manifest, determinism, incremental, and graph assertions pass |
| Format | `cargo fmt --all -- --check` | exit 0, no diff |
| Baseline Clippy | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-language-wave-<phase> cargo clippy --workspace --lib --bins --locked -- -D warnings` | exit 0 |
| Baseline tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-language-wave-<phase> cargo test --workspace --lib --bins --locked` | exit 0 |

Each language adds `scripts/qualify_<language>_universal.py` with `fixture`,
`pinned`, `quality-audit`, and `performance` modes, modeled on
`scripts/qualify_ruby_universal.py`. Each mode must use caller-provided clean
checkouts and temporary outputs; it must not clone, mutate, compile, or execute
qualification repository code.

## Scope

**In scope across the program**:

- `crates/compass-files/src/detect.rs` and tests only if recognition rules need
  a documented correction;
- `crates/compass-languages/src/{registry.rs,evidence_pipeline.rs,engine.rs}`;
- new `crates/compass-languages/src/evidence/{swift,dart,scala,groovy}.rs`;
- deletion or narrowing of the replaced direct files
  `src/{swift,dart,groovy}.rs` and Scala-only generic-engine branches;
- bounded project evidence for `Package.swift`, `pubspec.yaml`, `build.sbt`,
  and Gradle/Groovy project inputs;
- `crates/compass-languages/src/frameworks/` for Vapor and Dart framework
  migration;
- `crates/compass-resolve/src/evidence/` and focused closed language policies;
- removal of replaced Swift member-table and Scala/Groovy JVM stub behavior;
- framework expansion/targeting required by the migrated packs;
- focused fixtures, conformance/resolver tests, qualification harnesses,
  manifests, performance evidence, compatibility notes, changelog, and current
  support documentation.

**Out of scope**:

- runtime parser downloads, dynamic grammar loading, Graphify dependencies, or
  executing untrusted project builds;
- compiler-grade completeness, IDE/LSP transport work, or mandatory Swift,
  Dart, Scala, Groovy, Gradle, sbt, Flutter, or Xcode installations at normal
  Compass runtime;
- speculative cross-language calls based on JVM/native family, package,
  import, or terminal-name similarity;
- changing `compass.graph/1`, the universal evidence schema, public lookup
  budgets, or `LanguageCapability` solely to accommodate one language;
- broad framework expansion beyond preserving existing Vapor, Dart heuristic,
  Play, Spock, and Gradle behavior;
- vendored grammar changes. If a pinned grammar cannot represent a required
  stable construct, stop and propose a separate attributed vendor update with
  parser conformance evidence.

## Git and delivery workflow

- Branches: `advisor/020-p<phase>-<language-or-baseline>`.
- Use conventional commits matching current history, for example
  `feat(languages): add Swift universal evidence candidate`.
- One phase per PR. Keep candidate and production hard-cut changes separate.
- Do not push, merge, or open a PR unless the operator explicitly requests it.
- Every PR description records compatibility effect, exact checks, pinned
  corpus/toolchain identities, performance comparison, and checks not run.

## Phase map

| Phase | Deliverable | Depends on | Production behavior changes? |
| --- | --- | --- | --- |
| 0 | Freeze contracts, direct baselines, corpora, and independent oracles | none | No |
| 1 | Build a qualification-only Swift v1 candidate | 0 | No |
| 2 | Qualify, migrate Vapor, and atomically hard-cut Swift | 1 | Yes, Swift only |
| 3 | Build a qualification-only Dart v1 candidate | 0 | No |
| 4 | Qualify, migrate Dart frameworks, and atomically hard-cut Dart | 3 | Yes, Dart only |
| 5 | Build a qualification-only Scala v1 candidate | 0 | No |
| 6 | Qualify Play targeting and atomically hard-cut Scala | 5 | Yes, Scala only |
| 7 | Build a qualification-only Groovy v1 candidate | 0; reuse JVM decisions from 5 where landed | No |
| 8 | Qualify Gradle/Spock behavior and atomically hard-cut Groovy | 7 | Yes, Groovy only |
| 9 | Run mixed-language release qualification and close public support docs | 2, 4, 6, 8 | Documentation/status only unless a gate exposes a defect |

Phases 1, 3, and 5 can run in parallel after Phase 0. Phase 7 may also run in
parallel, but it must adopt—not duplicate—the exact-language JVM boundary from
Phase 5 if that boundary has landed.

## Phase 0: Freeze contracts, baselines, corpora, and independent truth

### Phase context

This phase changes no production extraction. At commit `88abe4c0`, all four
languages use established direct paths and none is registered in
`UniversalEvidenceRegistry`. The purpose is to prevent implementation from
silently redefining existing behavior or measuring itself as its own oracle.

### Phase scope and work

- Create one versioned repository manifest per language under
  `tests/qualification/`, using full immutable SHAs, purpose, dialect/profile,
  required source globs, exclusion policy, and clean-checkout requirements.
- Create qualification-only source-oracle helpers and canonical output schemas.
  Pin toolchain/package versions and verify byte-deterministic repeated output.
- Add established-path baselines covering relation counts, graph/evidence
  digests, diagnostics, omitted facts, identity collisions, cold/warm/neutral
  edit/semantic edit/restore timing, and peak RSS.
- Add focused fixtures for every semantic decision listed above, including
  positive, negative, ambiguity, malformed, UTF-8, multiline, repeated-site,
  and limit cases. Baseline corrections are recorded as intended changes; they
  are not forced into parity.
- Define per-language target capability lists. A capability is excluded unless
  the source oracle can inventory it and the audit can satisfy at least 100
  accepted records for that capability identity.
- Record performance gates in each baseline: cold median no worse than the
  larger of 110% of established or established plus one second; warm and
  fact-neutral median no worse than the larger of 115% or established plus
  100 ms/250 ms respectively; peak RSS no worse than the larger of 115% or
  established plus 32 MiB. A stricter existing repository gate wins.

### Phase acceptance criteria

The phase checklists below are historical gates: a criterion describing the
pre-cutover registry records the state verified before the following atomic
hard cut. The final registry state is recorded in Phase 9 and the program done
criteria.

- [x] Four manifests validate, use full SHAs, and point only to clean mounted
  read-only checkouts.
- [x] Each oracle output is byte-identical across two runs and includes exact
  toolchain/provider identity, complete inventory digest, partial-file count,
  and exact source ranges.
- [x] Each established baseline is reproduced twice with identical graph
  bytes on cold rebuild and warm cache reuse; edit/restore returns to the exact
  baseline hash.
- [x] The candidate capability list has positive, negative, ambiguity, limit,
  and independent-oracle strata for every advertised capability.
- [x] No production registry, extractor dispatch, resolver, or framework pack
  changes appear in the diff.
- [x] `git diff --check` and all new harness unit tests pass.

## Phase 1: Build a qualification-only Swift v1 candidate

### Phase context

Swift currently uses `crates/compass-languages/src/swift.rs`, file-stem IDs,
`RawCall`, `swift_type_table`, and legacy member resolution. The existing
Vapor source pack remains production-active. This phase adds a parallel
candidate API callable only by tests and qualification; it must not register
`compass.swift` or alter production graphs.

### Phase scope and work

- Add `evidence/swift.rs` as a bounded AST consumer using the already prepared
  Swift Tree-sitter root. Emit module-qualified declarations/scopes/bindings,
  protocol and extension evidence, exact imports/typealiases, calls,
  construction, type references, ownership, receiver evidence, and truthful
  diagnostics.
- Encode overload identity with argument labels and distinguish type kinds,
  constructors, deinit, subscript, operators, nested declarations, enum cases,
  properties, local bindings, actors, and extensions.
- Emit complete direct-base evidence only when parser recovery does not overlap
  the declaration. Preserve ambiguous class/protocol classification rather
  than inheriting the direct extractor's “first base” heuristic.
- Add `swift_universal_conformance.rs` and a qualification-only resolver module
  covering module imports, nested scopes, overloads, extensions, protocol
  dispatch, optional/result chains, ambiguity, builtins, UTF-8, malformed
  source, repeated calls, and evidence limits.
- Compare the candidate against Phase 0 baselines and the SwiftSyntax oracle.
  Record intentional ID/relation corrections separately from regressions.

### Phase acceptance criteria

- [x] `UniversalEvidenceRegistry::pipeline("swift")` remains `None` and normal
  `Engine::extract` output is byte-identical to the Phase 0 Swift baseline.
- [x] The candidate batch validates, contains no direct graph nodes/edges or
  `RawCall`, is deterministic, and advertises only audited capabilities.
- [x] Every emitted occurrence slices the original UTF-8 bytes exactly;
  repeated same-line occurrences remain distinct.
- [x] Duplicate module/type/method names, competing extensions, unknown
  receivers, and native-family near matches remain unresolved.
- [x] `cargo test -p compass-languages --test swift_universal_conformance
  --locked` and `cargo test -p compass-resolve --test universal_resolution
  --locked swift` pass with the required external target directory.

## Phase 2: Qualify, migrate Vapor, and atomically hard-cut Swift

### Phase context

Phase 1 produced a non-production Swift evidence candidate. Production still
uses the direct Swift publisher, `swift_type_table`, legacy member resolution,
and the established `vapor-routes` source pack. This phase may begin only when
the candidate passes fixture conformance and a reproducible pinned-corpus
comparison; this phase owns the blocking quality audit and performance gate.

### Phase scope and work

- Complete SwiftPM project evidence: package/module name, bounded target source
  roots, test targets, dependencies, and contained module imports. Do not
  evaluate `Package.swift` or invoke SwiftPM.
- Prefer generic resolver stages; add `LanguagePolicyKind::Swift` only for
  extension/protocol/overload rules that cannot be expressed generically.
- Convert Vapor to one `vapor-swift` universal descriptor and matching resolver
  adapter keyed by the same pack ID. Consume exact call/import/ownership facts;
  preserve grouped literal routes and explicit handlers; keep opaque closures
  unresolved.
- Run the independent three-corpus audit. Meet every threshold in the reference
  contract: 2,000 accepted records, 400 per corpus, 100 per relation and
  advertised capability, cluster diversity, precision/Wilson/recall gates, and
  zero critical violations.
- Atomically register `compass.swift` version 1 as `Qualifying`, switch Engine
  publication to universal evidence, require valid cached evidence, remove the
  direct publisher/`swift_type_table`/legacy compatibility resolver, replace
  `vapor-routes`, and update cache fingerprints without changing unrelated
  language versions.
- Update Swift qualification, framework-route, compatibility, changelog, and
  language-status documentation.

### Phase acceptance criteria

- [x] No production Swift file publishes through `src/swift.rs`, `RawCall`,
  `swift_type_table`, or `resolve_swift_registry_compatibility`; searches find
  no active references to the replaced route.
- [x] `UniversalEvidenceRegistry::pipeline("swift")` returns
  `compass.swift`, version 1, `Qualifying`, with the exact audited capability
  list.
- [x] Cold, warm, forced rebuild, alternate checkout, fact-neutral edit,
  semantic edit, delete, rename, and restore cases are deterministic and meet
  Phase 0 performance/RSS gates.
- [x] The Swift quality-audit report meets all numerical and zero-tolerance
  gates and binds the exact source-oracle inventory and graph digest.
- [x] Vapor activation/negative/ambiguity/handler/range/limit tests pass and
  descriptor/expansion registry sets match exactly.
- [x] Swift conformance, resolver, framework, crate, core determinism, full
  fixture qualification, product boundary, baseline Clippy, and baseline tests
  all pass.

## Phase 3: Build a qualification-only Dart v1 candidate

### Phase context

Dart currently bypasses its linked grammar and uses `src/dart.rs`, a regex
extractor with `raw_calls: None`. Flutter, BLoC, Riverpod, and navigation
relations are mixed into language extraction. There is no `pubspec.yaml`
project evidence. This phase adds a test-only AST candidate and leaves normal
Dart output unchanged.

### Phase scope and work

- Add `evidence/dart.rs` using the prepared Dart Tree-sitter AST. Emit library-
  qualified declarations, lexical scopes, parts, imports/exports/prefixes,
  `show`/`hide`, typedefs, classes/mixins/extensions/extension types, members,
  functions, constructors, annotations, calls, type references, receivers, and
  exact diagnostics.
- Model named/factory constructors, getters, setters, operators, named/default
  arguments, cascades, null-aware access, generics, and local/result types
  without resolving `dynamic` or ambiguous extension dispatch.
- Add a bounded `pubspec.yaml` parser to qualification-only project context for
  package name, dependencies, SDK constraints, and project-contained roots.
  Never run `pub`, Flutter, builders, or generated code.
- Split current Dart framework facts into candidate evidence-backed adapters;
  they remain unregistered until Phase 4.
- Add Dart conformance and resolver tests plus differential reports against the
  Dart analyzer oracle and Phase 0 direct baseline.

### Phase acceptance criteria

- [x] `UniversalEvidenceRegistry::pipeline("dart")` remains `None`; normal Dart
  extraction and current framework output match the Phase 0 baseline.
- [x] Candidate output validates, is byte deterministic, has exact ranges, and
  contains no direct graph records or `RawCall`.
- [x] `part`/`part of`, package/relative imports, prefixes, filters, exports,
  duplicate names, named constructors, and ambiguous extensions have positive
  and fail-closed tests.
- [x] `dynamic`, malformed syntax, nonliteral navigation, and out-of-project
  package paths never create invented local targets.
- [x] Dart conformance and `universal_resolution dart` targeted tests pass.

## Phase 4: Qualify, migrate Dart frameworks, and atomically hard-cut Dart

### Phase context

Phase 3 supplied a qualification-only Dart AST candidate and candidate project
context. Production still uses the regex direct extractor and embeds framework
rules. This phase performs the sole production switch after independent audit
and performance gates pass.

### Phase scope and work

- Promote bounded `pubspec.yaml` project facts into normal project evidence,
  fingerprinting package name, contained roots, dependencies, and diagnostics.
- Add Dart-specific resolver policy only for language rules such as extension
  selection or named constructors that generic stages cannot express.
- Register focused universal packs for the established behavior: use stable
  separate IDs for Flutter navigation, BLoC, and Riverpod when their activation
  and accepted evidence differ. Exact pack IDs and dependencies must be frozen
  in tests; one catch-all detector is not acceptable.
- Complete the three-corpus source-oracle audit and Phase 0 performance gates.
- Atomically register `compass.dart` version 1 `Qualifying`, use universal
  publication, reject stale cache entries lacking current evidence, delete the
  regex direct path, and remove embedded framework branches.
- Update Dart qualification, compatibility, changelog, framework, and status
  documentation.

### Phase acceptance criteria

- [x] No production Dart path calls `src/dart.rs` or emits framework relations
  from the language producer.
- [x] The production registry exposes only `compass.dart` version 1
  `Qualifying`; project/package changes invalidate only relevant extraction.
- [x] The full audit satisfies all record-count, diversity, precision, Wilson,
  recall, and zero-critical-violation gates.
- [x] Framework packs require positive manifest/source activation, reject wrong
  frameworks, preserve exact anchors and repeated occurrences, and remain
  bounded/deterministic.
- [x] Cold/warm/rebuild/incremental/delete/rename/restore graphs and evidence are
  deterministic and within Phase 0 performance/RSS limits.
- [x] Dart targeted suites, full language/resolver crates, core determinism,
  fixture qualification, product boundary, Clippy, and baseline tests pass.

## Phase 5: Build a qualification-only Scala v1 candidate

### Phase context

Scala currently uses generic Tree-sitter extraction with Scala-only helper
branches in `engine.rs`; collection resolution may rewire Scala stubs through
the broad JVM family. Play config routes can target Scala handlers. This phase
adds test-only evidence for Scala 2 and Scala 3 and does not register it.

### Phase scope and work

- Add `evidence/scala.rs` over the prepared tree. Emit packages/scopes,
  classes/traits/objects/companions/enums/case declarations, methods,
  constructors, vals/vars, type aliases, imports/selectors/aliases/exclusions,
  Scala 3 exports, annotations, calls, construction, ownership, receivers,
  type parameters/bounds, and inheritance/mixins.
- Preserve overload signatures, multiple parameter lists, named/default
  arguments, by-name/varargs types, nested packages, and exact companion
  identities. Emit givens and extension declarations as declarations, but do
  not claim implicit search or compiler-synthesized dispatch.
- Add bounded qualification-only `build.sbt`/project metadata sufficient to
  identify source roots, Scala version/dialect when explicit, and dependency
  coordinates without evaluating sbt.
- Establish one explicit cross-JVM invariant test matrix: equal package/name
  Java, Kotlin, Groovy, and Scala declarations never cross-resolve without
  exact fresh compiler evidence.
- Add conformance/resolver tests and differential reports against scala.meta,
  SemanticDB where available, and the established baseline.

### Phase acceptance criteria

- [x] The production registry still returns no Scala pipeline and normal Scala
  graphs remain at the Phase 0 baseline.
- [x] Candidate batches validate, sort deterministically, preserve exact Scala
  2/3 anchors, and advertise no implicit/macro/compiler-only capability.
- [x] Companions, overloads, imports, exclusions, wildcards, exports, givens,
  extensions, inheritance, ambiguity, malformed syntax, and limits have direct
  tests.
- [x] Cross-JVM collision tests publish no guessed calls or type edges.
- [x] Scala conformance and `universal_resolution scala` tests pass.

## Phase 6: Qualify Play targeting and atomically hard-cut Scala

### Phase context

Phase 5 produced a non-production Scala candidate and exact-language JVM
tests. Production still uses generic Scala branches and JVM-family stub
rewiring. The existing `play-routes-config` pack is a configuration producer,
not a reason to infer Scala/Java target identity.

### Phase scope and work

- Promote bounded sbt/Scala project evidence needed for module/package/source-
  root resolution. Keep build evaluation and compiler invocation optional and
  outside normal runtime.
- Prefer generic resolution. Add `LanguagePolicyKind::Scala` only for
  companion, extension, or overload rules backed by explicit source facts.
- Update Play target resolution so an exact Scala handler resolves from Scala
  package/owner/signature evidence. Java or injected handlers remain separate;
  equal terminal spelling is explicit ambiguity.
- Complete the three-corpus quality audit and performance gates.
- Atomically register `compass.scala` version 1 `Qualifying`, remove Scala-only
  generic-engine helpers and Scala participation in broad JVM stub rewiring,
  enforce current evidence on cache reuse, and update docs/contracts.

### Phase acceptance criteria

- [x] Searches show no active Scala-specific branches in the direct generic
  walker and no JVM-family terminal-name fallback for Scala.
- [x] The registry returns `compass.scala` version 1 `Qualifying` with the exact
  audited capability list.
- [x] Scala 2 and Scala 3 each appear in fixture and pinned-corpus evidence; an
  unknown dialect does not fabricate a dialect or change identity.
- [x] The full independent audit and Phase 0 performance/RSS gates pass.
- [x] Play route tests cover exact Scala, exact Java, injected, duplicate,
  ambiguous, malformed, and unresolved handlers with correct direction and
  anchors.
- [x] Scala targeted/full suites and all repository gates listed in Phase 2
  pass.

## Phase 7: Build a qualification-only Groovy v1 candidate

### Phase context

Groovy currently bypasses its grammar and parses lines with regular
expressions. It has a separate Spock feature branch, raw member calls, limited
class/method identity, and broad JVM stub compatibility. `.gradle` is treated
as Groovy source. This phase replaces none of that in production; it builds a
test-only AST candidate.

### Phase scope and work

- Add `evidence/groovy.rs` over the prepared Groovy tree. Emit packages,
  scripts, classes/interfaces/traits/enums/records/annotations, methods,
  constructors, fields/properties, closures, imports/static imports/aliases,
  annotations, calls, construction, ownership, type references, inheritance,
  receivers, and quoted Spock feature declarations.
- Give top-level scripts source-scoped owners. Preserve exact closure and
  feature ranges. Do not interpret Gradle DSL or dynamic metaprogramming as
  exact local calls.
- Add bounded qualification-only Gradle/Groovy project facts from checked-in
  settings/build/property/version-catalog files without executing Gradle or
  Groovy. Reuse the exact-language JVM boundary from Scala instead of adding a
  second family heuristic.
- Add conformance/resolver tests covering static/dynamic calls, aliases,
  traits, scripts, closures, Spock, Gradle, parser recovery, ambiguity, UTF-8,
  repeated sites, and limits.
- Compare against the pinned Groovy compiler AST oracle and direct baseline.

### Phase acceptance criteria

- [x] Production Groovy/Gradle graphs and registry state remain unchanged.
- [x] Candidate batches validate and are deterministic; the candidate never
  executes repositories, Gradle, Groovy scripts, transforms, or AST macros.
- [x] `methodMissing`, metaclass mutation, dynamic GStrings, DSL calls, and
  cross-JVM near matches remain unresolved.
- [x] Spock quoted features are exact declarations without advertising an
  unaudited `Tests` relationship capability.
- [x] Groovy conformance and `universal_resolution groovy` tests pass.

## Phase 8: Qualify Gradle/Spock behavior and atomically hard-cut Groovy

### Phase context

Phase 7 produced a qualification-only Groovy candidate. Production still uses
the regex/line extractor, Spock branch, `RawCall`, and JVM stub rewiring. This
phase switches only after the Groovy compiler oracle, three corpora, and
performance evidence pass.

### Phase scope and work

- Promote safe Gradle/Groovy project evidence required for contained modules,
  dependencies, source roots, and framework activation. Preserve dynamic DSL
  calls as external/unresolved facts.
- Add `LanguagePolicyKind::Groovy` only for source-proven language behavior not
  covered by generic stages. Never use a “dynamic language” broad fallback.
- Keep Spock declaration/test classification in the language producer or a
  narrowly scoped universal pack based on evidence ownership; freeze the choice
  and pack ID before implementation. Do not create subject-under-test edges
  without independent proof.
- Complete the three-corpus audit and performance gates.
- Atomically register `compass.groovy` version 1 `Qualifying`, delete the regex
  and Spock direct branches, remove Groovy JVM stub rewiring, enforce evidence-
  aware cache reuse, and update documentation.

### Phase acceptance criteria

- [x] No production call reaches `src/groovy.rs`, regex class/method/call
  extractors, or JVM terminal-name rewiring for Groovy.
- [x] The registry returns `compass.groovy` version 1 `Qualifying` with only
  audited capabilities.
- [x] Apache Groovy, Gradle, and Spock corpora meet the full audit and diversity
  gates with zero fabricated/dynamic/cross-language targets.
- [x] `.groovy` application sources, `.gradle` scripts, and Spock features each
  have dedicated deterministic incremental coverage and meet performance/RSS
  limits.
- [x] Groovy targeted/full suites and all repository gates listed in Phase 2
  pass.

## Phase 9: Qualify the mixed-language release and close support claims

### Phase context

Swift, Dart, Scala, and Groovy are now independently hard-cut version-1
`Qualifying` pipelines. This phase proves they coexist with existing languages,
frameworks, caches, and output contracts. It does not weaken a failing
language gate or promote a pipeline to `Qualified` automatically.

### Phase scope and work

- Extend the Code Graph v1 fixture corpus from recognition-only files to a
  reviewable multilingual slice containing imports, declarations, calls,
  construction, members, inheritance/traits, ambiguity, malformed files,
  framework facts, and exact negative cross-language collisions.
- Run clean, warm, forced, alternate-checkout, edit, delete, rename, and restore
  qualification over the combined corpus. Assert byte-identical graphs and
  evidence where the input state is equivalent.
- Verify universal registry order/uniqueness, framework descriptor/adapter
  parity, cache invalidation isolation, diagnostics, limits, publication
  omission accounting, stable IDs, direction, multiplicity, anchors, and
  provenance.
- Update `docs/design/language-architecture.md`,
  `docs/implementation/universal-evidence.md`,
  `docs/reference/universal-semantic-evidence.md`, user-facing language/support
  docs, `COMPATIBILITY.md`, and `CHANGELOG.md`. Update `MIGRATION.md` only if a
  user must discard or rebuild artifacts manually rather than through normal
  fingerprint invalidation.
- Record each language's exact `Qualifying` evidence and open promotion work.
  Change a language to `Qualified` only in a separate approved decision backed
  by its complete audit artifact.

### Phase acceptance criteria

- [x] All four languages appear once in the sorted universal registry, each at
  version 1 `Qualifying`, with no production direct fallback.
- [x] The mixed fixture proves exact-language boundaries: Swift/native and
  Scala/Groovy/Java/Kotlin name collisions never create cross-language edges;
  Dart package names never bind unrelated filesystem stubs.
- [x] All four independent audit artifacts still meet their thresholds against
  the exact release-candidate graph and source inventories.
- [x] `cargo fmt`, workspace lib/bin Clippy, workspace lib/bin tests, product
  boundary, CLI product contract, and `qualify_code_graph_v1.sh --fixtures-only`
  pass with the required external target directory.
- [x] `git diff --check` passes; the implementation status contains only
  intended source, test, fixture, qualification, and documentation changes
  plus pre-existing user paths preserved untouched; no generated graph,
  `.compass/`, `compass-out/`, credentials, or external-repository changes are
  present.

## Cross-phase test plan

Every language needs tests at four layers:

1. `compass-languages`: parser/evidence conformance, identity, exact UTF-8
   ranges, capabilities, deterministic ordering, malformed syntax, and limits.
2. `compass-resolve`: local, lexical, import/package, member, hierarchy,
   overload/argument, ambiguity, external, and negative cross-language cases.
3. `compass-core`/`compass-graph`: cold/warm/incremental/delete/rename/restore,
   cache-version enforcement, stable normalized graph, omissions, diagnostics,
   direction, multiplicity, and provenance.
4. Qualification: three independent corpora, source-oracle recall, accepted-
   edge precision, framework behavior, target-cluster diversity, performance,
   RSS, and exact release-candidate binding.

Tests must model existing conformance suites such as
`kotlin_universal_conformance.rs`, the language modules under
`crates/compass-resolve/tests/universal_resolution/`, Ruby's independent
qualification harness, and `code_graph_v1_determinism.rs`. They must not use
Graphify, real credentials, network services, runtime grammar downloads, or
untrusted project execution.

## Program done criteria

All items must hold; a green registration test alone is insufficient.

- [x] Swift, Dart, Scala, and Groovy are each registered once as version-1
  `Qualifying` universal evidence pipelines.
- [x] Each production extractor emits validated typed evidence directly and
  has no dual direct graph publisher, raw-call fallback, or replaced resolver.
- [x] Each language has exact project/module/package identity, conservative
  imports/calls/members/hierarchy, explicit ambiguity, and no guessed
  cross-language endpoints.
- [x] Existing Vapor, Dart application-framework, Play, Spock, and Gradle
  behavior is preserved or intentionally corrected through evidence-backed,
  bounded, independently tested contracts.
- [x] Each language passes its three-corpus independent quality audit and Phase
  0 performance/RSS gates.
- [x] Equivalent clean, warm, forced, alternate-checkout, incremental, and
  restored inputs publish byte-identical artifacts.
- [x] Public compatibility, support, framework, implementation, and changelog
  documentation accurately distinguishes recognition, established support,
  `Qualifying`, and `Qualified`.
- [x] All targeted and repository-wide verification gates pass on the final
  release-candidate commit.

## STOP conditions

Stop and report rather than improvising if any of these occurs:

- The mounted workspace or the phase-specific external Cargo target directory
  is unavailable or unwritable.
- A selected corpus is not at its pinned SHA, is dirty, requires executing
  repository code to inventory source, or cannot be parsed with the pinned
  qualification-only oracle.
- The vendored grammar lacks a required stable construct or exceeds parser
  recovery limits on a corpus. Do not add regex fallback or modify `vendor/`
  inside this plan.
- A proposed evidence fact requires changing `compass.graph/1`, the universal
  evidence schema, a public limit, or central publisher language branching.
- Candidate and established paths would both publish in production, even
  behind an environment variable.
- An audit misses a numerical, diversity, recall, precision, or zero-tolerance
  gate. Keep the direct production path active until corrected.
- Resolution requires choosing among multiple source-valid candidates,
  selecting the first filesystem/hash iteration result, or using JVM/native
  family or terminal-name similarity.
- A framework migration cannot prove exact activation or handler ownership.
  Preserve the established pack until a separate bounded design is approved.
- A phase's verification fails twice after a reasonable correction, or the
  implementation needs files outside that phase's stated scope.

## Maintenance notes

- Producer versions are per-language cache identities. Increment only the
  changed language for later semantic changes; do not bump the universal
  evidence schema for producer-local evolution.
- Project evidence and framework pack descriptors are fingerprint inputs.
  Review bounded scans, symlink containment, deterministic ordering, and stale
  cache rejection whenever new manifest fields are added.
- Reviewers should scrutinize exact range slicing, overload/argument identity,
  incomplete evidence, ambiguity, cross-language boundaries, and removal of
  replaced paths more than total node/edge growth.
- Optional compiler/SCIP enrichment is a separate fresh, bounded provider
  profile. It may strengthen exact endpoints but never replaces structural
  evidence or becomes a normal runtime dependency.
- Future Swift macros, Dart code generation, Scala implicit/compiler synthesis,
  and Groovy metaprogramming require separate capability and provider audits;
  this plan deliberately leaves unsupported dynamic meaning unresolved.

## Findings considered and rejected

- **Treat parser availability as support**: rejected because all four parsers
  are already linked while semantic and qualification maturity differ sharply.
- **Keep the direct publisher as fallback after registration**: rejected
  because Compass's hard-cut contract permits one production path only.
- **Migrate all four languages in one code change**: rejected because one
  language's audit or grammar gap must not block, weaken, or destabilize the
  others.
- **Share one JVM resolver across Java, Kotlin, Scala, and Groovy by terminal
  name**: rejected because package/family proximity is not an exact endpoint
  and creates fabricated cross-language edges.
- **Retain Dart/Groovy regex extraction beside AST evidence**: rejected because
  it duplicates identities and lets line heuristics bypass evidence validation.
- **Run SwiftPM, pub, sbt, Gradle, Flutter, or repository tests during normal
  extraction**: rejected because Compass is native, local-first, bounded, and
  must not execute untrusted project code.
- **Claim compiler-grade or dynamic-language completeness**: rejected. The v1
  structural tier publishes only independently auditable source evidence and
  explicit qualified externals.
