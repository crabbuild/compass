# Universal semantic evidence

Compass resolves source relationships through a language-neutral evidence
contract. Python, Go, Rust, and Java use this contract in production. Other
languages keep their existing extractors until a dedicated change removes the
old algorithm and adds the universal adapter atomically.

This is a hard-cutover interface. It has no raw-fact translation layer, shadow
mode, terminal-name fallback, or runtime dependency on Graphify.

## Evidence contract

`SemanticEvidenceBatch` is the unit stored with one source extraction. Its
`AdapterIdentity` names one language and producer and lists only capabilities
the adapter actually emits. The batch contains six bounded collections:

- `DeclarationFact` identifies a source-backed declaration and its existing
  graph node, kind, name, qualified name, optional module/package, lexical
  scope, signature, optional bounded canonical parameter-type vector, optional
  complete-direct-base marker for Java types, and exact range.
- `ScopeFact` identifies a lexical scope, optional owning declaration, parent
  scope, and exact range.
- `BindingFact` records an import, import alias, re-export, local alias,
  call-result, or package binding. Its target is qualified; a proven local
  declaration may also be named directly. A call-result binding names the
  exact callable that initialized a receiver and may record the zero-based
  output selected by a destructuring assignment. It may preserve an exact
  nominal result type proven in the same file, reference an earlier
  call-result binding to represent a bounded receiver chain, and retain one
  non-call-result fallback binding for incomplete project-wide evidence. Chain
  references must exist, remain acyclic, and stay within the evidence depth
  limit. Resolution otherwise requires a unique
  callable and either one published return type or an in-range exact output
  position; unpositioned multi-result calls remain unresolved. A return
  candidate's proposed qualified name is not authoritative receiver evidence:
  downstream call-result resolution may use it only after the candidate
  resolves to an exact source declaration or an explicitly qualified external
  identity. An unresolved prelude or imported spelling must not be
  reinterpreted as a repository-local type.
- `OccurrenceFact` records one exact use site and its role, owner, spelling,
  optional qualifier, lexical scope, and range. Repeated uses are separate
  occurrences.
- `RelationshipCandidate` connects a source declaration and occurrence to a
  typed relationship plus target constraints. It is evidence awaiting
  resolution, not permission to choose a convenient node. When the parser has
  already identified the target declaration, `exact_target_declaration_id`
  preserves that identity; downstream resolution must not replace it with a
  same-named overload. A callable candidate may carry a bounded positional
  vector of optional canonical argument types. The shared resolver selects an
  overload first through a uniquely identical parameter vector. If every
  argument type is known, Java may then use proven primitive widening,
  boxing/unboxing, array, complete source-hierarchy, and stable core-Java
  conversions, but only when one applicable vector is more specific than all
  other applicable vectors. Unknown hierarchy or a competing conversion
  remains unresolved.
- `EvidenceDiagnostic` records a bounded extraction problem without creating
  a graph fact.

Every fact ID is a deterministic SHA-256 identity over length-prefixed inputs:
the unchanged extraction-semantics identity, language, producer, source path,
fact category, semantic role, relevant names, and source range. Builders sort
and deduplicate typed facts before validation.

Rust generic parameters are source-backed `parameter` declarations. Type,
lifetime, and const parameters receive owner-qualified identities and lexical
scope; repeated spellings in a type, implementation, and method remain
distinct. Proven signature, field, return, and bound occurrences target the
parameter declaration rather than an unrelated same-named type.

Rust associated types are source-backed `type_alias` declarations scoped to
their trait or exact implementation. A `Self::Type` occurrence retains the
exact receiver declaration and implemented trait. The resolver may follow a
complete, bounded source-proven supertrait hierarchy and select the one
associated realization declared by that exact receiver. Competing traits,
duplicate realizations, incomplete or external hierarchy branches, and missing
declarations remain unresolved. The associated type's value is represented
separately by its source-anchored reference, preserving the distinction between
the contract and its concrete realization.

Rust `self.method(...)` calls use the complete method declarations collected
for the source receiver. A unique declaration may resolve exactly. When more
than one local trait or implementation method has the same spelling and the
available argument evidence cannot choose among them, the call remains
unresolved; it must not fall through to an external or deferred placeholder.
The absence of a local method with that spelling does not suppress a genuinely
external inherent method on an external receiver.

