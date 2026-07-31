# Balanced Source-Grounded Graph Quality Design

## Status

Approved in conversation on 2026-07-30. This design governs the quality phase
after semantic-dominance phase two. Production implementation must precede
focused regression tests, as requested by the user.

This is explicitly not a red-green TDD workflow. Each delivery increment
implements and reviews production behavior first, then adds conformance and
regression coverage before qualification and commit.

## Goal

Build a universal semantic evidence framework that lets present and future
languages and frameworks produce consistently high-quality Compass graphs.
Use real source occurrences as the authority, achieve at least 99.5% observed
overall precision with a two-sided 95% Wilson lower bound of at least 99%, and
recover at least 95% of source-derived facts for every advertised capability.
Do not optimize for raw edge count or literal Graphify overlap.

## Background

Phase two produced valid, deterministic graphs for pinned Django and Entire
revisions:

| Repository | Tool | Nodes | Unique edges |
| --- | --- | ---: | ---: |
| Django | Compass | 55,120 | 130,847 |
| Django | Graphify | 50,845 | 158,710 |
| Entire | Compass | 21,711 | 72,250 |
| Entire | Graphify | 20,585 | 61,062 |

The current comparator classifies 53,005 Django Graphify reference edges as
`module_import_projected_to_symbol`. Those edges use an import statement as
the relationship occurrence and project the imported target onto class or
symbol owners. The import occurrence is invalid as proof of the projected
owner-to-target reference, but the previous report overstated the conclusion
by describing the entire family as false relationships. Some source-target
dependencies may still be real at later use sites. This phase must distinguish:

- a fabricated occurrence;
- a wrong owner, target, or relationship kind;
- a real dependency represented at a different occurrence; and
- a genuine Compass recall gap.

The current residual audit also shows:

- Django has genuine unresolved decorator, base-type, call, and import
  candidates mixed with external/built-in facts and config-shape differences.
- Entire has real receiver-containment and imported-type gaps mixed with
  Graphify cross-language and terminal-name joins. Examples include Go
  `cleanup()` calls matched to a shell function, and `time.Time` or
  `context.Context` matched to unrelated repository-local types.
- Compass currently passes all four Django and Entire semantic query oracles,
  but query success alone does not prove relationship precision or recall.

## Quality policy

Compass uses balanced source-grounded quality:

- Every behavioral relationship must have a real source occurrence.
- Declaration relationships use their declaration occurrence.
- Resolution must use lexical, import, module, package, ownership, or
  qualified external evidence.
- Label-only repository-wide joins are forbidden.
- Ambiguous candidates remain ambiguous or unresolved.
- Edge count is diagnostic metadata, never a quality score.
- Graphify supplies recall hypotheses, not ground truth.
- Fabricated occurrences, cross-language matches, and unsafe substitution of
  a same-named local target are critical violations with zero tolerance.
- A language or framework may advertise only capabilities that pass the
  universal conformance suite.

## Architecture

### Universal semantic evidence model

The extraction layer emits a typed internal evidence model independent of
tree-sitter, any specific language, and the public graph schema. Its logical
records are:

- `DeclarationFact`: a declared symbol, stable identity components, kind,
  language, module/package, lexical scope, and declaration occurrence;
- `ScopeFact`: lexical scope identity, owner, parent, and source range;
- `BindingFact`: import, export, alias, receiver, namespace, module, or package
  binding visible in a scope;
- `OccurrenceFact`: an exact source use with owner, syntax role, original
  spelling, qualifier, and source range;
- `RelationshipCandidate`: a typed relationship request connecting an
  occurrence or declaration to constrained endpoint identities;
- `ResolutionConstraint`: language, role, scope, module/package, binding,
  target kind, and external-identity requirements; and
- `AdapterCapabilities`: the capability set the producer claims and must prove
  through conformance.

These records form the universal contract. Public `compass.graph/1` nodes and
edges remain the validated output rather than becoming a language-extension
surface.

### Language adapters

Every language integrates through one registered adapter contract. An adapter
may use tree-sitter, a source-driven parser, compiler evidence, or a bounded
combination, but it emits the same universal evidence records. Its capability
descriptor may include:

- declarations and lexical scopes;
- imports, exports, aliases, modules, namespaces, and packages;
- calls and construction;
- type references, returns, fields, inheritance, implementation, and
  embedding;
- members, receivers, and ownership;
- decorators, annotations, attributes, macros, or equivalent metadata; and
- qualified standard-library and external identity.

