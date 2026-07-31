# Semantic dominance phase-four qualification

## Scope and conclusion

Phase four replaces Python's single-direct-base `super()` shortcut with typed,
bounded hierarchy dispatch in the universal resolver. It adds exact ordered
base evidence, a capability-gated receiver strategy, a safe direct-successor
proof, and complete C3 linearization when every required source class is
known.

The change improves source-grounded recall without accepting Graphify's
incorrect hierarchy targets. Every new Django call edge was independently
verified, and Entire's canonical topology remains exactly unchanged. Strict
Graphify-superset quality and performance dominance are still not achieved.

No version value changed. There is no compatibility projection or
old-algorithm fallback. Graphify was not rerun, and `graphify update .` was
not run.

## Revisions and corpora

- Implementation commit: `f1ffda7`
  (`feat(quality): add universal hierarchy dispatch`)
- Query-scale fix: `58c09ee`
  (`perf(query): eliminate unused path bindings`)
- Comparison branch: `codex/compass-semantic-dominance-phase-3`
- Django: `1c5927f04a853c79ac9b098eab92fb328ff9e4ad`
- Entire: `279b988597f1037c14cdd4c46765a5552e067d17`
- Graphify baseline: retained Graphify 0.9.30 outputs from the same pinned
  checkouts

## Implementation

### Universal hierarchy contract

`ResolutionConstraint` now carries typed hierarchy facts:

- ordered direct bases plus a completeness flag; and
- an exact receiver identity with an explicit dispatch strategy.

`HierarchyDispatch` is a declared adapter capability. Python emits the full
contract for zero-argument `super()` and cannot simultaneously request the
old qualified, lexical, local, imported, or external resolution paths.
Adapters for future languages must declare their own compiler-appropriate
strategy; they cannot reinterpret Python C3 or use terminal-name search.

### Exact base publication

Direct bases resolve only to one exact qualified internal declaration or a
qualified external endpoint. Lexical and same-name fallback are prohibited.
This removes false hierarchy edges such as an imported
`django.db.models.Transform` being rebound to an unrelated GIS-local
`Transform`, while exact nested sibling classes retain their enclosing
identity.

### Bounded receiver dispatch

The resolver indexes ordered bases and directly owned members:

1. With a complete receiver base list, it can prove a member declared on the
   exact first base even when later ancestry is external.
2. Otherwise, it requires the complete traversed hierarchy to consist of
   unique exact source classes, computes an acyclic and consistent bounded C3
   order, and selects one directly declared member on the first matching
   class.
3. Dynamic or incomplete bases, explicit-argument `super`, cycles,
   inconsistent linearizations, ambiguous members, unresolved required
   ancestors, and bound overflow fail closed.

### Dead path-binding elimination

The first complete query qualification exposed a separate product issue:
Entire's unanchored two-hop CompassQL workload exceeded the intentional
256 MiB working-memory limit because the executor materialized a named path
that the projection never read. Removing only the unused binding made the
query pass, and its output was byte-identical to the named form run with a
1 GiB limit.

The compiler now removes a path binding only when it has one declaration, no
expression reference, and no wildcard projection. Referenced paths and
`RETURN *` retain the binding. This is a query-plan optimization; it does not
change graph extraction or raise a resource limit.

## Real graph quality

### Graph size and topology

The comparator canonicalizes duplicate payload representations, so raw and
canonical edge counts differ.

| Corpus | Graph | Nodes | Raw edges | Canonical edges |
| --- | --- | ---: | ---: | ---: |
| Django | Compass phase three | 63,796 | 158,245 | 147,908 |
| Django | Compass phase four | 63,892 | 159,047 | 148,710 |
| Django | Graphify baseline | 50,842 | 158,704 | 158,704 |
| Entire | Compass phase three | 58,391 | 152,151 | 151,257 |
| Entire | Compass phase four | 58,391 | 152,151 | 151,257 |
| Entire | Graphify baseline | 20,585 | 61,062 | 61,062 |

Django adds 473 call edges and removes no call edges relative to phase three.
Base topology also changes because incorrect same-name local substitutions are
removed and exact internal or qualified external bases are published. Entire
has zero added or removed canonical nodes or edges and retains digest
`6424ebd6100c61716d7aed22ccf05a2f0b702dce9f1c5d12fdeccbf033078932`.

Both graphs report zero validation errors. Cold and warm publication is
byte-identical:

| Corpus | Graph bytes | SHA-256 |
| --- | ---: | --- |
| Django | 201,270,657 | `e7673acb3d8d86b105ca191946d0bc738895617ea37b8e0e316a957edbd34f4a` |
| Entire | 169,597,446 | `96d51d3e917182915404482c59ecf87d4c229f8c9cd452256413d66a911318ba` |

### Graphify hypothesis coverage

Graphify facts are baseline hypotheses, not ground truth. “Rejected” means the
Graphify endpoint conflicts with Compass's stricter identity and use-site
rules; it is not counted as a recall failure.

| Corpus/fact | Exact | Dominated | Rejected | Ambiguous | Missing |
| --- | ---: | ---: | ---: | ---: | ---: |
| Django nodes, phase three | 50,189 | 131 | 0 | 61 | 461 |
| Django nodes, phase four | 50,189 | 169 | 0 | 95 | 389 |
| Django edges, phase three | 51,531 | 45,705 | 55,437 | 235 | 5,796 |
| Django edges, phase four | 51,531 | 45,874 | 55,437 | 295 | 5,567 |
| Entire nodes, phase four | 18,184 | 898 | 0 | 233 | 1,270 |
| Entire edges, phase four | 27,489 | 24,306 | 6,127 | 339 | 2,801 |

