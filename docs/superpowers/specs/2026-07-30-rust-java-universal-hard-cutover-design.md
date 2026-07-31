# Rust Phase 2 and Java Universal Hard-Cutover Design

**Date:** 2026-07-30

**Branch:** `codex/rust-universal-adapter`

## Objective

Deliver a quality-gated Rust Phase 2 while Rust remains
`UniversalCandidate`, then establish Java as `UniversalCandidate`. Both
languages stay on adapter version 1 and hard-cut to the universal extraction,
resolution, and graph-projection path. Neither language is translated,
dual-run, or retained behind a legacy publisher.

Peak resident memory is measured and reported but is not a blocking gate in
this phase. Graph quality, determinism, latency advantage, and regression
safety are blocking gates.

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
parser. It also avoids deriving evidence from legacy graph records, which
would lose lexical scope and occurrence identity.

The rejected alternatives are:

- a dedicated Java graph extractor that duplicates the generic traversal and
  creates a second publication model;
- post-hoc evidence reconstruction from legacy nodes and edges;
- a benchmark-specific patch layer that adds Rust or Java name-resolution
  rules directly to the central publisher.

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
  its legacy graph publisher is removed;
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

## Framework Integration

Rust web and Spring packs consume uniform universal evidence and normalized
graph facts. Their activation evidence, target constraints, occurrence
policy, resource limits, and conformance registration remain pack-owned, but
they no longer depend on a replaced Rust- or Java-specific publisher.

This phase does not hard-cut unrelated framework packs or languages.

## Failure Behavior

Malformed source may produce bounded partial evidence with diagnostics.
Invalid anchors, missing owners, scope cycles, duplicate conflicting symbol
identities, and resource-limit exhaustion fail closed for universal
projection. They do not trigger terminal-name fallback.

A hard cutover is not merged when a blocking gate fails. Because no dual-run
path is retained, rollback is the Git commit boundary rather than a runtime
feature flag.

## Verification Policy

Implementation is not test-driven. Production changes are implemented first,
then exercised with targeted conformance tests and broader regression suites.

Verification proceeds in this order:

1. adapter-local Rust or Java conformance tests;
2. full `compass-languages` tests;
3. full `compass-resolve` tests;
4. Rust web or Spring framework tests;
5. deterministic repeated fixture extraction;
6. normalized fixture comparison with Graphify;
7. pinned Bevy or Spring qualification, including cold, warm, incremental,
   restore, strict graph coverage, and peak RSS;
8. `cargo fmt --all -- --check`, `git diff --check`, and
   `graphify update .`.

The evidence report distinguishes official multi-sample qualification from
single-sample diagnostic probes and does not equate additional graph facts
with correctness.

## Delivery Sequence

1. Add the shared bounded evidence-emitter and universal resolver/projector
   interfaces.
2. Complete Rust Phase 2 evidence and hard-cut Rust.
3. Verify and qualify Rust against fixtures, Bevy, and Graphify.
4. Add Java adapter policy and evidence on the same interfaces.
5. Hard-cut Java and Spring's Java-facing integration.
6. Verify and qualify Java against fixtures, Spring, and Graphify.
7. Record remaining quality and RSS gaps without promoting either adapter to
   `UniversalComplete`.
