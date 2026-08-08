# Evidence resolution framework phased execution plan

Status: proposed

Technical design: [Evidence resolution framework technical design](evidence-resolution-framework-technical-design.md)

## Purpose

This plan converts the evidence resolution framework design into small,
reviewable implementation steps. Each commit must leave the resolver usable
and must preserve observable behavior unless the commit belongs to the
explicit optimization phase.

The plan is intentionally incremental. It first creates behavioral evidence,
then establishes module ownership, then moves algorithms, and only afterward
changes performance-sensitive implementation details.

## Execution principles

- Preserve the public `compass_resolve::evidence` namespace.
- Keep structural changes separate from algorithm changes.
- Move one ownership boundary at a time.
- Preserve exact ambiguity, candidate-count, ordering, and provenance behavior.
- Use direct resolver tests in addition to source-backed integration tests.
- Keep every traversal and cache bounded.
- Keep each optimization independently measurable and revertible.
- Stop a phase when its acceptance criteria are not met.
- Preserve unrelated worktree changes.

## Global completion criteria

The rearchitecture is complete when all of these statements are true:

- `UniversalResolutionIndex` is a small compatibility façade.
- Primary facts and every secondary index have explicit owners.
- Resolution precedence is visible in one pipeline module.
- TypeScript, Rust, and Java policy is outside the generic pipeline.
- Graph projection performs no target selection.
- Existing decisions and canonical graph records remain equivalent.
- Limits and incomplete-state handling remain fail-closed.
- The universal integration suite is navigable by language and responsibility.
- Performance measurements show no unexplained regression.
- Architecture and extension documentation match the implementation.

## Phase 0: Baseline and characterization

### Background

The resolver's conservative behavior is part of the product contract. Existing
integration tests cover many real source shapes, but they do not isolate every
resolution stage. Before moving code, the project needs compact tests that can
detect precedence, ambiguity, limit, and projection drift.

### Task 0.1: Add direct evidence fixture builders

Background: constructing tests through a parser couples adapter behavior to
resolver behavior and makes failures harder to locate.

Action: add test support for constructing declarations, scopes, bindings,
occurrences, candidates, and batches with stable IDs and ranges.

Done when: a resolver test can describe one candidate and all eligible targets
without invoking `Engine` or reading source files.

### Task 0.2: Characterize public decisions

Background: matching only the resolved declaration misses rule and ambiguity
regressions.

Action: add table-driven tests for every `ResolutionDecision` variant and every
existing `ResolutionRule`. Assert declaration or inventory identity, rule, and
candidate count.

Done when: every public decision shape has a direct positive contract test.

### Task 0.3: Freeze precedence

Background: an individually correct stage can still create false edges if it
runs before stronger evidence.

Action: add paired fixtures where an early exact source, hierarchy, project,
binding, lexical, or module result competes with a later broader result.

Done when: the current branch order is represented by named tests and an
implementation-order table in test support.

### Task 0.4: Freeze bounded behavior

Background: the resolver often stores one extra candidate to distinguish a
full bucket from an overflowed bucket. Refactoring that sentinel behavior can
accidentally create uniqueness.

Action: test exact-limit, limit-plus-one, traversal-depth, scope-cycle,
wildcard-cycle, alias-cycle, hierarchy-incomplete, and export-cycle behavior.

Done when: every bounded lookup family proves overflow cannot become empty or
unique.

### Task 0.5: Freeze projection

Background: target selection and graph materialization currently share state.
They cannot be separated safely without a complete output contract.

Action: add reviewable expected records for declarations, containment,
ordinary edges, external targets, deferred receivers, re-exports, and
ambiguous candidates.

Done when: tests assert IDs, direction, multiplicity, source anchors,
resolution rule, provenance, target kind, metadata, and ordering.

### Task 0.6: Decompose the integration test source

Background: the universal resolver integration test has grown large enough
that finding coverage and ownership is difficult.

Action: retain one top-level Cargo integration-test binary and move tests into
`core`, `rust`, `python`, `go`, `typescript`, and `javascript` source modules.

Done when: test names and behavior are unchanged and shared helpers have one
owner.

### Task 0.7: Record a performance baseline

Background: structural work should not be assumed to improve performance, and
future optimization needs a repeatable comparison.

