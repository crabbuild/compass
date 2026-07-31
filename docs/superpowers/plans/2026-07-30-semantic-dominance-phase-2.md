# Semantic Dominance Phase Two Implementation Plan

> **Execution rule:** This phase is implementation-first. Production changes
> are made before focused regression tests are added. Graphify remains a
> development-only baseline; no Graphify behavior or dependency enters Compass
> production code.

**Goal:** Close the next set of source-backed semantic gaps on the pinned
Django and Entire repositories while reducing publication and resolution
overhead. Preserve shell entrypoints, publish Go embedding as a first-class
relationship, anchor Python decorator uses at their real occurrences, and
remove broad import-to-class inference that creates false relationships.

**Architecture:** Language extractors emit exact AST facts with source
occurrences. The resolver may canonicalize an endpoint only when package,
module, import, or ownership evidence selects one definition. The strict v1
publisher retains the resulting relationship vocabulary without collapsing
semantically distinct edges. The comparison harness proves exact coverage or
bounded dominance and separately records Graphify facts that are not safe to
copy, such as a standard-library Go type incorrectly joined to an unrelated
repository-local type.

**Tech stack:** Rust 2024, tree-sitter, serde/serde_json, SQLite, Python 3,
Cargo, the existing `benchmarks/performance` harness, Graphify 0.9.31 as an
explicit development oracle.

## Background

PR #86 merged into `main` at `82e73ce`. It delivered cache identity
correctness, byte-stable cold/warm graphs, unchanged-build reuse, bounded
artifact retention, faster semantic comparison, identifier-aware query
ranking, cold-build preflight elimination, and reduced graph-publication
cloning.

The merged real-corpus comparison is deliberately fail-closed:

| Repository | Fact | Exact | Dominated | Ambiguous | Missing |
| --- | --- | ---: | ---: | ---: | ---: |
| Django | Nodes | 50,466 | 6 | 25 | 345 |
| Django | Edges | 149,993 | 336 | 26 | 8,349 |
| Entire | Nodes | 19,866 | 663 | 6 | 50 |
| Entire | Edges | 53,593 | 6,104 | 18 | 1,347 |

Fresh phase-two indexes were rebuilt from the final graph artifacts using
comparison schema v2. The dominant actionable categories are:

- 48 shell entry nodes are extracted but discarded before publication because
  `bash_entrypoint` is stored only in nested metadata rather than the strict
  node-kind field. Their file containment and top-level function calls are
  consequently lost.
- 59 measured Go embedding relationships are extracted with exact occurrences
  but strict publication collapses `embeds` into `contains`.
- Python import-use inference joins every imported name to every class in a
  file and anchors the relationship at the import statement. This creates
  false positives and misses real decorator occurrences such as
  `@isolate_apps` at line 6.
- Some generated Go receiver placeholders are ambiguous only because
  comparison discards exact label case before considering the two real,
  case-distinct package types (`EphemeralStore` and `ephemeralStore`).
- Many residual Go `Context`, `Response`, and similar baseline edges are not
  safe targets. Graphify binds qualified standard-library or external types to
  unrelated same-named repository definitions. Compass must retain a qualified
  external endpoint rather than reproduce that false resolution.

The current performance evidence is:

| Repository | Compass cold p50 | Compass warm p50 | Graphify cold | Ratio |
| --- | ---: | ---: | ---: | ---: |
| Django | 12.355s | 2.182s | 49.985s | 4.05x |
| Entire | 4.055s | 0.586s | 17.650s | 4.35x |

The branch begins with `f1a39a1`, which skips sorting/deduplicating evidence
vectors of length zero or one. This optimization remains part of phase two.

## Production data flow

```text
source file
  -> language AST extraction
       -> typed nodes + exact relationship occurrences
       -> qualified unresolved endpoint facts when a definition is external
  -> collection-wide resolver
       -> import/package/owner constrained canonicalization
       -> no label-only cross-module joins
  -> strict compass.graph/1 publication
       -> first-class embeds / calls / references / contains semantics
       -> deterministic validation
  -> graph.json
  -> development-only Graphify comparison and residual audit
```

## Semantic rules

- `embeds` is distinct from `contains`: embedding affects promoted fields and
  methods and must survive the public schema.
- A shell entrypoint is a source-backed callable at line 1, contained by its
  file and owning only top-level calls.
- Python decorator relationships are emitted only for decorators present on
  that class or callable and use the decorator AST range.
- Import evidence may resolve a decorator or base to one qualified repository
  definition. It must not infer that every class uses every imported symbol.
