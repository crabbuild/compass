# Semantic dominance phase-five qualification

## Scope and conclusion

Phase five removes the last pre-universal Python import parser and projection,
makes nested imports lexically scoped, and gives explicit source bindings
precedence over same-named lexical declarations. It also makes exact target
source identity available to collision disambiguation and treats a true
identity alias as terminal without accepting multi-node alias cycles.

The hard cutover improves graph precision and provenance. It does not achieve
strict Graphify-superset quality: Compass still has missing and ambiguous
baseline hypotheses, and the comparator counts more missing edges after exact
per-item spans and corrected endpoints replace Graphify-compatible
whole-statement anchors. Every relationship changed by the final binding
correction was independently source-verified, but that population audit is
not a claim of repository-wide 100% precision.

No version value changed. No legacy compatibility projection, shadow path, or
fallback remains. Graphify was not rerun, and `graphify update .` was not run.

## Revisions and corpora

- Python hard-cut commit: `d7ec9ea`
  (`feat(resolve): hard-cut Python import evidence`)
- Exact binding correction: `c92bb6d`
  (`fix(resolve): prioritize exact import bindings`)
- Comparison branch: `codex/compass-semantic-dominance-phase-3`
- Pinned Django: `1c5927f04a853c79ac9b098eab92fb328ff9e4ad`
- Pinned Entire: `279b988597f1037c14cdd4c46765a5552e067d17`
- Graphify baseline: retained Graphify 0.9.30 artifacts from those pinned
  checkouts; Graphify was not executed in this phase
- Final performance-only Django: `2b0e08a149b2036352684b8247f35c031cf61504`
- Final performance-only Entire: `ca1e2bf69e6fbf565380565d3f0928f9f9c37154`

## Implementation

### One authoritative Python import path

`compass-resolve` no longer reparses Python to discover imports, definitions,
or re-exports. The retired parser, import-guided resolver, producer, resolution
rules, provenance variant, and compatibility tests were deleted. Python
imports, aliases, re-exports, calls, constructions, decorators, annotations,
and bases now resolve from the typed universal evidence batch only.

There is no feature flag or dual publication. Tests assert that
`compass.resolve.python-imports` and its three old resolution rules cannot
appear in production output.

### Scope-correct bindings and exact occurrences

Python imports nested inside a function or class are stored only in the
owning scope. A local binding can shadow a file binding, but it cannot become
visible to a sibling declaration. Module imports remain file-scoped. Every
imported item retains its exact parser range rather than inheriting the whole
statement span.

### Shared explicit-binding precedence

The universal resolver now evaluates an occurrence's explicit binding before
same-named lexical declarations. This fixes Python wrappers that import a
same-named implementation inside the wrapper and Go qualified calls whose
package receiver collides with a local declaration. The resolved declaration's
exact source file is carried through collision disambiguation and stripped
before graph publication.

Exact identity aliases terminate normally. Real multi-node cycles still fail
closed. The same rule is language-neutral and can be reused by future direct
adapters without adding terminal-name search or a language-specific resolver.

## Real graph quality

### Graph size, determinism, and validation

The comparator canonicalizes duplicate payload representations, so raw and
canonical edge counts differ.

| Corpus | Graph | Nodes | Raw edges | Canonical edges |
| --- | --- | ---: | ---: | ---: |
| Django | Compass phase four | 63,892 | 159,047 | 148,710 |
| Django | Compass phase five | 63,797 | 145,219 | 144,295 |
| Django | Graphify retained baseline | 50,845 | 158,710 | 158,710 |
| Entire | Compass phase four | 58,391 | 152,151 | 151,257 |
| Entire | Compass phase five | 58,391 | 152,161 | 151,267 |
| Entire | Graphify retained baseline | 20,585 | 61,062 | 61,062 |

Both phase-five graphs report zero validation errors. Cold and warm output is
byte-identical:

