# Semantic dominance phase-six qualification

## Scope and conclusion

Phase six closes the known Python runtime-declaration ownership gap. Compass
now represents functions and classes declared inside functions or methods,
including methods of those runtime classes, and assigns every downstream fact
to its exact lexical declaration owner. Exact declaration identity is carried
through the universal evidence contract and shared resolver; there is no file-
owned fallback or compatibility projection.

The hard cutover recovers all five phase-five runtime imports and adds 11,941
source-grounded Django relationships. The complete added population passed an
independent source audit. This is a material recall and ownership-quality
improvement, but it is not strict Graphify-superset quality or a claim of 100%
repository-wide precision. Some external targets remain bounded inferred
identities rather than exact internal declarations.

No version value changed. No legacy projection, shadow algorithm, feature flag,
or fallback was added. Graphify was not rerun, and `graphify update .` was not
run.

## Revisions and corpora

- Implementation commit: `5409b3c92e810b18f88807bd9860cdef15bc4526`
  (`feat(resolve): model Python runtime declarations`)
- Branch: `codex/compass-semantic-dominance-phase-3`
- Pinned quality-audit Django:
  `1c5927f04a853c79ac9b098eab92fb328ff9e4ad`
- Pinned quality-audit Entire:
  `279b988597f1037c14cdd4c46765a5552e067d17`
- Graphify baseline: retained artifacts for the pinned checkouts; Graphify was
  not executed in this phase
- Standardized performance Django:
  `ea303dee45f4b703b31f73ceb235f11742d3c518`
- Standardized performance Entire:
  `450f625e143804de962ed26fb355480844e057bc`

The benchmark helper package is already named
`benchmarks/performance/compass`. Its hard rename was merged in commit
`6ca7baa`; the former package and import name are absent.

## Implementation

### Exact lexical declaration ownership

The generic declaration walk now carries an exact lexical parent rather than
only an optional class parent. Python traversal enters function bodies and
emits nested functions and runtime classes under their enclosing function or
method. Methods remain owned by the exact runtime class, and functions nested
inside functions remain functions rather than being projected as methods.

Node identity derives from the lexical owner's graph identity and source name.
Repeated same-named declarations under one owner use a source-line
discriminator. Runtime classes also receive their lexical qualified name, so
their methods and downstream evidence share the same identity in raw extraction
and universal evidence.

### Exact-target evidence and fail-closed resolution

`ResolutionConstraint` can carry an exact source declaration ID. The field is
optional in the universal model, but this phase emits it only for Python
runtime-nested ownership where the parser proves the target. The evidence
validator rejects dangling exact targets, and the shared resolver selects only
the declared allowed target. It does not fall back to a terminal name, outer
owner, file, or sibling scope.

The mechanism is language-neutral. A future adapter can supply the same exact
declaration constraint after its own atomic hard cutover. Other language
behavior is unchanged in this increment.

## Pinned real-graph quality

### Size, determinism, and validation

The comparator canonicalizes duplicate payload representations, so raw and
canonical edge counts differ.

| Corpus | Graph | Nodes | Raw edges | Canonical edges |
| --- | --- | ---: | ---: | ---: |
| Django | Compass phase five | 63,797 | 145,219 | 144,295 |
| Django | Compass phase six | 68,761 | 157,056 | 156,092 |
| Django | Graphify retained/sanitized | 50,842 | 158,704 | 158,704 |
| Entire | Compass phase five | 58,391 | 152,161 | 151,267 |
| Entire | Compass phase six | 58,391 | 152,161 | 151,267 |

Both phase-six graphs report zero validation errors. Cold and warm output is
byte-identical:

| Corpus | Graph bytes | SHA-256 |
| --- | ---: | --- |
| Django | 203,384,611 | `0da8bfb778ce9db3116db46f9eade0868bd34bd0490af4c81bdfa70598d4da34` |
| Entire | 169,615,207 | `d654e803f10807217da0895091a72ef9b69351028affd637a299aca429ad74ff` |

