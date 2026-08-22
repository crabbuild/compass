---
meta:
  contentType: Reference
  title: How universal evidence crosses Compass crate boundaries
  navLabel: Universal Evidence
  category: Implementation
  overview: Reference for the contracts that connect source extraction to normalized graph publication.
  goal: Define the implementation boundaries and invariants for universal evidence pipelines.
  audience:
    - Compass language contributors
    - resolver contributors
  contentPlan:
    - crate ownership
    - grammar and producer contracts
    - evidence lifecycle
    - resolution and projection rules
    - errors, performance, and qualification
  openQuestions: []
---

# How universal evidence crosses Compass crate boundaries

Universal evidence is the semantic boundary between source-specific extraction and language-neutral graph construction. This reference defines the owning crates, required data, lifecycle, failure behavior, and transition gates.

## Implementation status

The contracts on this page distinguish implemented hard cuts from approved
future work.

| Status | Implementation |
| --- | --- |
| Available now | `compass-languages` owns the source registry, parsers, established extractors, and universal evidence schema version 2 (extraction semantics version 3) |
| Available now | C#, Dart, Go, Groovy, Java, Kotlin, PHP, Python, Ruby, Rust, Scala, Swift, TypeScript, and JavaScript are entries in the hard-cut `UniversalEvidenceRegistry`; each entry pairs a `UniversalEvidenceProducer` with a `UniversalEvidenceQualification` state |
| Available now | `EvidenceBuilder` emits bounded `SemanticEvidenceBatch` values for all registered universal languages; Swift, Dart, Scala, and Groovy use direct language modules backed by a shared bounded AST-first traversal, while each retains a distinct version-1 producer identity |
| Available now | The language-wave parity profiles preserve quoted Groovy/Spock feature declarations, Dart library namespaces and bounded `part`/`part of` plus import/export selectors, Swift enum/struct/extension/type-alias/member identities, and Scala companion plus import-selector identities without enabling unaudited test or dynamic-dispatch capabilities |
| Available now | `UniversalResolutionIndex` resolves and projects hard-cut evidence without a language-name branch |
| Available now | Rust has passed its Phase 2 quality audit; all registered pipelines remain explicitly `Qualifying` until their complete independent audit gates promote them |
| Planned | `GrammarProvider` and grammar provenance |
| Planned | Independent source-oracle audits for pipelines without complete artifacts, plus separate promotion decisions for every `Qualifying` pipeline |

Do not treat a planned interface as a shipped public API until its implementation and qualification commits land.

## Crate ownership

Each crate has one primary responsibility in the language pipeline.

| Crate or package | Responsibility |
| --- | --- |
| `vendor/compass-tree-sitter-language-pack` | Pinned grammars, static linkage, parser creation, and grammar metadata |
| `compass-languages::registry` | Source recognition, producer selection, evidence pipelines, capabilities, and budgets |
| `compass-languages::evidence` | Hard-cut semantic evidence model and bounded AST-backed builder |
| `compass-languages::evidence_pipeline` | Universal evidence producers, qualification states, capability declarations, and registry validation |
| Evidence producers | Language-specific syntax classification and identity normalization |
| `compass-resolve::evidence` | Collection indexes, fail-closed target selection, and shared graph projection |
| Framework modules | Activation evidence, target constraints, occurrence policy, and framework relations |
| `compass-graph` | Normalized graph construction, deduplication, clustering, and analysis |

The vendored package must not import Compass evidence, graph, resolver, or framework crates.

## Current source locations

These files define the current implementation.

