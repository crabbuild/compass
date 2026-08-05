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

## Current and planned status

This architecture is transitioning one language at a time. The status labels below prevent planned contracts from being mistaken for shipped behavior.

| Status | Behavior |
| --- | --- |
| Available now | The vendored package supplies 37 pinned static Tree-sitter grammars |
| Available now | Python and Go are registered hard-cut adapters: they emit semantic evidence and use shared resolution and projection |
| Available now | Rust Phase 2 is a quality-gated, hard-cut version-2 `UniversalCandidate`; its replaced publisher and collection resolution branches are removed |
| Available now | Java is a hard-cut version-3 `UniversalCandidate`; its replaced publisher and Java member resolver are removed, and post-cutover pinned-corpus qualification is complete |
| Available now | The remaining production languages keep their established extraction and resolution paths |
| Planned | Later languages transition independently after language-specific qualification |

The transition is language-by-language, not a migration away from a deprecated
system. An established path remains supported until that language passes its
own hard-cut gates. A hard-cut language has no production dual-run or graph
translation fallback.

## Language architecture at a glance

The current architecture has one grammar boundary and two intentionally
separate publication routes. The registry selects exactly one route for a
language.

```text
files + project manifests
          |
          v
 source / adapter registry
          |
          +--> source-driven producer ------------------------------+
          |                                                         |
          +--> vendored static grammar pack                         |
                         |                                          |
                         v                                          |
                  one Tree-sitter AST                               |
                         |                                          |
                         v                                          |
               adapter-local semantics                              |
                         |                                          |
              +----------+-----------+                              |
              |                      |                              |
              v                      v                              |
       hard-cut adapter       established adapter                   |
 Python / Go / Rust / Java    remaining languages                   |
              |                      |                              |
              v                      v                              |
    SemanticEvidenceBatch v1   graph facts + unresolved calls       |
              |                      |                              |
              v                      v                              |
      universal indexes        existing collection resolvers        |
              |                      |                              |
              +----------+-----------+------------------------------+
                         |
              verified compiler artifacts
                    (SCIP / project analyzers)
                         |
              exact-anchor semantic join
                         |
                         v
             framework facts and resolution
                         |
                         v
              Code Graph v1 normalization
                         |
                         v
                 graph.json + caches
```

Framework packs consume normalized declarations and exact occurrences after language resolution. They do not reinterpret malformed language evidence.

The hard-cut route is selected by `AdapterRegistry`. Presence in the source
registry or availability of a grammar does not select it. On the current
branch, the registry contains `go`, `java`, `python`, and `rust`. Go is at
adapter version 3, Java is at version 3, Python is at version 9, and Rust is at
version 2. Candidate status
describes qualification maturity; it does not re-enable the removed direct
route. Adapter-version changes invalidate cached evidence for only the changed
language. Go identities retain the repository-relative directory prefix and
use the parsed package clause to distinguish external test packages.

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

Document adapters are a narrow exception to the general registry boundary:
the pinned Markdown and HTML bindings are linked directly by
`compass-languages` because the current vendored build subset does not expose
those document loaders. They still use the same Tree-sitter ABI, source-driven
Engine API, deterministic range policy, and no-runtime-download guarantee.

## How the layers differ

The grammar pack, the historical CodeGraph kernel, and universal evidence solve
different problems. They are complementary concepts, not interchangeable
implementations.

| Layer | Primary output | Owns language semantics | Owns cross-file resolution | Extensibility boundary |
| --- | --- | --- | --- | --- |
| Vendored `compass-tree-sitter-language-pack` | Parser and Tree-sitter AST | No | No | Add or update a pinned grammar |
| CodeGraph kernel language module | Direct row-buffer nodes, edges, and references | Yes | Only the kernel's direct contracts | Add or modify a complete per-language publisher |
| Compass universal evidence adapter | Declarations, scopes, bindings, occurrences, and candidates | Syntax-to-evidence policy only | No; shared resolver owns it | Add language policy without changing the central publisher |

The CodeGraph kernel's files such as `python.rs` and `java.rs` are specialized,
direct publishers. They parse, walk, normalize, and emit the kernel's graph
rows, including language-specific compatibility behavior. That kernel is not a
dependency of the Compass Rust workspace described here.

Universal evidence is the more extensible Compass boundary because a new
language contributes syntax classification and truthful capabilities while
reusing target selection, ambiguity policy, graph projection, provenance,
limits, and conformance gates. This does not make the vendored grammar pack
obsolete: the pack remains the reproducible parser substrate under both the
hard-cut and established adapter routes.

## Source and producer registry

The Compass registry decides which producer owns a source file. Parser availability and semantic support remain separate states.

A producer descriptor records:

- source identity and extractor kind
- an optional grammar requirement
- adapter ID, version, and profile
- declared semantic capabilities
- evidence and resolution budgets

An Abstract Syntax Tree (AST) producer declares one grammar requirement. A source-driven producer, such as a manifest or template extractor, declares none.

The architecture distinguishes five states:

1. grammar available
2. source recognized
3. established extraction available
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
| Declarations | Stable symbol identity, kind, owner, signature, bounded canonical parameter types, and source anchor |
| Scopes | Lexical parentage and ownership |
| Bindings | Imports, aliases, packages, modules, and wildcard or static bindings |
| Occurrences | Exact call, type, annotation, import, bound, and macro sites |
| Candidates | Allowed target kinds, qualifiers, bounded canonical argument types, signatures, and external identity |
| Diagnostics | Limit, syntax, and structural failures |

