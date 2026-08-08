# Evidence resolution framework technical design

Status: proposed

Scope: `compass-resolve` universal semantic evidence resolution and graph
projection

Companion plan: [Evidence resolution framework phased execution plan](evidence-resolution-framework-phased-execution-plan.md)

## Overview

Compass currently implements universal evidence indexing, cross-file target
selection, language-specific resolution, and graph projection in one large
module. The implementation is conservative and well covered by integration
tests, but its ownership boundaries are difficult to see and expensive to
extend safely.

This design retains one shared resolution contract while separating four
internal layers:

```text
validated semantic evidence
            |
            v
    fact store + indexes
            |
            v
 generic resolution pipeline
            |
            +---- explicit language-policy hooks
            |
            v
    resolution decisions
            |
            v
 graph projection/materialization
```

The design does not create an independent resolver for every language. Shared
precedence, boundedness, ambiguity, determinism, and provenance remain in the
kernel. A language policy owns only semantics that cannot be represented by
the generic evidence fields, such as TypeScript package exports, Rust
associated types, or Java overload conversion.

## Context

The universal evidence route has three conceptual steps:

1. A language adapter converts syntax into declarations, scopes, bindings,
   occurrences, relationship candidates, and diagnostics.
2. `compass-resolve` selects project-wide targets from immutable evidence.
3. The projector converts declarations and unique decisions into normalized
   graph records.

The current `evidence.rs` module combines all three resolver-side concerns. It
owns public decision types, primary fact storage, every secondary index,
index construction, resolution precedence, recursive traversal, language
rules, path normalization, and node/edge materialization.

This concentration creates several practical problems:

- A language change requires understanding unrelated indexing and projection
  behavior.
- Resolution precedence is mixed with the implementation of individual
  stages.
- Secondary-index completeness and truncation behavior are repeated across
  multiple collection types.
- TypeScript and JavaScript policy occupies a disproportionately large part of
  the shared resolver.
- Performance work cannot isolate index construction, candidate selection,
  and projection ownership cleanly.
- Most tests exercise extraction and resolution together, making resolver
  regressions harder to localize.
- Documentation describes a language-neutral resolver even though a few
  source-proven language semantics necessarily require explicit policy.

## Goals

- Preserve the current public resolver API and graph contract.
- Make resolution precedence readable in one short orchestration module.
- Give every primary fact and secondary index one clear owner.
- Separate target selection from graph projection.
- Concentrate language-specific policy behind explicit, narrow hooks.
- Preserve fail-closed ambiguity and bounded incomplete-state handling.
- Make deterministic ordering visible at every publication boundary.
- Create isolated seams for unit, contract, integration, and performance
  tests.
- Support measured performance optimization without coupling it to structural
  refactoring.
- Make adding future language policy predictable and reviewable.

## Non-goals

- Change `SemanticEvidenceBatch` or its schema version.
- Change graph node, edge, identity, provenance, or serialization contracts.
- Add or remove a universal language cutover.
- Add new `ResolutionRule` values during the structural refactor.
- Relax ambiguity, target-kind, scope, hierarchy, or source constraints.
- Add terminal-name fallback.
- Add a dynamic plugin system or runtime language loading.
- Give language policies direct graph-publication access.
- Redesign public limits in the same change.
- Claim performance improvement without corpus measurements.

## Required invariants

The rearchitecture must preserve these product properties.

### Evidence fidelity

- Resolution may use only validated evidence and admitted project metadata.
- A target must satisfy the relationship candidate's allowed kinds, language,
  source, signature, arity, hierarchy, and identity constraints.
- Exact source or binding evidence must outrank broader lookup.
- Multiple valid targets must remain ambiguous.
- Missing evidence must not be replaced with a convenient local target.

### Bounded work

- Aggregate evidence limits are checked before large reservations.
- Every recursive or graph-like walk has a depth or visit budget.
- Wildcard, alias, export, hierarchy, overload, and return expansion remain
  bounded.
- Overflow is an incomplete or ambiguous result, never an empty result and
  never a unique result.
