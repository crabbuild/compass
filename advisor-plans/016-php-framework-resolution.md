# Plan 016: Complete Composer, Blade, and Eloquent framework resolution

> **Executor instructions**: Deliver this plan as four language/framework
> phases. Read `docs/design/language-architecture.md` and
> `docs/reference/universal-semantic-evidence.md` before changing evidence or
> resolution. Never select the first matching PSR-4 root, Blade file, or model
> class when multiple candidates remain valid.
>
> **Drift check (run first)**:
> `git diff --stat 6680842c..HEAD -- crates/compass-languages crates/compass-resolve crates/compass-graph crates/compass-model fixtures/code-graph/routes docs/reference/framework-routes.md`
> Stop and reconcile if Composer autoload facts or Eloquent relationship edges
> already exist under a different contract.

## Status

- **Priority**: P1
- **Effort**: L (four phases)
- **Risk**: HIGH
- **Depends on**: none
- **Category**: direction / framework support
- **Planned at**: commit `6680842c`, 2026-08-10

## Why this matters

Laravel routes already resolve to controllers and Blade/Eloquent extractors
exist, but the graph stops short of repository-bounded Composer namespace
resolution, exact Blade target binding, and model relationship topology. That
leaves impact analysis incomplete precisely across controller, view, and model
boundaries. These are adjacent extensions of existing project-evidence,
framework-pack, and resolver ownership—not a new PHP parser.

## Current state and constraints

- `crates/compass-languages/src/project_evidence.rs:474-500` reads Composer
  dependencies but does not retain `autoload.psr-4` or
  `autoload-dev.psr-4` namespace roots.
- `crates/compass-languages/src/templates.rs:239-320` recognizes Blade
  includes, Livewire tags, and `wire:click`, but references are not bound to
  unique repository files/callables.
- `crates/compass-languages/src/frameworks/enterprise.rs:474-520` maps an
  Eloquent model class to a table but does not publish `hasOne`, `hasMany`,
  `belongsTo`, `belongsToMany`, or polymorphic model relations.
- `crates/compass-languages/src/frameworks/php.rs` and
  `crates/compass-resolve/tests/php_ruby_jvm_routes.rs:63+` are the closest
  exact route-to-controller evidence and negative-test patterns.
- Framework packs activate only from project evidence, retain anchors, and use
  a closed relationship vocabulary. Extend that vocabulary deliberately.
- Paths must stay inside the repository root. Composer prefixes and directories
  may overlap or be arrays; ambiguity is an explicit result.
- Dynamic PHP, container bindings, view composers, runtime namespaces, and
  macro-generated Eloquent relations are not exact structural evidence.

## Target contracts

Introduce three typed evidence families:

1. `ComposerAutoloadRoot { namespace_prefix, directory, development,
   manifest_anchor }`, validated and normalized at the project-evidence layer.
2. `TemplateReference { kind, logical_name, source_anchor, candidates }`, with
   resolver state `exact|ambiguous|unresolved|limit`.
3. `ModelRelationshipFact { owner, method, relation_kind, target_reference,
   source_anchor, arguments }`, emitted only from source-backed Eloquent method
   calls.

Published graph edges use stable canonical edge kinds already supported by
`compass.graph/1` where possible, with framework-specific relation details and
provenance. If a new public edge kind is necessary, update validation, query,
renderers, history fingerprints, docs, and compatibility in the same phase.

## Commands executors will need

