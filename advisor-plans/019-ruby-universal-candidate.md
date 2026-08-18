# Plan 019: Hard-cut Ruby to a qualified universal candidate

> **Executor instructions**: Deliver this as a sequence of reviewable PRs. Read
> this plan completely, then read `AGENTS.md`,
> `docs/design/language-architecture.md`,
> `docs/implementation/universal-evidence.md`,
> `docs/implementation/evidence-resolution-framework-technical-design.md`, and
> `docs/reference/universal-semantic-evidence.md` before changing source. Run
> every verification gate and confirm its expected result before starting the
> next phase. Ruby must not have production dual-running: keep the established
> path active while the candidate is qualification-only, then switch the
> registry, Ruby publisher, resolver, and Rails source pack atomically.
>
> **Drift check (run before every phase)**:
> `git diff --stat b53c3ea2..HEAD -- crates/compass-files crates/compass-languages crates/compass-resolve crates/compass-model crates/compass-graph crates/compass-core fixtures/code-graph tests/qualification scripts benchmarks/performance docs PERFORMANCE.md COMPATIBILITY.md MIGRATION.md CHANGELOG.md advisor-plans`
> Reconcile any changed adapter versions, evidence fields, Ruby/Rails paths,
> qualification thresholds, or cache contracts before proceeding. If live code
> contradicts the “Current state” section, stop and update this plan first.

## Status

- **Priority**: P1
- **Effort**: XL, delivered as eight independently reviewable phases
- **Risk**: HIGH
- **Depends on**: no implementation prerequisite; the final scheduled/release
  gate should consume plan 005 or an equivalent exact-commit qualification gate
- **Category**: language architecture, correctness, resolution, framework,
  performance, tests, documentation
- **Planned at**: commit `b53c3ea2`, 2026-08-16

### Execution status (2026-08-17)

The implementation has completed the semantic hard cut through Phase 5. Ruby
is now published through `compass.ruby.candidate` and the `rails-ruby`
universal pack, with the old Ruby publisher/resolver removed from production.
Phase 0 source-oracle and pinned-commit evidence is complete. Phase 6 now has
a corrected file-only incremental publication path plus copy-on-write snapshot
staging: a five-sample unchanged update on a 305-file Rails subtree reuses all
305 files in a 0.1959–0.2045 second range (0.1963 second median), while the
fact-neutral edit extracts one file and restores byte-for-byte. Full Rails
qualification records a 2.9616 second unchanged warm median, a 165.412 second
fact-neutral edit, and exact restore hash equality. Ruby-only captures for
Rails, Discourse, and RuboCop are
recorded; full mixed-language Discourse/RuboCop roots remain outside the Ruby
qualification scope because their Markdown parser resource envelope is not
yet hardened. The delivered hardening includes an explicit 8 MiB pipeline
worker stack, constant-time `super` owner lookup, allocation-light reopened-
type hierarchy checks, deferred shared-store GC for small incremental
publications, exact fact-state admission (preventing stale cached-source
graphs), and copy-on-write snapshot staging with a portable copy fallback.
The fact-neutral publisher retains unchanged-file extraction status,
parser-recovery diagnostics, and coverage instead of reclassifying cached
files as extracted; the strict fixture restore gate passes byte-for-byte.
Phase 7 audit gates now pass: 89,981 accepted relationships, 100% observed
precision, 98.5567% source-oracle recall, zero ambiguity, and zero critical
violations. Ruby remains intentionally `UniversalCandidate`; promotion is a
separate decision and has not been made here.

## Why this matters

Ruby is recognized and parsed, but it still uses Compass's generic publisher
and a separate Ruby member resolver. That path captures classes, modules,
methods, simple calls, and one superclass, but it cannot represent Ruby's
constant nesting, reopened types, instance-versus-singleton method spaces,
ordered mixins, literal metaprogramming, or conservative receiver dispatch with
the evidence and ambiguity guarantees used by newer language adapters.

The practical result is visible on Rails-scale code: the July qualification
published 59,504 nodes and 96,958 edges but omitted 1,457 edges, with invalid
class-to-module inheritance/mixin endpoints called out as a recurring defect.
Ruby should move to the shared universal evidence and resolution path so exact
anchors, limits, ambiguity, provenance, incremental caching, and framework
facts are enforced consistently.

The intended endpoint of this plan is a hard-cut version-1 Ruby
`UniversalCandidate`, not `UniversalComplete`. Candidate promotion remains
blocked on the independent 2,000-relationship quality audit and all precision,
recall, and zero-tolerance gates in
`docs/reference/universal-semantic-evidence.md`.

## Current state

- `crates/compass-languages/src/registry.rs:326-332` recognizes `.rb` and
  `.rake` as the generic Ruby language. Shebang recognition is also present.
- `crates/compass-languages/src/config.rs:83-93` configures only `class`,
  `module`, `method`, `singleton_method`, and generic `call` syntax. It does not
  encode Ruby identity or scope semantics.
- `crates/compass-languages/src/engine.rs:1665-1670` contains a Ruby-only
  superclass branch inside the generic walker.
- `crates/compass-languages/src/engine.rs:2180-2204` turns unresolved Ruby calls
  into `RawCall` values; receiver type is deliberately absent.
- `crates/compass-languages/src/engine.rs:2341-2356` parses Ruby calls through
  only the `method` and optional `receiver` fields.
- `crates/compass-languages/src/engine.rs:3216-3231` extracts only the first
  superclass constant and immediately creates a raw inheritance edge.
- `crates/compass-resolve/src/members.rs:107` always invokes the established
  Ruby resolver when raw calls are present.