- Internal caches, if introduced, have explicit aggregate limits.

### Determinism

- Equivalent evidence produces equivalent decisions and graph records.
- Hash iteration order is never observable.
- Candidate IDs, declaration slots, decisions, nodes, and edges are sorted at
  contract boundaries.
- Parallel candidate resolution collects into deterministic ordering before
  publication.

### Provenance and projection

- Relationship direction and multiplicity remain unchanged.
- Exact occurrence and declaration anchors are preserved.
- The selected resolution rule remains attached to the materialized edge.
- Deferred and external targets retain their existing identities and merge
  behavior.
- No edge is materialized for an ambiguous or unresolved candidate.

## Public compatibility boundary

The existing public namespace remains stable:

```rust
compass_resolve::evidence::{
    ResolutionDecision,
    ResolutionEvidence,
    ResolutionRule,
    UniversalResolutionIndex,
    UniversalResolutionLimits,
}
```

The following methods retain their current signatures and behavior:

```text
UniversalResolutionIndex::new
UniversalResolutionIndex::new_with_inventory
UniversalResolutionIndex::candidate_ids
UniversalResolutionIndex::resolve
UniversalResolutionIndex::materialize
```

The internal implementation may become a façade over a fact store, index set,
resolver, and projector. This is not a public migration and does not require a
schema or command compatibility change.

## Target module layout

```text
crates/compass-resolve/src/evidence/
|-- mod.rs
|-- api.rs
|-- facts.rs
|-- budget.rs
|-- project.rs
|-- index/
|   |-- mod.rs
|   |-- builder.rs
|   |-- names.rs
|   |-- hierarchy.rs
|   |-- wildcards.rs
|   `-- returns.rs
|-- resolve/
|   |-- mod.rs
|   |-- pipeline.rs
|   |-- bindings.rs
|   |-- members.rs
|   |-- hierarchy.rs
|   `-- wildcards.rs
|-- languages/
|   |-- mod.rs
|   |-- rust.rs
|   |-- java.rs
|   `-- typescript/
|       |-- mod.rs
|       |-- modules.rs
|       |-- members.rs
|       |-- types.rs
|       `-- overloads.rs
`-- projection/
    |-- mod.rs
    |-- nodes.rs
    `-- edges.rs
```

This tree is an ownership map, not a requirement to maximize file count.
Modules should remain combined when their types and invariants cannot be
understood independently.

## Component design

### Public façade

`evidence/mod.rs` owns public re-exports and the compatibility-facing
`UniversalResolutionIndex` methods. It delegates construction to
`IndexBuilder`, target selection to `Resolver`, and graph publication to
`Projector`.

The façade must not contain language algorithms, index loops, or graph-record
construction.

### Public API types

`api.rs` owns `UniversalResolutionLimits`, `ResolutionRule`,
`ResolutionEvidence`, and `ResolutionDecision`.

These types are shared by construction, resolution, projection, and tests.
They must not depend on language-policy modules.

### Fact store

`FactStore` owns validated primary records:

```rust
struct FactStore {
    declarations: AHashMap<String, DeclarationFact>,
    declaration_ids: Vec<String>,
    occurrences: AHashMap<String, OccurrenceFact>,
    bindings: AHashMap<String, BindingFact>,
    candidates: AHashMap<String, RelationshipCandidate>,
    scopes: AHashMap<String, ScopeFact>,
    definition_ranges: BTreeMap<String, EvidenceRange>,
}
```

The declaration ID vector remains the canonical slot table for compact
secondary indexes. `FactStore` provides checked ID-to-slot and slot-to-record
operations. It does not perform resolution.

### Lookup budgets

`LookupBudget` is an internal semantic wrapper around
`UniversalResolutionLimits`.

Initially, its lookup fields map directly to `candidates_per_lookup`. The
wrapper gives distinct names to operations that currently share that value:

```text
candidate storage capacity
scope visits
alias visits
hierarchy depth
wildcard expansion
export traversal
overload candidates
return-chain expansion
```

