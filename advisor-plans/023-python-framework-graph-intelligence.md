# Plan 023: Make Python framework graphs source-proven and production-qualified

> **Executor instructions**: Deliver this program as a sequence of small,
> reviewable PRs. Read this plan completely, then read `AGENTS.md`,
> `COMPATIBILITY.md`, `docs/design/language-architecture.md`,
> `docs/implementation/extending-compass.md`,
> `docs/implementation/universal-evidence.md`,
> `docs/implementation/evidence-resolution-framework-technical-design.md`,
> `docs/design/managed-language-analyzers.md`, and
> `docs/reference/universal-semantic-evidence.md` before changing source. Run
> every phase gate and confirm its expected result before starting the next
> phase. Do not execute repository Python code, import a target repository,
> install its dependencies, or make a network request during extraction or
> qualification.
>
> **Drift check (run before every phase)**:
>
> ```bash
> git diff --stat dd3bf47b..HEAD -- \
>   COMPATIBILITY.md CHANGELOG.md MIGRATION.md PERFORMANCE.md \
>   crates/compass-model crates/compass-files crates/compass-languages \
>   crates/compass-resolve crates/compass-graph crates/compass-core \
>   crates/compass-query crates/compass-output crates/compass-cli crates/compass-mcp \
>   packages/compass-viewer crates/compass-output/assets/viewer \
>   fixtures tests/qualification benchmarks/performance scripts docs advisor-plans
> ```
>
> If an in-scope file changed, compare the live implementation with “Current
> state” below. Mechanically update paths when ownership is unchanged. STOP if
> a producer version, evidence schema, project-evidence schema, framework-pack
> contract, route-stage contract, or qualification threshold changed.

## Status

- **Status**: IMPLEMENTED; PRODUCTION QUALIFICATION BLOCKED (all scoped implementation slices are integrated; the existing Phase 9 independent-audit gate remains intentionally failed)
- **Priority**: P1
- **Effort**: XXL; ten phases delivered as separate PRs
- **Risk**: HIGH
- **Depends on**: no implementation prerequisite; the final release claim
  should consume Plan 005 or an equivalent exact-commit production gate
- **Category**: correctness, language architecture, framework architecture,
  tests, performance, documentation
- **Planned at**: commit `dd3bf47b`, 2026-08-25

### Execution record (2026-08-25)

Phases 0–8 were implemented as isolated, reviewable commits in the executor
worktree:

| Phase | Commit | Result |
|---|---|---|
| 0 | `dc99b6c4` | baseline and qualification skeleton |
| 1 | `96bae8bc` | Python source roots and `.pyi` identity |
| 2 | `57e1aa2c` | conservative typed evidence and call results |
| 3 | `24299bb1` | generic route stages and strict consumers |
| 4 | `60fefb52` | atomic universal replacement of `python-web` |
| 5 | `f5889e35` | FastAPI, Starlette, and Pydantic graph intelligence |
| 6 | `d7741e62` | Django and Django REST Framework graph intelligence |
| 7 | `d9b2d760` | Flask depth, SQLAlchemy, and Celery universal facts |
| 8 | `d766b49b` | optional managed `scip-python` enrichment |

The initial Phase 9 slice added the bounded qualification checkout-root
override in `62a09878`. Ten clean, detached pinned corpora were provisioned beneath
`/Volumes/Workspace/Github/qualification-clean`; the pre-existing FastAPI
checkout, which contains untracked `graphify-out` artifacts, was not changed.
The deterministic pinned source-inventory report is recorded outside the
repository at
`/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks/phase9/python-framework-pinned.json`
with digest
`6e23f51799a466680f0cf0809e307051190141a8a4f56fc5d975d70e3fed2150`.

The initial production qualification gate was intentionally stopped at source
inventory because no independently adjudicated `compass.quality-audit/2`
ledger existed; the Django source scope also had one explicit syntax-error
file requiring review. The continuation below adds the bounded ledger builder,
but the resulting independent thresholds still fail, so
`productionQualified: false` remains authoritative. Performance, promotion
documentation, and lifecycle promotion remain deferred until the measured
recall gaps and malformed-file disposition are resolved.

### Phase 9 continuation (2026-08-25)

The missing audit seam and one storage blocker were resolved in two additional
isolated commits:

| Commit | Result |
|---|---|
| `f467f97e` | Snapshot term indexing now drops terms that normalize to empty after Unicode decomposition. A focused Hindi/Devanagari combining-mark regression passes. This fixes the default-store `terms/""` failure without changing ordinary search terms. |
| `2de2b81c` | Adds a bounded, stdlib-AST source-oracle provider and `compass.quality-audit/2` ledger builder. It records exact accepted, missing, and ambiguous source facts, rejected files, anchors, target identities, and provenance; it never labels a graph edge correct solely because Compass emitted it. |

The clean pinned FastAPI corpus was indexed twice with the default snapshot
store after `f467f97e`: both runs produced 2,842 files, 174,216 nodes, and
195,477 edges, with byte-identical graph SHA-256
`fdc7e6fd07615b1012932a413fa0a0c1b7450fa561436f9cc12f054cae7f57c2`.
The builder and audit tests passed 18/18; the five-corpus ledger contained
283,366 records and retained Django's explicit rejected syntax-error file.
The strict evaluator intentionally failed the release claim:
`passed=false`, `eligibleForQualityClaim=false`, precision `1.0` (Wilson lower
bound `0.999971`), recall `0.841841`, calls recall `0.765822`, imports recall
`0.770550`, and 29,443 ambiguous records. Starlette route recall was
`0.514851`; Celery, DRF, Flask, Pydantic, and SQLAlchemy lacked the required
accepted evidence; Django parsed 2,912/2,913 files. No threshold was weakened,
no generated ledger was committed, and `productionQualified` remains false.

The full `compass-graph` test suite also reproduces two deterministic,
pre-existing `graph_v1_normalization` assertion failures unrelated to the
snapshot change. The ten-corpus performance/lifecycle run and promotion docs
remain deferred until the recall floors, missing framework corpora, and Django
syntax-error disposition are resolved.

The final native qualification slice added exact Django signal connections and
scoped the independent re-export oracle to modules present in the pinned
source scope (`28727ac5`, `f5bc3fc7`; integrated here as `d02a45a4` and
`905a579f`). The release binary SHA-256 was
`d8a52e3372633cf3bcdc2566c2cc8cf9f7a74010feba01e563e3748d31569bbc`.
Fresh default-SQLite graphs were produced without changing either checkout:

| Corpus | Files / nodes / edges | Graph SHA-256 |
|---|---:|---|
| Django | 3,097 / 73,958 / 190,227 (2 nodes and 5 edges omitted as partial) | `8624c3a7b33f340dae1aee75b4d18a6eae40654c7aca157c2a5995ef7134a758` |
| NetBox | 1,301 / 23,531 / 48,928 (1 edge omitted as partial) | `078373e18de671263881a740138e74f9d2eeba9ec0ab2572eea8b8097195be59` |