- `crates/compass-resolve/src/members.rs:611-676` indexes bare constant labels
  globally, resolves only unique terminal names, treats mixin calls specially,
  and otherwise resolves capitalized receivers or a missing `receiver_type`.
  It has no lexical constant path, require/load evidence, reopen handling, or
  separate singleton/instance member space.
- `crates/compass-languages/src/adapters.rs:270+` has no Ruby adapter profile.
  The closest dedicated emitter is `evidence/php.rs`; the closest small
  candidate and hard-cut tests are the Kotlin emitter/conformance/resolver
  suites.
- `crates/compass-resolve/src/evidence/languages/policy.rs:9-30` has no Ruby
  policy variant. Unknown languages use only generic resolution.
- `crates/compass-languages/src/frameworks/mod.rs:292` registers Rails as the
  established `rails-routes` source pack.
- `crates/compass-languages/src/frameworks/ruby.rs:11-127` receives a syntax
  tree but scans source lines and regular expressions inside
  `Rails.application.routes.draw`; it does not consume universal Ruby evidence.
- `crates/compass-resolve/src/frameworks/mod.rs:79-105` has no universal Rails
  expansion adapter.
- `crates/compass-languages/src/project_evidence.rs:789` records Gemfile
  dependencies, but project evidence has no Ruby load-root, require, autoload,
  or Zeitwerk contract.
- `tests/qualification/code-graph-v1-corpus.json` checks only that a trivial
  Ruby file is classified. Rails flow fixtures exist, but there is no Ruby
  universal conformance fixture, independent source oracle, quality-audit
  manifest, or language-specific performance manifest.
- `docs/superpowers/reviews/2026-07-30-best-effort-heavy-framework-qualification.md`
  records the older Rails baseline: 4,973 tracked files, 44.64 seconds cold,
  59,504 nodes, 96,958 edges, one omitted node, 1,457 omitted edges, and zero
  identity collisions. This is historical evidence only; Phase 0 must recapture
  the baseline at the planned commit.

### Post-implementation state

The pre-change inventory above is retained as the rationale for the cut. The
delivered tree now has a bounded `evidence/ruby.rs` emitter, a Ruby adapter
profile (`compass.ruby.candidate`, version 1), Ruby method-space-aware
resolution, exact contained `require_relative` decisions, and a single
`rails-ruby` universal framework pack. Ruby extraction has no production
`RawCall` publisher, and the replaced Ruby member resolver and Rails line/regex
route publisher are no longer active. The pipeline worker stack is explicitly
bounded at 8 MiB so deep valid Ruby DSL/test trees produce bounded partial
evidence instead of aborting the build. The independent Ripper oracle and
qualification harness remain separate from production and Graphify.

## Required semantic decisions

Freeze these decisions in tests and documentation before writing the complete
emitter. Do not let implementation convenience decide them implicitly.

1. **Constants and lexical nesting**: canonical type/constant names use Ruby's
   `A::B` spelling. `module A; class B` and `class A::B` retain different
   lexical lookup scopes even when they name the same runtime constant.
2. **Ruby modules**: publish Ruby `module` declarations as graph `trait` nodes.
   A Ruby module is both a constant namespace and a composable method owner;
   `trait` is the existing v1 type/container kind that can legally participate
   in `mixes_in`. Preserve language=`ruby` and the Ruby source spelling so
   consumers can distinguish it from traits in other languages. Do not publish
   a second duplicate `module` node for the same Ruby constant.
3. **Reopening**: every source declaration keeps its own evidence identity and
   anchor, while all reopenings of one fully qualified class/module share one
   graph node identity. Members from reopenings join the same owner only after
   exact constant resolution. Competing method definitions remain ambiguous
   unless source/load evidence establishes one definition; filesystem or batch
   order is never a tiebreaker.
4. **Method spaces**: instance and singleton methods are distinct. Use one
   documented codec throughout extraction, resolution, queries, and Rails
   handlers (recommended: `Owner#method` for instance methods and
   `Owner.method` for singleton methods). Never collapse `def call` and
   `def self.call` onto one declaration ID.
5. **Top-level methods**: give top-level methods a source-scoped identity.
   Cross-file binding requires explicit, contained require/load evidence; do
   not treat every top-level method in the repository as one global overload
   set.
6. **Mixins**: `include`, `prepend`, and `extend` emit exact `UsesTrait`
   occurrences with context identifying the operation. All publish
   `mixes_in`; dispatch may use them only where the Ruby policy proves a unique
   target. `extend` affects the receiver's singleton method space; it must not
   inject instance methods.
7. **Dynamic behavior**: `send`, `public_send`, `method_missing`, runtime
   `class_eval`/`module_eval`, nonliteral `define_method`, dynamic constants,
   and runtime load-path mutation never create convenient exact edges. Literal
   forms may emit bounded occurrences or declarations only when the source
   construct and owner are exact.
8. **Interop**: JRuby or native extensions do not authorize Ruby-to-Java/C
   terminal-name matching. Cross-language calls require exact fresh compiler
   or project evidence at both anchored endpoints; otherwise they remain
   external/unresolved.

## Target version-1 capability claim

Register only capabilities actually covered by positive, negative, ambiguity,
limit, and corpus evidence. The initial target set is:

- declarations and lexical scopes;
- namespaces/constant ownership;
- traits (`module`, `include`, `prepend`, `extend`);
- imports and aliases (`require_relative`, contained literal `require`,
  `autoload`, `alias`, and `alias_method` where exact);
- calls and construction;
- base types and conservative hierarchy dispatch;
- members, ownership, receivers, and qualified external references.