This improves auditability without changing public behavior. Any future split
of public limits requires separate compatibility review.

### Bounded candidate collections

Repeated `(items, complete)` structures should converge on one internal
abstraction after existing behavior has been characterized:

```rust
struct BoundedCandidates<T> {
    items: Vec<T>,
    complete: bool,
    observed: usize,
}
```

The exact representation may differ, but it must support these queries:

```text
is complete
contains zero candidates
contains exactly one complete candidate
contains multiple candidates
overflowed before all candidates were observed
```

Only a complete collection with one compatible item may resolve uniquely.

### Resolution indexes

`ResolutionIndexes` groups secondary indexes by query responsibility:

```rust
struct ResolutionIndexes {
    names: NameIndexes,
    hierarchy: HierarchyIndexes,
    wildcards: WildcardIndexes,
    returns: ReturnIndexes,
    typescript: Option<TypeScriptIndexes>,
    rust: Option<RustIndexes>,
}
```

`NameIndexes` owns qualified-name, module-name, scope-name,
source-directory-name, inventory, alias, and owner-member lookup.

`HierarchyIndexes` owns direct bases, direct subtypes, completeness markers,
and members grouped by owner.

`WildcardIndexes` owns wildcard bindings by scope and module, wildcard
re-exports, and source-proven Rust wildcard targets.

`ReturnIndexes` owns ordinary and outer nominal return candidates by callable.

Language index bundles are optional so unrelated corpora do not pay their
construction and memory costs.

### Index builder

`IndexBuilder` constructs immutable state in explicit phases:

```text
validate aggregate limits
validate each evidence batch
collect and uniquify primary facts
assign declaration slots
build name and module indexes
build project indexes
build hierarchy indexes
build wildcard indexes
build return indexes
build required language indexes
sort, deduplicate, and finalize
```

Every phase receives validated counts and reports profiling information. A
phase must not reserve from untrusted input before aggregate bounds are
checked.

Construction errors remain actionable strings until a separate typed-error
change is justified. Error wording should not be changed incidentally during
module extraction.

### Project context

`ProjectContext` owns repository-root and project-level naming information:

```rust
struct ProjectContext {
    root: PathBuf,
    go_module_path: Option<String>,
    typescript_modules: TypeScriptProjectModuleIndex,
    typescript_metadata: TypeScriptProjectMetadataIndex,
}
```

It also owns pure path codecs for Python modules, Go packages, source
directories, and TypeScript importer/module keys.

Project path handling remains platform-portable and repository-contained.
Language policies may query this context but may not mutate it during
resolution.

### Resolution database

`ResolutionDb` is a borrowed, read-only view over facts, indexes, project
context, and budgets:

```rust
struct ResolutionDb<'a> {
    facts: &'a FactStore,
    indexes: &'a ResolutionIndexes,
    project: &'a ProjectContext,
    budget: LookupBudget,
}
```

Shared and language-specific resolver functions receive `ResolutionDb`
instead of the entire public index type. This makes their dependencies
explicit and prevents projection access.

### Candidate context

`CandidateContext` computes lookup inputs once:

```rust
struct CandidateContext<'a> {
    candidate: &'a RelationshipCandidate,
    occurrence: Option<&'a OccurrenceFact>,
    binding: Option<&'a BindingFact>,
    language: &'a str,
    qualifier: Option<&'a str>,
}
```

Fallback bindings may produce a derived candidate context, but must not modify
stored evidence.

### Stage outcomes

Resolver stages need to distinguish inapplicability from a terminal decision:

```rust
enum StageOutcome {
    Continue,
    Decided(ResolutionDecision),
}
```

An ambiguous result is terminal. An unresolved result is terminal only where
the current contract says that a constraint must fail closed. A stage that has
no relevant evidence returns `Continue`.

### Resolution pipeline

`resolve/pipeline.rs` contains one explicit sequence of named stages. It must
not use a dynamic vector of callbacks because stage ordering is a semantic
contract.

The implementation is transposed from the current resolver and frozen by
characterization tests. Conceptually, it proceeds from the strongest evidence
to the broadest allowed evidence:

