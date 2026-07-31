# Balanced Source-Grounded Graph Quality Design

## Status

Approved in conversation on 2026-07-30. This design governs the quality phase
after semantic-dominance phase two. Production implementation must precede
focused regression tests, as requested by the user.

## Goal

Improve Compass graph precision and recall using real source occurrences as
the authority. Achieve at least 99% observed precision on a deterministic,
stratified audit while recovering as many verified relationships as possible.
Do not optimize for raw edge count or literal Graphify overlap.

## Background

Phase two produced valid, deterministic graphs for pinned Django and Entire
revisions:

| Repository | Tool | Nodes | Unique edges |
| --- | --- | ---: | ---: |
| Django | Compass | 55,120 | 130,847 |
| Django | Graphify | 50,845 | 158,710 |
| Entire | Compass | 21,661 | 72,298 |
| Entire | Graphify | 20,585 | 61,062 |

The current comparator classifies 53,005 Django Graphify reference edges as
`module_import_projected_to_symbol`. Those edges use an import statement as
the relationship occurrence and project the imported target onto class or
symbol owners. That evidence is invalid, but the previous report overstated
the conclusion by describing the entire family as false relationships. Some
source-target dependencies may be real at later use sites. This phase must
distinguish:

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

## Architecture

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

Existing raw graph metadata may carry the contract when it can do so without
duplicating the public schema. A new internal type is justified only if the
same fields are otherwise interpreted differently by multiple extractors or
resolvers.

### Constrained target resolution

The resolver considers evidence in this order:

1. exact lexical declaration;
2. explicit import alias or package-qualified identity;
3. unique same-module or same-package definition;
4. qualified external endpoint;
5. unresolved or ambiguous endpoint.

It never falls back to a repository-wide terminal-label match. Resolution
indexes must be bounded and keyed by language, module/package, spelling, and
role so the implementation does not reintroduce imports × symbols work.

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

Rust and Java serve as regression languages in this phase. Their extractors
need not migrate to a new internal representation unless the shared contract
requires a small compatibility change. The design must leave a schema-stable
adoption path for JavaScript/TypeScript and Ruby.

## Independent quality measurement

### Precision audit

Audit at least 1,000 Compass relationships using a deterministic stratified
sample across repositories, languages, relationship kinds, confidence tiers,
and high-frequency target clusters. Each record verifies independently:

- source owner;
- target identity;
- relationship kind; and
- occurrence range.

Any incorrect dimension makes the record incorrect. Report raw counts,
observed precision, sample composition, and a two-sided 95% Wilson confidence
interval. The sample contains at least 200 records per corpus and at least 50
records for each available required relationship family; no single repeated
target cluster may supply more than 10% of a stratum. The delivery gate is at
least 99% observed precision. A relation or language with a material
regression cannot be hidden by the aggregate.

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
convert ambiguous or rejected facts into silent passes.

### Audit manifest

The checked-in audit manifest records:

- schema version;
- corpus name and commit;
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

- At least 99% observed precision on the complete stratified audit.
- The report publishes confidence bounds and all incorrect samples.
- Genuine source-derived recall improves on Python and Go.
- No Java or Rust precision decrease greater than one percentage point and no
  source-derived recall decrease greater than 5% relative to the phase-two
  graph. Every smaller regression remains itemized.
- Both improvement graphs publish with zero validation errors.
- Cold and warm graphs are byte-identical for every measured Compass corpus.
- All mandatory semantic query oracles pass.
- No cold or warm build regression greater than 10% without diagnosis and
  explicit acceptance.
- Focused owning-crate tests, the performance harness, and the full workspace
  test suite pass.
- Run `graphify update .` in the parent repository after code changes.

## Implementation order

The phase is implementation-first:

1. add or consolidate the internal occurrence contract;
2. implement constrained Python and Go production resolution;
3. implement occurrence-aware comparison and the independent audit harness;
4. add focused regression tests for the completed behavior;
5. run real-corpus qualification and critique the output;
6. update documentation, commit, push, and update or create the pull request.

Tests still cover every production change, but they follow the initial
implementation rather than driving it.

## Error handling

- Resource or candidate limits fail closed with bounded diagnostics.
- Ambiguous resolution never selects an arbitrary target.
- Missing external definitions retain qualified unresolved endpoints.
- Invalid audit records fail qualification rather than being skipped.
- Unsupported language roles remain explicit and cannot fall through to
  terminal-name matching.
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