Do not advertise decorators, static type references, reexports, tests, macros,
or complete hierarchy dispatch merely because a fixture contains a related
syntax form. Literal `attr_reader`/`attr_writer`/`attr_accessor` and
`define_method(:literal)` can land as declarations during the candidate phase,
but `Macros` becomes an advertised capability only after its own audit stratum
passes.

## Pinned corpora and independent truth

Use at least three materially different Ruby corpora so one Rails convention
or target cluster cannot dominate the audit:

| Corpus | Repository | Commit to pin | Purpose |
| --- | --- | --- | --- |
| Rails | existing read-only `/Volumes/Workspace/Github/rails/rails` | `cc7d47f4419ba983fc9d06bffece57778fa671c5` | framework internals, concerns, reopenings, DSL-heavy code |
| Discourse | `https://github.com/discourse/discourse.git` | `699ad46536f619396e73720c7652dbfc7a1f86c0` | large Rails application, controllers/models/jobs/plugins |
| RuboCop | `https://github.com/rubocop/rubocop.git` | `c034d8b6804788856321d78c480f9f007bd85a8d` | non-Rails gem, nested modules, visitors, aliases, tests |

Clone missing corpora only below
`/Volumes/Workspace/Github/<owner>/<repository>`, treat them as read-only, and
record a relative-path/content inventory digest. If an existing checkout is at
a different revision, create a separate named worktree/check-out location on
the mounted volume; do not reset or clean it.

The independent source oracle must use a pinned Ruby standard-library parser
(`Ripper`) in qualification only. It must record exact `RUBY_VERSION` and
`RUBY_REVISION`, translate line/column positions to UTF-8 byte ranges, reject
partially parsed files for recall accounting, bound files/bytes/constructs/
depth/output, and emit a canonical inventory digest. If Ripper cannot provide
exact nonempty ranges for a required relationship family, stop and propose a
pinned Prism-based oracle; do not reuse Tree-sitter as its own oracle and do not
lower the recall gate.

Graphify-only facts, if sampled, belong only in the
`graphify_hypothesis` pool defined by `compass.quality-audit`. They are not
truth and must never become runtime, fixture, fallback, or CI dependencies.

## Commands executors will need

Every Cargo invocation must use a unique mounted target directory for the
implementation checkout.

For this checkout, prefix each Cargo command with the mounted/offline
qualification environment (the examples below abbreviate it in prose):

```bash
PROJECT_ROOT=/Volumes/Workspace/Github/compass-ruby-parser-root \
TSLP_OFFLINE=1 \
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal
```

Do not fall back to a local `target/` directory when the mounted volume is not
available.

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Target preflight | `test -d /Volumes/Workspace && mkdir -p /Volumes/Workspace/crabbuild-target/compass-ruby-universal && test -w /Volumes/Workspace/crabbuild-target/compass-ruby-universal` | exit 0 |
| Language conformance | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal cargo test -p compass-languages --test ruby_universal_conformance --locked` | all Ruby cases pass |
| Language crate | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal cargo test -p compass-languages --locked` | exit 0 |
| Resolver contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal cargo test -p compass-resolve --test universal_resolution ruby --locked` | all Ruby cases pass |
| Rails pack | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal cargo test -p compass-resolve --test php_ruby_jvm_routes rails --locked` | Rails route cases pass until replaced by a dedicated universal-pack test |
| Publication | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal cargo test -p compass-core --test code_graph_v1_publication_resilience ruby --locked` | valid v1 graph, zero Ruby omissions/collisions |
| Fixture gate | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal ./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0; all byte comparisons true |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0; no Graphify/runtime Ruby dependency |
| Focused lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal cargo clippy -p compass-languages -p compass-resolve -p compass-model -p compass-graph -p compass-core --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Native baseline | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal cargo clippy --workspace --lib --bins --locked -- -D warnings && CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal cargo test --workspace --lib --bins --locked` | exit 0 |
| Format | `cargo fmt --all -- --check && git diff --check` | exit 0, no diff errors |

Add one Ruby qualification entry point with checked-in `--help` and a stable
machine-readable summary; name the exact command in the implementation PR once
created. It must support fixture-only, pinned-corpus, quality-audit, and
performance modes without silently changing the production graph.

## Scope

**In scope**:

- dedicated Ruby universal evidence extraction;
- Ruby constant, scope, reopen, method-space, require, alias, mixin, hierarchy,
  receiver, construction, call, and literal metaprogramming semantics;
- a closed Ruby resolver policy only where shared evidence rules are
  insufficient;
- conversion of Rails routes to one universal `rails-ruby` framework pack;
- Ruby cache invalidation, publication, query/impact compatibility, fixtures,
  independent oracle, quality audit, real-repository qualification, docs, and
  performance measurement;
- removal of only Ruby's replaced generic-publisher branches, raw-call path,
  `resolve_ruby_members`, and established Rails detector registration during
  the atomic cutover.

**Out of scope**:

- executing Ruby, Bundler, Rake, Rails, application initializers, gems, or
  arbitrary project code during normal Compass extraction;
- scanning installed gems, `.bundle`, `vendor/bundle`, or network registries;
- inferring runtime load order from directory order;
- resolving `method_missing`, dynamic `send`, computed constants, nonliteral
  requires/autoloads, runtime eval, or monkey patches without exact evidence;
- treating Gemfile gem names as require paths or local declarations;
- broadening `compass.graph/1` endpoint validation to admit false Ruby edges;
- adding a Ruby runtime dependency to the released Compass binary;
- promoting Ruby to `UniversalComplete` before the complete audit gates pass.

## Git and delivery workflow

- Use one branch/PR per phase or per tightly coupled phase pair. Suggested
  branches: `agent/ruby-universal-phase-0`, then `phase-1`, and so on.