| Purpose | Command | Expected result |
| --- | --- | --- |
| Target preflight | `test -d /Volumes/Workspace && mkdir -p /Volumes/Workspace/crabbuild-target/compass-main && test -w /Volumes/Workspace/crabbuild-target/compass-main` | exit 0 |
| Language tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-languages --locked` | pass |
| Resolver tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-resolve --test php_ruby_jvm_routes --locked` | pass |
| Graph tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo test -p compass-graph --locked` | pass |
| Qualification | `./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-main cargo clippy -p compass-languages -p compass-resolve -p compass-graph --all-targets --locked -- -D warnings` | exit 0 |
| Format/boundary | `cargo fmt --all -- --check && sh scripts/check_product_boundary.sh` | exit 0 |

## Scope

**In scope**:

- Composer manifest evidence and PHP namespace/import target resolution;
- Blade include/extends/component/Livewire/action references already recognized
  by the extractor;
- evidence-gated Eloquent relationship methods and graph projection;
- Laravel fixtures, negative/ambiguity tests, public framework docs, cache
  versions, and qualification.

**Out of scope**:

- executing Composer autoloaders, Artisan, PHP, Blade compilation, or service
  providers;
- Packagist/network lookup or vendor-directory scanning by default;
- IoC container runtime bindings, macros, magic methods, model events, or query
  semantics;
- choosing a target by terminal-name similarity;
- treating Laravel conventions alone as exact evidence.

## Phase 1: Capture bounded Composer PSR-4 evidence

**Context**: Project evidence owns manifest interpretation. The language
extractor should not rediscover repository roots independently for each file.

**Deliverables**:

1. Parse only object entries under `autoload.psr-4` and
   `autoload-dev.psr-4`; support one string or a bounded string array per
   prefix. Preserve manifest location and whether the root is dev-only.
2. Normalize namespace prefixes to a canonical trailing `\\`, reject empty or
   malformed prefixes, and resolve directories relative to the Composer
   manifest directory using existing path-containment primitives.
3. Store roots deterministically in `ProjectEvidence` and include them in the
   meaning-affecting project-evidence/cache fingerprint.
4. Bound manifests, entries, prefix bytes, path bytes, roots per prefix, and
   total roots. A limit emits an explicit diagnostic.
5. Add tests for nested Composer roots, arrays, overlapping prefixes, duplicate
   roots, `autoload-dev`, `../` escape, absolute paths, symlinks, non-UTF-8
   platform paths where supported, malformed JSON, and deterministic order.

**Acceptance criteria**:

- only contained directories become candidates;
- duplicates canonicalize without losing distinct manifest provenance;
- overlapping prefixes remain distinct and longest-prefix matching is
  testable, not resolved during parsing;
- invalid/limited manifests emit typed diagnostics and never silently become
  “no autoload rules”;
- language tests and Clippy pass.

## Phase 2: Resolve PHP namespaces and Laravel handlers through PSR-4

**Context**: `compass-resolve` owns project-wide target selection. The resolver
must combine PHP namespace/import evidence with Composer roots and declarations.

**Deliverables**:

1. Add a PSR-4 index keyed by canonical namespace prefix and repository path.
   Apply the longest matching prefix; for every configured root, derive the
   candidate relative path and validate exact file/declaration identity.
2. Resolve PHP imports, fully qualified names, Laravel controller/action
   handlers, and model class references through this index before any
   conservative existing fallback.
3. If zero, two, or more valid declarations remain, publish unresolved or
   ambiguous candidates with anchors. Never use Composer array order as a
   semantic tiebreaker.
4. Keep dev roots visible and allow tests to use them, but do not let a dev
   declaration silently override a production declaration.
5. Add collection tests for aliases, group imports, nested manifests, longest
   prefix, multiple roots, class/file case behavior by platform, duplicates,
   missing files, and same-named controllers.

**Acceptance criteria**:

- a unique, contained declaration resolves exactly with import/manifest
  provenance and source anchor;
- overlapping roots produce the same deterministic candidate set independent
  of filesystem iteration;
- ambiguity and limit cases retain every bounded candidate and no exact edge;
- route direction and multiplicity remain unchanged;
- resolver tests and fixture qualification pass.

## Phase 3: Bind Blade references without executing templates

**Context**: Blade extraction already recognizes useful reference syntax. This
phase turns logical references into exact graph targets only when repository
evidence makes them unique.

**Deliverables**:

1. Resolve literal `@include`, `@extends`, components, and Livewire component
   names against bounded conventional roots such as `resources/views` only
   when Laravel project evidence is active.
2. Convert dotted logical names to contained paths and support the exact Blade
   suffix. Namespaced views remain unresolved until a static, source-backed
   namespace registration contract is separately added.
3. Bind `wire:click="method"` only to a unique callable on the source-backed
   component class already resolved by exact evidence. Expressions, arguments,
   magic methods, and dynamic names remain references/diagnostics.
4. Publish directional edges with logical name, declaration/reference anchors,
   candidate state, and framework origin. Preserve duplicate occurrences when
   multiplicity is part of the graph contract.
5. Add fixtures for include/extends/component/Livewire positives, missing
   views, duplicate roots, dynamic expressions, path escape strings, case
   collisions, and near-match files.

**Acceptance criteria**:

- exact literal references bind to exactly one contained target;
- dynamic and ambiguous references never create exact edges;
- every published relation retains the Blade occurrence anchor;
- no template is rendered and no PHP/Artisan process is invoked;
- targeted tests and qualification pass.

## Phase 4: Publish evidence-gated Eloquent relationships

**Context**: Eloquent relationship methods are executable PHP, but common
literal `$this->relation(Related::class, ...)` forms contain direct structural
evidence. The method call proves a declared relationship, not database runtime
behavior or referential integrity.

**Deliverables**:

1. Detect returns/calls to exact Eloquent receiver methods `hasOne`, `hasMany`,
   `belongsTo`, `belongsToMany`, `morphOne`, `morphMany`, `morphTo`, and
   `morphToMany` only in activated Eloquent model classes.
2. Record owner model, enclosing method, relation kind, target class reference,
   literal key/table arguments when safe, and exact call anchor. For `morphTo`
   with no target class, publish the relationship declaration and explicit
   unresolved polymorphic target—never fabricate a model.
3. Resolve target class through PHP imports + PSR-4. Publish exact,
   directional model relationship edges only for unique targets; preserve
   ambiguity and source anchors otherwise.
4. Extend framework-pack capability/vocabulary validation and all graph/query/
   export projections required by the selected edge representation.
5. Add fixtures for every method, aliases, inverse directions, two same-named
   models, dynamic class expressions, non-Eloquent lookalikes, helper methods,
   malformed calls, and relationship multiplicity.

**Acceptance criteria**:

- every supported exact shape publishes the documented owner-to-target
  direction and relation kind;
- polymorphic/dynamic/ambiguous targets remain explicit and do not become
  convenient exact edges;
- relationship occurrence, class declarations, Composer root, provenance, and
  confidence survive graph normalization and history round-trip;
- query/impact can traverse the new relationship in the documented direction;
- docs and qualification cover positive, negative, ambiguity, and limit cases;
- all targeted checks pass.

## Done criteria

- [ ] All four phases meet their acceptance criteria.
- [ ] Composer roots are repository-contained, bounded, deterministic evidence.
- [ ] Blade and model references publish exact edges only for unique targets.
- [ ] Eloquent edges preserve kind, direction, anchors, provenance, and ambiguity.
- [ ] Cache/graph contracts and public docs are updated intentionally.
- [ ] Applicable baseline and code-graph qualification pass.
- [ ] `advisor-plans/README.md` marks this plan DONE.

## STOP conditions

Stop if exact support requires executing Composer/PHP/Artisan, scanning an
unbounded `vendor/` tree, weakening path containment, or treating declaration
order as identity. Stop if the proposed Eloquent edge kind cannot round-trip
through model validation, history, query, and export without a reviewed public
contract update.

## Maintenance notes

Composer semantics and filesystem case behavior are easy to conflate. Keep
namespace matching, path containment, declaration identity, and platform path
rules separate and independently tested. When adding another Laravel
convention, require an activation rule, evidence shape, ambiguity policy,
limits, and negative fixtures in the same change.