- Case-sensitive spelling may disambiguate otherwise compatible generated
  owners; case folding alone may never select among case-distinct definitions.
- External Go types remain qualified external facts. They are not rebound to a
  repository type merely because the final identifier segment matches.
- Comparison rules may recognize stronger Compass semantics, but may not erase
  relation kind, occurrence, module, or endpoint conflicts.

## Acceptance gates

- Both graphs publish with zero validation errors and deterministic canonical
  digests.
- Entire publishes all source-backed shell entrypoints and all extracted Go
  embedding occurrences without converting them to `contains`.
- Django decorator edges use exact decorator lines; broad import-to-every-class
  edges are absent.
- Entire ambiguous generated-owner nodes decrease from 6 without a label-only
  resolution rule.
- No exact or dominated coverage regression in unaffected relation families.
- Focused owning-crate tests and the full workspace test suite pass after the
  production implementation.
- Django and Entire cold/warm builds are remeasured; any regression greater
  than 10% is diagnosed before delivery.
- Query oracles for Django URL resolution/model save and Entire checkpoint
  creation/repository state continue to pass.
- Residual missing facts are grouped into genuine Compass gaps, external facts,
  and demonstrated Graphify false positives.
- Do not rerun Graphify during final Compass verification; retain the pinned
  Graphify 0.9.31 artifacts as the comparison baseline.

## Execution tasks

### Task 1: Preserve source-backed relationship vocabulary

Production first:

- [x] Add `EdgeKind::Embeds` to `compass.graph/1`, its string form, validation,
  and raw-v1 mapping.
- [x] Publish Bash entrypoints as typed callable nodes instead of generic nodes
  discarded during normalization.
- [x] Keep exact AST relationship sites on both facts.

Post-implementation verification:

- [x] Add model/v1 serialization and endpoint validation coverage for
  `embeds`.
- [x] Add Bash extraction/publication coverage for file → entrypoint and
  entrypoint → function calls.

### Task 2: Replace Python class-import inference with exact decorator facts

Production first:

- [x] Extract decorators on Python classes, functions, and methods at the
  decorator occurrence.
- [x] Carry qualified import targets on those exact edges.
- [x] Extend qualified Python endpoint resolution to safe decorator/reference
  relations.
- [x] Remove the collection-wide rule that connects every import to every
  class.

Post-implementation verification:

- [x] Cover direct, aliased, re-exported, class, and method decorators.
- [x] Prove an unused import creates no class relationship.
- [x] Prove repeated decorator occurrences remain distinct.

### Task 3: Tighten generated owner and unresolved target identity

Production first:

- [x] Preserve package-qualified Go type spellings for external type
  references and embedding targets.
- [x] Resolve same-package receiver owners only through package identity.
- [x] Use exact case as a fail-closed discriminator for generated Graphify
  owner placeholders in the development comparator.
- [x] Retain qualified external targets instead of joining common names across
  modules.

Post-implementation verification:

- [x] Cover case-distinct package types and method owners.
- [x] Cover qualified external Go types that collide with a local type.
- [x] Cover same-package embedding and receiver ownership.

### Task 4: Optimize the changed paths

Production first:

- [x] Avoid work proportional to all imports × all classes.
- [x] Reuse per-file import maps and bounded candidate indexes.
- [x] Keep the already committed trivial evidence sort fast path.

Post-implementation verification:

- [x] Profile Django and Entire cold builds with internal phase timings.
- [x] Confirm deterministic cold/warm output and unchanged-build reuse.

### Task 5: Real-corpus qualification and delivery

- [x] Build release Compass from the final tree.
- [x] Rebuild Django and Entire graphs from clean output directories.
- [x] Re-index and compare both graphs against Graphify 0.9.31.
- [x] Run the four semantic query oracles.
- [x] Record before/after quality, latency, peak RSS, and residual categories
  in the performance review.
- [x] Preserve the retained Graphify baseline without rerunning Graphify.
- [x] Commit only phase-two files, push the branch, and create a pull request
  targeting `main`.

## Qualification outcome

The implementation passes the source-backed acceptance gates. It deliberately
does not claim a literal Graphify superset: the strict raw comparison now
classifies Graphify's import-line-to-every-class Python inference as rejected
baseline evidence. Qualified Go types rebound to unrelated local names remain
visible as missing conflicts. Neither family is a relationship Compass should
reproduce. Exact decorator occurrences, shell entrypoints, Go embeddings,
package-aware identities, query oracles, deterministic publication, and build
performance are recorded in the performance review.
