# Semantic Graphify Dominance Without Placeholder Duplication

**Status:** Approved for implementation design on 2026-07-30

## Purpose

Compass will use Graphify as a differential quality oracle without copying
Graphify representations that are less precise than Compass's native graph.
The next qualification phase will distinguish facts that Compass genuinely
misses from Graphify placeholders and flat ownership edges that Compass has
already replaced with anchored definitions and richer ownership.

The work also fixes the path-sensitive cache regression introduced by the
performance branch, adds missing empty Python module facts, and retains
genuinely unresolved external calls with bounded identities and exact evidence.

## Evidence

The pinned Entire comparison reports 720 Graphify nodes and 7,470 Graphify
edges as absent from Compass. Deeper inspection shows:

- 668 of the 720 nodes have a same-named, source-anchored Compass definition.
- 5,159 of 5,176 missing `references` edges already target the corresponding
  anchored Compass definition.
- Generated Go receiver types are repeated by Graphify at method sites, while
  Compass owns those methods under one canonical type.
- Graphify can express file-to-method containment where Compass expresses the
  more precise file-to-type-to-method path.

Those cases are representation differences, not native Compass omissions.
Publishing duplicate placeholders or flat containment edges would increase
ambiguity, memory use, and hub risk.

The pinned Django comparison also exposes genuine gaps:

- 610 represented source files are empty Python modules or package
  initializers for which Compass emits no file node.
- External member calls such as `mock.patch` can disappear when their target is
  outside the indexed corpus.
- 172 prototype-assigned methods in the vendored Select2 bundle remain a
  separate JavaScript extraction class.

The performance branch's path-specific cache key has one confirmed regression.
On macOS, a temporary path can be presented through `/var` while the cache root
is canonicalized through `/private/var`. Saving and loading then derive
different keys. The fix must normalize root aliases without resolving the
source leaf, because resolving the leaf would again collapse distinct logical
symlink paths such as `AGENTS.md` and `CLAUDE.md`.

## Goals

- Define a deterministic semantic-dominance contract for cross-tool quality.
- Preserve Compass's canonical generated-type ownership.
- Emit deterministic file nodes for selected empty Python modules.
- Retain qualified unresolved external calls with exact call-site evidence.
- Fix canonical-versus-logical root aliases in path-sensitive cache keys.
- Keep literal Graphify differences visible as diagnostics.
- Preserve deterministic graphs, zero validation errors, and the qualified
  build and query performance envelope.

## Non-goals

- Duplicating every Graphify placeholder in Compass's native graph.
- Treating a matching label alone as proof that two facts are equivalent.
- Adding Graphify, Python, or network access to Compass production execution.
- Relaxing ambiguity or evidence requirements to improve aggregate recall.
- Solving JavaScript prototype-assignment extraction in the same implementation
  pass. It is the next measured extraction class after this work.
- Fixing the viewer-asset CI failure already present on `main`; this work only
  fixes failures introduced by the performance branch.

## Semantic coverage contract

The comparator will report three disjoint outcomes for each shared Graphify
fact:

1. **Exact:** Compass has the same normalized semantic fact.
2. **Dominated:** Compass has a more precise fact and a deterministic proof of
   equivalence.
3. **Missing:** neither an exact fact nor an allowed dominance proof exists.

Quality acceptance requires every Graphify fact declared in scope for a
qualification phase to be exact or dominated. This phase covers empty Python
module identity, generated Go type/owner identity, and qualified unresolved
call targets. JavaScript prototype assignments remain visible as missing in the
full report and are explicitly scheduled for the following phase. Literal
differences remain reported, but they do not fail native quality when an
approved dominance proof exists.

### Resolved endpoint dominance

A Graphify edge to a placeholder may be dominated by a Compass edge to an
anchored definition only when all of the following hold:

- the edge source is exact or independently dominated;
- the normalized relation is the same;
- the Graphify target is sourceless or is a receiver/type placeholder emitted
  at a use site rather than its declaration;
- the Compass target has declaration evidence and a source anchor;
- normalized symbol name, language family, and available package/module
  evidence agree;
- relationship context agrees when present; and
- the relationship occurrence or enclosing declaration anchor identifies the
  same use.

Label equality by itself is never sufficient. Ambiguous candidates remain
missing and are reported with their competing targets.

### Canonical owner dominance

A repeated generated receiver/type node may be dominated by one canonical
Compass type when:

- the repeated node name matches the declared type;
- the use and declaration are in the same language package or module;
- the Compass method is owned by that declared type; and
- the Graphify occurrence is attached to the same method declaration or type
  reference.

Compass will not publish a duplicate receiver node solely for compatibility.

### Containment-path dominance

A Graphify `contains(A, B)` fact may be dominated by a Compass ownership path
from `A` to `B` when:

- every hop is a structural ownership relation;
- the path stays in the same source file and language;
- every intermediate node is source-anchored; and
- the path is unique within a small fixed hop bound.

The initial bound is two hops, covering file-to-type-to-method without enabling
arbitrary reachability matches.

## Architecture

### Cache root-alias correction

`compass-files::Cache` will retain both:

- the canonical root used for confinement and portable paths; and
- the lexical absolute root supplied by the caller.

The source cache key will derive its logical relative path by stripping either
root, preferring the lexical root and falling back to the canonical root.
Neither the source leaf nor a symlinked file will be canonicalized for identity.
Paths that match neither root retain the current explicit fallback behavior.

This keeps `/var/.../a.md` and `/private/var/.../a.md` equivalent while keeping
`AGENTS.md` and `CLAUDE.md` distinct.

### Empty Python module facts

