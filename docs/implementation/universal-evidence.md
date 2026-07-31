---
meta:
  contentType: Reference
  title: How universal evidence crosses Compass crate boundaries
  navLabel: Universal Evidence
  category: Implementation
  overview: Reference for the contracts that connect source extraction to normalized graph publication.
  goal: Define the implementation boundaries and invariants for universal language adapters.
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

> **Who this page is for:** contributors implementing language adapters, resolution, projection, and framework integration.
>
> **You will learn:** what each crate owns, which facts cross boundaries, and which invariants a language transition must preserve.
>
> **Prerequisites:** [Language architecture](../design/language-architecture.md) and [Extraction pipeline](extraction-pipeline.md).
>
> **Reading time:** 12 minutes.

## Implementation status

The contracts on this page combine available and approved planned behavior.

| Status | Implementation |
| --- | --- |
| Available now | `compass-languages` owns the source registry, parsers, direct adapters, and `UniversalEvidence` version 1 |
| Available now | Rust emits Phase 1 call occurrences and relationship candidates |
| Available now | `compass-resolve` merges universal evidence without dropping it |
| Planned | `GrammarProvider`, grammar provenance, and producer-registry validation |
| Planned | Bounded `EvidenceBuilder`, local projector, and collection resolver |
| Planned | Rust Phase 2 hard transition, followed by Java candidate transition |

Do not treat a planned interface as a shipped public API until its implementation and qualification commits land.

## Crate ownership

Each crate has one primary responsibility in the language pipeline.

| Crate or package | Responsibility |
| --- | --- |
| `vendor/compass-tree-sitter-language-pack` | Pinned grammars, static linkage, parser creation, and grammar metadata |
| `compass-languages::registry` | Source recognition, producer selection, adapter descriptors, capabilities, and budgets |
| `compass-languages::universal` | Current evidence schema, plus the planned builder, diagnostics, and local projection |
| Adapter modules | Language-specific syntax classification and identity normalization |
| Planned `compass-resolve::universal` | Collection indexes, fail-closed target selection, and graph projection |
| Framework modules | Activation evidence, target constraints, occurrence policy, and framework relations |
| `compass-graph` | Normalized graph construction, deduplication, clustering, and analysis |

The vendored package must not import Compass evidence, graph, resolver, or framework crates.

## Current source locations

These files define the available implementation before the planned Phase 2 changes.

| Source | Current role |
| --- | --- |
| [`vendor/compass-tree-sitter-language-pack/README.md`](../../vendor/compass-tree-sitter-language-pack/README.md) | Compass grammar-pack policy |
| [`vendor/compass-tree-sitter-language-pack/build.rs`](../../vendor/compass-tree-sitter-language-pack/build.rs) | Static grammar selection and compilation |
| [`crates/compass-languages/src/registry.rs`](../../crates/compass-languages/src/registry.rs) | Source recognition and adapter profiles |
| [`crates/compass-languages/src/engine.rs`](../../crates/compass-languages/src/engine.rs) | Parsing, adapter dispatch, direct post-processing, and framework detection |
| [`crates/compass-languages/src/universal.rs`](../../crates/compass-languages/src/universal.rs) | Version-1 evidence types |
| [`crates/compass-languages/src/rust_lang.rs`](../../crates/compass-languages/src/rust_lang.rs) | Rust Phase 1 direct output and evidence emission |
| [`crates/compass-languages/src/facts.rs`](../../crates/compass-languages/src/facts.rs) | Per-file extraction container |
| [`crates/compass-resolve/src/lib.rs`](../../crates/compass-resolve/src/lib.rs) | Collection merge and current resolution sequence |
| [`crates/compass-resolve/src/members.rs`](../../crates/compass-resolve/src/members.rs) | Current direct-adapter member-call resolution |

## Grammar provider boundary

The Compass-side grammar provider isolates adapters from the vendored implementation.

The provider must:

- load a canonical grammar ID
- report static availability
- configure a Tree-sitter parser
- return pack version, parser-source identity, and ABI version
- reject runtime download and dynamic loading

The provider returns immutable grammar provenance with the prepared parse tree. Grammar provenance participates in extraction cache identity.

An AST-backed adapter consumes:

```text
source path
borrowed source bytes
prepared Tree-sitter root
immutable grammar provenance
producer descriptor
```

A source-driven adapter consumes the path, source bytes, and producer descriptor without a grammar or tree.

## Producer descriptor boundary

The producer descriptor binds source recognition to extraction policy without putting semantics into the grammar pack.

Its conceptual shape is:

```rust
struct ProducerDescriptor {
    source_id: &'static str,
    extractor_kind: ExtractorKind,
    grammar: Option<GrammarRequirement>,
    adapter: AdapterDescriptor,
    limits: EvidenceLimits,
}
```

