# Universal semantic evidence

Compass resolves source relationships through a language-neutral evidence
contract. Python and Go use this contract in production. Other languages keep
their existing extractors until a dedicated change removes the old algorithm
and adds the universal adapter atomically.

This is a hard-cutover interface. It has no raw-fact translation layer, shadow
mode, terminal-name fallback, or runtime dependency on Graphify.

## Evidence contract

`SemanticEvidenceBatch` is the unit stored with one source extraction. Its
`AdapterIdentity` names one language and producer and lists only capabilities
the adapter actually emits. The batch contains six bounded collections:

- `DeclarationFact` identifies a source-backed declaration and its existing
  graph node, kind, name, qualified name, optional module/package, lexical
  scope, and exact range.
- `ScopeFact` identifies a lexical scope, optional owning declaration, parent
  scope, and exact range.
- `BindingFact` records an import, import alias, re-export, local alias, or
  package binding. Its target is qualified; a proven local declaration may
  also be named directly.
- `OccurrenceFact` records one exact use site and its role, owner, spelling,
  optional qualifier, lexical scope, and range. Repeated uses are separate
  occurrences.
- `RelationshipCandidate` connects a source declaration and occurrence to a
  typed relationship plus target constraints. It is evidence awaiting
  resolution, not permission to choose a convenient node.
- `EvidenceDiagnostic` records a bounded extraction problem without creating
  a graph fact.

Every fact ID is a deterministic SHA-256 identity over length-prefixed inputs:
the unchanged extraction-semantics identity, language, producer, source path,
fact category, semantic role, relevant names, and source range. Builders sort
and deduplicate typed facts before validation.

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

Validation traverses each collection by stable fact ID, so equivalent input
orders return the same first error.

## Adapter registration

`AdapterRegistry::universal_profile(language)` is the authority for universal
cutover. A returned `AdapterProfile` means universal evidence is mandatory.
Python and Go are currently registered. An unregistered language does not
silently claim universal behavior.

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

1. exact declaration in the lexical scope or a parent scope;
2. explicit import, alias, or re-export binding;
3. exact qualified identity;
4. unique same-module or same-package declaration;
5. source-scoped qualified external endpoint when explicitly allowed;
6. ambiguous or unresolved.

Language and allowed target kinds are filtered before uniqueness is decided.
Case-insensitive or terminal-name equality cannot select a target. Cross-
language candidates cannot resolve. Candidate lists are bounded before they
can affect memory or runtime.

Materialization preserves the occurrence range and resolution rule. Exact
local targets remain extracted evidence. Qualified external targets are
source-scoped and inferred; they never substitute a same-named repository
node.

The following resolution behavior is forbidden:

- repository-wide terminal-label search;
- picking the first candidate after sorting;
- resolving across languages;
- projecting an import line onto every imported symbol;
- inventing a call, reference, owner, or occurrence;
- using Graphify at runtime; and
- falling back to a removed language-specific resolver.

### Owner-qualified receiver dispatch

Receiver dispatch uses the same language-neutral candidate contract. An
adapter may set `ResolutionConstraint.qualified_name` to an exact member
identity only when source, compiler, or type evidence proves the receiver
owner. The shared resolver then requires one compatible declaration with that
identity. It does not need a language-specific member search.

Python demonstrates the conservative minimum. For
`super().method(...)`, the direct adapter records the enclosing type and its
explicit bases. It emits `Base::method` only when:

- the call uses zero-argument `super()`;
- the source method has an enclosing class;
- that class has exactly one explicit, statically nameable base;
- imports or the current module provide the exact base identity; and
- the target method exists as an exact source declaration on that base.

Multiple inheritance, dynamic bases, explicit-argument `super`, external-only
bases, and methods inherited beyond the direct base remain unresolved. A
future Python MRO capability must provide ordered, source-grounded MRO evidence
before recovering those cases.

Other adapters should reuse this pattern with evidence appropriate to their
language: compiler-selected overloads, trait or interface implementation
facts, typed receivers, or statically resolved superclass members. They must
not encode a guessed owner merely because a member name is unique in the
repository. This extension uses the existing constraint model and does not
require a version change.

## Framework pack registration

`FrameworkPackDescriptor` is the registration contract for framework meaning
derived from universal language evidence. It declares:

- stable pack ID and source, config, or template kind;
- universal languages and required capabilities;
- dependency markers and manifest policy;
- named activation rules;
- accepted semantic roles and emitted relation families;
- exact-evidence or exact-anchored-heuristic occurrence policy; and
- nonzero candidate, include-depth, alias-expansion, and per-file fact limits.

`FrameworkPackRegistry::validate_descriptors` rejects duplicate IDs,
unregistered languages, unsupported capabilities, missing roles or relations,
capability bypasses, unnamed heuristic activation, required manifests without
dependency markers, invalid ordering, and zero limits.

No current raw framework detector is projected into this registry. A future
framework change removes its previous registration and adds the universal
descriptor and implementation together.

### Minimal framework descriptor

```rust
use compass_languages::{
    CandidateRelation, FrameworkLimits, FrameworkManifestPolicy,
    FrameworkOccurrencePolicy, FrameworkPackDescriptor, FrameworkPackKind,
    FrameworkPackRegistry, LanguageCapability, SemanticRole,
};

let descriptor = FrameworkPackDescriptor {
    id: "example-python-handlers",
    kind: FrameworkPackKind::Source,
    languages: &["python"],
    required_capabilities: &[LanguageCapability::Calls],
    dependency_markers: &["example/framework"],
    manifest_policy: FrameworkManifestPolicy::Required,
    activation_rules: &["decorated-handler"],
    accepted_roles: &[SemanticRole::Call],
    emitted_relation_families: &[CandidateRelation::Calls],
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
commits and graph hashes, advertised adapter/framework capabilities, required
relations, and records from three independent pools:

- `accepted` audits Compass-published edges for precision;
- `source_oracle` audits independently collected source constructs for
  recall; and
- `graphify_hypothesis` classifies Graphify-only facts without treating them
  as truth.

Each record includes corpus, adapter, optional framework pack, capability,
language, relation, confidence, target cluster, source and target
expectations, exact occurrence range, normalized snippet SHA-256, judgment,
and reason. `represented_elsewhere` also names the actual graph fact.

The harness verifies corpus revision, graph digest, snippet bytes, and graph
fact occurrence before calculating metrics. Invalid accepted edges remain in
the precision denominator. Ambiguous and rejected hypotheses remain explicit.
A conformance manifest is always ineligible for production claims.

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

Python and Go are the only universal adapters in this increment. The contract
is designed for any language or framework, but Java, Rust, other languages,
and existing framework packs are not yet cut over or qualified. Do not infer
support from file extensions, existing raw graph output, or total node and
edge counts.