- Follow the repository's terse imperative commit style, for example
  `Add Ruby universal evidence emitter`.
- Keep corpus checkouts and generated graphs outside the repository and out of
  commits.
- Do not push or open a PR unless the operator explicitly requests it.
- At the end of each phase, record exact commands, graph hashes, corpus commits,
  inventory digests, relation-family counts, diagnostics, and timings in a
  phase-specific qualification report under `docs/implementation/`.

## Phase map

| Phase | Outcome | Production Ruby route changes? |
| --- | --- | --- |
| 0 | Frozen established baseline and independent oracle | No |
| 1 | Ruby identity and graph representation contract | No |
| 2 | Qualification-only bounded evidence emitter | No |
| 3 | Qualification-only project and resolver semantics | No |
| 4 | Universal Rails pack ready behind candidate evidence | No |
| 5 | Atomic hard cut to Ruby `UniversalCandidate` | Yes, once |
| 6 | Measured cold/warm/incremental optimization | Candidate remains active |
| 7 | Complete quality audit and release qualification | Candidate; promotion is separate |

## Phase 0: Freeze truth, established behavior, and performance

**Context**: Ruby's current graph is incomplete but it is the compatibility
baseline. Changes cannot be judged from total node/edge growth alone. This
phase creates reproducible evidence before any production Ruby behavior moves.

**Primary files to add or update**:

- `scripts/ruby_source_oracle.rb` (qualification-only independent oracle);
- `scripts/qualify_ruby_universal.py` or an equivalent bounded orchestrator;
- `scripts/tests/test_ruby_source_oracle.py`;
- `tests/qualification/ruby-universal-repositories.toml`;
- `tests/qualification/ruby-universal-baseline.json`;
- `docs/implementation/ruby-universal-qualification.md`;
- `benchmarks/performance/repositories.toml` only if the shared runner can
  express all Ruby workloads without weakening existing entries.

**Steps**:

1. Pin the three corpora above, verify clean revisions, inventory admitted Ruby
   files (`.rb`, `.rake`, shebang Ruby, and fixed Ruby filenames already owned
   by discovery), and record content digests.
2. Build the established Compass binary once from `b53c3ea2`; compilation time
   is excluded. Run forced cold, unchanged warm, one-file semantic edit,
   fact-neutral trailing-comment edit, rename, delete, and restore workloads.
3. Capture canonical graph hash, file/node/edge counts, relation-family counts,
   node-kind counts, Ruby diagnostics, omissions, collisions, parse recovery,
   and stage timings. Run each timed warm/incremental case at least five times;
   retain medians and dispersion. Measure RSS but keep it non-blocking.
4. Implement the independent Ripper oracle for declaration ownership,
   instance/singleton methods, inheritance, include/prepend/extend, literal
   require/autoload/aliases, construction, and statically named calls. Run it
   twice and require byte-identical canonical output.
5. Create a small curated baseline of established correct, missing, ambiguous,
   and incorrect graph facts. Do not translate missing established facts into
   candidate requirements unless the source oracle proves them.

**Acceptance criteria**:

- all corpus revisions and inventory digests are explicit and reproducible;
- clean, warm, and restored established graphs are byte-identical per corpus;
- every baseline record names a source file, exact UTF-8 range, relation family,
  graph fact or explicit absence, and judgment;
- the oracle fails closed on malformed/partial input and produces identical
  bytes on two runs;
- build time is excluded from Compass timings and corpus trees remain clean;
- no product source, adapter registration, or production Ruby graph changes in
  this phase.

**Verify**: the new oracle unit suite and baseline command both exit 0; rerun
`git -C <corpus> status --porcelain=v1 --untracked-files=all` and expect empty
output for every corpus.

## Phase 1: Freeze Ruby identity and representation contracts

**Context**: Reopened constants and separate method spaces make identity the
highest-risk design decision. Lock it before implementing broad traversal.

**Primary files to update**:

- `docs/reference/universal-semantic-evidence.md`;
- `docs/design/language-architecture.md` only to describe the planned Ruby
  contract, not shipped status;
- `crates/compass-languages/src/evidence/model.rs` and validation only if the
  existing optional fields cannot encode method/mixin context;
- focused evidence/projection contract tests.

**Steps**:

1. Specify canonical constant, class, trait/module, instance method, singleton
   method, top-level method, closure, parameter, instance variable, class
   variable, and constant identities with Unicode examples.
2. Prove that two reopenings of `A::B` coalesce to one graph node while
   retaining both definition anchors and deterministic member ownership.
3. Prove `A#call` and `A.call` remain distinct through evidence validation,
   resolution, v1 normalization, query indexing, and history serialization.
4. Map Ruby modules to graph trait nodes and validate legal `class -mixes_in->
   trait` and `trait -mixes_in-> trait` endpoints. Do not weaken endpoint rules.
5. Decide whether existing occurrence `context` can encode
   `include|prepend|extend` and method space. Add an optional typed field only
   if string context would be ambiguous or unauditable; if added, keep the
   universal evidence schema at v1 only when backward-compatible and update all
   validators/fingerprints/tests.

**Acceptance criteria**:

- an identity table with examples is checked into the semantic-evidence
  reference;
- identity is invariant to checkout path, file enumeration, extraction worker
  count, and batch order;
- reopened constants do not collide or duplicate graph nodes;
- duplicate method definitions remain multiple evidence declarations and do
  not silently select one target;
- instance/singleton methods cannot resolve across method spaces;
- no new public graph kind or relaxed endpoint pair is required. If that
  assumption fails, stop for a separate compatibility design review.

