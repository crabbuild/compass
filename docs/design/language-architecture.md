---
meta:
  contentType: Conceptual
  title: How Compass turns source into universal evidence
  navLabel: Language Architecture
  category: Design
  overview: Conceptual map of the Compass language pipeline and its independently qualified transitions.
  goal: Explain the grammar, adapter, evidence, resolution, and graph-publication boundaries.
  audience:
    - Compass contributors
    - technical evaluators
  contentPlan:
    - current and planned status
    - layer ownership
    - direct and universal adapter profiles
    - language-by-language transition policy
    - quality and failure boundaries
  openQuestions: []
---

# How Compass turns source into universal evidence

Compass separates parsing from semantic interpretation and graph publication. The vendored language pack supplies pinned parsers, language adapters interpret syntax, and the universal evidence framework gives independently qualified languages one resolution and publication path.

> **Who this page is for:** contributors and technical evaluators working on language extraction.
>
> **You will learn:** which layer owns grammars, semantic facts, cross-file resolution, graph publication, and framework targeting.
>
> **Prerequisites:** [System architecture](architecture.md) and [Graph model](../concepts/graph-model.md).
>
> **Reading time:** 10 minutes.

## Current and planned status

This architecture is transitioning one language at a time. The status labels below prevent planned contracts from being mistaken for shipped behavior.

| Status | Behavior |
| --- | --- |
| Available now | The vendored package supplies 37 pinned static Tree-sitter grammars |
| Available now | Python, Go, Java, and the remaining production languages use established direct adapters |
| Available now | Rust is a version-1 `UniversalCandidate` that emits initial universal evidence alongside its current graph records |
| Planned | Rust Phase 2 removes its replaced direct publisher and uses shared projection exclusively |
| Planned | Java becomes a version-1 `UniversalCandidate` after Rust passes its blocking gates |
| Planned | Later languages transition independently after language-specific qualification |

Python and Go are not deprecated while they use direct adapters. Their current implementations remain supported and protected by regression tests.

## Language architecture at a glance

The target architecture has one grammar boundary and one semantic evidence boundary.

```text
source registry
      |
      +--> source-driven producer ------------------+
      |                                            |
      +--> GrammarProvider                         |
              |                                    |
              v                                    |
        vendored grammar pack                      |
              |                                    |
              v                                    |
        prepared Tree-sitter AST                   |
              |                                    |
              +----------------+-------------------+
                               v
                    adapter-local policy
                               |
                               v
                    UniversalEvidence v1
                               |
                               v
                 shared resolver and projector
                               |
                               v
                    normalized Compass graph
```

Framework packs consume normalized declarations and exact occurrences after language resolution. They do not reinterpret malformed language evidence.

## Grammar substrate

The vendored `compass-tree-sitter-language-pack` package owns parser availability and reproducibility.

It owns:

- the pinned grammar catalog and aliases
- static parser linkage
- build-time grammar completeness checks
- parser construction
- Tree-sitter Application Binary Interface (ABI) compatibility
- grammar provenance used by cache identity

It does not own:

- Compass adapter profiles
- semantic capabilities
- declaration or relationship identity
- cross-file resolution
- graph publication
- framework behavior

Compass disables runtime grammar downloads and dynamic loading. A grammar update changes grammar provenance and invalidates affected extraction caches without changing the universal evidence schema.

## Source and producer registry

The Compass registry decides which producer owns a source file. Parser availability and semantic support remain separate states.

A producer descriptor records:

- source identity and extractor kind
- an optional grammar requirement
- adapter ID, version, and profile
- declared semantic capabilities
- evidence and resolution budgets

An Abstract Syntax Tree (AST) producer declares one grammar requirement. A source-driven producer, such as a manifest or template extractor, declares none.

The registry reports five distinct states:

1. grammar available
2. source recognized
3. established direct extraction available
4. universal candidate
5. universal complete

Grammar availability does not imply complete semantic support.

## Adapter-local semantic policy

Each adapter owns the syntax rules and identity normalization unique to its language.

Examples include:

- Rust traits, impl ownership, macros, and `use` trees
- Java packages, overload signatures, annotations, and interfaces
- Python modules, aliases, decorators, and dynamic member occurrences
- Go packages, receiver ownership, imports, and interfaces