The Django canonical digest is
`153c62f21d85780193cdfcc63d5d322be040725c3f0a8c4e48ba19c65f3cb6ed`.
Entire is byte-identical to phase five, confirming that the Python-specific
traversal change did not alter the Go corpus.

### Complete added-population audit

Exact-set comparison against phase five adds 4,969 nodes and 11,941
relationships. Net graph growth is smaller because repeated source declarations
remove or rekey a small set of prior collision identities.

| Added node kind | Count |
| --- | ---: |
| Class | 2,937 |
| Function | 1,070 |
| Method | 655 |
| Import placeholder | 175 |
| Type-alias placeholder | 131 |
| Variable placeholder | 1 |
| Total | 4,969 |

All 4,662 declaration nodes match Python AST file, line, source name, kind, and
lexical parent. Of those, 4,650 are runtime-nested declarations; the remaining
12 are source-backed repeated top-level declarations or descendants that were
previously hidden by identity collisions. All 307 placeholders have bounded
source anchors. Target confidence is exact for 284 and inferred for 23; the
latter are not described as exact internal declarations.

| Added relation | Count | Exact target confidence | Inferred target confidence |
| --- | ---: | ---: | ---: |
| Contains | 4,661 | 4,661 | 0 |
| Calls | 2,734 | 1,232 | 1,502 |
| Extends | 2,194 | 1,677 | 517 |
| Instantiates | 1,990 | 1,990 | 0 |
| References | 296 | 226 | 70 |
| Routes to | 55 | 55 | 0 |
| Imports | 11 | 5 | 6 |
| Total | 11,941 | 9,846 | 2,095 |

Every relationship has an exact source declaration or use-site occurrence.
“Inferred target confidence” means the target is bounded by qualified binding,
import path, or framework evidence but is not an exact internal declaration
identity. It does not mean a repository-wide terminal-name guess. The complete
audit accepted every added fact under these rules, but the result qualifies
only this changed population and does not establish the production 99% Wilson
lower-bound gate for every pre-existing graph fact.

All five phase-five runtime-import gaps are recovered under exact owners:

- `MigrationLoader` under
  `BaseDatabaseCreation::serialize_db_to_string::get_objects`;
- `redirect_to_login` under `AdminSite::admin_view::inner`;
- `Model` under
  `create_forward_many_to_many_manager::ManyRelatedManager::_get_target_ids`;
- `redirect_to_login` under `user_passes_test::decorator::_redirect_to_login`;
- `translation` under `no_translations::wrapper`.

### Graphify hypothesis coverage

Graphify facts remain comparison hypotheses, not ground truth. Rejected facts
conflict with stricter Compass source identity or use-site rules; missing and
ambiguous facts remain unresolved gaps.

| Django fact | Phase | Exact | Dominated | Rejected | Ambiguous | Missing |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Nodes | Five | 50,189 | 169 | 0 | 95 | 389 |
| Nodes | Six | 50,192 | 168 | 0 | 96 | 386 |
| Edges | Five | 47,388 | 45,524 | 55,396 | 295 | 10,101 |
| Edges | Six | 47,350 | 45,756 | 55,492 | 298 | 9,808 |

Phase six reduces missing Django edge hypotheses by 293 and increases combined
exact-plus-dominated coverage by 194. Exact matches fall by 38 because nested
ownership and repeated source occurrence identity are now more precise;
rejected rises by 96 and ambiguous by three. The improvement is therefore
better source-grounded recall and ownership, not blanket comparator dominance.
Strict Graphify-superset status still fails.

## Performance

Standardized run `phase6-runtime-declarations-final` used clean commit
`5409b3c`, three build repetitions, ten samples per query workload, and the
current remote corpus heads listed above. It completed with an overall PASS:
all samples were eligible, every correctness and query oracle passed, and the
run reported no gate issues.