**Verify**: focused model/evidence/projection tests pass and serialize the same
canonical bytes in forward and reversed input order.

## Phase 2: Build the qualification-only Ruby evidence emitter

**Context**: The emitter must be exercised without registering Ruby in
`UNIVERSAL_ADAPTERS`. Production continues through the established generic
path during this phase.

**Primary files to add or update**:

- create `crates/compass-languages/src/evidence/ruby.rs`;
- update `crates/compass-languages/src/evidence/mod.rs`;
- extend the hidden qualification API in
  `crates/compass-languages/src/engine.rs` without selecting it in normal
  extraction;
- create `crates/compass-languages/tests/ruby_universal_conformance.rs`;
- create `fixtures/code-graph/qualification/rich.rb`.

**Steps**:

1. Consume the already prepared Tree-sitter root and borrowed bytes exactly
   once. Use bounded passes for declarations/scopes, literal bindings, and
   semantic occurrences; cap traversal depth and every fact family through
   `EvidenceBuilder`/`EvidenceLimits`.
2. Emit file, class, Ruby-module-as-trait, instance method, singleton method,
   constructor (`initialize` remains a method but `Class.new` is construction),
   closure/block/lambda, parameter, constant, instance-variable/property, and
   supported literal metaprogramming declarations with exact identifier
   anchors.
3. Preserve lexical scopes for nested `class`/`module`, `class A::B`, methods,
   singleton classes (`class << self`), blocks, lambdas, rescue clauses, and
   pattern bindings where they affect name lookup.
4. Emit inheritance, `include`, `prepend`, and `extend` occurrences with
   qualified constant syntax and operation context. Mark direct-base
   completeness truthfully; external or dynamic ancestors make it incomplete.
5. Emit literal `require_relative`, contained literal `require`, `autoload`,
   `alias`, `alias_method`, `attr_reader`, `attr_writer`, `attr_accessor`, and
   literal `define_method` facts. Dynamic arguments produce bounded diagnostics
   or unresolved occurrences, never declarations with invented names.
6. Emit calls/construction with source caller, receiver spelling, method space,
   argument count, literal argument types where provable, and allowed target
   kinds. Recognize constant receivers, `self`, `super`, local variables with a
   single source-proven construction assignment, and parameters only when
   source evidence gives a nominal receiver. Unknown receivers stay unresolved.
7. On parser recovery, retain only facts whose nodes/ranges are trustworthy,
   add `partial_parser_recovery`, and never emit zero-width non-file anchors.

**Acceptance criteria**:

- the hidden Ruby candidate API returns valid version-1 evidence, but ordinary
  `Engine::extract_source_graph_only` still returns the established Ruby graph;
- fixtures cover nested/reopened constants, instance and singleton methods,
  all parameter forms, Unicode, blocks/lambdas, superclass, every mixin form,
  aliases, literal requires/autoload, construction, `self`/`super`, and exact
  literal metaprogramming;
- negative fixtures cover dynamic require/send/define_method/eval, ambiguous
  receiver assignments, duplicate method definitions, invalid constants,
  parser recovery, nesting depth, and every evidence budget;
- every non-file fact has a nonempty exact UTF-8 range contained by its source;
- two runs and reversed traversal fixtures produce byte-identical evidence;
- no raw nodes, raw edges, or `RawCall` values are emitted by the candidate;
- production Ruby output and cache fingerprints remain unchanged.

**Verify**: `ruby_universal_conformance` passes, `validate_evidence` accepts all
positive batches, all negative cases fail or diagnose with the expected typed
code, and a production Ruby snapshot matches the Phase-0 established hash.

## Phase 3: Add bounded Ruby project and resolution semantics

**Context**: Project-wide target selection belongs in `compass-resolve`.
Generic terminal-name lookup is unsafe for Ruby because constants reopen,
method spaces differ, load order is dynamic, and mixins alter dispatch.

**Primary files to add or update**:

- create `crates/compass-resolve/src/evidence/languages/ruby.rs`;
- update `crates/compass-resolve/src/evidence/languages/mod.rs` and
  `policy.rs`;
- add Ruby-only indexes in `evidence/index/` only where shared indexes cannot
  represent method space, reopen groups, or mixin operation;
- extend `ProjectEvidence` only for bounded contained Ruby load roots if direct
  source evidence proves they are needed;
- create `crates/compass-resolve/tests/universal_resolution/ruby.rs`.

**Steps**:

1. Resolve constants by exact lexical nesting, explicit absolute `::`, and
   exact contained require/autoload bindings before considering any broader
   inventory. A terminal constant match is never sufficient when multiple
   qualified constants exist.
2. Resolve `require_relative` by normalized contained path with Ruby extension
   and index fallbacks. Resolve literal `require` only when the admitted project
   inventory yields one contained source target under an explicit bounded load
   root. Gemfile dependencies remain qualified external evidence.
3. Merge reopened class/module member inventories by exact graph identity.
   Preserve duplicate definitions and lookup completeness; one complete
   compatible candidate may resolve, truncation or duplicates may not.
4. Add method-space-aware lookup for constant receivers, constructed locals,
   `self`, bare calls, and `super`. Use source-proven superclass/mixin links and
   bounded cycle detection. For competing include/prepend/reopen order across
   files with no proven runtime load order, return ambiguous rather than
   simulating Ruby's runtime.
5. Treat literal method aliases as owner- and method-space-scoped bindings.
   Detect alias cycles and enforce the shared lookup budget.
6. Preserve exact external targets only when the source spelling is qualified.
   Reject cross-language declarations even when terminal names and project
   paths match.
7. Profile validation, Ruby index construction, candidate ordering, decisions,
   and projection separately. Build Ruby indexes only when Ruby evidence is
   present.