Django missing hypotheses improve by 72 nodes and 229 edges. Ambiguous counts
rise by 34 nodes and 60 edges because the comparator preserves multiple
plausible identities instead of selecting one without sufficient evidence.
Comparator status remains failing; these residuals cannot honestly be
described as a strict superset.

### Independent precision audit

An independent Python-AST and graph-hierarchy audit examined all 473 newly
published Django call edges. For every edge it checked:

- the exact call syntax and source location;
- the lexical source class and complete ordered direct-base list;
- independently computed C3 order where needed;
- direct member ownership on the selected class; and
- target file and declaration line.

Results:

| Proof | Verified | Failed |
| --- | ---: | ---: |
| Exact direct successor | 181 | 0 |
| Complete C3 linearization | 292 | 0 |
| Total | 473 | 0 |

Observed precision for the full new-edge population is 100%, exceeding the
requested 99% sampled target for this change set. This is not a claim that
every pre-existing repository edge has been independently audited.

## Performance

### Controlled phase-three comparison

The original pinned Django and Entire commits were rebuilt three times from
the final release binary. All samples were correctness-eligible.

| Corpus | Workload | p50 (s) | p95 (s) | Peak RSS (MiB) | p50 vs phase three |
| --- | --- | ---: | ---: | ---: | ---: |
| Django | cold | 12.139 | 14.206 | 4,737.25 | +2.46% |
| Django | warm | 1.620 | 1.663 | 512.05 | -0.44% |
| Django | incremental | 21.510 | 23.283 | 4,984.45 | -1.88% |
| Entire | cold | 7.905 | 8.318 | 2,798.00 | +6.00% |
| Entire | warm | 0.970 | 1.001 | 422.64 | +4.64% |
| Entire | incremental | 16.476 | 16.608 | 4,007.23 | +4.50% |

Django peak RSS changes are +2.29% cold, +2.51% warm, and -0.25%
incremental. Entire changes are +0.31%, +0.50%, and +0.91%. Django's cold
p95 is +3.88%; Entire's is +10.11%. The macOS harness does not expose retired
instruction counts, so no instruction-count claim is made.

The suite follows remote HEAD by design. During final qualification Django had
advanced to `902e5c0fb2d3f0772efbfebc8ff135926a2bb47a`; the complete final run
on that larger checkout also passed, with cold p50 13.369 seconds and peak RSS
4,560.39 MiB. Those numbers are useful current-scale evidence but are not used
for the phase-three delta above.

### Query qualification

All natural-language and CompassQL workloads passed warm-up plus 10/10
measured batches on both corpora. Representative bounded-path results:

| Corpus | Eligible | p50 (s) | p95 (s) | Peak RSS (MiB) |
| --- | ---: | ---: | ---: | ---: |
| Django `902e5c0` | 10/10 | 1.774 | 1.828 | 1,538.75 |
| Entire `279b988` | 10/10 | 1.916 | 2.555 | 1,838.56 |

Before dead-binding elimination, Entire failed warm-up and 10/10 samples with
`CQL3006` at the unchanged 256 MiB working-memory limit. The final run
`phase4-c3-final3-qualified` passed every gate.

Using the retained Graphify cold references gives an indicative 5.49x Django
and 3.23x Entire speed ratio. Entire therefore remains below the 5x target,
and Compass peak memory remains substantially higher on both corpora. Phase
four is not performance dominance.

## Verification

Completed:

```text
cargo test -p compass-resolve --test universal_resolution
CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 \
  CARGO_PROFILE_TEST_STRIP=none cargo test --workspace
PYTHONWARNINGS=error::ResourceWarning \
  python3 -m unittest discover -s benchmarks/performance/tests -v
cargo fmt --all -- --check
cargo clippy -p compass-languages -p compass-resolve --lib -- -D warnings
cargo clippy -p compass-languages -p compass-resolve --tests -- \
  -D warnings -A clippy::expect-used -A clippy::unwrap-used \
  -A clippy::panic
cargo test -p compass-cypher --test core_language
cargo test -p compass-query --test cql_execution
cargo clippy -p compass-cypher -p compass-query --lib -- -D warnings
cargo build --release --locked -p compass-cli --bin compass
python3 benchmarks/performance/harness.py run \
  --repository django --repository entire --workload all \
  --build-repeats 3 --query-batches 10 \
  --run-id phase4-c3-final3-qualified
git diff --check
```

The focused universal resolver suite passed 14/14, the strict Python
benchmark suite passed 81/81, and the complete workspace suite passed,
including scale and documentation tests. Explicit network/model acceptance
tests remain intentionally ignored. Strict production linting passed; test
linting permits assertion-only `expect`, `unwrap`, and `panic`. The final
large-repository run passed every build, query, CompassQL, determinism, and
correctness gate.

## Remaining gaps

- Django still has 389 missing and 95 ambiguous Graphify node hypotheses plus
  5,567 missing and 295 ambiguous edge hypotheses.
- Entire still has 1,270 missing and 233 ambiguous nodes plus 2,801 missing
  and 339 ambiguous edges.
- Module, re-export, and generated-type identity remain important declaration
  and endpoint gaps.
- Dynamic, external, incomplete, cyclic, inconsistent, or over-bound
  hierarchies intentionally remain unresolved.
- Other languages need explicit typed dispatch strategies backed by their
  compiler, type, trait, interface, or superclass evidence before advertising
  the capability.
- Repository-wide precision and recall thresholds for all pre-existing facts
  remain unqualified.
- Entire cold-build speed and Compass peak memory remain performance blockers.

The honest outcome is an exhaustively verified hierarchy-recall improvement
with no observed precision regression, not a flawless graph or final semantic
dominance.