Adapters may define language-specific parsing and qualification rules. They
may not implement repository-wide label fallback, bypass occurrence
requirements, publish arbitrary graph edges, or weaken universal ambiguity
handling. Tree-sitter and source-driven adapters use the same registration,
capability, limit, and conformance interfaces.

Adding a language requires an adapter, registry entry, capability declaration,
and conformance fixtures. It does not require changes to graph publication,
the shared resolver, or framework-pack execution unless the language
introduces a genuinely new universal syntax role. New roles require a
coordinated evidence-model change rather than an adapter-local escape hatch.
Existing extraction, producer, graph, adapter, and framework version values
remain unchanged.

### Source-occurrence contract

Language extraction and resolution share a language-neutral occurrence
contract. Each candidate relationship carries:

- source file and exact byte/line range;
- lexical owner at the occurrence;
- syntax role, such as call, decorator, annotation, base type, field type,
  embedding, import, or containment;
- original spelling and qualifier;
- visible import, module, and package scope;
- producer rule and confidence tier.

The internal evidence records carry this contract directly. A language moves
to this contract through a hard cutover in its extractor; Compass does not
translate its prior raw facts through a compatibility projection or maintain
dual resolution algorithms for that language.

### Universal constrained resolver

The resolver considers evidence in this order:

1. exact lexical declaration;
2. explicit import alias or package-qualified identity;
3. unique same-module or same-package definition;
4. qualified external endpoint;
5. unresolved or ambiguous endpoint.

It never falls back to a repository-wide terminal-label match. Resolution
indexes must be bounded and keyed by language, module/package, spelling, and
role so the implementation does not reintroduce imports × symbols work.

The resolver is shared across adapters. Language-specific qualification is
provided through registered binding and identity policies; candidate
selection, uniqueness, cross-language isolation, external endpoints,
ambiguity, provenance, and resource limits remain universal. A policy returns
constraints or normalized identities, not a chosen arbitrary graph node.

### Framework evidence packs

Framework support consumes universal language evidence through a registered
pack contract. Each pack declares:

- stable pack ID and supported language capabilities;
- project activation evidence, dependency markers, and manifest policy;
- accepted declaration and occurrence roles;
- source, target, and ownership constraints;
- relationship kind and confidence policy;
- exact occurrence requirements;
- candidate and fan-out limits; and
- conformance fixtures and framework-specific query oracles.

A framework pack may add domain meaning to proven language facts. It cannot
invent an occurrence, resolve by terminal label, or bypass the universal
resolver and publisher. Config and template packs use the same evidence,
activation, limits, and conformance contract as source packs.

Adding a framework requires a pack and its fixtures. It does not require a
central resolver or publisher change.

### Capability and conformance registry

One registry exposes adapter and framework-pack capabilities to extraction,
qualification, and diagnostics. Conformance covers:

- stable declarations and scopes;
- exact occurrence slicing, including Unicode and multiline constructs;
- aliases, qualification, shadowing, and re-exports;
- repeated occurrences and overloads;
- ambiguous and external targets;
- wrong-language and same-name collision controls;
- resource-limit behavior;
- deterministic cold/warm facts; and
- every advertised relationship family.

Unsupported capabilities remain explicit. The engine must not infer support
from a file extension or silently fall back to lower-quality generic logic.

### Publication

Behavioral edges publish only with a real relationship occurrence.
Containment, inheritance, embedding, and declaration typing use the
declaration range. Qualified external endpoints remain explicit when the
repository does not contain the definition. Publication retains relation kind
and occurrence identity and continues to validate the closed v1 endpoint
matrix.

### Development-only qualification

The quality pipeline is:

```text
source corpus
  -> independent occurrence candidates
  -> Compass extraction and constrained resolution
  -> validated typed graph
  -> deterministic source audit
  -> Graphify difference audit
  -> precision, recall, conflict, and performance report
```

Graphify does not enter production code, runtime dependencies, or mandatory
Compass extraction.

## Extensibility and hard cutover

The current `Engine`, `Extraction`, framework `SourcePack` registry, and
collection resolver already provide useful boundaries but allow language facts
and resolution rules to use inconsistent string metadata. Each selected
language or framework changes atomically:

1. introduce the universal evidence types, registry, resolver, and
   conformance harness without changing existing version constants;
2. hard-cut Python and Go extraction and resolution to complete universal
   adapters;
3. hard-cut Java and Rust to complete universal adapters to prove the contract
   works across object-oriented, package-oriented, trait-oriented, macro, and
   ownership-heavy syntax;