**Acceptance criteria**:

- positive tests cover lexical/absolute constants, contained require paths,
  reopen member union, aliases, singleton and instance dispatch, superclass,
  include/prepend/extend, construction, bare calls, and `super`;
- every positive family also has missing, duplicate, ambiguous, cyclic,
  truncated, wrong-method-space, wrong-language, and malicious-path cases;
- reversing files/batches produces identical nodes, edges, decisions, rules,
  candidate counts, and ordering;
- no terminal-only Ruby call, class, mixin, or require target becomes exact;
- an incomplete hierarchy or candidate bucket cannot appear unique;
- non-Ruby resolver tests and performance stay unchanged within measurement
  noise;
- the candidate is still qualification-only and production Ruby output still
  matches Phase 0.

**Verify**: the Ruby resolver module and complete `compass-resolve` test suite
pass; direct decision tests assert exact `ResolutionDecision`, rule, candidate
count, language, and occurrence anchor.

## Phase 4: Convert Rails routing to a universal framework pack

**Context**: A hard-cut universal language cannot re-enter an established
line/regex detector. Rails must consume Ruby evidence through the same static
framework-pack contract as Spring, ASP.NET, and PHP frameworks.

**Primary files to update**:

- `crates/compass-languages/src/frameworks/pack.rs`;
- `crates/compass-languages/src/frameworks/mod.rs`;
- rewrite `crates/compass-languages/src/frameworks/ruby.rs` to consume exact
  Ruby evidence and AST anchors;
- `crates/compass-resolve/src/frameworks/mod.rs` and `ruby.rs`;
- create `crates/compass-resolve/tests/rails_universal_pack.rs`;
- extend Rails fixtures and framework-route docs.

**Steps**:

1. Add one `rails-ruby` `FrameworkPackDescriptor` with explicit Ruby input
   capabilities, Rails activation evidence, route relationship claims, and
   pack limits. Add exactly one project-wide expansion adapter with the same
   ID.
2. Recognize `Rails.application.routes.draw` and route DSL calls from the AST
   and Ruby occurrences. Preserve nested `namespace`, `scope`, literal path,
   `to:`, hash-rocket, `via`, controller/action, and exact block anchors.
3. Add bounded source-proven composition for `draw`, route concerns, and
   mounted engines only where literal references and contained files make the
   target unique. Otherwise preserve unresolved/ambiguous route facts.
4. Resolve handlers to exact Ruby instance methods using the Phase-3 identity
   codec and exact controller constant. Never map a route to a singleton method
   or same-named controller in another namespace.
5. Preserve established positive routes and add wrong-framework, lookalike DSL,
   dynamic path/handler, ambiguous controller, missing action, nested namespace,
   malformed syntax, ordering, and fact-limit tests.

**Acceptance criteria**:

- framework descriptor and expansion registries match exactly;
- every published route/`routes_to` edge retains exact route and handler
  evidence, direction, operation/path, framework, and resolution state;
- current Rails symbol, hash-rocket, and namespace fixtures remain exact;
- dynamic or ambiguous handler/path/controller forms publish no invented exact
  edge;
- the universal pack runs only on validated Ruby candidate evidence and does
  not rescan source lines with regex as its semantic authority;
- the established `rails-routes` pack remains production-active until Phase 5,
  but qualification invokes only the universal pack—never both in one graph.

**Verify**: `rails_universal_pack` and generic framework registry tests pass;
fixture qualification produces the same route facts under clean, warm,
rebuild, incremental-restore, and relocated-checkout runs.

## Phase 5: Atomically hard-cut production Ruby

**Context**: This is the only phase that changes the production route. It must
land as one coherent commit/PR after Phases 0–4 pass against fixtures and all
three corpora.

**Primary files to update**:

- `crates/compass-languages/src/adapters.rs`;
- `crates/compass-languages/src/evidence/mod.rs` and `engine.rs`;
- `crates/compass-resolve/src/members.rs`;
- framework pack/expansion registries;
- cache/manifest tests in `compass-core` and `compass-files` as needed;
- graph qualification manifests/oracle, docs, compatibility, migration, and
  changelog.

**Steps**:

1. Add sorted `RUBY_CAPABILITIES` and adapter profile
   `id="compass.ruby.candidate"`, `language="ruby"`, `version=1`,
   `profile=UniversalCandidate`.
2. Route normal `.rb`, `.rake`, fixed filenames, and Ruby shebang extraction to
   the dedicated emitter. The same registered emitter must back the hidden
   qualification API.
3. Remove only Ruby's branches from the generic walker and raw-call collection;
   remove `resolve_ruby_members` and its invocation. Do not disturb other
   established languages that share `engine.rs` or `members.rs`.
4. Replace the established `rails-routes` registration with `rails-ruby` and
   enable its matching expansion adapter in the same commit.
5. Invalidate Ruby cache entries through adapter identity/version. Cached Ruby
   entries containing replaced raw nodes/edges/calls must fail compatibility
   and reextract; non-Ruby caches remain reusable.
6. Expand strict fixture vocabulary/producer assertions for Ruby declarations,
   trait modules, calls, construction, inheritance, mixins, imports, aliases,
   and Rails routes. Require zero Ruby publication omissions and identity
   collisions.
7. Update current-state docs to say hard-cut Ruby `UniversalCandidate`, and
   explicitly state that complete audit gates remain pending.

**Acceptance criteria**:

- one production Ruby file has exactly one semantic publisher and one Rails
  pack; no dual facts or translation fallback exist;
- `rg "resolve_ruby_members|add_ruby_parent_edge|rails-routes"` returns no
  active production implementation/registration matches (historical docs may
  remain clearly historical);