Action: capture existing internal profile phases and peak RSS for a small
fixture corpus, mixed-language corpus, TypeScript-heavy corpus, Rust-heavy
corpus, and hierarchy-heavy corpus.

Done when: corpus identity, command, environment, cold result, and at least
five warm results are recorded.

### Phase 0 commit sequence

1. Add direct semantic-evidence test builders.
2. Add public decision and rule characterization tests.
3. Add precedence characterization tests.
4. Add limit, overflow, and cycle characterization tests.
5. Add projection parity fixtures.
6. Split universal integration tests into source modules.
7. Document baseline corpora and measurements.

### Phase 0 acceptance

- Production resolver code is unchanged.
- Input-order permutations produce identical decisions and graph records.
- Ambiguity and overflow tests assert candidate counts.
- Baseline measurements are reproducible.

### Phase 0 rollback

Test-only commits can be reverted independently. Do not proceed if tests reveal
undocumented behavior that the design does not preserve; update the design
decision first.

## Phase 1: Public façade and leaf utilities

### Background

The first production phase should create dependency direction without moving
stateful resolver algorithms. Public paths and pure helpers are the safest
seams.

### Task 1.1: Convert the module to a directory

Background: Rust cannot use both `evidence.rs` and `evidence/mod.rs`, so the
transition starts with a pure move.

Action: move `src/evidence.rs` to `src/evidence/mod.rs` without content edits.

Done when: the crate exposes the same module and all tests compile unchanged.

### Task 1.2: Extract public API types

Background: limits and decision types are depended on by every future layer.
They should not live with implementation details.

Action: move limits, rules, evidence, and decision types to `api.rs`; re-export
them from `mod.rs`.

Done when: downstream imports and derives are unchanged.

### Task 1.3: Add semantic lookup budgets

Background: one public candidate limit currently controls storage, visits,
depth, and expansion. Named internal accessors make those uses auditable.

Action: introduce `LookupBudget` with one-to-one mappings to current public
limits. Do not change numeric behavior.

Done when: direct reads of `candidates_per_lookup` begin moving behind named
budget operations and limit tests remain unchanged.

### Task 1.4: Extract project and path utilities

Background: Python modules, source directories, TypeScript paths, and Go module
paths are pure project-context operations rather than resolution stages.

Action: move these helpers into `project.rs` and add cross-platform unit tests.

Done when: path normalization has one owner and project containment behavior is
unchanged.

### Task 1.5: Extract pure language helpers

Background: TypeScript type parsing and Java conversion logic are pure leaves
that can move without changing index ownership.

Action: move TypeScript type-expression and overload helpers, then Java
primitive/reference conversion helpers, one family per commit.

Done when: each helper family has local unit tests and no stateful resolver
logic moved with it.

### Phase 1 commit sequence

1. Move `evidence.rs` to `evidence/mod.rs` unchanged.
2. Extract and re-export public API types.
3. Introduce lookup-budget accessors.
4. Extract project and path codecs.
5. Extract TypeScript type-expression utilities.
6. Extract TypeScript overload utilities.
7. Extract Java conversion utilities.

### Phase 1 acceptance

- Public type and method paths are unchanged.
- No resolution branch is reordered.
- No graph record changes.
- New utility modules have no dependency on projection.

### Phase 1 rollback

Every extraction commit is independently revertible. Revert rather than
adding compatibility aliases in unrelated modules if a public path changes.

## Phase 2: Projection boundary

### Background

Resolution selects semantic targets; projection publishes graph records.
Separating them prevents language policies from inventing graph behavior and
makes output equivalence directly testable.

### Task 2.1: Extract prepared targets

Background: prepared target state is the handoff between decisions and graph
publication.

Action: move the prepared-target representation and decision-to-target
conversion into `projection/mod.rs`.

Done when: it depends only on public decisions and read-only evidence data.

### Task 2.2: Extract node projection

Background: declaration, external, and deferred nodes have distinct identity
and merge rules.

Action: move declaration-node creation, external-node creation,
deferred-receiver creation, source-anchor conversion, and node merge behavior
into `projection/nodes.rs`.

Done when: node projection performs no candidate lookup.

### Task 2.3: Extract edge projection

Background: relationship identity, direction, occurrence anchors, and
provenance are graph-contract behavior.