Rust typed receivers follow the same provenance rule across parameters,
locals, fields, and callable results. An unqualified prelude spelling is not a
repository-local receiver unless a source declaration or explicit import
proves that ownership. Unshadowed `Option` and `Result` use their canonical
standard-library identities; other unproven spellings remain unresolved
rather than becoming crate-qualified placeholders.

### Required invariants

`validate_evidence` rejects a batch when any of these conditions is false:

- the adapter language and producer are non-empty;
- fact IDs are unique and all references resolve;
- source paths are normalized, relative, and remain inside the corpus;
- ranges are non-empty, byte ordered, and line/column ordered;
- every fact and exact-language constraint agrees with the adapter language;
- behavioral candidates have an exact occurrence;
- each binding, role, and relationship is covered by an advertised
  `LanguageCapability`;
- external resolution is used only by an adapter advertising
  `ExternalReferences`; and
- declarations, scopes, bindings, occurrences, candidates, diagnostics, and
  diagnostic messages remain within `EvidenceLimits`.

Callable type vectors are limited to 256 positions and 1,024 bytes per known
type identity. Their length must equal the source-level parameter or argument
count. Unknown argument positions are explicit rather than guessed.

`directBasesComplete` is valid only on Java class, interface, enum, record, or
annotation-type declarations. It is false when parser recovery overlaps the
declaration. The resolver may traverse source inheritance only while every
visited declaration proves that its complete direct-base set was emitted.

Validation traverses each collection by stable fact ID, so equivalent input
orders return the same first error.

## Adapter registration

`AdapterRegistry::universal_profile(language)` is the authority for universal
cutover. A returned `AdapterProfile` means universal evidence is mandatory.
Python, Go, Rust, and Java are currently registered. An unregistered language
does not silently claim universal behavior.

An adapter profile must:

1. use the same normalized language name as `Registry`;
2. advertise a non-empty, sorted, duplicate-free capability list;
3. emit every advertised role and relationship directly from parser or
   source-driven facts;
4. validate every completed batch; and
5. stay within the shared evidence limits.

Tree-sitter and source-driven adapters produce the same records. Tree-sitter
adapters normally obtain byte and line coordinates with `range_for_node`.
A source-driven parser constructs the identical `EvidenceRange` from its own
token offsets. Downstream resolution does not branch on parser technology.

### Minimal in-tree adapter

This abbreviated example shows the production API shape. The language must
also have a real extractor and registry case; registering a profile without
direct emission is invalid.

```rust
use compass_languages::{
    AdapterProfile, AdapterRegistry, CandidateRelation, EvidenceBuilder,
    EvidenceLimits, EvidenceRange, LanguageCapability, ResolutionConstraint,
    SemanticRole,
};

const WREN_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Calls,
];

static WREN_PROFILE: AdapterProfile = AdapterProfile {
    language: "wren",
    capabilities: WREN_CAPABILITIES,
};

let function_range = EvidenceRange {
    source_file: "src/main.wren".to_owned(),
    start_byte: 0,
    end_byte: 30,
    start_line: 1,
    start_column: 0,
    end_line: 3,
    end_column: 1,
};
let call_range = EvidenceRange {
    source_file: "src/main.wren".to_owned(),
    start_byte: 17,
    end_byte: 23,
    start_line: 2,
    start_column: 2,
    end_line: 2,
    end_column: 8,
};

let mut builder = EvidenceBuilder::new(
    &WREN_PROFILE,
    "compass.languages.wren",
    "src/main.wren",
    EvidenceLimits::default(),
);
let function = builder.declare(
    "function",
    "existing-graph-node-id",
    "run",
    "main.run",
    Some("main"),
    None,
    function_range.clone(),
)?;
let scope = builder.open_scope("function", Some(&function), None, function_range)?;
let call = builder.occur(
    SemanticRole::Call,
    &function,
    "work",
    None,
    Some(&scope),
    call_range,
)?;
builder.relate(
    CandidateRelation::Calls,
    &function,
    Some(&call),
    None,
    "work",
    ResolutionConstraint {
        exact_language: Some("wren".to_owned()),
        module_or_package: Some("main".to_owned()),
        scope_id: Some(scope),
        allowed_target_kinds: vec!["function".to_owned()],
        ..ResolutionConstraint::default()
    },
)?;
let batch = builder.finish()?;
```