The final two-corpus ledger is external at
`/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks/phase9/python-quality-audit-django-netbox-signals-final.json`
(108,717,589 bytes, raw SHA-256
`d12ddb6f23bbab56bacbb58b9c692f190b312fe7609183043ac0a943a22d5ae1`). Its
canonical evaluator digest is
`b8ff74ea3f0f176c72b3ffc27bef89fe5cd0edae98dab6b58353956f0679ee13`.
Precision is `1.0` (67,568/67,568; Wilson lower bound `0.99994315`), recall
is `0.97900307` (51,988/53,103), and both corpora remain below the release
claim because required capability/relation floors are absent or below 0.95.
The most important failures are aliases `0.928669`, imports `0.942275`,
Django routes `0.940051`, FastAPI DI/routes `0/100`, Flask routes `0/100`,
Pydantic modeling `0/100`, SQLAlchemy modeling/persistence `0/100`, DRF
routes `0/100`, Celery scheduling/messaging `0/100`, Starlette routes
`0/100`, and all required `consumes/maps_to/produces/schedules/subscribes/
triggers` relation minima `0/100`. Django and Python framework source
coverage remains `2912/2913` because the pinned syntax-error file is retained
as an explicit rejected region. `productionQualified: false` is therefore
authoritative; no threshold, denominator, or status was weakened.

The implementation commits are cherry-picked into this worktree through
`905a579f`; the parent worktree contains only the reviewed plan and README
status edits beyond those commits. Large graphs and ledgers remain outside the
repository as required.

The final bounded native correction in this worktree is `717a4522`. It promotes
only exact Django signal bindings to source-anchored Event endpoints before
publishing `subscribes`; external signal symbols receive a deterministic,
qualified Event placeholder only when the imported signal and local subscriber
are exact. Same-terminal lookalikes, rebound signals, dynamic receivers, and
ambiguous targets remain unresolved. Resolver/language gates passed 10/10,
12/12, 6/6, and 25/25, with resolver library Clippy and formatting clean.

Fresh default-store validation used release binary SHA-256
`9ddffb5976221ab426014163a48bbf9469aba61337e91fd3ea4106d9dc798aa9`. Django
produced 146,552 nodes and 287,720 links
(graph SHA-256
`35091783bd5d87d3de72c562fb9cb9e1aaab0ee14be659aa1cebfcd1d905f868`), and
NetBox produced 75,982 nodes and 129,063 links (graph SHA-256
`77ddb8836310a27bbee32f033cd797c5766706682018bf3a63beabd35505b81`). All 3
Django and all 29 NetBox source-proven `subscribes` edges survived publication;
no signal-specific node or edge was omitted.

The post-fix two-corpus audit is external at
`/Volumes/Workspace/crabbuild-target/compass-136d-python-signal/phase9/python-quality-audit-django-netbox-signals-v6.json`
(120,399,225 bytes, raw SHA-256
`25581f784171fe3d3284969374dc119d126ba6cf1b6c78920643e59002c68ba3`). Its
canonical evaluator manifest is `c9c6ece78bbdd18dbc223d5de91e481568c3d30338fc4c48edf9ed5d8eeb4809`:
precision `1.0` (77,886/77,886; Wilson lower `0.99995068`), recall
`0.99343856` (70,555/71,021), and F1 `0.99670848`. The signal relation now
has 15/15 source recovery; the fresh corpus graphs preserve 3 Django and 29
NetBox `subscribes` edges. The release claim
remains intentionally blocked by the independent audit's aliases `0.931413`,
Django route `0.940051`, fixed per-pack/relation minima, target-cluster
concentration, and Django source coverage `2912/2913` due the explicit syntax
error. No threshold or denominator was weakened.

## Why this matters

Compass already runs Python through the hard-cut `compass.python` universal
evidence pipeline. It preserves imports, re-exports, aliases, decorators,
construction, ownership, C3 dispatch, and conservative `self`, `cls`, and
`super()` behavior. The remaining weakness is not parser availability; it is
the semantic seam between Python project identity, typed bindings, framework
meaning, and independent qualification.

Today a common `src/` layout can give `src.acme.api` to code imported as
`acme.api`; `.pyi` files are recognized inconsistently; Python calls omit
arity and typed call-result evidence; and Django, FastAPI, and Flask still pass
through one legacy `python-web` scanner that rebuilds aliases and receivers
from source. Celery and ORM facts remain substring/regex-based. The checked-in
framework gate proves a few synthetic flows, not production precision and
recall on representative applications.

This program makes Python framework intelligence consume the same exact,
bounded language evidence as the rest of the hard-cut graph. Its release
endpoint is independently qualified Django/DRF, FastAPI/Starlette/Pydantic,
Flask, SQLAlchemy, and Celery behavior. Dynamic imports, monkey patching,
runtime decorator replacement, computed routes, and untyped metaclass behavior
remain explicitly unresolved.

## Current state

- `crates/compass-languages/src/evidence_pipeline.rs:158-173,498-506`
  registers Python producer `compass.python`, version 11, as `Qualifying` with
  declarations, lexical scopes, imports, re-exports, aliases, calls,
  construction, decorators, type references, base types, hierarchy dispatch,
  members, ownership, and external references.
- `crates/compass-languages/src/evidence/build.rs:1188-1202` collects the
  current Python declarations, imports, aliases, module variables, value
  references, and calls inside the shared direct-evidence state.
- `crates/compass-languages/src/evidence/build.rs:2315-2351` emits signature
  annotations as references but does not use them to type local bindings or
  call results.
- `crates/compass-languages/src/evidence/build.rs:6704-6719,6797-6798` records
  call argument count and chained result bindings only for Go, leaving Python
  call arity and argument types empty.
- `crates/compass-languages/src/evidence/build.rs:6882-6966` infers a Python
  local receiver from `x = Foo()` and falls back to `module.Foo` without first
  proving that `Foo` is not a parameter or local factory.
- `crates/compass-languages/src/evidence/build.rs:9309-9333` derives Python
  module identity directly from the source path and strips only `.py`.
- `crates/compass-languages/src/registry.rs:257-262` recognizes `.py` but not
  `.pyi`, while `crates/compass-resolve/src/lib.rs:5631-5634` already treats
  both suffixes as Python.
- `crates/compass-languages/src/project_evidence.rs:1323-1328,1467-1495`
  reads Python dependency names from `pyproject.toml` but publishes no Python
  source-root or package-layout evidence.
- `crates/compass-resolve/src/evidence/project.rs:5-23` has TypeScript and Go
  project context but no Python module index; its Python fallback repeats the
  repository-path-to-module conversion.
- `crates/compass-languages/src/frameworks/mod.rs:372-387` registers one
  established `python-web` source pack. No Python descriptor appears in
  `crates/compass-languages/src/frameworks/pack.rs:775-785`.
- `crates/compass-languages/src/frameworks/python.rs:19-218` handles literal
  Django URL calls and direct Flask/FastAPI route decorators.
  `crates/compass-languages/src/frameworks/python.rs:220-336` handles only
  simple `include_router` and `register_blueprint` mounts.
- `crates/compass-languages/src/frameworks/python.rs:426-469` stores receiver
  declarations in one file-wide map keyed by spelling, without lexical binding
  identity or source order.
- `crates/compass-languages/src/frameworks/python.rs:471-507` publishes a
  default Flask route as `ANY` and recognizes FastAPI dependencies only from a
  decorator-level `dependencies=[Depends(name)]` expression.
- `crates/compass-resolve/src/frameworks/python.rs:68-160` performs one-pass
  FastAPI/Flask mount expansion and does not compose an already mounted route
  through another parent.
- `crates/compass-languages/src/frameworks/enterprise.rs:26-46,114-181`
  activates Celery, Django ORM, and SQLAlchemy with source substrings and emits
  line-regex task or table facts. These facts are labeled AST-origin even
  though the source proof is lexical.
- `crates/compass-languages/src/frameworks/model.rs:70-98` has route stages but
  no dependency or security stage.
- `crates/compass-resolve/src/frameworks/domain.rs:71-90,233-275` converts all
  role facts into a React-specific `ui_role` path, so the generic framework
  substrate cannot yet attach existing `model`, `service`, `repository`,
  `consumer`, `producer`, `test`, or `fixture` node roles.