4. hard-cut existing framework packs to the universal pack contract; and
5. hard-cut remaining adapters in prioritized batches without changing the
   resolver or publisher contract.

An adapter not yet selected for cutover continues to run its current algorithm
unchanged. It is not projected into universal evidence and cannot advertise
universal capabilities. Once selected, the old language-specific path is
removed in the same change that enables the universal path. Cached extractions
that lack required universal evidence are treated as cache misses and
re-extracted; the extraction-semantics version remains unchanged.

### Delivery decomposition

The universal program is too large for one undifferentiated implementation
plan. It is delivered as independently reviewable increments:

1. universal evidence model, capability registry, shared resolver,
   conformance harness, and direct Python/Go cutover;
2. Java and Rust hard cutover proving language portability;
3. universal framework-pack hard cutover; and
4. prioritized hard cutover of remaining language adapters.

Each increment has its own implementation-first plan, production changes,
post-implementation tests, real-corpus evidence, commit, and review gate. The
final universal quality claim requires increments one through three. Increment
four expands the set of capabilities covered by that claim without changing
the core contracts.

## Production improvement scope

### Occurrence-aware semantic comparison

The development comparator may recognize a stronger Compass relationship only
when source occurrence and compatible endpoints agree. Examples:

- `instantiates` may dominate a generic Graphify `calls` edge at the same
  occurrence;
- a qualified external endpoint may refute an unsafe same-name local target;
- a uniquely resolved owner may dominate a broader placeholder owner.

It may not erase a source occurrence, relationship-kind, language, module, or
target conflict. Rejected baseline evidence remains separately counted and
auditable.

### Python

Recover source-backed decorator, annotation, base-type, and imported callable
uses through direct imports, aliases, package initializers, and bounded
re-export chains. Emit each use at its own AST occurrence and lexical owner.
Built-ins and external symbols remain qualified when no repository definition
exists. The removed import-to-every-class expansion must not return.

### Go

Resolve imported field types and embeddings through explicit package paths.
Repair receiver ownership when declaration order, generated code, exported
case, or private case creates placeholders. Preserve standard-library and
external package identities rather than joining their terminal names to local
types. Cross-language terminal-name resolution is forbidden.

### Other languages

Java and Rust become complete vertical adapters in this phase, not only
regression languages. They prove that the universal contracts cover
namespaces, overloads, annotations, inheritance, interfaces, traits, impl
ownership, macros, imports, and external packages without Python- or Go-shaped
special cases.

JavaScript/TypeScript, Ruby, C#, PHP, Swift, C/C++, and the remaining
registered languages retain their current extraction algorithms until their
hard-cutover increments. They are not translated or dual-run. Their existing
graph output must not regress before cutover, and each later cutover uses the
same universal registry and resolver without central publisher changes.

### Framework packs

Existing Python web, Rails, Spring, Go web, Rust web, ASP.NET, Vapor,
TypeScript web, filesystem-route, enterprise-domain, config, and template
packs hard-cut to the universal pack interface. Activation, accepted evidence,
target constraints, occurrence policy, resource limits, and conformance
registration become uniform, and the replaced pack path is removed. Spring
and the Rust web packs provide the non-Python/Go vertical proof.

## Independent quality measurement

### Precision audit

Audit at least 2,000 Compass relationships using a deterministic stratified
sample across repositories, languages, relationship kinds, confidence tiers,
and high-frequency target clusters. Each record verifies independently:

- source owner;
- target identity;
- relationship kind; and
- occurrence range.

Any incorrect dimension makes the record incorrect. Report raw counts,
observed precision, sample composition, and a two-sided 95% Wilson confidence
interval. The sample contains at least 400 records per corpus, at least 100
records for each available required relationship family, and at least 100
records for every advertised adapter capability; one record may satisfy
multiple intersecting strata. No single repeated target cluster may supply
more than 10% of a stratum.

The delivery gates are at least 99.5% observed overall precision, a 95% Wilson
lower bound of at least 99%, and at least 99% observed precision for every
advertised language capability. Fabricated occurrences, cross-language
matches, and unsafe local-target substitutions have zero tolerance regardless
of aggregate precision.

### Recall audit

Recall candidates come from two independent pools:

1. source constructs collected independently of the published Compass graph;
2. Graphify-only facts treated as hypotheses.

Each uncovered candidate receives exactly one classification:

