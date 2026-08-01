# Rust Phase 2 and Java Universal Hard-Cutover Design

**Date:** 2026-07-30

**Branch:** `codex/rust-universal-adapter`

## Objective

Deliver a quality-gated Rust Phase 2 while Rust remains
`UniversalCandidate`, then establish Java as `UniversalCandidate`. Both
languages stay on adapter version 1 and hard-cut to the universal extraction,
resolution, and graph-projection path. Neither language is translated,
dual-run, or retained behind its previous direct publisher.

Peak resident memory is measured and reported but is not a blocking gate in
this phase. Graph quality, determinism, latency advantage, and regression
safety are blocking gates.

## Terminology and Language-by-Language Transition

Compass's existing Python, Go, JavaScript/TypeScript, Ruby, C#, PHP, Swift,
C/C++, and other production extractors are **established direct adapters**.
They remain supported, produce useful graphs today, and continue to receive
regression protection until each language independently qualifies for the
universal path.

The registry profile describes publication architecture, not implementation
quality or deprecation status:

- `Direct` means the established adapter publishes its current graph and
  unresolved-call records through the existing resolution path;
- `UniversalCandidate` means the adapter publishes universal evidence and has
  passed its language-specific candidate gates;
- `UniversalComplete` means the adapter has additionally passed the complete
  capability and conformance gates defined for that language.

The current internal `AdapterProfile::Legacy` name is replaced by
`AdapterProfile::Direct`. Serialized compatibility accepts the old `legacy`
spelling when reading existing metadata, but new metadata emits `direct`.
This terminology change does not alter adapter version 1, graph output, or a
language's support level.

Adoption proceeds as independent language transitions. It is not framed as
replacement of an inferior system:

1. The established direct adapter remains the production implementation while
   the language's universal increment is developed.
2. Development does not dual-run or translate production extraction.
3. Checked fixtures and a pinned real-world corpus establish the direct
   adapter's quality and performance baseline.
4. The language transitions atomically only after its universal candidate
   proves no normalized-output regression, better target quality, deterministic
   output, and acceptable performance.
5. Only that language's replaced direct publisher and resolver branches are
   removed. Every other language remains on its established direct path.
6. A language with no scheduled universal increment remains fully supported;
   grammar availability alone does not put it into a deprecated state.

Universal evidence is therefore a convergence architecture for independently
qualified languages, not a judgment that all current adapters need immediate
replacement.

## Layered Ownership

The vendored `compass-tree-sitter-language-pack` remains a grammar substrate,
not a Compass semantic extension system. It owns:

- the pinned grammar catalog and aliases;
- static parser linkage and build-time completeness checks;
- parser construction and grammar ABI compatibility;
- grammar provenance sufficient to invalidate extraction caches.

The vendored package must not depend on Compass evidence, graph, resolver, or
framework crates. Compass wraps it behind a small `GrammarProvider` boundary;
adapter code does not depend on vendor implementation details.

Compass owns the remaining layers:

1. The source registry maps a file to one producer descriptor.
2. The grammar provider prepares one parser and AST for an AST-backed
   producer.
3. The adapter-local semantic policy interprets syntax and emits one universal
   evidence batch.
4. The universal resolver resolves evidence across the collection.
5. The universal projector publishes the normalized Compass graph.
6. Framework packs target normalized declarations and exact occurrences.

Non-AST producers skip the grammar layer and enter at step 3. The universal
evidence contract therefore remains useful for configuration, template,
manifest, and document sources without pretending that those sources have a
Tree-sitter grammar.

## Architectural Choice

The universal evidence API, bounded resolver, and graph projector are
source-agnostic and serve every registered language adapter. Rust and Java
are the first hard-cutover consumers in this delivery and populate the shared
contract from their existing Tree-sitter AST traversals. Later AST-backed
languages reuse the same emitter helpers. Non-AST extractors, including
configuration, template, manifest, and document producers, submit the same
evidence contract from their existing extraction algorithms.

Each language supplies syntax classification and identity normalization
through an adapter-local policy. The central resolver and publisher operate
only on declarations, scopes, bindings, occurrences, and relationship
candidates; they do not branch on Rust, Java, or any other language.

Languages outside this phase retain their current extraction and graph output
until their own hard-cutover increments. They are not translated or dual-run.
Registering a later adapter must not require a central resolver or publisher
change.