- `crates/compass-resolve/src/frameworks/qualification.rs:55-173` already owns
  a strict, versioned framework-evidence expectation schema. Extend this
  contract instead of inventing a separate Python-only assertion format.
- `tests/qualification/code-graph-v1-repositories.toml:3-62` contains three
  checked-in flows each for Django, Flask, and FastAPI.
  `tests/qualification/code-graph-v1-semantic.json:5-84,761-779` contains one
  semantic positive and one near-match negative per Python web framework.
- `benchmarks/performance/compass/occurrences.py:80-101,122-265` is an
  independent Python `ast` source oracle, but it currently inventories only
  calls and imports.
- `docs/reference/universal-semantic-evidence.md:694-712` requires at least
  2,000 audited accepted relationships, per-corpus/relation/capability floors,
  99.5% observed precision, a 99% Wilson lower bound, 95% source recall, and
  zero critical failures; it explicitly says the checked-in conformance audit
  does not qualify Python.
- `docs/design/managed-language-analyzers.md:498-529` already selects verified
  `scip-python` as the preferred optional analyzer and keeps dynamic behavior
  partial. Do not invent a second analyzer architecture.

## Goals

1. Resolve Python modules through source-only project evidence, including
   `src/` layouts, namespace packages, monorepos, and `.pyi` modules.
2. Add conservative typed Python bindings, call arity, callable returns, and
   bounded call-result propagation without treating `Any` as proof.
3. Replace `python-web` and the Python portion of `enterprise-domain-facts`
   with independently versioned universal framework packs.
4. Model route composition, dependency/security flow, schemas, ORM mappings,
   and job topology using existing Code Graph v1 nodes and relationships.
5. Qualify each advertised Python/framework capability against independent,
   pinned, read-only corpora and representative applications.
6. Preserve Compass's native, credential-free, Python-runtime-free default
   structural build.

## Non-goals

- Executing Python, Django settings, migrations, import hooks, application
  factories, dependency providers, pytest plugins, or framework CLIs.
- Installing target-repository packages or resolving a virtual environment
  implicitly.
- Guessing dynamic routes, reflection, monkey patches, metaclass-generated
  members, computed attributes, or values typed `Any`/`Unknown`.
- Adding a Python runtime, Graphify, a vector store, model credentials, or
  network access to normal extraction, resolution, tests, or fallback paths.
- Adding new public node kinds or edge kinds. Existing `route`, `job`,
  `schema`, `migration`, `class`, `field`, `property`, `parameter`, and
  `database_*` kinds plus existing `routes_to`, `depends_on`, `maps_to`,
  `produces`, `consumes`, `schedules`, `triggers`, `registers`, `type_of`,
  `returns`, and `references` relationships are sufficient for this plan.
- Qualifying pandas, NumPy, PyTorch, notebook runtime semantics, Cython/native
  extension bodies, Airflow, Streamlit, aiohttp, Sanic, or Litestar.
- Adding pytest framework semantics in this program. Pytest needs a separate
  testing-capability plan because its fixture/test topology widens the
  framework descriptor contract beyond the web/data/job scope here.
- Rewriting immutable historical realizations. Older graphs remain immutable;
  users rebuild a new realization after producer/pack changes.

## Required technical design

### 1. Python project and module identity

Add a bounded source-only Python project model to `ProjectEvidence` and the
universal resolver.

Target public Rust shape in `compass-languages`:

```rust
pub struct PythonImportRoot {
    pub manifest: String,
    pub directory: String,
    pub package_prefix: Option<String>,
    pub kind: PythonImportRootKind,
}

pub enum PythonImportRootKind {
    ProjectRoot,
    SrcLayout,
    SetuptoolsPackageDir,
    SetuptoolsFind,
    PoetryPackage,
    HatchWheelPackage,
}
```

The exact field visibility may follow existing `ComposerAutoloadRoot`, but the
serialized project-evidence contract must retain the manifest, normalized
contained directory, optional package prefix, and rule kind. Bump
`compass.framework-project-evidence/3` to `/4` and add a migration/cache test.

Rules:

1. Parse only static TOML values. Support PEP 621 metadata plus explicit
   setuptools `package-dir`/`packages.find.where`, Poetry `packages`, and Hatch
   wheel `packages`. A conventional project root is always a candidate. A
   contained `src/` directory may be a candidate when explicitly declared or
   when it is the sole bounded directory containing Python packages.
2. Normalize roots relative to their nearest `pyproject.toml`. Reject absolute
   paths, `..`, symlink escapes, NUL, oversized values, excessive roots, and
   duplicate conflicting declarations with typed diagnostics.
3. Namespace packages do not require `__init__.py`; admissibility comes from a
   contained import root, not filesystem order.
4. Build a `PythonProjectModuleIndex` in `compass-resolve` mapping source file
   to all admissible module keys and module key to all source files. Keep
   bounded completeness with the existing candidate budgets.
5. A unique project-derived key becomes the producer's canonical
   `module_or_package` and qualified-name prefix. No candidates falls back to
   the current contained repository-relative identity. Multiple distinct keys
   retain the repository-relative identity, publish an ambiguity diagnostic,
   and never select the first root.
6. Register `.pyi` as Python. Normalize `.py` and `.pyi` to the same module key.
   A paired source file remains the graph declaration owner; matching stub
   facts may supplement types/signatures. Stub-only declarations are
   source-backed and publish normally with explicit stub provenance. A source/
   stub mismatch produces `python_stub_source_conflict` and no guessed merge.
7. Bump the Python producer from version 11 to the next live version only when
   the module and stub rules enter production together. Add `MIGRATION.md` and
   `CHANGELOG.md` entries because qualified names and graph IDs can change for
   uniquely proven `src/` layouts.

### 2. Typed Python evidence

Reuse the current universal schema fields; do not create Python-only graph
attributes when `DeclarationFact`, `BindingFact`, `ResolutionConstraint`,
`TypeOf`, and `Returns` already represent the meaning.

The initial sound subset is:

- source parameter declarations, `parameter_count`, variadic status, and
  canonical parameter annotations;
- return annotations and direct source-proven return candidates;
- annotated assignments and class fields;
- literal `TypeAlias`/PEP 695 aliases where the pinned grammar supports them;
- call argument count for positional plus keyword arguments, with `*args` or
  `**kwargs` marking arity incomplete rather than pretending to be exact;
- straight-line dominating bindings such as `x: Service = Service()` and
  `x = Service()` when the initializer target is one exact class;
- source-proven call-result bindings when the selected callable has exactly
  one nominal return candidate;
- bounded chains through properties, static/class methods, and callable
  objects only after their own exact declarations are present;
- string/forward annotations resolved under module/import scope, never by a
  repository-wide terminal-name fallback.

The producer must reject initializer receiver inference when the constructor
spelling is shadowed by a parameter, local, comprehension, lambda, `global`, or
`nonlocal` binding. Conditional, loop-carried, exception, match, descriptor,
metaclass, monkey-patch, and dynamic-factory flows remain unresolved until
independently qualified.

### 3. Generic framework roles and Python route stages

Generalize the existing role publication seam before adding Python model and
service roles:

- `RawFrameworkRoleFact` remains the source contract.
- `compass-resolve` resolves the subject through `FrameworkTargetIndex` and
  validates the role against the existing `NodeRole` vocabulary.
- Existing `ui_role` inputs remain accepted for compatibility, but new facts
  use the neutral internal kind `framework_role`.
