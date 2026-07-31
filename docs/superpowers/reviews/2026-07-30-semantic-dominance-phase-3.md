# Semantic dominance phase-three qualification

## Scope and conclusion

Phase three improves two distinct layers:

1. the external evaluator can recognize one compatible Compass target at an
   exact mapped owner, relation, file, and source location; and
2. Compass can resolve conservative Python `super().method(...)` calls to an
   exact direct-base method through the universal evidence resolver.

The product change is source-grounded and high precision. It does not establish
strict Graphify-superset quality, the full production audit threshold, or
performance dominance. The comparison result remains failing because real
missing and ambiguous facts remain.

No version value changed. There is no compatibility projection or old-algorithm
fallback. Graphify was not rerun, and `graphify update .` was not run.

## Revisions and corpora

- Phase-three verification commit before this review:
  `52c3a1abeeac6dd614e38b3d211f9100bcbfa859`
- Comparison base: `origin/main` at
  `f0e9085` (`Merge pull request #92`)
- Django: `1c5927f04a853c79ac9b098eab92fb328ff9e4ad`
- Entire: `279b988597f1037c14cdd4c46765a5552e067d17`
- Graphify baseline: retained Graphify 0.9.30 outputs built previously from the
  same corpus checkouts

## Implementation

### Exact occurrence comparison

The comparator indexes Compass edges by canonical relation, mapped source,
occurrence file, and occurrence location. A Graphify call or reference is
classified as dominated only when that exact use site has one Compass target
and the target definitions are compatible. Multiple or incompatible targets
still fail closed.

The analyzer now reports ambiguous nodes explicitly. Comparator SQLite
connections are deterministically closed, and imports use the renamed
`benchmarks.performance.compass` package.

### Exact direct-base dispatch

The direct adapter retains each declaration's enclosing type and a bounded map
of explicit base identities. A zero-argument `super().method(...)` call receives
an exact `Base::method` constraint only for one statically nameable base. The
shared universal resolver must still find the exact source declaration.

The implementation rejects:

- multiple inheritance;
- dynamic base expressions;
- explicit-argument `super`;
- external-only base methods;
- methods not declared directly on the base; and
- any repository-wide or terminal-name fallback.

This is extensible without Python-specific resolver behavior. A future adapter
can provide the same exact owner-qualified constraint from compiler-selected
overloads, typed receivers, traits, interfaces, or superclass evidence.

## Real graph quality

### Graph size

The comparator canonicalizes duplicate payload representations, so both raw
publication counts and canonical comparison counts are shown.

| Corpus | Graph | Nodes | Raw edges | Canonical edges |
| --- | --- | ---: | ---: | ---: |
| Django | Compass before direct-base dispatch | 63,796 | 157,560 | 147,223 |
| Django | Compass phase three | 63,796 | 158,245 | 147,908 |
| Django | Graphify baseline | 50,842 | 158,704 | 158,704 |
| Entire | Compass phase three | 58,391 | 152,151 | 151,257 |
| Entire | Graphify baseline | 20,585 | 61,062 | 61,062 |

Django added 688 exact direct-base call edges and removed three incorrect call
edges from `copy()` methods to a local `copy` variable. The net increase is 685
edges. Entire topology is unchanged, as expected for a Python-only product
change.

### Graphify hypothesis coverage

These numbers classify Graphify facts as exact, semantically dominated,
intentionally rejected, ambiguous, or still missing. Graphify is a baseline
hypothesis source, not ground truth.

| Corpus/fact | Exact | Dominated | Rejected | Ambiguous | Missing |
| --- | ---: | ---: | ---: | ---: | ---: |
| Django nodes, before | 50,189 | 131 | 0 | 61 | 461 |
| Django nodes, phase three | 50,189 | 131 | 0 | 61 | 461 |
| Django edges, before | 51,530 | 44,065 | 55,437 | 257 | 7,415 |
| Django edges, phase three | 51,531 | 45,705 | 55,437 | 235 | 5,796 |
| Entire nodes, before | 18,184 | 898 | 0 | 233 | 1,270 |
| Entire nodes, phase three | 18,184 | 898 | 0 | 233 | 1,270 |
| Entire edges, before | 27,489 | 24,238 | 6,127 | 397 | 2,811 |
| Entire edges, phase three | 27,489 | 24,306 | 6,127 | 339 | 2,801 |

Across comparator and product changes, Django missing edge hypotheses fall by
1,619 and ambiguous edges fall by 22. Entire missing edges fall by 10 and
ambiguous edges by 58.

The product-only direct-base change moves one Django edge to exact, 273 to
dominated, and 274 out of missing. The other new direct-base calls are valid
Compass-only facts for which Graphify has no matching hypothesis.