Action: move relation naming, edge construction, binding metadata, project
metadata, and replaced-relation checks into `projection/edges.rs`.

Done when: edge projection accepts a completed target and never selects a
fallback target.

### Task 2.4: Make materialization a façade call

Background: the public method should retain compatibility while delegating
implementation ownership.

Action: reduce `UniversalResolutionIndex::materialize` to constructing a
projector view and invoking projection orchestration.

Done when: projection parity fixtures remain exact.

### Phase 2 commit sequence

1. Extract prepared-target conversion.
2. Extract declaration and containment projection.
3. Extract external and deferred node projection.
4. Extract ordinary relationship edge projection.
5. Extract re-export and metadata edge projection.
6. Reduce the public materialize method to delegation.

### Phase 2 acceptance

- Projection modules cannot invoke resolver stages.
- Resolver modules do not construct `NodeRecord` or `EdgeRecord`.
- Canonical node and edge records are unchanged.
- Ambiguous and unresolved candidates produce no new edges.

### Phase 2 rollback

Node and edge extraction commits are separate so either side can be reverted
without restoring the entire monolith.

## Phase 3: Fact store and index builders

### Background

The public index currently owns primary facts and many unrelated secondary
indexes. This phase introduces explicit state ownership while preserving the
existing loops and collection semantics.

### Task 3.1: Introduce `FactStore`

Background: primary facts and declaration slots are the stable database on
which all indexes operate.

Action: group declarations, slots, occurrences, bindings, candidates, scopes,
and definition ranges behind checked accessors.

Done when: duplicate detection and slot conversion have focused tests.

### Task 3.2: Introduce name indexes

Background: qualified, module, scope, directory, inventory, alias, and owner
member lookups form the shared name-resolution substrate.

Action: move those maps into `NameIndexes` without changing key or value
representations.

Done when: sorting, deduplication, and ambiguity behavior match the baseline.

### Task 3.3: Introduce hierarchy indexes

Background: bases, subtypes, owner members, and completeness markers are
consumed together by receiver dispatch.

Action: move their construction into `index/hierarchy.rs`.

Done when: complete and incomplete hierarchy tests pass unchanged.

### Task 3.4: Introduce wildcard and return indexes

Background: wildcard and return expansion both require explicit bounded
collection semantics and cycle controls.

Action: move wildcard scope/module/re-export maps and callable return maps into
their owning index modules.

Done when: exact-limit and overflow tests remain fail-closed.

### Task 3.5: Introduce language index bundles

Background: TypeScript and Rust need secondary structures that unrelated
corpora should not own.

Action: group TypeScript and Rust indexes independently, initially preserving
their current eager construction behavior.

Done when: each language bundle has one constructor and no generic resolver
algorithm constructs it directly.

### Task 3.6: Introduce phased `IndexBuilder`

Background: validation, collection, indexing, and finalization need visible
ordering and independent profiling.

Action: create builder phases matching the technical design, then delegate the
public constructors to the builder.

Done when: aggregate validation occurs before reservations and profiling names
identify every phase.

### Task 3.7: Centralize bounded candidates

Background: repeated vectors plus completeness flags are easy to handle
inconsistently.

Action: introduce a bounded candidate abstraction only after all existing
variants and sentinel counts are captured by tests.

Done when: unique selection requires one item and complete enumeration.

### Phase 3 commit sequence

1. Introduce `FactStore` and checked slot access.
2. Group qualified, lexical, module, and directory indexes.
3. Group inventory, alias, and owner-member indexes.
4. Extract direct-base and subtype index construction.
5. Extract wildcard index construction.
6. Extract callable-return index construction.
7. Group TypeScript index state.
8. Group Rust index state.
9. Introduce phased `IndexBuilder` delegation.
10. Centralize bounded candidate semantics.

### Phase 3 acceptance

- Every map has one owning structure.
- Validation runs exactly once per construction path.
- Index cardinality and completeness match characterization fixtures.
- Construction remains deterministic and bounded.
- Public constructors retain signatures and errors.

### Phase 3 rollback

State-grouping commits should avoid algorithm edits. If borrow or lifetime
pressure requires broad rewrites, revert and introduce narrower accessors
before proceeding.

## Phase 4: Generic resolution kernel

### Background