- This plan may attach only existing roles: `controller`, `route_handler`,
  `middleware`, `service`, `resolver`, `consumer`, `producer`, `subscriber`,
  `repository`, and `model`.
- A missing, ambiguous, invalid, or truncated role target remains a typed
  diagnostic and does not modify a node.

Add `dependency` and `security` to `RawRouteStageRole`, resolver
`RouteStageRole`, and public `RouteStage`. This is the only planned public graph
vocabulary widening. Update the strict Rust and TypeScript consumers in the
same PR. A dependency provider receives existing node role `service`; a
security provider receives existing role `middleware`. The route edge retains
the precise `dependency` or `security` stage so consumers do not confuse DI
with HTTP middleware.

FastAPI stage ordering is deterministic but does not pretend to reproduce
runtime scheduling:

1. application dependencies;
2. outer-to-inner included-router dependencies;
3. route-decorator dependencies;
4. handler parameter dependencies in declaration order;
5. handler.

Nested subdependencies are separate `depends_on` edges between exact provider
callables. Store scope/depth/lifecycle in bounded detail; do not flatten an
arbitrary dependency graph into a false total execution order.

### 4. Universal framework packs

Create one descriptor and runtime adapter per semantic/version boundary:

| Pack ID | Framework identities | Capabilities | Relations |
| --- | --- | --- | --- |
| `fastapi-python` | `fastapi` | HTTP routes, dependency injection, security | `routes_to`, `depends_on` |
| `starlette-python` | `starlette` | HTTP routes | `routes_to`, `registers` |
| `django-python` | `django`, `django-orm` | HTTP routes, persistence, security | `routes_to`, `maps_to`, `depends_on` |
| `django-rest-framework-python` | `django-rest-framework` | HTTP routes, dependency injection, security | `routes_to`, `depends_on`, `registers` |
| `flask-python` | `flask` | HTTP routes | `routes_to`, `registers` |
| `pydantic-python` | `pydantic` | dependency injection/data modeling | `depends_on` |
| `sqlalchemy-python` | `sqlalchemy` | persistence | `maps_to`, `depends_on` |
| `celery-python` | `celery` | messaging, scheduling | `produces`, `consumes`, `schedules`, `triggers` |

If the live descriptor enum still lacks a truthful data-modeling capability,
add `FrameworkCapability::DataModeling` and allow `DependsOn` for dependency
injection, security, or data modeling. This enum is an internal registration
contract; it does not add a public edge kind. Do not label Pydantic as
persistence merely to satisfy validation.

Each pack:

- uses semantics version 1 at initial cutover;
- declares sorted dependency markers (`fastapi`, `starlette`, `django`,
  `djangorestframework`, `flask`, `pydantic`, `sqlalchemy`, `celery` as
  applicable);
- uses advisory manifest activation plus exact import/call/decorator/base-type
  evidence; a dependency name alone never emits a fact;
- accepts only the Python roles/capabilities it actually consumes;
- emits exact evidence or a named bounded convention; substring activation and
  unlabeled regex provenance are forbidden;
- preserves declaration IDs, occurrence anchors, multiplicity, source order,
  ambiguity, and pack-specific limits.

Target module layout:

```text
crates/compass-languages/src/frameworks/python/
|-- mod.rs          shared evidence/AST join and deterministic fact keys
|-- syntax.rs       bounded borrowed-tree helpers; no second parse
|-- routing.rs      shared literal path, argument, and receiver helpers
|-- fastapi.rs
|-- starlette.rs
|-- django.rs
|-- drf.rs
|-- flask.rs
|-- pydantic.rs
|-- sqlalchemy.rs
`-- celery.rs

crates/compass-resolve/src/frameworks/python/
|-- mod.rs
|-- mounts.rs       bounded receiver-ID mount multigraph
|-- django.rs       urlpatterns/include/namespace composition
|-- drf.rs          router/viewset expansion
`-- dependencies.rs inherited FastAPI dependency stages
```

The language adapter may consult the borrowed Tree-sitter tree for literal
arguments, but a framework call/decorator/base/annotation must join to the
matching universal occurrence or declaration range. It must never rebuild an
independent import table or select receivers by file-wide spelling.

### 5. Framework semantics

#### FastAPI, Starlette, and Pydantic

Support, in this order:

1. Exact `FastAPI`/`APIRouter`/Starlette receiver declarations and aliases.
2. HTTP decorators, `api_route`, `add_api_route`, `Route`, `WebSocketRoute`,
   and FastAPI WebSocket decorators with literal/static paths and exact
   handlers.
3. A receiver-ID mount multigraph for `include_router`, Starlette `Mount`, and
   repeated/nested mounts. Preserve every valid mount, prefix, anchor, and
   multiplicity. Cycles and over-limit traversals are explicit diagnostics.
4. FastAPI dependencies from application/router/include/route lists,
   parameter defaults, `Annotated[..., Depends(...)]`, and `Security`.
   Resolve aliases and nested subdependencies; record `yield` lifecycle without
   running providers.
5. Request and response schema dependencies from handler annotations,
   `response_model`, and exact Pydantic `BaseModel` subclasses. Preserve unions,
   generic arguments, forward references, and unresolved dynamic model values.
6. Pydantic model fields, validators, serializers, computed fields, and model
   inheritance as existing declaration/role/type/reference evidence. Do not
   create synthetic schema declarations when the class node already exists.
7. Middleware, lifespan, and background-task registrations only when both
   registration and callable target are source-proven.

#### Django and Django REST Framework

Support, in this order:

1. Bind `path`, `re_path`, legacy `url`, `include`, and `i18n_patterns` to exact
   imports/re-exports. A same-named local helper or rebound import is a negative.
2. Follow only lists/tuples/concatenations/imported pattern collections that
   contribute to `urlpatterns`. Support namespaces, application names, local
   list/tuple includes, imported `urlpatterns`, custom converters, and
   class-based `.as_view()` dispatch. Dynamic path computation remains
   unresolved.
3. Model `SimpleRouter`/`DefaultRouter.register`, viewsets, standard action
   templates, and `@action(detail=..., methods=..., url_path=...)`. Generated
   routes require a closed router/version template and exact viewset methods;
   custom/dynamic routers remain unresolved.
4. Attach existing `controller`/`route_handler` roles to views/viewsets and
   `depends_on` edges to exact serializers, permissions, authentication
   classes, filters, and throttles.
5. Mark exact Django model subclasses with existing `model` role. Emit fields,
   foreign keys, many-to-many/one-to-one targets, managers, and serializer-to-
   model dependencies from structural evidence.
6. Preserve current explicit `Meta.db_table` mapping. Add conventional table
   naming only as a named, versioned convention and publish `maps_to` only when
   a matching source/introspected database-table node exists. Never synthesize
   a table to make the relation exact.
7. Add source-proven signals, middleware/settings registration, and admin
   registration after route/DRF/ORM strata qualify. Migrations are visible
   through existing Python calls and migration files; complete migration-state
   simulation is out of scope.

#### Flask

1. Change an undecorated-method `@app.route` from `ANY` to declared `GET`.
   Retain implicit HEAD/OPTIONS as metadata, not additional declared routes,
   unless a separate contract review approves them.
2. Support exact `Flask`/`Blueprint` bindings, application factories,
   `get`/`post`/`put`/`patch`/`delete`, `add_url_rule`, `MethodView.as_view`,
   nested blueprints, hooks, error handlers, and source-proven prefixes.
3. Compose blueprints through the same bounded receiver-ID mount multigraph as
   FastAPI, without conflating same-named receivers in different modules.