The evidence builder sorts and deduplicates facts deterministically. Per-file limits bound declarations, scopes, bindings, occurrences, candidates, and scope depth.

## Shared resolution and projection

The collection resolver converts evidence into relationships without branching on language names.

Resolution considers:

1. exact local owner
2. exact lexical scope
3. explicit import
4. same package or module
5. compatible signature or argument count
6. explicit external identity

Multiple valid targets remain unresolved. Wildcard and terminal-name matches never outrank exact evidence.

The projector converts resolved evidence into the normalized Compass graph
contract. It preserves exact occurrence anchors and derives containment from
declaration ownership and scope parentage. When a parser has already emitted
the owned declaration, the containment candidate carries that exact
declaration identity; it never re-selects an overload from name and arity.

### Compiler artifact enrichment

Tree-sitter remains the native structural baseline, but an enabled Program
analysis may also consume offline SCIP evidence. Compiler facts do not replace
the Java AST and do not independently invent graph
relationships. Compass projects a compiler-selected Java call target only
when all of these conditions hold:

- the artifact revision is verified against the current source manifest;
- Tree-sitter emitted a Java `Calls` or `Constructs` candidate at the exact
  same repository-relative byte range;
- the compiler target symbol has exactly one local definition whose identifier
  range exactly matches one Java declaration;
- the target declaration kind satisfies the AST candidate; and
- all compiler providers at the call site agree on the target symbol.

When those conditions hold, the compiler identity may disambiguate overloads
and replace the structural call edge at that occurrence. The published edge
keeps the AST wiring site, records artifact origin, and uses the
`compiler-exact-anchor` rule; `program.json` retains the provider descriptors.
A stale or unverified artifact, a compiler reference outside an AST-proven
call, an external-only symbol, or provider disagreement leaves the structural
result unchanged.

This boundary is intentionally narrower than Program IR merging. SCIP encodes
symbol references, and Compass's current decoder retains a call-resolution
fact for each non-definition reference. The exact AST call join is therefore
the semantic guard that prevents a field read or type reference from becoming
a call edge.

The projection contract also accepts `Project` provider batches so a future
bounded `javac`, JDT, or language-server analyzer can reuse the same join and
failure policy. No such analyzer is invoked by the current build pipeline; it
requires its own process isolation, limits, freshness contract, and
qualification before it can become a shipped provider.

### Compiler artifact enrichment

Tree-sitter remains the native structural baseline, but an enabled Program
analysis may also consume offline SCIP evidence. Compiler facts do not replace
the Java AST and do not independently invent graph relationships. Compass
projects a compiler-selected Java call target only when all of these
conditions hold:

- the artifact revision is verified against the current source manifest;
- Tree-sitter emitted a Java `Calls` or `Constructs` candidate at the exact
  same repository-relative byte range;
- the compiler target symbol has exactly one local definition whose identifier
  range exactly matches one Java declaration;
- the target declaration kind satisfies the AST candidate; and
- all compiler providers at the call site agree on the target symbol.

When those conditions hold, the compiler identity may disambiguate overloads
and replace the structural call edge at that occurrence. The published edge
keeps the AST wiring site, records artifact origin, and uses the
`compiler-exact-anchor` rule; `program.json` retains the provider descriptors.
A stale or unverified artifact, a compiler reference outside an AST-proven
call, an external-only symbol, or provider disagreement leaves the structural
result unchanged.

This boundary is intentionally narrower than Program IR merging. SCIP encodes
symbol references, and Compass's current decoder retains a call-resolution
fact for each non-definition reference. The exact AST call join is therefore
the semantic guard that prevents a field read or type reference from becoming
a call edge.

The projection contract also accepts `Project` provider batches so a future
bounded `javac`, JDT, or language-server analyzer can reuse the same join and
failure policy. No such analyzer is invoked by the current build pipeline; it
requires its own process isolation, limits, freshness contract, and
qualification before it can become a shipped provider.

## Established and universal profiles

Profiles describe publication architecture, not implementation quality.

| Profile | Meaning |
| --- | --- |
| `Direct` | The established adapter publishes its current graph and unresolved-call records |
| `UniversalCandidate` | The hard-cut adapter publishes universal evidence and is being qualified against candidate gates |
| `UniversalComplete` | The adapter has passed the complete capability and conformance gates |

Documentation uses **established** or `Direct` for the active non-universal
route. A historical internal enum variant still uses the name `Legacy`; that
implementation detail is not a quality label and must not be used to describe
supported languages.

## Language-by-language transitions

Each language keeps its established direct implementation until its universal candidate proves the transition is safe.

```text
established adapter baseline
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

## Framework-pack boundary

Framework detection is downstream of language parsing but upstream of final
Code Graph v1 publication. Packs emit anchored route or domain facts; the
framework resolver validates targets and materializes typed relationships.

The Java Spring source pack is the first production universal framework pack.
It consumes exact Java annotation, call, import, type, ownership, and hierarchy
evidence and derives HTTP, bean, injection, messaging, scheduling, persistence,
transaction, and security meaning before framework resolution. Its Java legacy
detectors are removed atomically; Kotlin Spring routing remains on its explicit
established pack until Kotlin has a universal language adapter. Established
source, config, and template adapters execute through the same static runtime,
which owns selection, activation, limits, and publication without requiring a
runtime plugin ABI. Other packs retain their established semantics until their
own qualification and hard cut.

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
