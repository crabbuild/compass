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

The contracts on this page distinguish implemented hard cuts from approved
future work.

| Status | Implementation |
| --- | --- |
| Available now | `compass-languages` owns the source registry, parsers, established adapters, and semantic evidence version 1 |
| Available now | Python, Go, Rust, and Java are entries in the hard-cut `AdapterRegistry`, all at adapter version 1 |
| Available now | `EvidenceBuilder` emits bounded `SemanticEvidenceBatch` values for all four hard-cut languages |
| Available now | `UniversalResolutionIndex` resolves and projects hard-cut evidence without a language-name branch |
| Available now | Rust has passed Phase 2 qualification; Java remains `UniversalCandidate` after completing post-cutover pinned-corpus qualification |
| Planned | `GrammarProvider`, grammar provenance, and producer-registry validation |
| Planned | Independently qualified hard cuts for the remaining registered languages |

Do not treat a planned interface as a shipped public API until its implementation and qualification commits land.

## Crate ownership

Each crate has one primary responsibility in the language pipeline.

| Crate or package | Responsibility |
| --- | --- |
| `vendor/compass-tree-sitter-language-pack` | Pinned grammars, static linkage, parser creation, and grammar metadata |
| `compass-languages::registry` | Source recognition, producer selection, adapter descriptors, capabilities, and budgets |
| `compass-languages::evidence` | Hard-cut semantic evidence model and bounded AST-backed builder |
| `compass-languages::adapters` | Hard-cut adapter profiles, version-1 capability declarations, and registry validation |
| Adapter modules | Language-specific syntax classification and identity normalization |
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
| [`crates/compass-languages/src/registry.rs`](../../crates/compass-languages/src/registry.rs) | Source recognition and adapter profiles |
| [`crates/compass-languages/src/adapters.rs`](../../crates/compass-languages/src/adapters.rs) | Hard-cut language registry and capability declarations |
| [`crates/compass-languages/src/engine.rs`](../../crates/compass-languages/src/engine.rs) | Parsing, mutually exclusive adapter dispatch, and framework detection |
| [`crates/compass-languages/src/evidence/`](../../crates/compass-languages/src/evidence/) | Hard-cut evidence model, validation, limits, and AST builder |
| [`crates/compass-languages/src/evidence/build.rs`](../../crates/compass-languages/src/evidence/build.rs) | AST-to-evidence policy for the hard-cut language set |
| [`crates/compass-languages/src/facts.rs`](../../crates/compass-languages/src/facts.rs) | Per-file extraction container |
| [`crates/compass-resolve/src/lib.rs`](../../crates/compass-resolve/src/lib.rs) | Collection merge and current resolution sequence |
| [`crates/compass-resolve/src/evidence.rs`](../../crates/compass-resolve/src/evidence.rs) | Universal indexes, resolution rules, and shared materialization |
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

The architectural profile names are `Direct`, `UniversalCandidate`, and
`UniversalComplete`. A historical compatibility evidence type still serializes
an internal `legacy` variant. Treat it as an old wire identifier, not the name
or quality status of an established adapter.

Rust and Java remain adapter version 1 through their candidate transitions. Grammar provenance and extraction semantics invalidate caches without changing the adapter version.

## Evidence contract

Each successfully prepared hard-cut source produces one
`SemanticEvidenceBatch`. The adapter version remains 1; hard cutover does not
increment it.

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
            +--> AdapterRegistry miss ------------------------------+
            |                                                       |
            |                                                       v
            |                                             established extractor
            |                                                       |
            v                                                       v
AdapterRegistry hit                                      graph/raw/language facts
            |
            v
vendored grammar -> one Tree-sitter AST
            |
            v
adapter-local AST policy
            |
            v
EvidenceBuilder -> SemanticEvidenceBatch v1
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

Established adapters continue through their current resolution paths until
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

This table describes the current branch.

| Language | Profile | Publication path |
| --- | --- | --- |
| Python | Hard-cut universal | `SemanticEvidenceBatch` plus shared resolution and projection; no replaced collection resolver |
| Go | Hard-cut universal | `SemanticEvidenceBatch` plus shared resolution and projection; no replaced Go collection resolver |
| Rust | Hard-cut `UniversalCandidate` | Version-1 evidence plus shared resolution and projection; Phase 2 qualified and replaced Rust paths removed |
| Java | Hard-cut `UniversalCandidate` | Version-1 evidence plus shared resolution and projection; replaced Java paths removed and post-cutover corpus qualification complete |
| Remaining registered languages | Established direct adapters | Current language-specific or generic extraction paths |

Python, Go, Rust, and Java are hard-cut on this branch. Each later language
reuses the same hard-cut registry, evidence model, resolver, and projector
without adding language cases to the central publisher. A language's
transition does not alter the publication route of any other language.

## Framework-pack status

`FrameworkPackDescriptor` and `FrameworkPackRegistry` define the universal pack
contract and validate language capabilities, activation evidence, accepted
roles, relationship families, occurrence policy, and limits. The production
universal descriptor list is currently empty. Established source, config, and
template pack registries remain active until their individual hard cutovers.

This separation prevents a language hard cutover from implicitly changing
Rails, Spring, Rust web, ASP.NET, Vapor, TypeScript web, filesystem-route,
enterprise-domain, config, or template semantics.

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