#### SQLAlchemy and Celery

1. Replace Python ORM regexes with exact SQLAlchemy 2 `DeclarativeBase`,
   `Mapped`, `mapped_column`, `relationship`, `ForeignKey`, and table metadata
   evidence. Reuse existing class/field/property nodes and `model` role.
2. Keep `maps_to` fail-closed on an existing database table. Field/relationship
   targets use `type_of`/`references`/`depends_on` as appropriate; do not invent
   rows, queries, or runtime session flow.
3. Replace Celery regexes with exact imported decorator/call evidence for
   `shared_task`, application tasks, literal names/queues, `.delay`,
   `apply_async`, `send_task`, signatures, `chain`, `group`, `chord`, retry,
   and statically declared beat schedules.
4. Use existing job/message/queue nodes and
   `produces`/`consumes`/`schedules`/`triggers` edges. Preserve unresolved
   string task names and dynamic canvas elements without terminal-name guesses.

### 6. Optional managed analyzer

The native structural program must qualify independently. After it does, add
an explicit optional profile following `docs/design/managed-language-analyzers.md`:

- verified, pinned `scip-python` first;
- an explicit frozen environment manifest containing Python version, roots,
  search paths, project configuration, typeshed/stubs, editable packages,
  namespace policy, and library-code-for-types policy;
- no implicit `pip`, environment discovery, imports, or network;
- exact source-anchor join through `compass-program` before graph projection;
- typed receivers, overloads, protocols, properties, callable objects,
  callbacks, returns, and generated members only when analyzer evidence is
  fresh and source-matched;
- source/stub/analyzer disagreement retained as an explicit conflict;
- absence, timeout, stale output, unsupported version, or permission denial
  leaves the already-qualified native graph unchanged.

This optional phase is not permission to weaken the native qualification gate.

## Pinned corpora and independent truth

Use these immutable revisions. Revalidate the full SHA, license, cleanliness,
and source inventory before use. Revisions were read from the mounted checkout
or repository HEAD on 2026-08-25; never replace them with a mutable tag/branch.

| Corpus | Repository | Commit | Purpose |
| --- | --- | --- | --- |
| Django | `https://github.com/django/django.git` | `c9eb16a87e60c305fb3651459639f647cce498db` | language scale, URL patterns, CBVs, ORM, signals |
| FastAPI | `https://github.com/fastapi/fastapi.git` | `0c2b6aafd7a2e3a5bf1055ea0ed0a41da15ba5f4` | dependency/router/WebSocket framework semantics |
| FastAPI full-stack template | `https://github.com/fastapi/full-stack-fastapi-template.git` | `68adb40d37425f6f8668ec7e5a054500d045e43e` | representative FastAPI/Pydantic/SQL application |
| NetBox | `https://github.com/netbox-community/netbox.git` | `194f3bbde20dd298d0cc2cad0e8d9134d548b431` | representative Django/DRF application |
| Pydantic | `https://github.com/pydantic/pydantic.git` | `55084b74b3abfdfeee61105b9840e968a0739800` | model/validator/serializer/type stress |
| SQLAlchemy | `https://github.com/sqlalchemy/sqlalchemy.git` | `0770f6e96ba40b192837d5191dd95e9a5838cef3` | declarative/relationship/type stress |
| Starlette | `https://github.com/encode/starlette.git` | `04d7869c1ec3016fa11f58382a9ac7bc14b46662` | explicit route/mount/middleware semantics |
| Django REST Framework | `https://github.com/encode/django-rest-framework.git` | `751a19fe1b237beca9af7d587fce55d3e09d3741` | routers/viewsets/actions/serializers |
| Flask | `https://github.com/pallets/flask.git` | `d318b683471101618febed18996405ad26462110` | application factories/blueprints/views/hooks |
| Celery | `https://github.com/celery/celery.git` | `8d2bccca0478cad48f31a75eaebc0ce389f65425` | task/canvas/routing/schedule semantics |

Existing checkouts under `/Volumes/Workspace/Github` are read-only inputs. Put
missing repositories below `/Volumes/Workspace/Github/<owner>/<repository>`;
do not clone into the Compass tree or `/tmp`, and do not reset/clean a checkout
that already exists. Use a separate detached worktree when an existing checkout
is at another revision.

The independent oracle uses Python's standard-library `ast` and `tokenize`
only in qualification. It must never import corpus code. It records exact UTF-8
byte ranges, source globs, every scanned-file status, parser identity, inventory
digest, and bounded construct counts. Source globs must describe the claimed
population explicitly; do not silently omit a syntax-error file after scanning
it. A corpus scope that includes unsupported syntax is ineligible until the
scope or oracle is corrected and reviewed.

The oracle must inventory at least:

- declarations, parameters, fields, imports/re-exports, decorators, bases,
  annotations, calls, constructions, member accesses, returns, and assignments;
- route registrations, mounts/includes, dependency/security providers,
  schemas/serializers/models, ORM fields/relationships, tasks, invocations,
  schedules, and exact framework roles;
- positive, negative, ambiguous, represented-elsewhere, unsupported, and
  limit-exceeded outcomes.

Do not use Tree-sitter or Compass framework facts as their own oracle. Do not
treat documentation examples or another product's graph as truth.

## Commands executors will need

Every Cargo command must use this checkout's unique mounted target directory.
Run the preflight first in every new shell. Do not fall back to local `target/`.