- Ruby extraction contains semantic evidence and no replaced raw calls or raw
  semantic relations;
- all three pinned corpora publish strict-valid `compass.graph/1` with zero
  Ruby omissions, zero Ruby identity collisions, and no unsupported
  class-to-module inheritance/mixin endpoints;
- clean, forced rebuild, warm, restored incremental, and relocated-checkout
  graphs are byte-identical per corpus;
- non-Ruby fixture hashes change only where shared qualification metadata
  intentionally includes the expanded vocabulary;
- unknown-major/version validation, cache rejection, and atomic publication
  tests pass;
- Ruby remains `UniversalCandidate` in code and documentation.

**Verify**: run every command in “Commands executors will need,” then run the
three pinned-corpus qualification mode. Every command exits 0 and each corpus
summary reports schema v1, Ruby adapter v1, zero validation errors, zero Ruby
publication omissions/collisions, and deterministic comparisons all true.

## Phase 6: Optimize cold, warm, and incremental Ruby builds

**Context**: Optimization begins only after semantic parity and hard-cut
correctness are frozen. One optimization per commit; compare canonical graph
bytes before and after each change.

**Steps**:

1. Profile parse, evidence passes, project inventory, Ruby index construction,
   candidate decisions, projection, persistence, and warm manifest checks on
   all corpora. Rank hotspots by wall time and allocation count; do not optimize
   from intuition.
2. Apply only evidence-neutral changes, such as sharing one AST classification
   pass, reserving from validated counts, interning repeated constant/method
   owner keys, storing compact declaration slots, omitting Ruby indexes from
   non-Ruby corpora, or memoizing bounded require/ancestor walks after measuring
   repeated work.
3. Verify fact-neutral edits reuse all unchanged Ruby extraction. Verify one
   semantic edit extracts only the changed file and incrementally updates only
   affected graph partitions/objects. Rename/delete/restore must remove stale
   declarations and reproduce the original graph bytes on restore.
4. Run at least one cold and five warm/incremental samples per corpus for each
   proposed optimization. Keep raw samples and medians in the qualification
   report. RSS is recorded but is not a blocking comparison metric.

**Acceptance criteria**:

- optimized and pre-optimization canonical nodes, edges, diagnostics,
  resolution decisions, and graph hashes are identical;
- median cold, warm, and semantic one-file incremental wall time do not regress
  by more than 3% on any corpus unless an explicit reviewed correctness
  tradeoff is recorded;
- an individual change is called an optimization only when its target phase
  improves median time by at least 10%; otherwise revert it or document it as a
  neutral refactor;
- unchanged warm runs report zero extracted files and reuse every eligible Ruby
  cache entry;
- fact-neutral one-file edits complete in the repository's incremental path and
  restored output is byte-identical;
- non-Ruby representative corpora regress by no more than 3%; Ruby-only indexes
  allocate no state when no Ruby evidence is present;
- all limits, ambiguity, ordering, and fixture gates remain green.

**Verify**: the Ruby performance mode emits a versioned JSON report containing
raw samples, medians, corpus/graph digests, adapter version, changed/reused file
counts, and stage timings; its regression evaluator exits 0.

## Phase 7: Complete the quality audit and release qualification

**Context**: A successful hard cut makes Ruby a candidate. It does not prove
complete quality. This phase applies the repository's independent audit gates
without weakening them for Ruby's dynamic semantics.

**Steps**:

1. Build `accepted`, `source_oracle`, and optional `graphify_hypothesis` pools
   using the pinned corpora and Ripper inventory. Verify every snippet hash,
   byte range, graph fact, provider identity, source inventory, corpus revision,
   and graph digest before scoring.
2. Stratify declarations/ownership, calls, construction, inheritance, mixins,
   imports/requires, aliases, member dispatch, and each advertised framework
   capability. Dynamic/unresolved facts count honestly toward recall where the
   source oracle proves a supported construct.
3. Include critical judgments for fabricated occurrences, unsafe local target
   substitution, cross-language matches, instance/singleton confusion, wrong
   reopen owner, wrong mixin method space, and path escape.
4. Add scheduled exact-commit qualification and a release gate only after the
   corpus process is reproducible on supported CI infrastructure. Generated
   graphs and private data remain outside the repository.
5. Promote to `UniversalComplete` only in a separate reviewed change after
   every threshold passes. Otherwise retain `UniversalCandidate` and publish
   the failing strata as actionable follow-up work.

**Acceptance criteria**:

- at least 2,000 audited accepted relationships total;
- at least 400 accepted records per corpus;
- at least 100 accepted records per required relationship family and per
  advertised capability identity;
- no target cluster exceeds 10% of a corpus/language/relation/capability
  stratum;
- observed precision is at least 99.5% overall and the two-sided 95% Wilson
  lower bound is at least 99%;
- every advertised capability has at least 99% observed precision and 95%
  source-oracle recall;
- zero fabricated occurrences, cross-language matches, unsafe local-target
  substitutions, method-space crossings, or repository path escapes;
- all three corpora remain strict-valid and deterministic under cold, warm,
  forced, incremental restore, worker-count, input-order, and relocated-path
  permutations;
- performance gates from Phase 6 pass;
- a failing gate leaves Ruby explicitly `UniversalCandidate` and fails the
  completion/release claim rather than changing thresholds.

**Verify**: the checked-in audit validator exits 0 only when all thresholds
above are met and emits a stable `compass.quality-audit` qualification summary.

## Cross-phase test plan