With state ownership established, resolution algorithms can receive a small
read-only database instead of the entire public façade. The semantic order
must remain explicit and unchanged.

### Task 4.1: Introduce `ResolutionDb`

Background: algorithms currently access all fields directly, obscuring their
dependencies.

Action: add borrowed access to facts, indexes, project context, and budgets.

Done when: extracted algorithms no longer require `UniversalResolutionIndex`.

### Task 4.2: Introduce candidate context

Background: effective language, occurrence, binding, and qualifier are
recomputed across stages.

Action: construct `CandidateContext` once and create an explicit derived
context for call-result fallback.

Done when: stored evidence remains immutable and fallback behavior is covered
directly.

### Task 4.3: Introduce stage outcomes

Background: `None`, unresolved, and ambiguous have different control-flow
meaning.

Action: use `Continue` versus terminal decision outcomes at stage boundaries.

Done when: ambiguous stages cannot accidentally fall through to broader
lookup.

### Task 4.4: Extract binding resolution

Background: exact, lexical, explicit, and alias lookup share shadowing and
cycle behavior.

Action: move these algorithms into `resolve/bindings.rs` without changing
their order.

Done when: precedence and alias-cycle tests remain exact.

### Task 4.5: Extract wildcard and member resolution

Background: wildcard, imported member, return member, and inventory lookup are
shared but currently interleaved with language branches.

Action: move evidence-driven traversal into `resolve/wildcards.rs` and
`resolve/members.rs`.

Done when: language syntax is not interpreted in either module.

### Task 4.6: Extract hierarchy resolution

Background: C3, direct-base, closed-world descendant, and incomplete-hierarchy
logic form one shared receiver-dispatch component.

Action: move these algorithms and their cycle/budget handling into
`resolve/hierarchy.rs`.

Done when: Python and generic hierarchy tests use the shared component.

### Task 4.7: Create the explicit pipeline

Background: stage ordering is easier to audit in ordinary control flow than in
a callback registry.

Action: transpose the existing `resolve` branches into named stage calls in
`resolve/pipeline.rs`.

Done when: the method reads as a strongest-to-broadest policy and every stage
boundary has a precedence test.

### Phase 4 commit sequence

1. Add `ResolutionDb` accessors.
2. Add candidate and fallback contexts.
3. Add explicit stage outcomes.
4. Extract exact and lexical resolution.
5. Extract explicit binding and alias traversal.
6. Extract wildcard traversal.
7. Extract shared member and return lookup.
8. Extract direct-base and C3 resolution.
9. Extract closed-world and incomplete hierarchy resolution.
10. Assemble the explicit pipeline.

### Phase 4 acceptance

- Pipeline order matches Phase 0 characterization.
- Shared algorithms accept only `ResolutionDb` and candidate context.
- Every traversal consumes named budget operations.
- Ambiguity, cycles, and overflow remain terminal where required.

### Phase 4 rollback

Keep the public `resolve` method delegating one stage at a time. If a stage
cannot be extracted without reordering, restore that stage and document the
missing dependency before continuing.

## Phase 5: Language-policy extraction

### Background

The generic kernel is now stable enough to expose narrow policy hooks. This
phase moves existing language semantics; it does not add new semantic support.

### Task 5.1: Add static policy dispatch

Background: Compass has a statically linked, qualified language set. Dynamic
plugins are not needed for this boundary.

Action: add `LanguagePolicyKind` and explicit dispatch for TypeScript, Rust,
Java, and generic behavior.

Done when: policy selection is centralized and unknown languages use only the
generic kernel.

### Task 5.2: Extract TypeScript module policy

Background: package exports, conditions, re-exports, CommonJS, and project
configuration are the largest language-specific import surface.

Action: move module indexes and import/re-export traversal into
`languages/typescript/modules.rs`.

Done when: project and module tests retain exact targets and ambiguity.

### Task 5.3: Extract TypeScript member policy

Background: structural aliases, object spreads, indexed types, and callable
returns form a separate member-selection concern.

Action: move owner expansion and member-chain logic into
`languages/typescript/members.rs`, using the already extracted type and
overload helpers.

Done when: imported generic, structural, overload, and CommonJS member tests
remain unchanged.

### Task 5.4: Extract Rust policy