An AST-backed adapter receives borrowed source bytes and one prepared tree. It does not parse the source again or call the language pack's generic intelligence pipeline.

After a language completes its universal transition, its adapter emits evidence only. The shared projector creates graph nodes and edges for both single-file and collection builds.

## Universal evidence

Universal evidence records what the adapter can prove before project-wide resolution.

The version-1 contract contains:

| Evidence | Purpose |
| --- | --- |
| Declarations | Stable symbol identity, kind, owner, signature, and source anchor |
| Scopes | Lexical parentage and ownership |
| Bindings | Imports, aliases, packages, modules, and wildcard or static bindings |
| Occurrences | Exact call, type, annotation, import, bound, and macro sites |
| Candidates | Allowed target kinds, qualifiers, signatures, and external identity |
| Diagnostics | Limit, syntax, and structural failures |

The evidence builder sorts and deduplicates facts deterministically. Per-file limits bound declarations, scopes, bindings, occurrences, candidates, and scope depth.

## Shared resolution and projection

The collection resolver converts evidence into relationships without branching on language names.

Resolution considers:

1. exact local owner
2. exact lexical scope
3. explicit import
4. same package or module
5. explicit external identity

Multiple valid targets remain unresolved. Wildcard and terminal-name matches never outrank exact evidence.

The projector converts resolved evidence into the normalized Compass graph contract. It preserves exact occurrence anchors and derives containment from declaration ownership and scope parentage.

## Direct and universal profiles

Profiles describe publication architecture, not implementation quality.

| Profile | Meaning |
| --- | --- |
| `Direct` | The established adapter publishes its current graph and unresolved-call records |
| `UniversalCandidate` | The adapter publishes universal evidence and has passed candidate gates |
| `UniversalComplete` | The adapter has passed the complete capability and conformance gates |

The current source enum still uses `AdapterProfile::Legacy`. The approved design renames it to `AdapterProfile::Direct`, accepts `legacy` while reading old metadata, and emits `direct` in new metadata.

## Language-by-language transitions

Each language keeps its established direct implementation until its universal candidate proves the transition is safe.

```text
direct adapter baseline
        |
        v
implement universal evidence policy
        |
        v
fixture and real-corpus qualification
        |
        +--> gate fails: improve candidate
        |
        v
atomic language transition
        |
        v
remove only that language's replaced path
```

No production dual-run or graph translation remains after transition. Other languages continue on their direct paths without central publisher changes.

## Quality and failure boundaries

The universal framework prefers defensible evidence over speculative graph size.

It classifies failures as:

- **Fatal substrate or structure failure:** return a structured extraction error and project no graph for the file
- **Bounded incomplete evidence:** project individually validated facts, mark the batch incomplete, and fail completeness qualification
- **Semantic ambiguity:** preserve occurrences and candidates, but publish no relationship edge

Missing grammars, invalid anchors, scope cycles, and conflicting identities are fatal. Parser error ranges and exhausted budgets create incomplete evidence. Competing overloads, imports, traits, receivers, or types remain ambiguous.

## Performance boundaries

The architecture controls work before optimizing language-specific rules.

Required properties include:

- one parse per AST-backed file
- bounded tree subpasses
- per-file string interning
- no in-process evidence serialization round trip
- ownership-based corpus merge
- separate parse, evidence, resolution, projection, and persistence timings

Rust and Java qualification requires Compass to remain faster than Graphify for comparable cold and warm workloads. Peak resident set size (RSS) remains measured but non-blocking during their candidate phases.

## Related pages

- [System architecture](architecture.md)
- [Universal evidence implementation architecture](../implementation/universal-evidence.md)
- [Extraction pipeline](../implementation/extraction-pipeline.md)
- [Extending Compass](../implementation/extending-compass.md)
- [Rust and Java hard-cutover design](../superpowers/specs/2026-07-30-rust-java-universal-hard-cutover-design.md)

**Next step:** read the [universal evidence implementation architecture](../implementation/universal-evidence.md) before changing the language registry, an adapter, or the resolver.
