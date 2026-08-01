# Spring Java Universal Framework-Pack Quality Review

## Decision

Compass commit `1946012aa67bba474f4016aa2d9f79010a3c1476` hard-cuts
Java Spring interpretation to the production `spring-java` universal
framework-pack registry. The pack consumes Java's version-1 universal semantic
evidence and exact syntax anchors; it does not discover Java declarations with
regular expressions and does not run beside the legacy Java Spring detector.

This is a framework-pack cutover, not a Java language-profile promotion. Java
remains `UniversalCandidate`. Kotlin Spring remains explicitly owned by the
established `spring-web-kotlin` path.

## Shipped contract

The descriptor declares bounded support for HTTP routes, beans, dependency
injection, messaging, scheduling, persistence, transactions, and security. Its
typed relation contract covers decoration, route targets, bean registration,
message handlers and producers, scheduled jobs, dependencies, and ORM mappings.

The implementation resolves:

- direct, array-valued, multiline, composed, inherited, and interface Spring
  mappings, including `@AliasFor`;
- controller path constants, concatenation, and static imports across files,
  while leaving ambiguous constants unresolved;
- component stereotypes, composed stereotypes, `@Configuration`, `@Bean`,
  constructor injection, and Spring Data repository beans;
- Kafka, RabbitMQ, and application-event consumers and exact call-backed
  producers;
- scheduled methods, JPA entities and tables, and transaction/security traits.

Universal Java evidence now preserves field and parameter annotations,
annotation-element metadata, and interface-extension facts required by those
semantics. Cross-file selection remains in `compass-resolve`; the language
framework runtime emits bounded evidence rather than publishing graph records.

## Checked-fixture determinism

The six checked Java fixtures from routes, scheduling, messaging, and JPA were
copied into an isolated corpus and extracted three times with the final release
binary. Every run published 45 nodes and 60 edges. The three graph files were
byte-identical, and their independently canonicalized graphs and occurrence
streams were identical:

- graph bytes SHA-256:
  `ea284581c9bbd071ef14e05c8e0c9fdbf14afa2259fc0fe24511ba1068d38442`
- canonical graph SHA-256:
  `f8947e839f5054fa4efd3c496a46d312b8ca75b6eda430c74f12f5500243fcbd`
- occurrence SHA-256:
  `7e8b67bd538c1e08c0cf959e255b0e404018d696259e1f04ec4459b7eae53495`

Best-effort publication reported the same one-node/one-edge omission in each
run for the fixture's dangling external `RequestMethod` import. The omission is
stable and does not invent an endpoint.

The dedicated six-test Spring pack suite covers positive, negative, ambiguity,
identity, direction, source occurrence, and multiplicity behavior. The existing
eight JVM route tests and six domain-resolution tests remain green, including
the Kotlin legacy route.

## Pinned Spring Framework comparison

The real-corpus qualification uses the same immutable inputs as Java's final
post-cutover review:

- Spring Framework: `eceebb3077dda9e1b19d73c0398ef022cd91f99c`
- Graphify: `4fe11092ccbe9f543608f140c790f68d5d83cae4`
- established Compass Java result: `e4599f9`
- universal Spring pack implementation: `1946012aa67bba474f4016aa2d9f79010a3c1476`
- qualified candidate: `d0be66c`
- samples: three cold, warm, incremental, and restore runs per tool

Every sample was eligible. The final candidate published 168,791 nodes and
683,564 canonical edges; Graphify published 138,830 nodes and 477,066 edges.
Both outputs were deterministic, with Compass graph SHA-256
`5a20342e18f46b9a455d442854cb4dc77945a38a651447684388dbc02a720518`
and Graphify graph SHA-256
`cdf2873b915709886c137b9b6e54259c6a9aa1f037efe8c7dee5020f41df1b59`.

### Graph quality