```text
exact source declaration
special constrained hierarchy or associated-type resolution
project-aware language import resolution
explicit binding or alias
source-proven call-result fallback
exact containment or ownership target
lexical scope lookup
same module or package lookup
bounded wildcard and member lookup
source inventory lookup
qualified external or deferred target
unresolved
```

This list describes ownership and relative strength. The implementation plan
must record the exact existing branch order before moving any stage.

### Shared resolver algorithms

Shared modules own algorithms driven by evidence rather than syntax:

- lexical scope traversal and shadowing;
- exact and explicit binding lookup;
- alias traversal and cycle detection;
- wildcard scope and module traversal;
- owner-qualified member lookup;
- return-type candidate lookup;
- direct-base and direct-successor dispatch;
- C3 linearization;
- closed-world descendant dispatch;
- incomplete hierarchy decisions;
- candidate filtering and unique-decision construction.

These modules may inspect the evidence language when language identity is part
of an index key. They must not interpret language syntax or package formats.

### Language policies

Language policy uses static dispatch through an internal enum:

```rust
enum LanguagePolicyKind {
    TypeScript,
    Rust,
    Java,
    Generic,
}
```

Static dispatch keeps the supported policy set auditable and avoids object
lifetime complexity. A trait should be introduced only if at least two policy
implementations need the same non-trivial hook and the trait makes ownership
clearer.

Policy hooks are intentionally narrow:

```text
resolve project-aware import before generic binding lookup
expand language-specific member-owner candidates
select a compatible overload from an already bounded set
resolve associated or projected types
normalize a language-specific qualified target
```

A policy may return candidates or a `StageOutcome`. It may not publish nodes
or edges, bypass target constraints, change global stage order, or turn
incomplete lookup into a unique result.

### TypeScript and JavaScript policy

The TypeScript policy owns:

- source-module and project-module keys;
- package exports, imports, conditions, and re-export traversal;
- named, default, namespace, CommonJS, and import-equals bindings;
- structural object members and source-proven object spreads;
- type aliases, generic substitution, mapped and indexed types;
- union, tuple, array, utility, and `keyof` projections;
- callable return-member chains;
- overload matching and bounded type-argument inference.

TypeScript and JavaScript remain one policy family because they share module
and member semantics while retaining language identity in index keys.

### Rust policy

The Rust policy owns:

- implementation-associated type indexes;
- implementation trait canonicalization;
- source-proven trait lineage;
- associated-type selection;
- trait implementation candidate selection;
- trait-member lookup;
- Rust-specific wildcard callable and module behavior.

Generic C3 and direct-base algorithms remain outside this policy.

### Java policy

The Java policy owns:

- overload applicability;
- primitive widening;
- boxing and unboxing;
- known reference conversion;
- parameter-vector specificity;
- unique most-specific overload selection.

Package and exact-name lookup remain shared because they are represented by
generic evidence fields.

### Go and Python behavior

Go currently needs project module and package-directory normalization but not
a broad language-specific target selector. Those codecs belong in
`ProjectContext`.

Python receiver dispatch uses generic hierarchy constraints and C3
linearization. It remains in the shared hierarchy resolver. A Python policy
module should be created only if future behavior cannot be expressed through
the existing evidence contract.

### Projection

Projection consumes immutable facts and completed decisions. It owns:

- declaration-to-node conversion;
- containment projection;
- external and deferred target creation;
- node merge rules;
- relationship name and direction mapping;
- edge identity, occurrence anchor, resolution rule, and provenance;
- deterministic node and edge ordering.

Projection does not perform target lookup. If it cannot materialize a valid
decision, it reports the existing failure path instead of selecting another
target.

## Error and incomplete-state model

The design preserves the existing three broad outcomes.

| Condition | Resolver behavior | Projection behavior |
| --- | --- | --- |
| Invalid or over-limit aggregate evidence | Index construction fails | No universal projection |
| Bounded incomplete lookup | Ambiguous or unresolved according to the current rule | No invented edge |
| Multiple compatible targets | `ResolutionDecision::Ambiguous` | No edge |
| No compatible target | `ResolutionDecision::Unresolved` | No edge unless existing explicit external/deferred evidence applies |
| One complete compatible target | Resolved decision with exact rule | Deterministic node and edge records |