| Corpus | Graph bytes | SHA-256 |
| --- | ---: | --- |
| Django | 188,252,089 | `93b035a0f871de0b024eda6a4bfffbfbc8d3ec1f1d80574918a98a18e32efbcb` |
| Entire | 169,615,207 | `d654e803f10807217da0895091a72ef9b69351028affd637a299aca429ad74ff` |

Canonical Compass digests are:

- Django:
  `304e71d34c1c04b28a36ea8c4588c813512c12d6958ab29927376f80f655fe21`
- Entire:
  `43a431d63a06b4719cc1cc6f184c7c58b72d51f3866295fdd1a54896ca48e835`

### Complete retired-edge accounting

Phase four had 13,802 raw V1 edges whose only extractor was
`compass.resolve.python-imports`: 13,091 imports and 711 exports. The complete
byte-containment audit classifies every one:

| Classification | Edges |
| --- | ---: |
| Exact universal replacement with a precise item span | 12,247 |
| Scope-correct ownership replacement | 258 |
| Corrected symbol export | 1,238 |
| Corrected endpoint identity | 33 |
| Redundant module projection | 21 |
| Unrepresented nested-declaration import | 5 |
| Total | 13,802 |

The five unrepresented facts are imports inside nested runtime declarations
that Compass does not currently model as nodes: `MigrationLoader` in
`creation.py`, `redirect_to_login` in `admin/sites.py`, `Model` in
`related_descriptors.py`, `redirect_to_login` in `auth/decorators.py`, and
`translation` in `core/management/base.py`. The old algorithm attached them
to the file, which was incorrect ownership. They remain an explicit
nested-declaration recall gap rather than being restored as false file edges.

The phase-five Django node count is 95 lower than phase four. Exact-set
comparison removes 101 and adds six nodes, all universal external import,
type-alias, or export placeholders. Most removed nodes were intermediate
package aliases that now resolve to anchored declarations. This is identity
cleanup, not deletion of source declarations.

### Graphify hypothesis coverage

Graphify facts are baseline hypotheses, not ground truth. “Rejected” means a
Graphify endpoint conflicts with Compass's stricter identity and use-site
rules; it is not counted as Compass recall. “Missing” still matters and is not
dismissed merely because the baseline can be wrong.

| Corpus/fact | Exact | Dominated | Rejected | Ambiguous | Missing |
| --- | ---: | ---: | ---: | ---: | ---: |
| Django nodes | 50,189 | 169 | 0 | 95 | 392 |
| Django edges | 47,388 | 45,526 | 55,396 | 295 | 10,105 |
| Entire nodes | 18,184 | 898 | 0 | 233 | 1,270 |
| Entire edges | 27,483 | 24,306 | 6,127 | 339 | 2,807 |

Strict superset status fails. Relative to phase four, Django's exact and
missing edge classifications worsen substantially. Two mechanisms account for
much of the delta: Graphify and the retired pass anchor an entire import
statement, while universal evidence anchors each imported item; and exact
import targets replace same-named local or intermediate alias targets. The
comparator is occurrence- and endpoint-sensitive, so these source-correct
changes stop matching baseline identities. This explains the delta but does
not prove all 10,105 missing hypotheses invalid. Those residuals remain work.

### Independent source audits

The final explicit-binding correction adds or retargets 96 Django
relationships and 16 Entire relationships relative to the first hard-cut
candidate. Independent audits checked the full changed population:

| Corpus/language | Verified | Failed | Proof |
| --- | ---: | ---: | --- |
| Django/Python | 96 | 0 | Python AST import binding, token, scope, target module, source file |
| Entire/Go | 16 | 0 | Go import path, package qualifier, target source inventory |

Examples include Django's local `templatize` import resolving to
`django/utils/translation/template.py` instead of its wrapper, identity module
imports resolving to their exact `signals.py` inventories, and Entire calls
such as `gitremote.ParseURL`, `logging.Close`, `strategy.FetchMetadataBranch`,
and `settings.Load` resolving to imported packages instead of same-named
local wrappers or stubs.

Observed precision for this complete changed-edge population is 100%, above
the requested 99% sampled target. It does not qualify every pre-existing edge
or establish the plan's repository-wide 99% Wilson lower-bound gate.