The same strict classifier used for Java's established result classified
439,235 of 477,066 Graphify edges as exact, dominated, or rejected with
stronger evidence. Overall handled coverage is therefore **92.07%**, compared
with **70.90%** for established Java. The candidate retains **129.86%** of the
baseline, exceeding the 95% gate. No relation family regressed against the
established baseline; `case_of` remains unchanged and every other family
improves.

| Relation | Graphify | Established handled | Exact | Dominated | Rejected | Missing | Ambiguous | Candidate handled |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| calls | 112,195 | 48.51% | 15,016 | 65,314 | 7,414 | 24,451 | 0 | 78.21% |
| case_of | 872 | 0.00% | 0 | 0 | 0 | 872 | 0 | 0.00% |
| contains | 102,310 | 95.19% | 32,694 | 65,476 | 0 | 4,133 | 7 | 95.95% |
| extends | 4,454 | 86.06% | 2,477 | 1,727 | 121 | 116 | 13 | 97.10% |
| implements | 4,379 | 95.39% | 1,988 | 2,054 | 202 | 132 | 3 | 96.92% |
| imports | 126,118 | 59.13% | 31,108 | 37,968 | 56,183 | 859 | 0 | 99.32% |
| references | 126,738 | 81.94% | 12,009 | 94,949 | 12,535 | 7,237 | 8 | 94.28% |

Calls remain the largest Java relationship gap. Imports meet the requested
priority at 99.32% handled coverage; the remaining 859 Graphify import facts
are unchanged from Java's final candidate. The Spring pack adds framework
semantics only when backed by universal Java evidence and does not turn
unresolved source calls into invented targets.

### Latency

| Tool and workload | Eligible | p50 | p95 | Peak RSS |
|---|---:|---:|---:|---:|
| Compass cold | 3/3 | 101.815 s | 102.499 s | 9,133.80 MiB |
| Graphify cold | 3/3 | 181.975 s | 189.013 s | 1,505.78 MiB |
| Compass warm | 3/3 | 3.847 s | 3.969 s | 51.92 MiB |
| Graphify warm | 3/3 | 67.656 s | 78.430 s | 1,978.12 MiB |
| Compass incremental | 3/3 | 3.783 s | 3.880 s | 52.02 MiB |
| Graphify incremental | 3/3 | 77.317 s | 78.744 s | 1,977.12 MiB |

Compass is **1.787x** faster cold, **17.586x** faster warm, and **20.438x**
faster incrementally. This satisfies the Java qualification requirement that
Compass cold and warm latency be lower than Graphify.

The generic performance harness reports `FAIL` because it additionally
requires a 5x cold speedup, lower cold peak RSS, and zero missing or ambiguous
Graphify facts. Those are not the Java post-cutover acceptance gates: all
requested coverage, relation-family non-regression, determinism, and
lower-latency gates pass. The full machine-readable run is preserved outside
the repository under run ID `20260801T161731Z`; generated graphs and local
qualification state are not committed.

## Verification

The following checks passed against the final code candidate:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- `cargo test --workspace --lib --bins --locked`
- all `compass-languages` and `compass-resolve` tests
- the six-test `spring_universal_pack` integration suite
- the eight-test JVM route and six-test domain-resolution suites
- release construction of `compass-cli`
- `scripts/check_product_boundary.sh`

The all-target targeted Clippy run also reaches a pre-existing unrelated lint
in `crates/compass-resolve/tests/python_import_provenance.rs`; the required
workspace library/binary Clippy baseline is green.

The repository-wide `qualify_code_graph_v1.sh --fixtures-only` gate remains
red on pre-existing cross-language qualification-manifest drift: current
Python, Go, Rust, and Java universal identities differ from stale expected
names, and the Rust fixture expects `type_of`, `returns`, and `documents`
vocabulary not emitted by the current runtime. No Graphify dependency or
fallback was added to bypass that independent debt.

## Final status

The production universal framework-pack registry is no longer empty for
Spring: `spring-java` is registered and Java Spring interpretation is hard-cut
to it. The separate Spring gap identified by the Java review is closed. Java
intentionally remains `UniversalCandidate`; promotion to `UniversalComplete`
is outside this change.