- genuine Compass omission;
- correct qualified external fact;
- ambiguous in source;
- wrong Graphify owner or target;
- wrong Graphify relationship;
- fabricated Graphify occurrence; or
- plausible dependency represented by Compass at a different real
  occurrence.

The report includes per-language and per-relation recall counts. It must not
convert ambiguous or rejected facts into silent passes. Every advertised
language or framework capability must recover at least 95% of its
source-derived oracle facts.

### Audit manifest

The checked-in audit manifest records:

- schema identifier;
- corpus name and commit;
- adapter, framework-pack, and advertised capability identities;
- language and relationship kind;
- source and target expectation;
- occurrence file and range;
- normalized source snippet hash;
- judgment and reason.

Qualification fails if the corpus revision differs, the snippet hash changes,
the manifest is too small, or required strata are absent. The harness must
produce the same sample and metrics for identical graph and corpus inputs.

## Confirming whether fewer edges are better

Removed edges are audited by file, target, relationship, and occurrence
clusters. A removed family is an improvement only when:

- its owner, target, relationship, or occurrence is invalid; and
- every recoverable real use in the audited source is represented separately
  or recorded as a Compass omission.

If a removed projected edge corresponds to a real later use that Compass does
not emit, the deletion improves occurrence precision but creates a recall
regression. The final report must state both outcomes. Net edge reduction is
not evidence of improvement.

## Corpora

Production improvements are qualified on:

- Django: Python improvement corpus;
- Entire: Go improvement corpus;
- Spring Framework: Java regression corpus; and
- Bevy: Rust regression corpus.

All corpora use pinned commits. Graphify and Compass must process the same
source revision. The existing Django URL-resolution/model-save, Entire
checkpoint/repository-state, Spring application-context/HTTP-dispatch, and
Bevy scheduling/plugin queries in `benchmarks/performance/repositories.toml`
are mandatory.

## Performance and correctness gates

- At least 99.5% observed overall precision on at least 2,000 audited edges.
- The two-sided 95% Wilson precision lower bound is at least 99%.
- Every advertised adapter capability has at least 99% observed precision and
  95% source-derived recall.
- Critical semantic violations have a count of zero.
- The report publishes confidence bounds and every incorrect sample.
- Genuine source-derived recall improves on Python and Go.
- Java and Rust pass the complete adapter contract and do not regress against
  their pinned pre-cutover graphs.
- Every cut-over framework pack passes activation, occurrence, target,
  resource-limit, and query-oracle conformance.
- All four qualification graphs publish with zero validation errors.
- Cold and warm graphs are byte-identical for every measured Compass corpus.
- All mandatory semantic query oracles pass.
- No cold or warm build regression greater than 10% without diagnosis and
  explicit acceptance.
- Focused owning-crate tests, the performance harness, and the full workspace
  test suite pass.

## Implementation order

Every delivery increment is implementation-first:

1. implement the universal evidence types, capability registry, constrained
   resolver, and conformance harness while keeping current version values;
2. hard-cut Python and Go adapters and production resolution to the universal
   path and remove the replaced algorithms;
3. hard-cut Java and Rust adapters and remove their replaced algorithms;
4. hard-cut framework-pack registration and enforcement to the universal
   contract and remove the replaced pack execution;
5. implement occurrence-aware comparison and the independent audit harness;
6. add conformance and focused regression tests for the completed production
   behavior;
7. run four-corpus qualification and critique the output; and
8. update documentation, commit, push, and update or create the pull request.

Tests still cover every production change, but they follow the initial
implementation rather than driving it.

## Error handling

- Resource or candidate limits fail closed with bounded diagnostics.
- Ambiguous resolution never selects an arbitrary target.
- Missing external definitions retain qualified unresolved endpoints.
- Invalid audit records fail qualification rather than being skipped.
- Unsupported language roles remain explicit and cannot fall through to
  terminal-name matching.
- Adapter and framework-pack capability claims that lack conformance evidence
  fail registration.
- An adapter that has not been cut over cannot advertise universal
  capabilities.
- A cut-over adapter missing universal evidence is re-extracted rather than
  interpreted through its prior algorithm.
- A partial graph or partial audit cannot satisfy the quality gate.

## Honest reporting requirements

The final review separates:

- precision improvements;
- recall improvements;
- representational changes;
- demonstrated Graphify errors;
- demonstrated Compass errors;
- unresolved and ambiguous facts;
- edge-count changes; and
- performance changes.

It must explicitly state whether strict Graphify-superset quality is achieved.
If not, it names the remaining genuine gaps and does not describe every
Graphify conflict as a Graphify false positive.
