# Source-grounded correctness hardening review

## Conclusion

This increment removes a confirmed false Python receiver edge and replaces the
remaining audit shortcuts with exact graph provenance, an independent Python
source oracle, and separate Compass, source-oracle, and Graphify-hypothesis
populations. It is a correctness improvement over PR #93, not a claim that the
repository graph is perfect or that Compass is a strict Graphify superset.

The retained pinned Django graph has 68,733 nodes, 164,664 raw edges, and
163,699 canonical edges with zero validation errors. Relative to PR #93, it has
28 fewer nodes and 7,607 more canonical edges. The edge increase is not treated
as quality by itself: 8,015 source-constrained receiver bindings were added and
408 older bindings were removed. An independent structural check found source
ancestor support for 8,009 of the 8,015 additions (99.925%). That check is not a
manual semantic-precision estimate, so it does not establish the production
99% Wilson lower-bound gate.

No version value changed. No compatibility projection or legacy receiver
fallback remains. Graphify and `graphify update .` were not run.

## Implementation

### Receiver dispatch hard cutover

Python `self.member()` and `cls.member()` evidence now carries the typed
`C3FromReceiver` strategy. Resolution checks only the exact receiver, a
source-proven hierarchy prefix, and a complete C3 linearization. An unproved or
ambiguous target fails closed and cannot enter lexical, same-module, imported,
or terminal-name fallback.

The confirmed false `AdminEmailHandler.emit() -> ServerFormatter.format()`
edge is absent from the retained graph. `emit()` retains its source-proven calls
to `AdminEmailHandler.format_subject()`, `AdminEmailHandler.send_mail()`, and
`copy.copy()`. Its inherited `self.format()` remains unresolved because the
external base implementation is not represented with enough source evidence;
that omission is preferable to the old false local binding.

### Audit provenance and independent source inventory

The comparator database now stores the SHA-256 of the exact raw graph artifact.
Candidate export refuses a graph/database mismatch and refuses databases that
predate this provenance field. SQLite connections are deterministically closed.

Candidate export now has three explicitly different populations:

- `compass_graph`: every eligible, exact source-bounded Compass relationship,
  with repeated same-line occurrences retained separately;
- `source_oracle`: calls and imports parsed directly from repository source,
  independently of either graph;
- `graphify_comparison`: comparison hypotheses only, never accepted precision
  evidence merely because Compass agrees with them.

The source-oracle interface is an extensible provider registry. Its first
provider uses Python AST ranges and exact UTF-8 byte offsets, records lexical
owners and target spelling, and pins its full construct inventory in the audit
manifest. Qualification reparses the corpus and rejects provider, parse-count,
rejected-file, or inventory-digest drift.

### Honest Django oracle coverage

The pinned Django source oracle scanned 2,927 Python files, fully parsed 2,926,
and emitted 202,271 independent constructs: 182,921 calls and 19,350 imports.
Its inventory digest is
`56399a7f333c656758f945ddf0b24b23ce1d207612dd71893b5a1a86ab77df64`.

One Django test fixture,
`tests/test_runner_apps/tagged/tests_syntax_error.py`, intentionally contains
invalid Python (`1syntax_error`). Compass publishes four nodes and six edges
from its valid prefix, but Python `ast` cannot produce a complete independent
inventory for the file. Qualification therefore fails closed instead of
silently excluding it. This is a real limit on any repository-wide completeness
claim.

## Graph quality compared with retained Graphify hypotheses

Graphify remains a baseline of hypotheses, not truth. The source-aware
classification moved as follows:

| Graph | Exact | Dominated | Rejected | Ambiguous | Missing |
| --- | ---: | ---: | ---: | ---: | ---: |
| PR #93 baseline | 47,350 | 48,964 | 55,492 | 298 | 6,600 |
| Receiver-hardened | 47,186 | 49,030 | 55,492 | 298 | 6,698 |

Combined exact-plus-dominated coverage decreases by 98 Graphify hypotheses.
That is not automatically a truth regression: the hard cutover deliberately
removes receiver bindings that have no proven hierarchy path. The confirmed
false edge is one example where fewer matched hypotheses is higher precision.
The remaining 6,698 missing hypotheses require source adjudication rather than
automatic recovery.

The retained graph file SHA-256 is
`6ad2895ccf71222991c97c1b217b8ccffbaec51e4443c824b2a8975e821bcf54`;
its canonical comparator digest is
`50e483f525e4f406a6ecfaaecba706dc9835b4ef2401c8cfebef1748360dffab`.