This preserves exact AST ownership and byte ranges while avoiding a second
parser. It also avoids deriving evidence from established direct graph
records, which would lose lexical scope and occurrence identity.

The rejected alternatives are:

- storing Compass adapter descriptors or semantic registration in the
  vendored language pack, because grammar upgrades must remain independent of
  Compass graph semantics;
- converting the language pack's generic `process()` output into universal
  evidence, because that introduces a lossy translation path and cannot
  preserve adapter-specific lexical identity;
- a dedicated Java graph extractor that duplicates the generic traversal and
  creates a second publication model;
- post-hoc evidence reconstruction from established direct nodes and edges;
- a benchmark-specific patch layer that adds Rust or Java name-resolution
  rules directly to the central publisher.

## Grammar Provider Contract

The Compass-side `GrammarProvider` exposes only grammar concerns:

- load a grammar by its canonical grammar ID;
- report whether that grammar is statically available;
- return grammar provenance consisting of grammar ID, language-pack version,
  pinned parser-source identity, and Tree-sitter ABI version;
- create or configure a parser without enabling runtime download or dynamic
  loading.

The default provider wraps the vendored static language pack. An AST-backed
adapter receives borrowed source bytes, the prepared Tree-sitter root, and
immutable grammar provenance. It never invokes the language pack's generic
intelligence `process()` pipeline and never parses the source again.

Grammar provenance is stored with extraction provenance and participates in
cache identity. A grammar revision changes cache identity, but does not by
itself change `compass.languages.evidence/1` or an adapter version. The
language-pack crate version must change when its pinned parser-source identity
changes.

## Producer Registration Contract

Compass owns a single producer registry. Each source registration resolves to
a `ProducerDescriptor` containing:

- source identity and extractor kind;
- an optional canonical grammar requirement;
- adapter ID, adapter version, profile, and evidence schema;
- declared semantic capabilities;
- evidence, scope, import-expansion, overload, and occurrence budgets.

Every AST-backed producer declares exactly one grammar requirement. Every
source-driven producer declares none. Registry validation fails before
extraction when an AST-backed producer requires a grammar that the static
provider cannot supply.

The registry distinguishes five states rather than conflating parser support
with semantic support:

1. grammar available;
2. source recognized;
3. established direct extraction available;
4. universal candidate;
5. universal complete.

For every source that passes grammar and parser preparation,
`UniversalCandidate` and `UniversalComplete` producers emit exactly one
`compass.languages.evidence/1` batch. A fatal substrate failure instead
returns a structured extraction error and no batch. Capability claims become
valid only when adapter conformance proves the corresponding evidence. Adding
a grammar alone never promotes semantic support. Adding a later universal
adapter may extend source registration and adapter-local policy, but cannot
require a shared resolver or projector change.

## Universal Adapter Contract

Every hard-cut adapter uses `compass.languages.evidence/1` and the shared
resolver/projector interfaces. Rust and Java both remain adapter version 1 and
use the `UniversalCandidate` profile in this delivery.

The shared, source-agnostic evidence builder provides bounded methods for:

- declaration facts with stable symbol identity, owner, kind, and exact
  source anchor;
- lexical scope facts with explicit parentage;
- import, alias, module, and package bindings;
- call, import, type-reference, trait/interface-bound, annotation, and macro
  occurrences;
- relationship candidates with allowed target kinds and explicit external
  identity.

Every evidence collection is deterministically sorted and deduplicated before
resolution. Limits apply per source file to declarations, scopes, bindings,
occurrences, candidates, scope depth, import expansion, and overload
candidates. Exceeding a limit produces a diagnostic and prevents that file's
partial universal graph projection from being reported as complete.

After hard cutover, an adapter emits no final graph nodes, graph edges, or
`RawCall` records. A local projection requested by a single-file API is still
performed by the shared universal projector, never by the adapter.

## Rust Phase 2

Rust remains adapter version 1 and `UniversalCandidate`. The phase adds and
advertises capabilities only after their evidence is emitted:

- declarations for modules, traits, structs, enums, type aliases, functions,
  methods, fields, constants, and macros;