| Source | Current role |
| --- | --- |
| [`vendor/compass-tree-sitter-language-pack/README.md`](../../vendor/compass-tree-sitter-language-pack/README.md) | Compass grammar-pack policy |
| [`vendor/compass-tree-sitter-language-pack/build.rs`](../../vendor/compass-tree-sitter-language-pack/build.rs) | Static grammar selection and compilation |
| [`crates/compass-languages/src/registry.rs`](../../crates/compass-languages/src/registry.rs) | Source recognition and evidence-pipeline selection |
| [`crates/compass-languages/src/evidence_pipeline.rs`](../../crates/compass-languages/src/evidence_pipeline.rs) | Hard-cut producer registry, qualification states, and capability declarations |
| [`crates/compass-languages/src/engine.rs`](../../crates/compass-languages/src/engine.rs) | Parsing, mutually exclusive evidence-pipeline dispatch, and framework detection |
| [`crates/compass-languages/src/evidence/`](../../crates/compass-languages/src/evidence/) | Hard-cut evidence model, validation, limits, and AST builder |
| [`crates/compass-languages/src/evidence/build.rs`](../../crates/compass-languages/src/evidence/build.rs) | AST-to-evidence policy for the hard-cut language set |
| [`crates/compass-languages/src/facts.rs`](../../crates/compass-languages/src/facts.rs) | Per-file extraction container |
| [`crates/compass-resolve/src/lib.rs`](../../crates/compass-resolve/src/lib.rs) | Collection merge and current resolution sequence |
| [`crates/compass-resolve/src/evidence.rs`](../../crates/compass-resolve/src/evidence.rs) | Universal indexes, resolution rules, and shared materialization |
| [`crates/compass-resolve/src/members.rs`](../../crates/compass-resolve/src/members.rs) | Current direct-extractor member-call resolution |

## Grammar provider boundary

The Compass-side grammar provider isolates evidence producers from the vendored implementation.

The provider must:

- load a canonical grammar ID
- report static availability
- configure a Tree-sitter parser
- return pack version, parser-source identity, and ABI version
- reject runtime download and dynamic loading

The provider returns immutable grammar provenance with the prepared parse tree. Grammar provenance participates in extraction cache identity.

An AST-backed producer consumes:

```text
source path
borrowed source bytes
prepared Tree-sitter root
immutable grammar provenance
producer descriptor
```

A source-driven producer consumes the path, source bytes, and producer descriptor without a grammar or tree.

## Universal evidence producer and pipeline boundary

The producer descriptor binds source recognition to extraction policy without putting semantics into the grammar pack.

Its conceptual shape is:

```rust
struct UniversalEvidenceProducer {
    id: &'static str,
    language: &'static str,
    version: u32,
    evidence_schema: &'static str,
    capabilities: &'static [LanguageCapability],
}

struct UniversalEvidencePipeline {
    producer: UniversalEvidenceProducer,
    qualification: UniversalEvidenceQualification,
}
```

This is the shipped registry contract. Language-specific emitters are wired to
the selected pipeline in the same change that registers the language and
removes its replaced publisher or resolver path.

Registry validation enforces:

- one grammar requirement for every AST-backed producer
- no grammar requirement for source-driven producers
- static availability for every required grammar
- evidence schema version 2 for universal pipelines
- unique source and producer identities
- bounded resource policies

The producer owns language-specific metadata. The pipeline is the runtime
selection unit and carries the lifecycle state separately from producer
identity. This separation makes it possible to promote a producer from
`Qualifying` to `Qualified` without changing the extraction API or inventing a
second extractor name.

## Producer metadata boundary

The producer metadata records semantic capability, not parser availability.

Producer metadata contains:

- producer ID
- source language
- producer version
- evidence schema
- capability claims

The emitted pipeline identity may additionally preserve parser dialect
provenance (for example `ts` or `tsx`).

The enclosing pipeline adds the qualification state (`Qualifying` or
`Qualified`) without changing producer identity.

The shared fact model also has additive optional identity metadata for
TypeScript/JavaScript qualification: declarations and bindings may carry a
value/type/namespace symbol-space tag, and import/re-export bindings may mark
`typeOnly`. Legacy producers omit these fields; their stable IDs do not gain an
identity component until a producer explicitly emits one. A type-only binding
must use the type or namespace space and is rejected otherwise.

The TypeScript/JavaScript producer also records exact module-specifier and
binding anchors for imports, resolves JSX member tags through proven namespace
receivers, treats `this` and private member identifiers as nominal source
evidence, and accepts only literal computed members. Dynamic member keys and
unproven `super` receivers remain unresolved; these semantics are production
evidence and remain subject to the independent audit gates.

The architecture names are `Direct` (the established language-specific route)
and `UniversalEvidencePipeline` (the shared route). The pipeline lifecycle
states are `Qualifying` and `Qualified`; neither state creates a second
extractor. A historical compatibility evidence type still serializes an
internal `legacy` variant. Treat it as an old wire identifier, not a current
pipeline state.