Inside `adapters.rs`, add the profile to the sorted
`UNIVERSAL_ADAPTERS` table only in the same change that connects extraction
and removes the replaced resolver path. `AdapterRegistry::validate()` must
then pass.

## Resolution

`UniversalResolutionIndex` validates and merges batches, bounds every index,
sorts candidate identities, and applies this order:

1. typed hierarchy policy for direct bases and receiver dispatch;
2. explicit import, alias, or re-export binding attached to the occurrence;
3. exact declaration in the lexical scope or a parent scope;
4. exact qualified identity, explicit member binding, or bounded wildcard
   re-export chain;
5. unique same-module or same-package declaration;
6. source-scoped qualified external endpoint when explicitly allowed;
7. ambiguous or unresolved.

Explicit bindings precede lexical lookup because a source import is direct
use-site evidence and must shadow a same-named enclosing declaration. A
binding can name an exact declaration, qualified declaration, or source
inventory endpoint. Resolution carries the exact target source temporarily
through collision disambiguation and removes that internal attribute before
publication. An identity alias such as `pkg.signals -> pkg.signals` is a
terminal mapping; a multi-node alias cycle remains ambiguous and fails
closed.

Rust glob imports are collective search scopes rather than ordered aliases.
When more than one glob is visible at a lexical use site, the resolver unions
the bounded repository-local declarations exposed by that scope and its
bounded re-export chains. It publishes a relationship only when one compatible
declaration remains. Two different eligible declarations, an incomplete
scope/module search, or an unproven external lowercase symbol remains
ambiguous or unresolved; source order never breaks the tie.

For a Rust child module, `use super::*` also exposes the bounded names imported
by its source-proven parent module, including a private parent glob import.
That parent scope is traversed only when it is an ancestor of the use site;
external, truncated, or competing glob scopes still fail closed.

Wildcard completeness may cross a Rust crate boundary only when the exact
target has a file, module, or enum declaration in the indexed source set. A
source-present sibling crate therefore participates in the bounded union of
visible glob scopes, while an unknown dependency keeps that union incomplete.
The resolver never assumes that a different crate root is external or that a
named dependency is local solely from its spelling.

For `impl Trait<Argument> for Implementer`, an implementer declaration proven
in the current source evidence owns an exact type-reference occurrence for
every non-primitive nested trait argument. Lookup uses the implementation
scope rather than the implementer's declaration scope, preserving imported
types and implementation-local type parameters. The outer trait remains represented
only by the `implements` candidate, so the same source token is not also
published as a generic reference. If the adapter cannot prove the implementer
declaration, the parser recovered through an error, or an argument has
competing bindings, Compass does not invent a reference endpoint.

When the implementer is an impl-scoped generic parameter, as in
`impl<T> Trait for T`, the exact scoped parameter declaration owns the
`implements` occurrence. Compass does not collapse that declaration into a
same-named type elsewhere in the repository. Ambiguous trait imports and
parser recovery overlapping either side of the implementation header fail
closed.