- lexical scopes for modules, traits, impl blocks, and callables;
- namespace identity for modules and qualified paths;
- trait and impl ownership;
- `use` bindings, including nested trees, aliases, `crate`, `self`, `super`,
  globs, and external package roots;
- exact type-reference, trait-bound, macro-invocation, import, and call
  occurrences;
- unique local and imported relationship candidates plus qualified external
  identities.

The universal resolver projects only unique, scope-valid targets. Qualified
external identities remain explicit unresolved targets when they prevent a
false local terminal-name binding. Ambiguous traits, methods, imports, and
type names remain candidates without graph edges.

Rust hard-cuts atomically:

- the registry selects only the version-1 universal Rust adapter;
- the universal projector is the only producer of Rust graph facts;
- the replaced Rust-specific semantic-enrichment and publication path is
  removed;
- Rust framework evidence consumes the resulting universal graph contract.

### Rust Blocking Gates

- All checked Rust fixtures retain their existing normalized graph facts.
- The pinned Bevy strict edge coverage exceeds 69.64%.
- Bevy call coverage remains at least 88.67%.
- Three repeated cold, warm, incremental, and restore extractions are
  deterministic.
- Compass remains faster than Graphify for comparable cold and warm Bevy
  workloads.
- All language, resolver, framework, and normalization suites pass.
- Peak RSS is recorded with the results but does not block the cutover.

## Java Universal Candidate

Java stays adapter version 1 and becomes `UniversalCandidate` through the same
emitter, resolver, and projector used by Rust.

The Java adapter emits:

- package and nested-type namespace scopes;
- class, interface, enum, record, annotation-type, constructor, method, field,
  and enum-constant declarations;
- stable overload identities composed from lexical owner, callable name,
  constructor/method role, and normalized parameter signature;
- normal, static, and wildcard import bindings;
- annotations and their exact occurrence sites;
- `extends`, `implements`, type-reference, constructor-call, method-call, and
  external-package candidates;
- exact scopes for packages, types, nested types, constructors, methods, and
  initializer blocks.

Resolution requires compatible declaration kind, package/import visibility,
lexical scope, owner, and overload signature. A call without enough argument
or receiver evidence to select one overload remains unresolved. Static
imports and wildcard imports expand only within configured candidate limits.

Java hard-cuts atomically:

- the registry selects only the version-1 universal Java adapter;
- existing generic Java AST recognition may be reused inside the adapter, but
  its replaced direct graph publisher is removed;
- no Java dual-run or graph reconciliation step remains;
- Spring's Java-facing evidence and target resolution consume the universal
  contract rather than a Java-specific publisher path.

### Java Blocking Gates

- Existing Java and Spring fixtures retain their normalized graph facts.
- Java overloads, packages, annotations, inheritance, interfaces, imports,
  calls, and external targets pass exact-anchor conformance cases.
- Three repeated Spring cold, warm, incremental, and restore extractions are
  deterministic.
- Strict Graphify coverage on the pinned Spring corpus improves over the
  pre-cutover baseline with no relation-family regression.
- Compass remains faster than Graphify for comparable cold and warm Spring
  workloads.
- All language, resolver, Spring framework, and normalization suites pass.
- Peak RSS is recorded but does not block candidate status.

## Shared Resolver and Projector

The resolver constructs immutable, bounded indexes keyed by:

- language and repository scope;
- lexical scope and parent scope;
- declaration kind and normalized identity;
- package/module identity;
- imported spelling and alias;
- callable owner, name, and normalized signature.

Resolution order is exact local owner, exact lexical scope, explicit import,
same package/module, and then explicit external identity. Wildcard or terminal
matches never outrank exact evidence. Multiple surviving candidates produce
an unresolved result.

The projector converts resolved evidence into the existing normalized graph
contract. Relationship sites retain the occurrence's exact source file and
byte range. Containment derives from declaration ownership and scope
parentage. The projector does not manufacture edges from unresolved
candidates.

## Performance Contract

AST-backed extraction parses each file exactly once. Adapters may make bounded
subpasses over the prepared tree when semantic ordering requires them, but
they may not reparse source or run the language pack's generic intelligence
pipeline.

The evidence builder interns repeated identities and spellings in a bounded
per-file arena during construction. The serialized version-1 evidence shape
may expose owned strings at its storage boundary, but in-process extraction,
resolution, and projection do not round-trip evidence through JSON or another
serialization format. Corpus merge transfers evidence and graph buffers by
ownership wherever the caller no longer needs the per-file extraction.