```bash
test -d /Volumes/Workspace && \
mkdir -p /Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks && \
test -w /Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks
```

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Format | `cargo fmt --all -- --check` | exit 0, no diff |
| Python language conformance | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo test -p compass-languages --test python_universal_conformance --locked` | all cases pass |
| Python pack extraction | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo test -p compass-languages --test python_framework_universal_packs --locked` | all pack cases pass |
| Python universal resolution | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo test -p compass-resolve --test universal_resolution python --locked` | all Python cases pass |
| Import/project resolution | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo test -p compass-resolve --test python_import_provenance --locked` | all cases pass |
| Python routes | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo test -p compass-resolve --test python_routes --locked` | all cases pass |
| Python universal framework publication | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo test -p compass-resolve --test python_frameworks_universal --locked` | all route/domain/role/relation cases pass |
| Domain regression | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo test -p compass-resolve --test domain_resolution --locked` | all language/framework domains pass |
| Graph contract | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo test -p compass-model --test code_graph_v1 --locked` | route-stage and role contract passes |
| Python oracle tests | `python3 -m unittest scripts.tests.test_python_framework_oracle scripts.tests.test_python_framework_quality_audit` | exit 0 |
| Fixture qualification | `./scripts/qualify_python_frameworks.sh --fixtures-only` | deterministic fixture report passes twice |
| Pinned qualification | `./scripts/qualify_python_frameworks.sh --pinned --baseline tests/qualification/python-framework-baseline.json` | every required stratum passes; report written outside checkout |
| Broad code graph | `./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0 |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0; no Graphify/Python-runtime boundary violation |
| Viewer strict contracts | `npm run typecheck:js && npm run test:js && node scripts/check_viewer_assets.mjs` | exit 0 |
| Targeted Clippy | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo clippy -p compass-languages -p compass-resolve -p compass-model -p compass-graph -p compass-core --all-targets --all-features --locked -- -D warnings` | exit 0 |
| Native baseline | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo clippy --workspace --lib --bins --locked -- -D warnings && CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks cargo test --workspace --lib --bins --locked` | exit 0 |

If `node_modules` is absent, run `npm ci` before the viewer commands. Do not
change the lockfile unless a separately reviewed dependency change requires it.

## Scope

**In scope**:

- `crates/compass-languages/src/evidence_pipeline.rs`
- Python-owned portions of `crates/compass-languages/src/evidence/` and
  `crates/compass-languages/src/evidence/build.rs`
- `crates/compass-languages/src/registry.rs`
- `crates/compass-languages/src/project_evidence.rs`
- `crates/compass-languages/src/frameworks/{mod.rs,model.rs,pack.rs,python.rs,python/,enterprise.rs}`
- `crates/compass-resolve/src/evidence/` Python project/policy/index paths
- `crates/compass-resolve/src/frameworks/`
- existing Code Graph v1 route-stage/role consumers in `compass-model`,
  `compass-graph`, `compass-core`, `compass-query`, `compass-output`,
  `compass-cli`, `compass-mcp`, and `packages/compass-viewer`
- generated viewer assets only when the source contract change requires them
- Python/framework fixtures and tests under `fixtures/`, crate `tests/`, and
  `tests/qualification/`
- independent qualification code under `benchmarks/performance/compass/` and
  `scripts/`
- `docs/reference/framework-routes.md`, the Python qualification/reference
  documentation, `PERFORMANCE.md`, `COMPATIBILITY.md`, `MIGRATION.md`, and
  `CHANGELOG.md`
- this plan and `advisor-plans/README.md` status only

**Out of scope**:

- unrelated language producers or framework semantics; shared substrate edits
  require regression tests proving their outputs remain unchanged
- vendored grammar changes unless the pinned Python grammar demonstrably lacks
  a required current syntax node; STOP before touching `vendor/`
- a new public graph major or new node/edge kind
- credential, environment, package-install, network, or Python execution paths
- query-ranking or UI redesign unrelated to the two new route-stage enum values
- performance optimization before semantic parity and qualification evidence

## Git workflow

- Branches use `codex/023-python-<phase-slug>`.
- Use conventional commits matching repository history, for example
  `feat(python): add source-root-aware module identity` or
  `test(python): add pinned framework qualification`.
- Keep each phase independently reviewable. Prefer one characterization commit,
  one implementation commit, and one docs/qualification commit per phase.
- Do not push, open a PR, or publish qualification artifacts unless instructed.
- Never commit generated graphs, `.compass/`, `compass-out/`, corpus checkouts,
  credentials, or machine-specific paths.

## Execution phases

### Phase 0: Freeze established behavior and build the qualification skeleton

1. Add `crates/compass-languages/tests/python_universal_conformance.rs` with
   exact evidence snapshots for modules, declarations, imports, aliases,
   decorators, annotations, construction, receivers, C3, ownership, malformed
   source, repeated occurrences, and deterministic ordering.
2. Add adversarial characterization cases before changing behavior:
   shadowed initializer constructors; `.pyi`; `src/` layout; same-named
   routers in different scopes; aliased/shadowed Django `path`; dotted and
   nested FastAPI mounts; Flask default route; dynamic values; and limit paths.
3. Add `tests/qualification/python-framework-repositories.toml`, a strict
   checked-in fixture expectation ledger, and fixture-only
   `scripts/qualify_python_frameworks.{sh,py}`. Reuse
   `FrameworkEvidenceExpectationSet` and `compass.quality-audit/2`.
4. Extract the Python `ast` provider from
   `benchmarks/performance/compass/occurrences.py` into a dedicated bounded
   module only if doing so keeps current provider identity/inventory tests
   byte-equivalent. Extend it capability by capability; do not rewrite the
   generic audit engine.
5. Record a current baseline with producer/pack IDs, evidence/graph digests,
   route/domain counts, omissions, diagnostics, cold/warm/forced timings,
   semantic and fact-neutral edits, delete/rename/restore, worker equivalence,
   and peak RSS. Label it `established-unqualified`; it is not a quality claim.

**Verify**:

```bash
python3 -m unittest scripts.tests.test_python_framework_oracle
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-languages --test python_universal_conformance --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-resolve --test python_routes --locked
./scripts/qualify_python_frameworks.sh --fixtures-only
```

Expected: all established cases pass; confirmed defects are encoded as
explicit expected-current-behavior tests or an `expectedGap` ledger, not silently
corrected in Phase 0; two fixture runs produce byte-identical reports.

### Phase 1: Make Python project identity and stubs deterministic

1. Add bounded Python import-root parsing and diagnostics to
   `project_evidence.rs`, modeled after contained Composer roots. Bump the
   project-evidence schema and cache identity.
2. Register `.pyi` in the language registry and normalize `.py`/`.pyi` module
   suffixes consistently in extraction and resolution.
3. Add `PythonProjectModuleIndex` to `ProjectContext` and use it for absolute
   and relative imports, re-exports, framework handler lookup, and external
   placeholder decisions.
4. Implement the unique/zero/ambiguous module identity rules above. Bump the
   Python producer version atomically; never dual-publish old and new IDs.
5. Implement source/stub pairing, stub-only declarations, conflicts, and cache
   invalidation. Preserve source anchors independently.
6. Add compatibility, migration, and release notes. Existing historical
   realizations remain unchanged; new builds use the new producer identity.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-languages project_evidence --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-languages --test python_universal_conformance --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-resolve --test python_import_provenance --locked
```

Expected: flat, `src/`, namespace, monorepo, paired-stub, stub-only, conflicting,
escape, duplicate-root, and ambiguity cases pass; repeated builds have equal
evidence and graph digests.

### Phase 2: Add conservative typed Python bindings and call results

1. Populate Python parameter count/types/variadic metadata and parameter
   declarations using existing universal fields.
2. Emit exact `TypeOf` and `Returns` candidates for supported annotations,
   annotated assignments, fields, and returns. Canonicalize unions, generics,
   `Optional`, `Annotated`, forward strings, and qualified imports within
   explicit bounds.
3. Record Python call arity and literal argument types only when complete.
   Starred arguments make arity incomplete.
4. Add source-ordered binding versions and exact initializer/call-result
   bindings for the sound subset. Apply lexical shadow checks before every
   receiver inference and delete the unproven `module.Foo` fallback.
5. Add protocols, properties, static/class methods, callable objects, and
   descriptors only one independently tested family at a time. Ambiguous
   overloads or multiple returns remain unresolved.
6. Bump the Python producer version again if Phase 2 lands separately from
   Phase 1; never change evidence semantics without a version bump.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-languages --test python_universal_conformance --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-resolve --test universal_resolution python --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-resolve --test python_import_provenance --locked
```

Expected: typed straight-line cases resolve exactly; shadowed, conditional,
dynamic, `Any`, conflicting-stub, and over-limit cases publish no fabricated
edge and retain explicit diagnostics.

### Phase 3: Generalize framework roles and dependency/security stages

1. Generalize `RawFrameworkRoleFact` publication from UI-only roles to the
   existing public `NodeRole` vocabulary. Preserve the React compatibility
   path and its exact output.
2. Add `dependency` and `security` route stages through language facts,
   resolution, model validation, graph normalization, task context, query,
   CLI/MCP serialization, viewer Zod contracts, fixtures, and generated assets.
3. Add `FrameworkCapability::DataModeling` if needed and update descriptor
   validation without widening the public edge vocabulary.
4. Add endpoint, ambiguity, truncation, multiplicity, ordering, unknown-enum,
   and older-reader failure tests.
5. Update `COMPATIBILITY.md` and the query/route reference for strict consumers.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-model --test code_graph_v1 --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-resolve --test framework_routes --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-core --test task_context --locked
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
```

Expected: existing React/route snapshots remain equivalent except for explicit
additive enum manifests; dependency and security round-trip through every
strict consumer and unknown values still fail closed.