Python file imports are visible at module scope. Function- and class-local
imports are indexed only in their owning lexical scope, so they cannot leak to
sibling functions or become file-owned facts. Each imported item retains its
own parser range. Python source is not reparsed by the collection resolver,
and there is no legacy import projection. A static `from package import *`
statement emits one source-anchored wildcard binding. Package `__init__.py`
wildcards can expose a uniquely declared repository-local symbol through a
bounded re-export chain; multiple wildcard sources or multiple eligible
declarations remain ambiguous rather than selecting an arbitrary target.
An unconditional module-level alias created by an imported
`functools.partial` is callable evidence only when its first argument is one
unique module-level function declaration and neither name was rebound before
the assignment. Compass publishes the alias as a source-backed function and
an exact reference to the wrapped function. Dynamic targets, conditional
assignments, shadowed factories, and ambiguous declarations fail closed.
One unconditional module-level assignment to an identifier publishes an exact
Python variable declaration when the name has no competing module binding,
deletion, import shadow, parser recovery, or proven callable-alias kind. A
direct initializer call adds `type_of` evidence only when its target resolves
to one source-backed class; functions and unresolved external factories do not
invent a type. Function- and class-local shadows do not invalidate the module
declaration. Explicit receiver calls such as `self.settings()` never borrow a
same-named unqualified import as their target.
Zero-argument Python `super()` calls use source-proven C3 dispatch after the
enclosing class. Compass may cross multiple source-backed bases only when the
required base sets are complete and uniquely resolved for an exact target. A
known later member behind unknown preceding ancestry may be published only as
an `INFERRED` possible dispatch. Cycles and ambiguous members fail closed.
Exact Python calls may form self-loops when the occurrence resolves to its
owning function or method, so direct recursion remains visible in the graph.
Python's statically local parameters and assignment targets shadow module and
enclosing declarations even when the assignment follows the use. `global` and
`nonlocal` directives alter that lookup as defined by the source; unresolved
local callable values remain unpublished rather than becoming false recursive
calls. Graph entity deduplication preserves an original `calls` self-edge but
does not turn two distinct pre-deduplication endpoints into recursion.

Language and allowed target kinds are filtered before uniqueness is decided.
Case-insensitive or terminal-name equality cannot select a target. Cross-
language candidates cannot resolve. Candidate lists are bounded before they
can affect memory or runtime.

Materialization preserves the occurrence range and resolution rule. Exact
local targets remain extracted evidence. Qualified external targets are
source-scoped and inferred; they never substitute a same-named repository
node.

Python callable-value occurrences publish a reference candidate for the exact
value use. They do not publish an indirect call merely because that value
resolves to a function or method: target identity and callability do not prove
invocation. An indirect call requires separate source or contract evidence that
the receiving API invokes the value. The current Python adapter does not infer
such contracts, so uninvoked argument, collection, assignment, and return uses
remain references.

Python call syntax does not prove whether its target is a function or class,
and capitalization is not semantic evidence. The adapter therefore emits a
call candidate that permits either callable declaration kind. Publication
normalizes a uniquely resolved class target to `instantiates`; an unresolved
target remains a call-shaped external or unresolved endpoint rather than an
invented constructor.

The following resolution behavior is forbidden:

- repository-wide terminal-label search;
- picking the first candidate after sorting;
- resolving across languages;
- projecting an import line onto every imported symbol;
- inventing a call, reference, owner, or occurrence;
- using Graphify at runtime; and
- falling back to a removed language-specific resolver.

### Owner-qualified receiver dispatch

Receiver dispatch uses typed `HierarchyConstraint` values on the same
language-neutral candidate contract:

- every ordered direct-base occurrence carries
  `DirectBase { base_set_complete }`;
- a member use carries `ReceiverDispatch` with an exact receiver identity and
  a registered linearization strategy; and
- the adapter must advertise `HierarchyDispatch`, which also makes cached
  evidence lacking these facts ineligible for reuse.

The shared resolver builds bounded direct-base and directly-owned-member
indices. Direct-base candidates are resolved by exact qualified identity
before all lexical and module rules. They may publish an exact source class or
a qualified external endpoint, but can never bind to a convenient same-named
local class.

`C3FromReceiver` checks the exact receiver first. It may then follow a
source-proven prefix: a member declared directly by the exact first base is
ordered before later bases, and a single-inheritance chain remains proven until
it reaches a multiple-base fork. If that prefix does not resolve the member,
the resolver requires the complete C3 linearization and selects the first class
that directly declares it. Python emits this strategy for `self.member(...)`
and `cls.member(...)`; neither form can fall through to lexical, imported,
module, or repository-wide terminal-name lookup.

`C3AfterReceiver` is used for zero-argument `super().member(...)`. It first
checks the exact first base when that direct successor is source-proven, then
uses the complete C3 linearization while skipping the receiver. Full C3
resolution requires every base to resolve uniquely, an acyclic and
C3-consistent hierarchy, a bounded traversal, and one unique selected member.