Parsing, evidence emission, collection resolution, graph projection, and
persistence have separate timing counters. Qualification records these
counters along with total latency and peak RSS. The design adopts the useful
performance principles of a native extraction kernel—one parse, compact
storage, bounded work, and delayed materialization—without adopting direct
per-language graph publication or a fallback publisher.

Peak RSS remains non-blocking for Rust Phase 2 and Java candidate status, but
bounded evidence collections, scope depth, import expansion, and overload
candidate counts are mandatory correctness constraints.

## Framework Integration

Rust web and Spring packs consume uniform universal evidence and normalized
graph facts. Their activation evidence, target constraints, occurrence
policy, resource limits, and conformance registration remain pack-owned, but
they no longer depend on a replaced Rust- or Java-specific publisher.

This phase does not hard-cut unrelated framework packs or languages.

## Failure Behavior

Failures have three explicit classes:

- **fatal substrate or structure failure:** missing grammar, grammar ABI
  mismatch, parser creation failure, null parse tree, invalid anchors, missing
  owners, scope cycles, or conflicting symbol identities produce a structured
  diagnostic and no universal projection for that file;
- **bounded incomplete evidence:** Tree-sitter error ranges or resource-limit
  exhaustion allow only individually validated facts outside the affected
  range or exhausted collection to project, mark the batch incomplete, and
  prevent it from satisfying a completeness gate;
- **semantic ambiguity:** multiple scope-valid overload, import, trait,
  receiver, or type candidates preserve the occurrence and candidates but
  publish no relationship edge.

An adapter must not emit evidence whose anchor overlaps an untrusted
Tree-sitter error range. Incomplete or ambiguous evidence never triggers
terminal-name fallback. A hard-cut adapter never falls back to its previous
direct publisher, and a framework pack cannot repair or reinterpret
malformed language evidence.

A hard cutover is not merged when a blocking gate fails. Because no dual-run
path is retained, rollback is the Git commit boundary rather than a runtime
feature flag.

## Verification Policy

Implementation is not test-driven. Production changes are implemented first,
then exercised with targeted conformance tests and broader regression suites.

Verification proceeds in this order:

1. grammar substrate tests prove every advertised static grammar loads, every
   registered grammar requirement is satisfiable, runtime download and
   dynamic loading remain disabled, and grammar provenance is stable;
2. universal contract tests prove deterministic ordering, exact UTF-8
   anchors, version-1 compatibility, resource bounds, and use by both AST and
   non-AST producers;
3. adapter-local Rust or Java conformance tests prove capability claims,
   malformed-source behavior, Unicode anchors, and absence of adapter-emitted
   graph nodes, edges, and raw calls after hard cutover;
4. full `compass-languages` tests;
5. full `compass-resolve` tests;
6. Rust web or Spring framework tests;
7. deterministic repeated fixture extraction;
8. normalized fixture comparison with Graphify;
9. pinned Bevy or Spring qualification, including cold, warm, incremental,
   restore, strict graph coverage, stage timings, and peak RSS;
10. `cargo fmt --all -- --check`, `git diff --check`, and
   `graphify update .`.

The evidence report distinguishes official multi-sample qualification from
single-sample diagnostic probes and does not equate additional graph facts
with correctness.

Changing a pinned grammar requires parser-source and ABI verification plus
affected-language fixture qualification. It invalidates extraction caches
through grammar provenance and does not change the universal evidence schema
or adapter version unless the semantic contract itself changes.

## Delivery Sequence

1. Add the Compass-side grammar-provider boundary, grammar provenance, and
   producer-registry validation without changing the vendored package's
   semantic responsibilities.
2. Add the shared bounded evidence-emitter and universal resolver/projector
   interfaces.
3. Complete Rust Phase 2 evidence and hard-cut Rust.
4. Verify and qualify Rust against fixtures, Bevy, and Graphify.
5. Add Java adapter policy and evidence on the same interfaces.
6. Hard-cut Java and Spring's Java-facing integration.
7. Verify and qualify Java against fixtures, Spring, and Graphify.
8. Record remaining quality and RSS gaps without promoting either adapter to
   `UniversalComplete`.