### Phase 4: Build candidate universal packs and atomically replace `python-web`

1. Refactor shared Python literal/AST helpers without changing established
   output. Characterize before moving `python.rs` into the target module tree.
2. Implement candidate `django-python`, `fastapi-python`, and `flask-python`
   detectors over universal evidence. Keep them out of production runtime
   registration while candidate fixture and diff tests run.
3. Implement the receiver-ID mount multigraph and Django include graph with
   bounded traversal, exact imports, cycles, multiplicity, and ambiguity.
4. Compare established and candidate facts. Preserve semantically correct
   route identity and ordering; explicitly approve the Flask `ANY` to `GET`
   correction, Django false-positive removal, exact provenance changes, and
   newly unresolved ambiguous cases.
5. In one atomic commit, register the three universal descriptors/runtime
   adapters, register matching resolver ownership, and remove `python-web`.
   No production dual-run, duplicate route, legacy fallback, or per-framework
   partial cut is allowed.
6. Bump pack semantics/cache identities and update fixtures/migration notes.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-languages --test python_framework_universal_packs --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-resolve --test python_routes --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-resolve --test python_frameworks_universal --locked
./scripts/qualify_python_frameworks.sh --fixtures-only
```

Expected: registry/adapter ownership matches exactly; no `python-web` runtime
pack remains; every positive has one exact fact; shadows, wrong frameworks,
dynamic values, ambiguous mounts, and limits remain negative/explicit.

### Phase 5: Complete FastAPI, Starlette, and Pydantic intelligence

1. Add imperative/WebSocket/Starlette route families and the complete bounded
   mount multigraph.
2. Add application/router/include/route/parameter dependency and security
   facts, inherited stages, subdependency edges, and yield lifecycle details.
3. Add Pydantic model roles, fields, validators/serializers, and endpoint
   request/response schema dependencies.
4. Add middleware, lifespan, and background-task registrations only where
   exact source evidence exists.
5. Register `starlette-python` and `pydantic-python` after their candidate
   gates pass. They may land independently because they replace no legacy
   production pack.
6. Add fixture query oracles for route -> security/dependency -> service and
   route -> request/response model impact.

**Verify**: run both Python pack tests, both Python resolver tests, fixture
qualification, and `cargo test -p compass-query --locked` with the mounted
target. Expected: every advertised FastAPI/Starlette/Pydantic stratum has
positive, negative, ambiguous, repeated, nested, and limit coverage.

### Phase 6: Complete Django and Django REST Framework intelligence

1. Add exact `urlpatterns` dataflow, includes, namespaces, i18n patterns,
   converters, and CBV dispatch.
2. Add DRF router/viewset/action expansion with closed, versioned route
   templates and explicit unresolved custom routers.
3. Add serializer, permission, authentication, filter, throttle, model, field,
   and relationship facts using existing roles/relations.
4. Add signals, middleware/settings, and admin registration only after route
   and DRF tests pass.
5. Register `django-rest-framework-python`; bump `django-python` semantics when
   adding its ORM/security capabilities.
6. Add fixture query oracles for URL -> viewset/action -> serializer -> model
   and signal/registration impact.

**Verify**: run Python pack/resolver/domain tests plus the fixture gate.
Expected: namespaces and route multiplicity are preserved; dynamic patterns,
custom routers, ambiguous serializers/models, and absent DB tables remain
unresolved without synthetic targets.

### Phase 7: Replace Python enterprise regexes with Flask, SQLAlchemy, and Celery facts

1. Complete Flask factories, shortcuts, `add_url_rule`, `MethodView`, nested
   blueprints, hooks, and error handlers.
2. Add `sqlalchemy-python` exact declarations, fields, relationships, and DB
   mappings.
3. Add `celery-python` exact tasks, invocations, canvas, queues, and schedules.
4. Compare new SQLAlchemy/Celery facts with the established Python enterprise
   output and explicitly disposition anchor/provenance/identity changes.
5. In one atomic commit, make `enterprise-domain-facts` no-op for Python and
   register `sqlalchemy-python`/`celery-python`. Preserve the established pack
   unchanged for every other language.
6. Delete Python regex activation/extraction after the cut; do not retain it as
   a fallback.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-resolve --test domain_resolution --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test -p compass-languages --test python_framework_universal_packs --locked
./scripts/qualify_python_frameworks.sh --fixtures-only
sh scripts/check_product_boundary.sh
```

Expected: comments/strings/aliases/multiline forms cannot create regex false
facts; evidence origin is exact or named heuristic; non-Python enterprise
fixtures remain byte-equivalent.

### Phase 8: Add optional `scip-python` enrichment

1. Implement the frozen environment/profile contract and managed artifact
   capability described above. Do not make it the default.
2. Ingest only a verified bounded SCIP artifact; join occurrences to exact
   source bytes and current producer/project identity.
3. Add conflict/staleness/cache/timeout/cancellation/permission tests and prove
   the native graph is unchanged when enrichment is unavailable.
4. Qualify typed receiver, protocol, property, callable-object, overload,
   callback, and return facts separately from native structural facts.

**Verify**: run the managed-analyzer contract tests selected by the live design
plus native Python fixture qualification with the analyzer absent. Expected:
native digests are identical when enrichment is disabled/unavailable; enabled
facts carry analyzer/profile provenance and exact anchors.

### Phase 9: Run pinned qualification, freeze performance, and promote truthful claims

1. Clone/create detached read-only corpus checkouts on the mounted volume and
   verify commit, license, source scope, inventory digest, and cleanliness.
2. Produce source-oracle inventories and graph-backed expectation ledgers for
   every advertised language and framework capability. Human-review accepted,
   rejected, ambiguous, and represented-elsewhere judgments.
3. Meet the existing production gates: at least 2,000 accepted relationships,
   400 per corpus, 100 per required relation and capability, target-cluster
   diversity, 99.5% observed precision, 99% Wilson lower bound, 99% precision
   and 95% recall per capability, and zero fabricated/cross-language/unsafe
   substitutions.
4. Record cold, unchanged warm, forced, semantic edit, fact-neutral edit,
   project-config edit, dependency-marker edit, delete/rename/restore,
   alternate checkout, one/default/max workers, interruption/resume, graph
   size, duration, and peak RSS. Normal promotion rejects more than 10%
   regression from the reviewed baseline; a larger intentional cost requires a
   written performance decision, not a silently widened budget.
5. Run pinned qualification twice against the exact release binary and require
   byte-identical canonical reports. Store large artifacts under the mounted
   target, not in the repository.
6. Update framework support docs with implemented, fixture-qualified,
   pinned-qualified, and production-qualified status per pack. Keep
   `compass.python` `Qualifying` unless the complete language capability audit,
   not merely framework qualification, passes and maintainers separately
   approve promotion.
7. Run the native baseline and applicable Code Graph/CLI/viewer gates from
   `AGENTS.md`; document every skipped gate and reason.

**Verify**:

```bash
./scripts/qualify_python_frameworks.sh --pinned \
  --baseline tests/qualification/python-framework-baseline.json
./scripts/qualify_python_frameworks.sh --pinned \
  --baseline tests/qualification/python-framework-baseline.json
./scripts/qualify_code_graph_v1.sh --fixtures-only
sh scripts/check_product_boundary.sh
cargo fmt --all -- --check
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo clippy --workspace --lib --bins --locked -- -D warnings
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-plan023-python-frameworks \
  cargo test --workspace --lib --bins --locked
npm run typecheck:js
npm run test:js
node scripts/check_viewer_assets.mjs
```