Receiver dispatch also preserves bounded runtime alternatives without
relabeling them as exact. The resolver builds a deterministic reverse-subtype
index. For each source-defined descendant with a complete valid C3 order, it
selects the runtime member for `self`/`cls`, or the first member after the
defining receiver for zero-argument `super()`. Every distinct proven target is
published with `INFERRED` confidence and the
`closed-world-receiver-dispatch` rule. If earlier ancestry is unknown but a
later direct base uniquely declares the member, that target may be published
with `INFERRED` confidence and the
`incomplete-hierarchy-receiver-dispatch` rule. These edges mean "possible for
this proven hierarchy," not "the only runtime target." An existing exact edge
is retained separately. The same inferred rule applies when a source-defined
descendant has unresolved external ancestry but a direct member on its first
source base is ordered before the defining mixin whenever that descendant can
be linearized. A fully source-known C3 inconsistency is invalid, not
incomplete, and publishes no possible edge.

The old single-direct-base and terminal-name shortcuts have been removed.
Multiple inheritance and methods inherited beyond the direct base use the same
shared C3 implementation.
Dynamic receiver bases and incomplete receiver base lists cannot produce an
exact target. Explicit-argument `super`, inconsistent hierarchies, ambiguous
members, and bound overflows remain unresolved and cannot produce possible
dispatch edges. An unresolved or external later ancestor blocks full C3
recovery but does not invalidate a unique member declared directly on the
exact first base. Repository-wide name equality is never evidence for a
possible target. Nested sibling bases use their enclosing class identity. A
receiver-dispatch candidate cannot also carry a qualified target and cannot
fall through to a same-named local or imported symbol.

Future adapters reuse the typed boundary with evidence appropriate to their
language: compiler-selected overloads, trait or interface order, typed
receivers, or statically resolved superclass members. A new language semantic
must add and qualify its own explicit strategy; it must not reinterpret C3 or
guess an owner because a member name is repository-unique. No version value
changes for this extension.

### Offline compiler evidence

Offline SCIP batches are Program evidence, not `SemanticEvidenceBatch`
records. For Java calls, Compass can join their symbol identities to this
contract without weakening its invariants: the compiler
reference must match an adapter-emitted call occurrence by normalized source
path and exact half-open byte range, and the compiler definition must match an
adapter-emitted declaration the same way. Only fresh, unanimous, locally
defined targets are projected. Non-call references, stale or unverified
documents, ambiguous definition anchors, and provider conflicts do not create
or retarget an edge.

The resulting graph provenance has artifact origin and the
`compiler-exact-anchor` rule while retaining the adapter's exact occurrence as
the relationship site. Tree-sitter-only builds and `--no-program` builds keep
the existing structural behavior.

The compact projection type also admits `Project` providers, but the current
pipeline does not run `javac`, JDT, or a language server. Adding one requires a
separately bounded and qualified `ProjectAnalyzer`; parser availability alone
does not enable it.

## Framework pack registration

`FrameworkPackDescriptor` is the registration contract for framework meaning
derived from universal language evidence. It declares:

- stable pack ID and source, config, or template kind;
- universal languages, required language capabilities, and framework
  capabilities;
- dependency markers and manifest policy;
- named activation rules;
- accepted semantic roles and typed framework relation families;
- exact-evidence or exact-anchored-heuristic occurrence policy; and
- nonzero candidate, include-depth, alias-expansion, and per-file fact limits.

`FrameworkPackRegistry::validate_descriptors` rejects duplicate IDs,
unregistered languages, unsupported capabilities, missing roles or relations,
capability bypasses, unnamed heuristic activation, required manifests without
dependency markers, invalid ordering, and zero limits.

The internal framework-pack runtime also registers established source,
configuration, and template adapters in the same deterministic table. Those
adapters retain their established semantics, while the descriptor registry
continues to validate only universal evidence-backed packs. A future framework
change adds its adapter to the runtime and, when it is universal, adds the
descriptor and project-wide expansion adapter in the same change.

### Minimal framework descriptor