This shape is illustrative until the planned registry work lands. The implementation may use equivalent focused types.

Registry validation enforces:

- one grammar requirement for every AST-backed producer
- no grammar requirement for source-driven producers
- static availability for every required grammar
- evidence schema version 1 for universal profiles
- unique source and adapter identities
- bounded resource policies

## Adapter descriptor boundary

The adapter descriptor records semantic capability, not parser availability.

Version-1 descriptors contain:

- adapter ID
- source language
- adapter version
- evidence schema
- publication profile
- capability claims

The approved profile names are `Direct`, `UniversalCandidate`, and `UniversalComplete`. Reading old serialized `legacy` metadata remains compatible, while new metadata emits `direct`.

Rust and Java remain adapter version 1 through their candidate transitions. Grammar provenance and extraction semantics invalidate caches without changing the adapter version.

## Evidence contract

Each successfully prepared universal source produces one `compass.languages.evidence/1` batch.

| Collection | Required information |
| --- | --- |
| Declaration | Symbol, name, qualified name, kind, owner, signature, scope, anchor, and line |
| Scope | Stable ID, owner, parent, anchor, and depth |
| Binding | Scope, spelling, identity, binding kind, anchor, and line |
| Occurrence | Owner, scope, role, spelling, qualifier, anchor, and line |
| Relationship candidate | Relationship kind, role, target kinds, qualifier, signature or argument evidence, anchor, scope, and external identity |
| Diagnostic | Stable category, affected collection or structure, source anchor when valid, and completeness impact |

The builder must:

- validate anchors against source length
- validate scope parentage and depth
- enforce collection-specific limits
- intern repeated strings
- sort deterministically
- deduplicate exact facts
- mark incomplete batches

Adapters do not create graph nodes, graph edges, or `RawCall` records after their universal transition.

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
adapter policy emits evidence
        |
        v
EvidenceBuilder::finish
        |
        +--> local projector for single-file APIs
        |
        v
retain batch for collection resolution
```

Adapters may make bounded subpasses over the prepared tree. They may not reparse source, invoke the language pack's `process()` pipeline, or derive evidence from direct graph records.

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

Established direct adapters continue through their current resolution paths until their own transition. The universal resolver ignores language names and operates only on evidence fields.

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
- stamp adapter and grammar provenance
- avoid edges for ambiguous candidates
- avoid terminal-name fallback
- produce deterministic record ordering

Single-file and collection APIs use the same projection rules. A language adapter cannot maintain a second private publisher.

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

A language transition starts from its established direct adapter as the production baseline.

The transition sequence is:

1. capture deterministic fixture and pinned-corpus baseline evidence
2. implement adapter-local universal evidence without production dual-running
3. add post-implementation contract and language conformance tests
4. compare normalized graph output and relation-family coverage
5. measure cold, warm, incremental, restore, and RSS behavior
6. hard-transition only after every blocking gate passes
7. remove only that language's replaced direct publisher and resolver branches

A failed gate returns work to the candidate implementation. It does not weaken the direct adapter's baseline or force other languages to transition.

## Current language profiles

This table describes the current branch before the planned Phase 2 implementation.

| Language | Profile | Publication path |
| --- | --- | --- |
| Rust | `UniversalCandidate` Phase 1 | Hybrid direct graph records plus initial universal evidence |
| Python | Established direct adapter | Generic extraction plus Python-specific import and call resolution |
| Go | Established direct adapter | Dedicated Go extraction plus Go-specific owner and import resolution |
| Java | Established direct adapter | Generic Java extraction and member resolution |
| Remaining registered languages | Established direct adapters | Current language-specific or generic extraction paths |

The Rust Phase 2 gate precedes Java. Python and Go have no transition in this delivery.

## Verification gates

Every universal transition verifies:

- grammar availability and provenance
- deterministic evidence ordering
- exact UTF-8 byte anchors
- resource limits
- malformed-source behavior
- declared capability coverage
- absence of adapter-emitted graph records
- full language and resolver regression suites
- framework targeting
- repeated fixture determinism
- pinned real-corpus graph quality
- cold and warm performance

Run `graphify update .` after code changes so the repository knowledge graph reflects the final architecture.

## Related pages

- [Language architecture](../design/language-architecture.md)
- [System architecture](../design/architecture.md)
- [Extraction pipeline](extraction-pipeline.md)
- [Extending Compass](extending-compass.md)
- [Rust and Java hard-cutover design](../superpowers/specs/2026-07-30-rust-java-universal-hard-cutover-design.md)

**Next step:** inspect the approved [Rust Phase 2 implementation plan](../superpowers/plans/2026-07-30-rust-universal-phase-2-hard-cutover.md) before changing universal evidence code.