Go and Java are at producer version 3, Python is at version 11, and Rust is at
version 15. Producer versions advance when a semantic evidence change
requires language-local cache invalidation; grammar provenance and the
extraction-semantics identity remain independent cache inputs.

## Evidence contract

Each successfully prepared hard-cut source produces one
`SemanticEvidenceBatch`. Hard cutover alone does not increment a producer
version, but later evidence-contract changes may do so to invalidate only that
language's cached facts.

| Collection | Required information |
| --- | --- |
| Declaration | Symbol, name, qualified name, kind, owner, signature, canonical parameter types, direct-base completeness, scope, anchor, and line |
| Scope | Stable ID, owner, parent, anchor, and depth |
| Binding | Scope, spelling, identity, binding kind, anchor, and line |
| Occurrence | Owner, scope, role, spelling, qualifier, anchor, and line |
| Relationship candidate | Relationship kind, role, target kinds, qualifier, signature, canonical argument types, anchor, scope, and external identity |
| Diagnostic | Stable category, affected collection or structure, source anchor when valid, and completeness impact |

The builder must:

- validate anchors against source length
- validate scope parentage and depth
- enforce collection-specific limits
- intern repeated strings
- sort deterministically
- deduplicate exact facts
- mark incomplete batches

Universal evidence producers do not create graph nodes, graph edges, or
`RawCall` records after their pipeline is selected.

## End-to-end ownership map

The following diagram shows the runtime boundary, not crate dependencies alone.

```text
compass-core build orchestration
            |
            v
compass-files discovers and fingerprints source
            |
            v
compass-languages::Registry selects LanguageSpec
            |
            +--> UniversalEvidenceRegistry miss ------------------------------+
            |                                                       |
            |                                                       v
            |                                             established extractor
            |                                                       |
            v                                                       v
UniversalEvidenceRegistry hit                                      graph/raw/language facts
            |
            v
vendored grammar -> one Tree-sitter AST
            |
            v
producer-local AST policy
            |
            v
EvidenceBuilder -> SemanticEvidenceBatch v2
            |
            +--------------------------+----------------------------+
                                       v
                         compass-resolve collection merge
                                       |
                         +-------------+--------------+
                         |                            |
                         v                            v
               UniversalResolutionIndex      established resolvers
                         |                            |
                         +-------------+--------------+
                                       v
                         framework target resolution
                                       |
                                       v
                           normalized Extraction
                                       |
                                       v
                              compass-graph
```
The two routes meet only after each has produced project-resolvable facts.
The universal route never reconstructs evidence from graph rows, and the
established route is not forced through a compatibility translator.

## Per-file lifecycle

The per-file path preserves one parser invocation and one evidence boundary.

```text
Registry::resolve(path)
        |
        v
validate ProducerDescriptor
        |
        v
GrammarProvider::parse once
        |
        v
producer policy emits evidence
        |
        v
EvidenceBuilder::finish
        |
        +--> local projector for single-file APIs
        |
        v
retain batch for collection resolution
```

Evidence producers may make bounded subpasses over the prepared tree. They may
not reparse source, invoke the language pack's `process()` pipeline, or derive
evidence from direct graph records.

## Collection lifecycle

Collection resolution owns all project-wide target selection.

```text
per-file extractions
        |
        v
ownership-based merge
        |
        v
immutable universal indexes
        |
        v
resolve unique relationships
        |
        v
project normalized graph records
        |
        v
resolve framework facts
```

Established extractors continue through their current resolution paths until
their own transition. The universal resolver ignores language names and
operates only on evidence fields. The merge discards replaced code relations
for evidence-backed extractions, retains source inventory and framework facts,
then materializes declarations and resolved relationships once.

## Resolution order

The universal resolver evaluates candidates in this order:

1. compatible declaration and relationship kind
2. exact local owner
3. exact lexical scope
4. explicit import or alias
5. same package or module
6. compatible signature or argument evidence
7. explicit external identity

An earlier exact match outranks a later broad match. Wildcard expansion and overload candidates stop at their configured limits.

The resolver publishes an edge only when one compatible target survives. It preserves unresolved occurrences and candidates for diagnostics and future analysis.

## Projection rules

The shared projector owns the normalized graph contract for every transitioned language.

It must:

- map declaration kinds to stable normalized symbol kinds
- derive containment from ownership and scope parentage
- preserve exact occurrence source anchors
- stamp producer and grammar provenance
- avoid edges for ambiguous candidates
- avoid terminal-name fallback
- produce deterministic record ordering