Refactoring may introduce internal typed errors, but public error behavior and
diagnostic text should be changed only in a dedicated compatibility-reviewed
commit.

## Performance design

The architecture exposes separate measurement boundaries for:

```text
evidence validation
primary fact collection
name and module index construction
hierarchy index construction
wildcard and return index construction
language index construction
candidate ordering
candidate decisions
node projection
edge projection
```

Potential optimizations are experiments rather than assumed improvements:

- classify facts once and feed multiple index builders;
- reserve only from validated aggregate counts;
- omit unused language index bundles;
- compact repeated compound keys with measured interning;
- memoize bounded alias, export, hierarchy, and return walks;
- cache normalized project-relative module keys;
- return declaration slots internally and allocate IDs only at the API edge;
- preserve top-level parallel candidate resolution with deterministic sorting.

Each optimization lands separately and must demonstrate a material time or
peak-memory improvement on its target corpus without semantic output changes.

## Test architecture

### Unit tests

Unit tests live beside pure utilities and index builders. They cover path
normalization, type parsing, overload conversion, C3 merge, bounded buckets,
slot conversion, sorting, deduplication, cycles, and completeness markers.

### Resolver contract tests

Direct evidence fixtures exercise each pipeline stage without parsing source.
They assert the complete `ResolutionDecision`, including rule and candidate
count.

Every stage needs positive, negative, ambiguity, overflow, and precedence
coverage.

### Integration tests

Source-backed tests continue to validate the complete adapter-to-graph path.
The large universal resolution suite should remain one Cargo test binary but
be divided into language and kernel source modules.

### Projection tests

Projection tests assert node and edge identity, direction, multiplicity,
anchors, provenance, target kind, external/deferred behavior, and deterministic
ordering.

### Qualification

Language and resolver changes continue to use fixture qualification and the
real-repository qualification required by the affected language profile.

## Performance acceptance policy

Structural phases require semantic parity and must not introduce a material
performance regression.

An optimization phase should satisfy all of these conditions:

- canonical graph records and direct decisions are unchanged;
- the target hotspot improves median time or peak RSS by at least ten percent;
- representative non-target corpora do not regress by more than three percent
  without an approved tradeoff;
- measurements include at least five warm runs and one cold run;
- overflow and deterministic-order tests continue to pass.

Thresholds may be adjusted when measurement noise is documented, but a change
must not be called an optimization without supporting evidence.

## Alternatives considered

### One complete resolver per language

Rejected because it duplicates ambiguity, ordering, limits, hierarchy,
projection, and provenance. It would allow languages to drift into different
graph contracts.

### Dynamic language plugin trait

Rejected for the initial design. Compass language support is statically linked
and qualification spans extraction, evidence, resolution, and projection.
Dynamic dispatch would add indirection without solving a current deployment
requirement.

### Move functions into files without changing state ownership

Rejected as the final architecture. Extension methods over one giant private
state object would reduce file size but preserve coupling and unclear index
ownership.

It remains an acceptable temporary step while extracting behavior in small
commits.

### Rewrite the resolver around a generic rule engine

Rejected because rule ordering is a product contract and should remain visible
in ordinary Rust control flow. A data-driven engine would make terminal versus
continuing outcomes and language exceptions harder to audit.

### Combine structural refactoring and algorithm changes

Rejected because semantic parity would be difficult to prove and regressions
would be hard to localize. Optimization begins only after the new ownership
boundaries are established.

## Risks and controls