## Performance

The complete Django/Entire run produced correct, deterministic graphs and
eligible query results, but its overall latency gate is not accepted as a clean
comparison. A concurrent release/LTO Rust compilation from another worktree
caused wall-time stalls. This was visible in query wall time while user CPU time
remained normal. The contaminated run is retained as
`source-grounded-hardening-final` rather than discarded.

Build p50 stayed within the 10% threshold in that run:

| Corpus | Workload | Current p50 (s) | Baseline p50 (s) | Change |
| --- | --- | ---: | ---: | ---: |
| Django | Cold | 11.719 | 12.460 | -5.9% |
| Django | Warm | 1.628 | 1.628 | 0.0% |
| Django | Incremental | 21.549 | 22.139 | -2.7% |
| Entire | Cold | 6.850 | 7.180 | -4.6% |
| Entire | Warm | 0.877 | 0.957 | -8.3% |
| Entire | Incremental | 15.932 | 16.094 | -1.0% |

A clean focused rerun then measured all seven Entire CompassQL workloads at
10/10 eligible samples. Every p50 and p95 was faster than the baseline:

| Workload | Current p50 (s) | Baseline p50 (s) | Change |
| --- | ---: | ---: | ---: |
| Aggregate | 1.333 | 1.344 | -0.8% |
| Anchored | 1.294 | 1.347 | -3.9% |
| Bounded path | 1.766 | 1.780 | -0.8% |
| One hop | 1.605 | 1.621 | -1.0% |
| Optional | 1.532 | 1.547 | -0.9% |
| Policy shaped | 1.420 | 1.443 | -1.6% |
| Scan | 1.347 | 1.377 | -2.1% |

The focused report exits nonzero only because its one-corpus inventory differs
from the two-corpus baseline inventory. That corpus-mismatch gate is expected;
the individual workload comparisons pass. No broad performance-dominance claim
is made. Peak build RSS remains high at roughly 4.8--5.2 GiB on Django.

## Verification

Completed after implementation:

```text
PYTHONWARNINGS=error::ResourceWarning python3 -m unittest discover -s benchmarks/performance/tests -v
cargo test -p compass-languages --test universal_evidence --locked
cargo test -p compass-resolve --test universal_resolution --locked
cargo test -p compass-core --test code_graph_v1_determinism --locked
cargo test -p compass-core --test code_graph_v1_publication_resilience --locked
cargo fmt --all -- --check
cargo clippy -p compass-languages -p compass-resolve --lib --locked -- -D warnings
git diff --check
```

Results: 87 Python tests, 18 language-evidence tests, 21 resolver tests, nine
determinism tests, and ten publication-resilience tests passed. Formatting,
focused production lint, and the diff check passed. A fresh full-workspace link
was not attempted after free disk fell below 5 GiB; the immediately relevant
crates and publication boundaries were exercised instead.

## Highest-value next improvements

1. Add independent source-oracle providers for Go, TypeScript/JavaScript,
   Java, and Rust through the same provider contract. Until then, cross-language
   audit quality is asymmetric.
2. Recover unresolved calls only from exact module identity, generated-type
   identity, import binding, receiver-type evidence, or framework-pack facts.
   Never recover them with terminal-name similarity.
3. Build a stratified, independently adjudicated production audit of at least
   2,000 Compass facts and enforce the 99% Wilson lower bound on held-out
   repositories. The checked-in audit remains conformance evidence only.
4. Define a tolerant source provider for intentionally invalid files that can
   prove constructs in parser-accepted regions without treating recovered
   syntax as complete. Its coverage boundary must remain explicit.
5. Avoid decoding the full 170--200 MB JSON graph for every CLI query. A
   reusable immutable query snapshot or indexed on-disk representation is the
   clearest query-latency and harness-throughput opportunity.
6. Profile graph assembly and publication retention with allocation attribution
   before attempting another memory optimization. The shared-buffer prototype
   was correctly removed because it did not improve peak RSS.
7. Harden performance qualification against external contention by recording
   system load and rejecting noisy samples, and make subset runs compare only
   the requested corpus instead of reporting an expected corpus mismatch.

These are the remaining paths toward high-confidence semantic dominance. None
should be implemented by increasing edge count without independent use-site
evidence.