Single-file and collection APIs use the same projection rules. A language
producer cannot maintain a second private publisher.

## Failure classes

The framework uses three failure classes.

| Class | Examples | Result |
| --- | --- | --- |
| Fatal substrate or structure | Missing grammar, ABI mismatch, null parse tree, invalid anchor, scope cycle, conflicting identity | Structured extraction error and no universal projection for the file |
| Bounded incomplete evidence | Tree-sitter error range, declaration limit, import expansion limit | Project validated facts, mark the batch incomplete, and fail completeness gates |
| Semantic ambiguity | Competing overloads, imports, traits, receivers, or types | Preserve the occurrence and publish no relationship edge |

Evidence cannot overlap an untrusted parser error range. Framework packs cannot repair malformed language evidence.

## Performance invariants

Performance constraints apply to the architecture before language-specific optimization.

Required invariants include:

- one parse per AST-backed file
- no evidence serialization between in-process stages
- bounded per-file arenas and collections
- ownership transfer during corpus merge
- immutable bounded resolver indexes
- delayed graph materialization
- separate stage timings

Qualification records parse, evidence, resolution, projection, persistence, total latency, and peak resident set size (RSS). Rust and Java must remain faster than Graphify on comparable cold and warm workloads.

## Language transition contract

A language transition starts from its established direct extractor as the
production baseline.

The transition sequence is:

1. capture deterministic fixture and pinned-corpus baseline evidence
2. implement producer-local universal evidence without production dual-running
3. add post-implementation contract and language conformance tests
4. compare normalized graph output and relation-family coverage
5. measure cold, warm, incremental, restore, and RSS behavior
6. hard-transition only after every blocking gate passes
7. remove only that language's replaced direct publisher and resolver branches

A failed gate leaves the pipeline `Qualifying`. It does not weaken the direct
extractor baseline or force other languages to transition.

## Current language pipeline states

This table describes the current branch.

| Language | Pipeline state | Publication path |
| --- | --- | --- |
| Python | Hard-cut universal | `SemanticEvidenceBatch` plus shared resolution and projection; no replaced collection resolver |
| Go | Hard-cut universal | `SemanticEvidenceBatch` plus shared resolution and projection; no replaced Go collection resolver |
| Rust | Hard-cut `Qualifying` | Version-15 producer evidence plus shared resolution and projection; bounded method-result chains, impl-scoped associated types, exact `Self::Type` returns, scoped generic parameters, and nested lexical calls are preserved, Phase 2 is qualified, and replaced Rust paths are removed |
| Java | Hard-cut `Qualifying` | Version-3 producer evidence plus shared resolution and projection; exact callable ownership, proven conversions, replaced Java paths removed, and post-cutover corpus qualification complete |
| Kotlin | Hard-cut `Qualifying` | Version-1 producer evidence plus shared resolution and projection; exact Kotlin-only source resolution, named/default arguments and extensions, replaced Kotlin paths removed, and complete quality-audit gates still pending |
| Ruby | Hard-cut `Qualifying` | Version-1 producer evidence plus shared resolution and projection; method-space-aware dispatch and Rails pack use the same pipeline while audit gates remain open |
| TypeScript | Hard-cut `Qualifying` | Version-5 producer evidence plus shared resolution and projection; TSX aliases this identity and the replaced generic publisher is removed |
| JavaScript | Hard-cut `Qualifying` | Version-5 producer evidence plus shared resolution and projection; CJS/ESM and package decisions retain source and provenance bounds |
| Swift | Hard-cut `Qualifying` | Version-1 AST-first evidence with exact declarations, scopes, imports, calls, construction, type/base references, members, ownership, and source-bounded diagnostics; Vapor uses the `vapor-swift` universal pack and Swift legacy member-table compatibility is removed |
| Dart | Hard-cut `Qualifying` | Version-1 AST-first evidence with bounded imports/exports, calls, construction, type/base references, members, ownership, and explicit language constraints; established Flutter/BLoC/Riverpod/navigation convention facts remain separately marked, source/manifest-activated, and bounded |
| Scala | Hard-cut `Qualifying` | Version-1 AST-first evidence with package scopes, declarations, imports, calls, construction, type/base references, members, ownership, and exact-language JVM boundaries; `build.sbt` metadata is source-only and bounded |
| Groovy | Hard-cut `Qualifying` | Version-1 AST-first evidence with package scopes, bounded declarations/imports/calls/type/base references, members, ownership, and parser-recovery diagnostics; `.gradle` is treated as Groovy and JVM-family stub rewiring excludes it |
| Remaining registered languages | Established direct extractors | Current language-specific or generic extraction paths |