Use `php_universal_conformance.rs`, `kotlin_universal_conformance.rs`,
`universal_resolution/php.rs`, `universal_resolution/kotlin.rs`, and
`spring_universal_pack.rs` as structural patterns. Add at least these groups:

- **Identity**: nested and qualified constants, reopenings across files,
  duplicate definitions, instance/singleton methods, top-level methods,
  Unicode, checkout relocation.
- **Syntax**: class/module/singleton class, methods, every parameter shape,
  blocks/lambda/proc, constants and variables, aliases, literal attr/define
  method, inheritance, include/prepend/extend, require/autoload.
- **Resolution**: lexical/absolute constants, exact require paths, constructed
  locals, self, bare calls, super, aliases, reopen member union, mixin dispatch,
  external targets.
- **Negative/ambiguity**: terminal collisions, duplicate methods, dynamic
  receivers, dynamic send/eval/require/define_method, mixed languages, unknown
  load roots, path escapes, cycles, incomplete hierarchies, missing files.
- **Limits/malformed**: every evidence and framework fact budget, traversal
  depth, alias/ancestor/require cycles, parser recovery, invalid UTF-8 handling
  at the established source-decoding boundary.
- **Publication**: identity, kind, direction, multiplicity, exact occurrence,
  provenance, resolution rule, candidate count, stable ordering, zero Ruby
  omissions/collisions.
- **Incremental**: semantic edit, fact-neutral edit, rename, delete, restore,
  Gemfile/project-evidence change, Rails route edit, cache-version rejection.
- **Rails**: route DSL positives, nested scopes/namespaces, concerns/draw/mount
  where supported, wrong framework, dynamic arguments, ambiguous controllers,
  wrong method space, limits, deterministic expansion.

## Done criteria

- [x] Phase 0 establishes reproducible established graphs, timings, corpus
  inventories, and an independent bounded Ruby source oracle.
- [x] Ruby identity, reopen, module/trait, and method-space contracts are
  documented and tested before broad extraction.
- [x] The candidate emitter has independent fixture/oracle coverage and is
  the sole production Ruby publisher after the atomic cut; the profile remains
  a candidate until the complete corpus gates pass.
- [x] Ruby resolution never uses terminal-name similarity as unique evidence.
- [x] Rails is one universal pack consuming validated Ruby evidence.
- [x] The atomic cut removes only Ruby's replaced publisher/resolver/framework
  paths and leaves no production dual run.
- [x] All strict fixture and pinned-corpus graphs are deterministic and valid;
  the Ruby qualification captures have no Ruby identity collisions and the
  audit has zero critical violations.
- [x] Performance gates pass with reproducible reports; RSS is recorded but
  non-blocking.
- [x] Ruby remains `UniversalCandidate` until every complete audit threshold
  passes.
- [x] Docs, compatibility notes, migration guidance, changelog, and
  `advisor-plans/README.md` reflect the actual shipped state.

## STOP conditions

Stop and report rather than improvising if:

- live adapter/evidence/cache contracts differ from this plan's assumptions;
- correct Ruby module or method-space representation requires a new public
  graph kind or weakening endpoint validation;
- Ripper cannot provide exact complete source-oracle coverage for a required
  stratum;
- exact target selection would require executing Ruby/Bundler/Rails/project
  code, scanning installed gems, or trusting runtime load order;
- reopened definitions or mixin precedence cannot be represented without
  selecting by file/batch/filesystem order;
- a candidate bucket is truncated but appears uniquely resolvable;
- normal production extraction would require both established and universal
  Ruby publishers at once;
- the Rails pack cannot consume validated Ruby evidence without reintroducing
  regex/line scanning as semantic authority;
- a phase's verification fails twice after one scoped correction;
- implementation requires modifying files outside that phase's declared scope;
- corpus revisions, inventories, oracle identity, or graph hashes drift during
  a qualification run.

## Maintenance notes

- Ruby syntax is not Ruby runtime. Keep structural facts, project/load
  evidence, and framework conventions separately attributed.
- Reopening and monkey patching make “last definition wins” dependent on
  runtime load order. Unless load order is exact source evidence, ambiguity is
  the correct graph result.
- `include`, `prepend`, and `extend` share composition vocabulary but not method
  lookup behavior. Review every dispatch change for method-space leakage.
- Rails autoloading and inflection are configurable. Never assume path-to-
  constant equivalence from snake/camel conversion alone; add a bounded,
  versioned Zeitwerk evidence design if later qualification proves it necessary.
- Adapter-version increments are required whenever meaning-affecting Ruby
  evidence changes. The shared evidence schema version changes only for a true
  contract-major change.
- Reviewers should inspect ambiguity/negative tests before graph-count gains.
  More edges are not evidence of better Ruby support.

## Findings considered and rejected

- **Keep Ruby on the generic walker and add more special cases**: rejected
  because identity, ambiguity, limits, exact anchors, framework integration,
  and cache ownership would remain split across `engine.rs` and
  `members.rs`.
- **Resolve all same-named methods on a unique class label**: rejected because
  namespaces, reopenings, instance/singleton spaces, and mixins make terminal
  uniqueness unsafe.
- **Publish Ruby modules as ordinary graph modules**: rejected because graph
  modules are not type/mixin endpoints; this caused invalid inheritance/mixin
  publication. Graph traits already provide the truthful composable type and
  container shape.
- **Execute Bundler/Rails or load application code for accuracy**: rejected by
  Compass's native, local-first, bounded product boundary.
- **Use Graphify as the qualification oracle**: rejected because it is a
  hypothesis source, not independent truth, and cannot become a Compass
  runtime/test/fallback dependency.
- **Promote immediately to `UniversalComplete` after hard cut**: rejected
  because candidate architecture and audited quality are separate gates.