## Performance

The standardized run `phase5-python-import-hard-cut-final` used clean commit
`c92bb6d`, three build repetitions, ten samples per query workload, and the
current remote corpus heads listed above. Every gate passed.

| Corpus | Workload | Eligible | p50 (s) | p95 (s) | Peak RSS (MiB) |
| --- | --- | ---: | ---: | ---: | ---: |
| Django | cold | 3/3 | 11.635 | 12.253 | 4,752.66 |
| Django | warm | 3/3 | 1.547 | 1.554 | 482.45 |
| Django | incremental | 3/3 | 19.175 | 20.536 | 4,935.83 |
| Entire | cold | 3/3 | 7.611 | 7.703 | 3,000.59 |
| Entire | warm | 3/3 | 0.891 | 0.940 | 421.89 |
| Entire | incremental | 3/3 | 15.146 | 21.561 | 4,025.20 |

All natural-language query workloads and all seven CompassQL shapes passed
10/10 samples on both corpora. Query p50 ranges from 1.363 to 1.643 seconds on
Django and 1.370 to 2.019 seconds on Entire. The benchmark follows remote
heads, so these numbers are performance qualification rather than a controlled
phase-four delta.

Retained Graphify timing still implies only about 5.49x Django and 3.23x
Entire cold-build speed ratios, and Compass peak RSS remains substantially
higher. Entire remains below the 5x target. Performance dominance is not
claimed, and fewer edges alone are not treated as a performance improvement.

## Verification

Completed after the implementation-first changes:

```text
cargo test -p compass-resolve --tests --locked
cargo test -p compass-languages --tests --locked
cargo test -p compass-core --test code_graph_v1_determinism --locked
CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 \
  CARGO_PROFILE_TEST_STRIP=none \
  cargo test --workspace --all-targets --locked
PYTHONWARNINGS=error::ResourceWarning \
  python3 -m unittest discover -s benchmarks/performance/tests -v
cargo fmt --all -- --check
cargo clippy -p compass-languages -p compass-resolve -p compass-core \
  --lib --locked -- -D warnings
cargo clippy -p compass-languages -p compass-resolve -p compass-core \
  --tests --locked -- -D warnings -A clippy::expect-used \
  -A clippy::unwrap-used -A clippy::panic
cargo build --release --locked -p compass-cli --bin compass
python3 benchmarks/performance/harness.py run \
  --repository django --repository entire --workload all \
  --build-repeats 3 --query-batches 10 \
  --run-id phase5-python-import-hard-cut-final
git diff --check
```

The strict Python benchmark suite passed 81/81. The full locked workspace
all-target suite passed, including resolver, route, determinism, publication,
query, and scale tests. Explicit network/model acceptance tests remain
intentionally ignored. Production and test lint checks passed. The final
large-repository suite passed every correctness, determinism, build, natural
query, and CompassQL gate.

The first full-workspace attempt exhausted local disk space while linking. It
was rerun from clean reproducible build outputs with sufficient space and
passed; this was an environment failure, not a test failure.

## Remaining gaps

- Django still has 392 missing and 95 ambiguous Graphify node hypotheses plus
  10,105 missing and 295 ambiguous edge hypotheses.
- Entire still has 1,270 missing and 233 ambiguous nodes plus 2,807 missing
  and 339 ambiguous edges.
- Five nested runtime imports remain unrepresented because their enclosing
  runtime declarations are not modeled as graph nodes.
- Module, generated-type, and external declaration identity plus unresolved
  dynamic call targets remain important recall gaps.
- The repository-wide accepted-edge/source-oracle audit thresholds are not
  yet met; only the complete population changed by this phase is qualified.
- Other languages and framework packs need atomic direct-evidence hard
  cutovers before they can claim the same guarantees.
- Entire cold-build speed and Compass peak memory remain performance blockers.

The honest outcome is a source-verified hard cutover with materially better
binding identity, occurrence precision, and scope correctness—not a strict
Graphify superset, a complete graph, or performance dominance.