Python, Go, Rust, Java, Kotlin, Ruby, TypeScript, JavaScript, Swift, Dart, Scala,
and Groovy are hard-cut on this branch.
Each later language
reuses the same hard-cut registry, evidence model, resolver, and projector
without adding language cases to the central publisher. A language's
transition does not alter the publication route of any other language.
The pinned Kotlin baseline, coverage deltas, performance results, and open
audit gates are recorded in
[Kotlin universal qualification](kotlin-universal-qualification.md).
Ruby's pinned three-corpus baseline, independent Ripper oracle, performance
samples, and qualifying-only audit boundary are recorded in
[Ruby universal qualification](ruby-universal-qualification.md).
Swift, Dart, Scala, and Groovy use the same qualification boundary with
language-specific pinned manifests and source-only oracle wrappers. Their
established direct-path fixture baselines are captured at the pre-cutover
revision `88abe4c071a19ec03b3bca132656830a02a47907` in
`tests/qualification/{swift,dart,scala,groovy}-universal-baseline.json`.
Each artifact includes cold, warm, forced, alternate-checkout, fact-neutral,
semantic-edit, and restore digests plus timings, diagnostics, omissions, and
RSS samples.

## Framework-pack status

`FrameworkPackDescriptor` and `FrameworkPackRegistry` define the universal pack
contract and validate language capabilities, framework capabilities, activation
evidence, accepted roles, typed relationship families, occurrence policy, and
limits. The production registry contains `spring-java`, `spring-kotlin`, and
`rails-ruby`.
They derive framework meaning only from exact language-keyed universal
evidence and publish through the shared framework resolver. Established source,
config, and template packs remain active until their individual hard cutovers.

All established and universal framework adapters now execute through one
static framework-pack runtime in `compass-languages`. The runtime owns pack
selection, manifest activation, adapter dispatch, per-pack fact budgets,
aggregate fact limits, deterministic ordering, and publication. Config and
template packs use the same lifecycle as source packs, while the universal
descriptor remains the contract for evidence-backed packs. Project-wide
expansion uses the same stable pack ID seam in `compass-resolve`; adding a
universal expander without a matching descriptor is rejected by resolver
tests.

The runtime keeps Spring's Java and Kotlin mappings, Express middleware,
Axum builders, Next.js file routing, and Vite configuration in separate
focused adapters. Shared project evidence supplies bounded configuration,
alias, plugin, and route-root metadata, while the qualification module checks
that each framework's expected routes resolve exactly before a fixture can
claim support.

The bounded project index also recognizes `Package.swift`, `pubspec.yaml`/
`pubspec.yml`, `build.sbt`, and Gradle build files. It records only checked-in
dependency coordinates, explicit package/toolchain metadata, and normalized
project-contained source roots; it never invokes SwiftPM, pub, sbt, Gradle, or
project scripts. These values are part of the deterministic project-evidence
fingerprint used for cache reuse.

## Verification gates

Every universal transition verifies:

- grammar availability and provenance
- deterministic evidence ordering
- exact UTF-8 byte anchors
- resource limits
- malformed-source behavior
- declared capability coverage
- absence of producer-emitted graph records
- full language and resolver regression suites
- framework targeting
- repeated fixture determinism
- pinned real-corpus graph quality
- cold and warm performance

Run `compass update .` after code changes so the repository knowledge graph reflects the final architecture.

## Related pages

- [Language architecture](../design/language-architecture.md)
- [System architecture](../design/architecture.md)
- [Extraction pipeline](extraction-pipeline.md)
- [Extending Compass](extending-compass.md)
- [Rust and Java hard-cutover design](../superpowers/specs/2026-07-30-rust-java-universal-hard-cutover-design.md)

**Next step:** inspect the approved [Rust Phase 2 implementation plan](../superpowers/plans/2026-07-30-rust-universal-phase-2-hard-cutover.md) before changing universal evidence code.