The 55,437 rejected Django edges and 6,127 rejected Entire edges are not treated
as recall failures. They are predominantly module-import-to-symbol projection
and qualified-external endpoints rebound to unrelated local declarations.

### Independent precision check

An independent Python `ast` audit examined every one of the 688 newly emitted
Django call edges. For each edge it verified:

- an exact zero-argument `super()` call at the reported line;
- the enclosing source class;
- exactly one explicit base;
- import-relative or same-module base identity;
- the target file and enclosing target class; and
- a direct method declaration at the reported target line.

Result: **688/688 correct, 0 failures, 100% observed precision for this change
set**. This exceeds the requested 99% sampled precision target for the new
edges, but it is not the repository-wide 2,000-record production
qualification. No claim is made that all existing Compass edges have been
audited at this rate.

## Performance

### Standardized Compass harness

All samples were correctness-eligible and the harness gate passed.

| Corpus | Workload | Samples | p50 (s) | p95 (s) | Peak RSS (MiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| Django | cold | 3/3 | 11.847 | 13.676 | 4,631.20 |
| Django | warm | 3/3 | 1.627 | 1.633 | 499.52 |
| Django | incremental | 3/3 | 21.923 | 22.354 | 4,997.17 |
| Entire | cold | 3/3 | 7.458 | 7.554 | 2,789.27 |
| Entire | warm | 3/3 | 0.927 | 0.932 | 420.53 |
| Entire | incremental | 3/3 | 15.766 | 15.965 | 3,971.20 |

The older `09833d7` timing is not a valid phase-three regression base because it
predates the universal Python/Go hard cutover. A same-day isolated
`origin/main` release build showed essentially the same executed work:

| Corpus | Build | Internal total (s) | Wall (s) | Retired instructions |
| --- | --- | ---: | ---: | ---: |
| Django | `origin/main` | 12.5 | 14.67 | 294,057,654,538 |
| Django | phase three | 12.6 | 12.77 | 294,208,214,392 |
| Entire | `origin/main` | 6.8 | 6.84 | 164,154,156,487 |
| Entire | phase three | 7.4 | 7.49 | 165,016,872,090 |

Retired instructions differ by approximately 0.05% on Django and 0.53% on
Entire. Single-run wall differences are not evidence of a material product
regression or improvement.

### Retained Graphify comparison

Using the retained equivalent cold builds:

| Corpus | Compass wall (s) | Graphify wall (s) | Speedup | Compass RSS | Graphify RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Django | 12.77 | 66.58 | 5.21x | 4.80 GB | 1.34 GB |
| Entire | 7.49 | 25.55 | 3.41x | 2.98 GB | 0.23 GB |

Django meets the 5x cold-build target in this retained comparison. Entire does
not. Compass peak memory is materially worse on both corpora, especially
Entire. Phase three therefore does not satisfy the requested performance
dominance goal.

## Verification

Completed:

```text
cargo fmt --all
cargo check -p compass-languages -p compass-resolve
cargo test -p compass-languages --test universal_evidence
cargo test -p compass-resolve --test universal_resolution
cargo test -p compass-resolve --test relationship_occurrences
python3 -m unittest discover -s benchmarks/performance/tests -p 'test_*.py'
cargo build --release --locked -p compass-cli --bin compass
python3 benchmarks/performance/harness.py run \
  --repository django --repository entire \
  --workload build --build-repeats 3
```

The Python harness passed 81/81 tests with `ResourceWarning` promoted to an
error. The focused Rust tests passed. An initial full workspace run reached
zero free bytes during `compass-global`; those failures were `ENOSPC`, not
assertions. After removing 56.2 GiB of reproducible development artifacts, the
full workspace test suite was rerun with incremental compilation disabled and
passed, including scale tests and doc tests. The standardized real-repository
harness also passed.

## Remaining gaps

- Comparator status remains failing: Django has 461 missing and 61 ambiguous
  Graphify nodes plus 5,796 missing and 235 ambiguous edges.
- Entire has 1,270 missing and 233 ambiguous nodes plus 2,801 missing and 339
  ambiguous edges.
- Most remaining missing edges have no compatible exact occurrence; the next
  work must improve extraction evidence rather than relax resolution.
- Generated/module type identity remains a significant node and endpoint gap.
- Python multiple-inheritance and inherited-beyond-direct-base dispatch require
  explicit MRO evidence.
- The repository-wide production precision/recall audit thresholds remain
  unqualified.
- Entire cold speed and Compass peak memory remain performance blockers.

The honest outcome is a high-confidence recall improvement with no known
precision regression, not a flawless graph or final semantic dominance.
