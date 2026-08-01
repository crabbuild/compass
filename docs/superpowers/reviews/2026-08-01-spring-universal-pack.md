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
- universal Spring pack: `1946012aa67bba474f4016aa2d9f79010a3c1476`
- samples: three cold, warm, incremental, and restore runs per tool

The comparison is recorded separately from the implementation commit so the
qualification report can cite the exact committed candidate that produced it.

## Verification

The following checks passed against the implementation commit:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --lib --bins --locked -- -D warnings`
- `cargo test --workspace --lib --bins --locked`
- all `compass-languages` and `compass-resolve` tests
- the six-test `spring_universal_pack` integration suite
- the eight-test JVM route and six-test domain-resolution suites
- release construction of `compass-cli`

The all-target targeted Clippy run also reaches a pre-existing unrelated lint
in `crates/compass-resolve/tests/python_import_provenance.rs`; the required
workspace library/binary Clippy baseline is green.