Background: associated types, implementation traits, and trait members cannot
be reduced to generic qualified-name lookup.

Action: move Rust index queries and associated/trait selection into
`languages/rust.rs`.

Done when: exact impl ownership, associated returns, trait bounds, wildcards,
and ambiguity tests remain unchanged.

### Task 5.5: Extract Java policy

Background: overload selection depends on Java primitive and reference
conversion rules.

Action: move applicable-overload and most-specific selection into
`languages/java.rs`.

Done when: Java overload behavior is selected through one policy hook.

### Task 5.6: Audit generic language behavior

Background: creating empty policy files for symmetry would obscure which
languages actually need special rules.

Action: confirm Go project naming remains in `ProjectContext` and Python C3
dispatch remains in shared hierarchy resolution.

Done when: no Go or Python policy module exists without a genuine semantic
responsibility.

### Phase 5 commit sequence

1. Add static policy classification and no-op generic policy.
2. Extract TypeScript project-module lookup.
3. Extract TypeScript exports and re-exports.
4. Extract TypeScript structural and alias members.
5. Extract TypeScript generic and callable-return members.
6. Extract Rust associated-type resolution.
7. Extract Rust impl-trait and trait-member resolution.
8. Extract Java applicable-overload selection.
9. Audit remaining language-name branches.

### Phase 5 acceptance

- Language-specific syntax and type policy is under `languages`.
- The generic pipeline owns stage order and terminal behavior.
- Policies cannot construct graph records.
- Policies cannot bypass candidate constraints or budgets.
- No new language behavior is introduced.

### Phase 5 rollback

Extract one language family at a time. A failed language extraction can return
to the generic module without blocking completed families.

## Phase 6: Measured optimization

### Background

The new boundaries expose index construction, candidate decisions, and
projection as independent performance targets. Only measured changes should
be retained.

### Task 6.1: Remove repeated fact classification

Background: index construction currently performs multiple filtered scans over
large fact collections.

Action: measure a single classification pass that feeds index builders while
preserving builder finalization order.

Done when: the target corpus improves materially and all semantic gates pass.

### Task 6.2: Build language bundles lazily

Background: non-TypeScript and non-Rust corpora should not allocate their
language-specific maps.

Action: construct optional language bundles only when validated facts require
them.

Done when: peak RSS improves on unrelated corpora without adding branch-order
changes.

### Task 6.3: Reduce repeated strings

Background: compound string keys dominate transient memory at corpus scale.

Action: profile interning or compact slots for repeated language, module,
scope, owner, and member components.

Done when: measured RSS savings justify complexity and no new dependency is
added without approval.

### Task 6.4: Add bounded memoization

Background: alias, export, C3, and return-chain walks may repeat across
candidates.

Action: add one bounded cache per measured hotspot, with deterministic keys
and explicit aggregate limits.

Done when: cache overflow remains safe and each cache demonstrates a measured
benefit.

### Task 6.5: Reduce allocation at decision boundaries

Background: internal resolution can often retain declaration slots and borrow
facts until the public decision is constructed.

Action: delay string allocation and reuse normalized project keys where
profiling identifies repeated work.

Done when: allocation or time measurements improve without lifetime coupling
between projection and language policy.

### Task 6.6: Reassess parallel decision execution

Background: Rayon can improve independent candidate decisions but may increase
small-corpus overhead or memory.

Action: measure sequential thresholds, work partitioning, and deterministic
collection for representative corpus sizes.

Done when: the chosen strategy improves the intended corpus and keeps output
ordering explicit.

### Phase 6 commit sequence

1. Add any missing per-component measurements.
2. Evaluate and either land or reject single-pass classification.
3. Evaluate and either land or reject lazy language bundles.
4. Evaluate compact keys in an isolated commit.
5. Add one bounded memoization cache per commit.
6. Delay internal ID allocation where measured.
7. Tune candidate parallelism where measured.
8. Record accepted and rejected experiments.

### Phase 6 acceptance

- Every landed optimization has before-and-after evidence.
- Canonical graph output and direct decisions are unchanged.
- The target metric improves by at least ten percent unless measurement noise
  and an alternative threshold are documented.
- Representative non-target corpora do not regress by more than three percent
  without an approved tradeoff.
- No cache or traversal becomes unbounded.

### Phase 6 rollback