The language extraction boundary will emit one deterministic file node for
every selected supported Python file, including a zero-byte file. The node
will carry:

- the logical repository-relative path;
- Python language and code-file classification;
- an `L1`/zero-byte source anchor; and
- exact file-selection evidence.

No synthetic function, class, or module body will be invented. Existing import
resolution can then target empty `__init__.py`, `__main__.py`, and other empty
module files.

### Qualified unresolved call retention

Cross-file resolution will retain a call when syntax proves the occurrence but
the target is outside the indexed corpus. It will create or reuse a sourceless
placeholder keyed by:

- language family;
- imported package/module when known;
- qualified receiver and callee; and
- repository/file scope as the conservative fallback.

The call edge will retain its exact relationship site and AST evidence while
the placeholder is explicitly marked unresolved. Existing graph-v1 placeholder
splitting and validation will enforce bounded scope and prevent a global
`patch`, `get`, or `register` hub.

If a unique anchored target exists, resolution continues to prefer it and no
external placeholder is retained.

### Differential comparator

The performance comparator will extend its temporary SQLite representation with
the fields needed to prove dominance:

- normalized name and qualified name;
- language, package, and module;
- declaration versus placeholder status;
- relationship context and occurrence anchor; and
- structural ownership adjacency.

Comparison remains streamed and indexed. It will produce counts and examples
for exact, dominated, ambiguous, and missing facts, grouped by reason, relation,
language, and source file. The comparator is development-only and does not
change Compass runtime output.

## Data flow

1. Compass and Graphify build the same pinned repository revision.
2. Each graph is normalized into source-anchored node and edge facts.
3. Exact fact inclusion is checked first.
4. Unmatched facts are evaluated by the bounded dominance rules.
5. Ambiguous or unproved matches remain missing.
6. The report separates literal differences from native semantic gaps.
7. Performance samples are eligible only after validation, determinism, and
   semantic coverage gates pass.

## Failure handling

- A cache path that cannot be made relative to either known root uses a stable,
  explicit fallback and never escapes the cache directory.
- An empty file that is unsupported or excluded by scope receives no node.
- An unresolved call without a qualified or safely scoped identity is reported
  as unresolved diagnostics rather than joined to a common-name hub.
- A dominance candidate set larger than one fails closed as ambiguous.
- A containment path longer than the fixed bound is not accepted.
- Comparator failure never changes or repairs production graph output.

## Test strategy

Implementation will follow red-green-refactor.

### Cache tests

- Reproduce a lexical-root/canonical-root alias with a symlinked root.
- Prove save through one root spelling and load through the other.
- Re-run the identical-content logical-path test.
- Prove a symlinked leaf remains distinct from its target.

### Extraction and resolution tests

- Empty `__init__.py` and `__main__.py` produce deterministic file nodes.
- Imports can target those empty module nodes.
- Repeated builds are byte-identical.
- Imported external `mock.patch` calls produce scoped placeholders and exact
  occurrence evidence.
- Same-named unresolved calls in different modules or files never collapse.
- A unique in-repository target suppresses the external placeholder.

### Comparator tests

- Exact facts continue to match exactly.
- A generated Go receiver placeholder is dominated by its canonical type.
- A file-to-method edge is dominated by a unique two-hop ownership path.
- Same-label cross-package candidates remain ambiguous.
- Same-label facts without compatible anchors do not dominate.
- Genuine missing facts still fail the gate.

### Real-corpus verification

Run fresh release builds and comparisons for:

- Django `50d706d0aebcc2d073c8d034b6e22fc98fad49f2`; and
- Entire `279b988597f1037c14cdd4c46765a5552e067d17`.

Record exact, dominated, ambiguous, and missing counts. Inspect representative
facts for every dominance reason rather than accepting aggregate totals alone.

## Performance safeguards

- Dominance matching stays in the external comparator.
- Runtime target indexes remain bounded and keyed; no corpus-wide
  label-by-label scan is added.
- Empty-file facts add one node per selected empty file and no body traversal.
- External placeholders are reused only within their qualified safe scope.
- Release build p50 and peak RSS may not regress more than 10% from the
  qualified Compass baseline:
  - Django cold: 12.402 seconds; warm: 1.949 seconds.
  - Entire cold: 4.28 seconds; warm: 0.82 seconds.
- The cold-build comparison must retain at least a 5x speedup over the measured
  Graphify baseline on both repositories.

## Acceptance criteria

1. The semantic-cache tests pass on macOS root aliases without weakening
   logical path identity.
2. The performance branch introduces no CI failure beyond failures reproduced
   on current `main`.
3. Selected empty Python files have deterministic, importable file nodes.
4. Qualified unresolved external calls survive with exact occurrence evidence
   and bounded placeholder identities.
5. Generated Go receiver/type placeholders are recognized as dominated by
   canonical anchored definitions without being duplicated in Compass.
6. Flat Graphify containment is recognized only through unique bounded
   structural paths.
7. The comparator reports exact, dominated, ambiguous, and missing facts
   separately and never treats label equality alone as equivalence.
8. Django and Entire graphs are deterministic and have zero validation errors.
9. Real-corpus examples for every dominance rule are manually inspected.
10. Build time and peak RSS stay inside the defined regression gates, and both
    cold builds remain at least 5x faster than Graphify.
11. Focused tests, relevant crate tests, formatting, and the performance-harness
    test suite pass.
12. `graphify update .` refreshes the parent repository graph after code
    changes.

## Follow-up

After this phase passes, the next measured extraction task is JavaScript
prototype-assigned methods, beginning with the 172 missing Select2 methods in
the Django corpus. It will use the same exact/dominated/missing report rather
than expanding the current phase opportunistically.