| Risk | Control |
| --- | --- |
| Resolution precedence changes | Direct stage-order characterization tests |
| Truncated candidates appear unique | Completeness-aware bounded collections |
| Language behavior leaks into the kernel | Static policy boundary and module dependency rules |
| Public imports break after file movement | Stable façade and public API compile tests |
| Path normalization changes portability | Cross-platform project-path unit tests |
| Parallel iteration changes output order | Explicit sort at every publication boundary |
| Test decomposition increases link time | Keep one top-level integration-test binary |
| Abstraction becomes more complex than behavior | Extract first; generalize only after multiple consumers exist |
| Performance work changes semantics | One measured optimization per commit with graph parity gates |

## Decision summary

- Keep one shared resolver contract.
- Separate facts, indexes, resolution, language policy, and projection.
- Preserve `UniversalResolutionIndex` as the public façade.
- Use compact declaration slots inside secondary indexes.
- Represent lookup completeness explicitly.
- Keep precedence in one explicit static pipeline.
- Use static language-policy dispatch.
- Create language modules only for genuine language-specific policy.
- Make optimization a later, measured phase.
- Preserve all current schemas and public behavior during rearchitecture.

## Implemented architecture

The phased execution completed the ownership split without changing the public
`compass_resolve::evidence` paths or the serialized graph contract. The active
implementation is organized as follows:

| Component | Ownership |
| --- | --- |
| `evidence/api.rs` | Public limits, decisions, rules, and evidence |
| `evidence/budget.rs` | Semantic per-lookup budget |
| `evidence/facts.rs` | Canonical validated facts |
| `evidence/index/builder.rs` | Validation, fact collection, bounded index construction |
| `evidence/index/mod.rs` | Secondary lookup indexes |
| `evidence/project.rs` | Repository paths, project metadata, and module-key utilities |
| `evidence/resolve/context.rs` | Read-only resolution database and normalized candidate context |
| `evidence/resolve/pipeline.rs` | Ordered rule precedence and fail-closed fallback |
| `evidence/resolve/{bindings,wildcards,members,hierarchy}.rs` | Generic resolver stages |
| `evidence/languages/policy.rs` | Closed static language-policy selection |
| `evidence/languages/typescript/` | TypeScript modules, members, types, and overloads |
| `evidence/languages/{rust,java}.rs` | Rust and Java policy |
| `evidence/projection/` | Prepared targets plus node and edge materialization |

The public `UniversalResolutionIndex` is now a facade. Resolution runs through
a borrowed `ResolutionDb`, creates one `CandidateContext`, and applies ordered
stages. A stage either continues or returns a final `ResolutionDecision`.
Language selection uses `LanguagePolicyKind`; it does not use runtime trait
objects, registries, or plugins.

### Implemented extension contract

An evidence-resolution extension must:

1. Add or reuse typed evidence in `compass-languages`.
2. Add bounded indexes in `evidence/index/` only when an existing index cannot
   answer the lookup.
3. Prefer a generic stage in `evidence/resolve/` when semantics are shared.
4. Add a closed `LanguagePolicyKind` branch only for language-specific rules.
5. Return ambiguity or unresolved state when evidence is incomplete.
6. Add direct `SemanticEvidenceBatch` contract tests, including negative,
   ambiguity, ordering, limit, and projection assertions.
7. Run the resolver crate tests, strict production Clippy, and code-graph
   fixture qualification.

### Performance result

The refactor retains compact declaration slots, preallocated fact maps,
bounded candidate vectors, deterministic parallel collection, and centralized
lookup budgets. On 2026-08-08, the 181-case `universal_resolution` test binary
reported 0.28 seconds after the split versus 0.29 seconds before it. Warm
end-to-end invocations were 0.62 and 0.65 seconds. Because no repeated lookup
regression was measured, no synchronization-bearing memoization cache was
added.

## Related pages

- [Evidence resolution framework phased execution plan](evidence-resolution-framework-phased-execution-plan.md)
- [Universal evidence implementation](universal-evidence.md)
- [Language architecture](../design/language-architecture.md)
- [Universal semantic evidence reference](../reference/universal-semantic-evidence.md)
- [Extending Compass](extending-compass.md)
- [Workspace tour](workspace-tour.md)

**Next step:** approve the ownership and compatibility decisions, then begin
Phase 0 of the execution plan by freezing resolver behavior.