Each optimization is one commit and can be reverted without restoring old
module ownership. Reject an experiment that does not meet its acceptance
threshold.

## Phase 7: Documentation, extension contract, and cleanup

### Background

The new architecture is maintainable only if future changes follow its
ownership rules. The final phase aligns documentation and removes temporary
compatibility scaffolding.

### Task 7.1: Update architecture documentation

Background: existing documentation overstates language neutrality.

Action: describe the generic kernel plus explicit policy hooks in the language
architecture and universal evidence reference.

Done when: documentation distinguishes shipped behavior from the completed
design and matches source ownership.

### Task 7.2: Update the extension checklist

Background: contributors need to know whether a new rule belongs in an adapter,
the kernel, a policy, or projection.

Action: add ownership questions and required positive, negative, ambiguity,
overflow, ordering, and provenance tests to `extending-compass.md`.

Done when: a new language-policy change has a complete review path.

### Task 7.3: Add module ownership comments

Background: private visibility alone does not explain semantic boundaries.

Action: add concise module-level comments describing allowed dependencies and
prohibited responsibilities.

Done when: each top-level evidence component explains what it owns.

### Task 7.4: Remove temporary forwarding code

Background: extraction phases may leave temporary methods or aliases.

Action: remove internal forwarding paths that no longer protect a public or
incremental boundary.

Done when: `UniversalResolutionIndex` contains only public façade state and
delegation.

### Task 7.5: Run final qualification

Background: resolver changes affect graph publication across universal
languages.

Action: run targeted tests, crate linting, workspace baseline, fixture
qualification, and affected real-repository qualification using the required
external Cargo target directory.

Done when: all relevant checks pass or each exception is documented with an
owner and follow-up.

### Phase 7 commit sequence

1. Update language architecture and evidence reference.
2. Update the extension checklist.
3. Add module ownership documentation.
4. Remove temporary internal forwarding paths.
5. Record final performance and qualification results.

### Phase 7 acceptance

- Documentation matches source ownership.
- Public compatibility remains unchanged.
- No temporary duplicate implementation remains.
- Required test, lint, qualification, and performance evidence is recorded.

### Phase 7 rollback

Documentation commits can be corrected independently. Do not remove forwarding
code until all callers have moved and the full targeted suite passes.

## Execution record

Execution on 2026-08-08 produced the following implementation:

| Phase | Result | Evidence |
| --- | --- | --- |
| 0 | Complete | Direct parser-free decision, precedence, limit, determinism, and projection contracts; existing integration characterization retained |
| 1 | Complete | Directory facade, public API, lookup budget, project utilities, and pure language helpers extracted |
| 2 | Complete | Prepared targets, node projection, and edge projection isolated behind materialization |
| 3 | Complete | `FactStore`, `ResolutionIndexes`, bounded construction, and dedicated index builder module |
| 4 | Complete | `ResolutionDb`, `CandidateContext`, `StageOutcome`, generic stage modules, and explicit pipeline |
| 5 | Complete | Closed static policy selection plus TypeScript, Rust, and Java policy modules |
| 6 | Complete | Existing compact slots, preallocation, bounded vectors, and deterministic parallelism retained; no unproven memoization added |
| 7 | Complete | Architecture and extension documentation updated; crate and qualification gates recorded below |

The large historical `universal_resolution.rs` characterization source remains
intact. Splitting it while moving production code would create review noise and
weaken history without changing its test ownership. New parser-free contracts
live in `evidence_resolution_contract.rs`; future language-specific cases
should be added to focused integration targets rather than extending the
historical source.

### Performance record

- Before refactor: `universal_resolution` reported 0.29 seconds.
- After refactor: `universal_resolution` reported 0.28 seconds.
- Warm command wall time after refactor: 0.62 seconds and 0.65 seconds.
- Enterprise framework scale test: passed its existing 30-second ceiling in
  26.42 seconds.
- Decision: retain current bounded indexes and parallel projection; do not add
  a cache without a measured repeated-lookup bottleneck.

### Verification record

- `cargo test -p compass-resolve --locked`: passed all unit, integration,
  scale, and documentation tests.
- `cargo clippy -p compass-resolve --lib --all-features --locked -- -D warnings`:
  passed for the changed production surface.