```rust
use compass_languages::{
    FrameworkCapability, FrameworkLimits, FrameworkManifestPolicy,
    FrameworkOccurrencePolicy, FrameworkPackDescriptor, FrameworkPackKind,
    FrameworkPackRegistry, FrameworkRelation, LanguageCapability, SemanticRole,
};

let descriptor = FrameworkPackDescriptor {
    id: "example-python-handlers",
    kind: FrameworkPackKind::Source,
    languages: &["python"],
    required_capabilities: &[LanguageCapability::Calls],
    framework_capabilities: &[FrameworkCapability::Messaging],
    dependency_markers: &["example/framework"],
    manifest_policy: FrameworkManifestPolicy::Required,
    activation_rules: &["decorated-handler"],
    accepted_roles: &[SemanticRole::Call],
    emitted_relation_families: &[FrameworkRelation::Handles],
    occurrence_policy: FrameworkOccurrencePolicy::ExactEvidence,
    limits: FrameworkLimits::default(),
};

FrameworkPackRegistry::validate_descriptors(&[descriptor])?;
```

After the implementation exists, add the descriptor to the private
`UNIVERSAL_FRAMEWORK_PACKS` table in `frameworks/pack.rs` in the same commit
that removes the old pack entry.

## Hard-cutover checklist

One language or framework cutover is complete only when:

1. production extraction emits typed evidence directly;
2. the capability profile states exactly what is emitted;
3. malformed or oversized evidence fails closed with a bounded diagnostic;
4. cached extraction without valid required evidence is treated as a cache
   miss while all existing version values remain unchanged;
5. collection resolution uses `UniversalResolutionIndex`;
6. replaced raw facts, resolver calls, helpers, and tests are deleted;
7. cold and cache-reused graphs and evidence batches are byte-identical;
8. language, scope, alias, ambiguity, external, Unicode, multiline, repeated-
   occurrence, and resource-boundary fixtures pass; and
9. the independent quality audit meets every numerical and zero-tolerance
   gate.

There is no transition period in which both algorithms publish facts.

## Quality audit

Run the deterministic harness with:

```bash
python3 benchmarks/performance/harness.py audit \
  --manifest path/to/audit.json \
  --graph path/to/graph.json \
  --corpus path/to/pinned/corpus
```

The manifest schema is `compass.quality-audit`. It records pinned corpus
commits and graph hashes, exact source-oracle provider identities and inventory
digests, advertised adapter/framework capabilities, required relations, and
records from three independent pools:

- `accepted` audits Compass-published edges for precision;
- `source_oracle` audits independently collected source constructs for
  recall; and
- `graphify_hypothesis` classifies Graphify-only facts without treating them
  as truth.

Each record includes corpus, adapter, optional framework pack, capability,
language, relation, confidence, target cluster, source and target
expectations, exact occurrence range, normalized snippet SHA-256, judgment,
and reason. `represented_elsewhere` also names the actual graph fact.

The harness reparses every pinned source-oracle corpus and verifies provider,
complete file coverage, and the full construct-inventory digest. It also
verifies corpus revision, graph digest, snippet bytes, and graph-fact occurrence
before calculating metrics. A missing, unsupported, partially parsed, or stale
source inventory fails closed. Invalid accepted edges remain in the precision
denominator. Ambiguous and rejected hypotheses remain explicit. A conformance
manifest is always ineligible for production claims.

A qualification requires:

- at least 2,000 audited accepted relationships;
- at least 400 accepted records per corpus;
- at least 100 accepted records per required relationship family;
- at least 100 accepted records per advertised capability identity;
- no target cluster above 10% of any corpus, language, relation, or capability
  stratum;
- at least 99.5% observed precision overall;
- a two-sided 95% Wilson precision lower bound of at least 99%;
- at least 99% observed precision and 95% source-oracle recall per advertised
  capability; and
- zero fabricated occurrences, cross-language matches, or unsafe local-target
  substitutions.

The checked-in `universal-core.json` is a deliberately small conformance
fixture. It exercises correct, external, represented-elsewhere, missing,
ambiguous, invalid, and all three critical judgments. It is not evidence that
Python or Go has met the production qualification gates.

## Current qualification boundary

Python, Go, Rust, and Java are hard-cut universal language adapters. Rust and
Java remain `UniversalCandidate`; this framework change does not promote Java.
`spring-java` is the first production universal framework pack and advertises
typed HTTP, bean, injection, messaging, scheduling, persistence, transaction,
and security capabilities. Kotlin Spring remains on its established detector.
Do not infer support for another language or framework from file extensions,
raw graph output, or total node and edge counts.