| Corpus | Workload | Eligible | p50 (s) | p95 (s) | Peak RSS (MiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| Django | Cold | 3/3 | 12.460 | 12.614 | 4,759.42 |
| Django | Warm | 3/3 | 1.628 | 1.631 | 515.92 |
| Django | Incremental | 3/3 | 22.139 | 31.784 | 5,179.86 |
| Entire | Cold | 3/3 | 7.180 | 8.274 | 2,871.61 |
| Entire | Warm | 3/3 | 0.957 | 0.980 | 420.84 |
| Entire | Incremental | 3/3 | 16.094 | 16.745 | 4,034.45 |

Django natural-query p50 is 1.617–1.690 seconds and CompassQL p50 is
1.508–2.191 seconds. Entire natural-query p50 is 1.388–1.402 seconds and
CompassQL p50 is 1.344–1.780 seconds. Every query row passed 10/10 samples.
Cold-build graph hashes were stable across all repetitions:

- Django: `bc9d99418cb68cab9cce41b52ece6b9cc2ca63f0f4cf4b5085a9a9d4f36662c0`
- Entire: `c1514900b4030d8865c08f796a72a2a369c9c17f624b78df177b2f24db7e8e73`

The harness did not compare against a promoted phase-five baseline, and the
remote corpus revisions differ. A non-controlled comparison to the previous
qualification shows mixed results: Django cold and warm p50 rise about 7.1%
and 5.2%, while incremental rises 15.5%; Entire cold improves about 5.7%, while
warm and incremental rise about 7.4% and 6.3%. The Django incremental result is
a performance concern. No performance improvement or dominance is claimed.
Peak build memory also remains high.

Separate direct pinned builds were run under compilation and disk pressure and
are not comparable to the standardized measurements: Django measured 39.07
seconds cold and 2.55 seconds warm, while Entire measured 6.96 and 1.63
seconds. They are disclosed only to avoid selecting favorable numbers; they
are not used for a performance conclusion.

## Verification

Completed after the implementation-first changes:

```text
cargo test -p compass-resolve --tests --locked
cargo test -p compass-languages --tests --locked
cargo test -p compass-core --test code_graph_v1_determinism --locked
CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_STRIP=none cargo test --workspace --all-targets --locked
PYTHONWARNINGS=error::ResourceWarning python3 -m unittest discover -s benchmarks/performance/tests -v
cargo fmt --all -- --check
cargo clippy -p compass-languages -p compass-resolve -p compass-core --lib --locked -- -D warnings
cargo clippy -p compass-languages -p compass-resolve -p compass-core --tests --locked -- -D warnings -A clippy::expect-used -A clippy::unwrap-used -A clippy::panic
cargo build --release --locked -p compass-cli --bin compass
python3 benchmarks/performance/harness.py run --repository django --repository entire --workload all --build-repeats 3 --query-batches 10 --run-id phase6-runtime-declarations-final
git diff --check
```

The strict Python benchmark suite passed 81/81. The full locked workspace
all-target suite passed, as did focused resolver/language tests, determinism,
format, production/test lint, and release build. The standardized large-repo
matrix passed every internal correctness, determinism, natural-query, and
CompassQL gate. Graphify and `graphify update .` were not executed.

## Remaining gaps

- Django still has 386 missing and 96 ambiguous Graphify node hypotheses plus
  9,808 missing and 298 ambiguous edge hypotheses.
- Exact-plus-dominated Graphify edge coverage improves, but exact matches fall
  and rejected/ambiguous classifications rise. These residuals need source
  adjudication rather than count-based acceptance.
- External module/generated-type identity and unresolved dynamic call targets
  remain the principal recall gaps.
- The 2,095 added relationships with inferred target confidence need stronger
  language or framework evidence before they can become exact internal target
  identities.
- The repository-wide 99% Wilson lower-bound precision gate is not yet
  qualified; this phase verifies the complete changed population only.
- Other languages require separate atomic adapter hard cutovers before they
  can claim identical runtime-declaration coverage.
- Django incremental p50 and peak build memory remain performance concerns.

The honest result is a substantial, source-verified Python recall and ownership
improvement with stable cross-language behavior—not strict Graphify dominance,
universal graph completeness, or a performance win.