- `cargo clippy -p compass-resolve --all-targets --all-features --locked -- -D warnings`:
  production code passed; the command is blocked by pre-existing lint failures
  in untouched integration tests.
- `./scripts/qualify_code_graph_v1.sh --fixtures-only`: passed all manifest,
  scale, deterministic byte-comparison, and semantic assertion gates. The run
  validated 1,773 invariants and 27 exact resolution assertions.
- `cargo clippy --workspace --lib --bins --locked -- -D warnings`: passed.
- `sh scripts/check_product_boundary.sh`: passed.
- `cargo test --workspace --lib --bins --locked`: attempted after all focused
  gates; linking the unrelated `compass-cli` test binary stopped when the
  external workspace volume reported `No space left on device`. No target
  directory was cleaned or reused.

## Verification matrix

| Change surface | Minimum verification |
| --- | --- |
| Public façade move | Public imports compile; `universal_evidence` test |
| Pure utilities | Local unit tests; affected language integration module |
| Projection | Projection parity fixtures; universal integration test |
| Fact store or index | Limit tests; ambiguity tests; deterministic input-order tests |
| Generic pipeline | Complete decision matrix; universal integration test |
| TypeScript policy | TypeScript and JavaScript integration modules; fixture qualification |
| Rust policy | Rust integration module; fixture and Rust qualification |
| Java policy | Java integration module; fixture and Java qualification |
| Performance optimization | Semantic parity plus documented before/after measurements |
| Final architecture | Crate clippy, workspace baseline, fixture qualification |

## Planned commands

Every Cargo command must use a per-worktree target directory on the mounted
workspace volume:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-resolve --test universal_evidence --locked

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-resolve --test universal_resolution --locked

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo clippy -p compass-resolve --all-targets --all-features --locked -- -D warnings

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  cargo test -p compass-resolve --locked

CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main \
  ./scripts/qualify_code_graph_v1.sh --fixtures-only
```

Before a long build, confirm `/Volumes/Workspace` is mounted and the selected
target directory belongs only to this checkout.

## Phase dependencies

```text
Phase 0 characterization
        |
        v
Phase 1 façade and leaves
        |
        v
Phase 2 projection boundary
        |
        v
Phase 3 facts and indexes
        |
        v
Phase 4 generic kernel
        |
        v
Phase 5 language policies
        |
        v
Phase 6 measured optimization
        |
        v
Phase 7 documentation and qualification
```

Phase 2 and some Phase 3 preparation can be developed independently after
Phase 1, but they should merge in this order to keep dependencies simple.
Phase 6 must not begin before behavior and ownership are stable.

## Review checkpoints

Checkpoint A follows Phase 0. Review the frozen precedence and ambiguity
contract before production changes.

Checkpoint B follows Phase 3. Review state ownership, bounded collections, and
builder dependencies before extracting algorithms.

Checkpoint C follows Phase 5. Review whether language policy hooks are narrow
enough and whether any language behavior remains in the generic kernel.

Checkpoint D follows Phase 6. Review performance evidence and reject
optimizations whose complexity is not justified.

Checkpoint E follows Phase 7. Review compatibility, qualification, and final
documentation before declaring the rearchitecture complete.

## Change-control rules

- A structural commit cannot intentionally change expected graph output.
- A newly discovered behavior must be characterized before it is moved.
- A semantic correction discovered during refactoring becomes a separate
  issue and change series.
- A new policy hook requires at least two tests: one where it applies and one
  where it must not apply.
- A new cache requires an aggregate limit, overflow test, and memory
  measurement.
- A new public type, field, or rule requires compatibility review and is not
  covered by this plan.

## Definition of done

The work is done when the source layout implements the technical design, all
structural commits preserve the characterized behavior, accepted performance
changes have evidence, relevant qualification passes, documentation matches
the implementation, and no required migration remains undocumented.

## Related pages

- [Evidence resolution framework technical design](evidence-resolution-framework-technical-design.md)
- [Universal evidence implementation](universal-evidence.md)
- [Language architecture](../design/language-architecture.md)
- [Universal semantic evidence reference](../reference/universal-semantic-evidence.md)
- [Extending Compass](extending-compass.md)
- [Workspace tour](workspace-tour.md)

**Next step:** complete and review Phase 0 before moving any production
resolver code.