Expected: all commands exit 0; both pinned reports have identical canonical
digests; every capability threshold and lifecycle row passes; corpus source
trees remain unchanged.

## Test plan

### Project/module identity

- Flat package, explicit and conventional `src/`, multiple roots, nested
  pyprojects, namespace packages, monorepos, editable/static package entries,
  Windows separators, non-UTF-8 paths where supported, symlink escapes,
  absolute/parent paths, duplicates, root limits, and deterministic ordering.
- `.py`, `.pyi`, paired source/stub, stub-only, mismatch, overload, protocol,
  invalid stub, rename/delete/restore, and cache reopen.

### Typed Python semantics

- Parameters/defaults/keyword-only/positional-only/variadics; forward refs,
  unions, generics, `Annotated`, type aliases, class fields, returns,
  properties, class/static methods, protocols, callable objects, and exact
  straight-line result chains.
- Shadowing in parameters, locals, comprehensions, lambdas, `global`,
  `nonlocal`, conditionals, loops, exception handlers, match, and rebindings.
- Multiple candidates, `Any`/`Unknown`, dynamic imports/factories, malformed
  source, parser recovery overlap, limit overflow, and cross-language negatives.

### Framework extraction and resolution

- Every syntax family listed in the framework semantics section with aliases,
  re-exports, source order, exact anchors, repeated registrations, multiple
  mounts, nesting, cycles, ambiguity, wrong-framework imports, comments/
  strings, shadowed receivers, dynamic values, and limits.
- Exact route operation/path/handler/stage/position, role, direction,
  multiplicity, provenance, pack ID/version, and unresolved candidates.
- Flask default GET regression; Django local `path` negative; FastAPI dotted
  and transitive mount; parameter/router/app dependencies; DRF generated route
  multiplicity; absent SQL table; Celery string task ambiguity.
- Existing React, Spring, Rails, PHP, and non-Python enterprise regression
  suites remain green after shared substrate edits.

### Lifecycle and qualification

- Cold/warm/force equality, fact-neutral and semantic edits, manifest/source-
  root/stub changes, pack activation changes, delete/rename/restore,
  interruption/resume, one/default/max workers, alternate checkout, read-only
  corpus enforcement, exact release binary identity, and no network/process
  execution.
- Precision, recall, Wilson, target-cluster diversity, anchor, multiplicity,
  ambiguity, truncation, completeness, and per-capability thresholds.

## Done criteria

All must hold:

- [ ] Python project roots and `.pyi` behavior are source-only, bounded,
      deterministic, versioned, and tested.
- [ ] Python calls carry complete arity/typed binding evidence only for the
      qualified sound subset; shadowed/dynamic flows fabricate no edge.
- [ ] `python-web` is absent from the production runtime and no duplicate or
      fallback Python route path remains.
- [ ] Python is disabled in `enterprise-domain-facts`; SQLAlchemy and Celery
      facts come from their universal packs.
- [ ] All eight target pack descriptors have nonzero independent semantics
      versions and exact resolver ownership.
- [ ] FastAPI dependency/security stages round-trip through every strict
      consumer; unknown stages fail closed.
- [ ] No new public node or edge kind was introduced.
- [ ] The complete fixture gate passes twice with identical output.
- [ ] The pinned production audit passes every existing threshold with zero
      critical failures and a byte-stable report.
- [ ] Cold/warm/edit/lifecycle/RSS rows meet the reviewed performance budget.
- [ ] `sh scripts/check_product_boundary.sh` confirms normal Python graph and
      framework behavior requires no Python runtime, analyzer, credentials,
      network, Graphify, or runtime grammar download.
- [ ] Targeted Clippy/tests, the workspace native baseline, CLI/product gate,
      JS strict contracts, and viewer asset determinism pass.
- [ ] `COMPATIBILITY.md`, `MIGRATION.md`, `CHANGELOG.md`, `PERFORMANCE.md`, and
      framework/qualification references describe the exact shipped state and
      unsupported dynamic forms.
- [ ] `git status --short` contains only reviewed in-scope changes and no
      generated graphs, corpus files, credentials, or local state.
- [ ] This plan and its `advisor-plans/README.md` row record the final status,
      exact verification commands, release binary digest, corpus manifest
      digest, and external qualification artifact path.

## STOP conditions

Stop and report; do not improvise if:

- `/Volumes/Workspace` is unavailable or the plan-specific target directory is
  not writable before a Cargo build or pinned qualification.
- A pinned corpus checkout is dirty, at another commit, lacks a reviewable
  license, or would need to be reset/cleaned.
- A required Python syntax form is absent from the pinned grammar and would
  require a vendored grammar change.
- The project/module identity change requires selecting one of multiple valid
  roots by ordering or convenience.
- Source/stub/analyzer facts conflict and there is no explicit conflict state.
- A framework fact can only be recovered by executing/importing Python,
  evaluating configuration, installing packages, or contacting a service.
- A framework target is selected only by a repository-wide terminal name,
  hash/filesystem iteration order, or an incomplete candidate set.
- A limit overflow would be represented as empty/success instead of incomplete
  or error.
- A phase requires a new public node or edge kind. Write a separate contract
  proposal and compatibility review first.
- The candidate/established diff cannot distinguish an intentional semantic
  correction from an identity/provenance regression.
- A targeted or shared regression gate fails twice after a reasonable scoped
  correction.
- The pinned audit misses a per-capability floor or any zero-tolerance gate.
  Keep the pack/status qualifying; do not weaken the threshold or documentation.
- The change requires touching out-of-scope language semantics or another
  framework's output cannot remain equivalent.

## Maintenance notes

- Review every Python producer or pack semantic change for its producer/pack
  version, cache fingerprint, history profile, fixture baseline, and pinned
  audit strata. A code-only detector change without a version bump is invalid.
- Keep module roots, import aliases, receiver bindings, mounts, and dependency
  graphs bounded collections with explicit completeness. One incomplete item
  prevents a unique exact decision.
- Framework adapters interpret source syntax and emit project-neutral facts;
  `compass-resolve` owns target selection, composition, ambiguity, and graph
  publication. Do not move project-wide lookup back into language adapters.
- A source-only structural graph remains the product baseline. Optional SCIP
  enrichment is additive, fresh, provenance-preserving, and independently
  removable.
- When framework majors change, add version/corpus evidence and bump only that
  pack's semantics version. Do not silently broaden a template across versions.
- Review FastAPI dependency order, DRF generated route templates, Django table
  naming, and Celery canvas multiplicity especially carefully; these are the
  highest-risk places to invent runtime meaning from source convenience.
- A later pytest plan should reuse the generic role and qualification substrate
  from this program, but must separately add and qualify testing-specific pack
  capabilities and `tests` relationships.

## Deferred directions and rejected shortcuts

- **Pyright as the first analyzer**: rejected. The repository design selects
  verified `scip-python` first and permits Pyright only behind a supported
  stable interface or pinned bridge.
- **Runtime framework introspection**: rejected because it executes untrusted
  project code and makes graph construction environment-dependent.
- **One permanent `python-frameworks` pack**: rejected because Django, DRF,
  FastAPI, Starlette, Flask, Pydantic, SQLAlchemy, and Celery need independent
  activation, semantics versions, capabilities, limits, and scorecards.
- **Keep regex extraction as recall fallback**: rejected because it weakens
  provenance and can reintroduce facts the universal path correctly leaves
  unresolved.
- **Promote Python after framework-only success**: rejected. Framework
  qualification can promote truthful pack claims, but the language pipeline
  remains `Qualifying` until its complete advertised capability audit passes.
