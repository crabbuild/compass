# Framework Activation Evidence Hardening Design

**Date:** 2026-07-30

**Status:** Approved for implementation planning

**Implementation root:** `/Users/haipingfu/graphify/compass`

## Purpose

Compass currently allows some framework detectors to activate from weak textual
or path evidence. The most concrete false positive is Laravel: a PHP file under
a directory named `routes` can cause `Route::get(...)` to be interpreted as a
Laravel route even when `Route` resolves to an unrelated class.

This work strengthens framework extraction in two ordered deliveries:

1. fix Laravel activation immediately; and
2. introduce one shared activation-evidence mechanism and migrate every
   existing framework pack to it.

The accuracy gain must not introduce repeated whole-file scans or change the
public framework-fact schema.

## Goals

- Laravel route facts are emitted only when the static-call receiver resolves
  to `Illuminate\Support\Facades\Route`.
- Laravel aliases and fully qualified facade references remain supported.
- A directory named `routes` is never sufficient Laravel evidence.
- Dynamic or unresolved Laravel handlers do not produce facts that claim exact
  route-to-handler resolution.
- Every framework detector passes through a shared activation decision before
  emitting framework facts.
- Exact imports, receiver identities, decorators, attributes, macros,
  manifests, and framework-owned configuration formats are distinguished from
  weak naming conventions.
- Weak path and filename conventions may support activation but may not, by
  themselves, activate code-driven framework facts.
- Near matches remain available to the generic language graph and do not
  become framework facts.
- Evidence is collected at most once per relevant syntax tree or declarative
  artifact and reused by the pack detector.
- Existing framework fact limits, provenance, route normalization, and public
  serialization remain compatible.

## Non-goals

- Replace generic language extraction or symbol resolution.
- Add probabilistic confidence scoring to the public graph format.
- Infer dynamic PHP values, execute framework code, or load application
  containers.
- Make package-manager network calls.
- Redesign route normalization, resource expansion, or handler identity.
- Add a public framework plugin API.
- Require all frameworks to use identical evidence. Each framework keeps an
  explicit policy matching its actual programming model.

## Chosen approach

Compass will use a staged, typed evidence model.

The Laravel delivery first proves the behavior with a PHP-specific import
table and AST receiver resolution. It is intentionally shippable on its own.
The second delivery extracts the reusable concepts into an internal evidence
module and moves all packs behind it.

A disposable Laravel-only string guard was rejected because it would still
mis-handle aliases and would be replaced during the second delivery. A
registry-first rewrite was rejected because it would delay the requested
Laravel correction and combine the behavioral fix with a wider refactor.

## Delivery 1: Laravel evidence gate

### Import and receiver resolution

The PHP framework detector will traverse the tree-sitter syntax tree once to
build a local import table. Each entry records:

- the local binding;
- the normalized fully qualified target;
- the import anchor; and
- whether the binding is explicit or aliased.

For example:

```php
use Illuminate\Support\Facades\Route;
use Illuminate\Support\Facades\Route as Router;
```

resolve `Route` and `Router`, respectively, to
`Illuminate\Support\Facades\Route`.

Laravel route extraction will inspect scoped-call AST nodes rather than search
the source for the literal text `Route::`. A call is eligible only when its
receiver is one of:

- a local name that resolves through the import table to the Laravel facade;
  or
- the fully qualified `\Illuminate\Support\Facades\Route` name.

An unqualified `Route` with no matching import, a local class named `Route`,
and an alias resolving to any other fully qualified class are ineligible. The
parent directory and filename are not part of this decision.

### Static route shapes

After receiver resolution, the detector retains the currently supported
Laravel operations:

- HTTP methods;
- `any`;
- `match`;
- `resource`; and
- `prefix(...)->group(...)` composition.

Paths, method lists, prefixes, controller names, and action names must remain
statically inspectable wherever Compass emits an exact normalized route or
handler reference. Supported literal string handlers and controller/action
arrays continue to work. Variables, computed array members, variable static
methods, and otherwise unresolved handlers produce no Laravel framework fact.

This is fail-closed framework extraction. The generic PHP extractor may still
record the underlying call and symbols.

### Laravel data flow

```text
PHP syntax tree
    |
    +--> one import-table traversal
    |
    +--> scoped-call traversal
             |
             +--> resolve receiver to exact facade identity
             |
             +--> validate static route shape
             |
             +--> normalize route and handler
             |
             `--> RawFrameworkFact
```

Prefix/group discovery will use the same resolved receiver identity. An
unrelated `Route::prefix(...)` cannot affect a valid or invalid route.

## Delivery 2: shared activation evidence

### Internal model

`frameworks/evidence.rs` will own internal activation types. They are not
serialized into the graph:

```text
ActivationEvidence
  framework
  kind
  canonical_identity
  local_identity
  anchor
  strength

EvidenceKind
  Manifest
  Import
  Receiver
  DecoratorOrAttribute
  Macro
  ConfigurationContract
  Convention

EvidenceStrength
  Direct
  Supporting
```

An `EvidenceSet` is scoped to one source file or declarative artifact and is
immutable after collection. A pack's `ActivationPolicy` declares the direct
evidence combinations required for each fact family. The evaluator returns an
activation decision plus the matched evidence for internal diagnostics.

The initial public `RawFrameworkFact` and provenance structures do not change.

### Evidence rules

Direct evidence proves a framework-owned construct:

- an import or namespace resolves to a framework package;
- a call receiver was constructed from or resolves to that package;
- a decorator, attribute, annotation, or macro resolves to the framework;
- a manifest explicitly declares the framework dependency; or
- a file satisfies a framework-owned declarative configuration contract.

Supporting evidence narrows context but does not prove ownership:

- generic directories such as `routes`, `controllers`, or `models`;
- conventional filenames without an exact framework artifact contract;
- framework-like method names; and
- capitalization or suffix conventions.

Declarative artifacts are distinct from generic path conventions. For example,
Play's `conf/routes` grammar and Drupal's `*.routing.yml` schema are direct
configuration-contract evidence because their parsers require the
framework-owned record shape. A directory merely named `routes` is supporting
evidence.

File-system routers such as Next.js and Nuxt require both their exact route
location contract and project/package evidence identifying the framework.
This prevents an arbitrary `pages` directory from activating a framework.

### Collection and evaluation

Framework detection receives a `FrameworkDetectionContext` containing the
path, language, source, syntax root when available, and project evidence when
the extraction caller has it. Language-aware collectors populate one
`EvidenceSet`; packs query it instead of rescanning source text independently.

The control flow becomes:

```text
source/artifact + project evidence
              |
              v
     language-aware collector
              |
              v
        immutable EvidenceSet
              |
       +------+-------------------+
       |                          |
 activation policy          construct parser
       |                          |
       +------------+-------------+
                    |
          emit only when both pass
```

Activation answers “does this construct belong to the framework?” Construct
parsing answers “what route or domain fact does it declare?” Keeping these
questions separate prevents a method name or path from silently becoming
framework identity.

### Pack migration

Every module invoked by `frameworks/mod.rs` will consume the shared mechanism:

- PHP: Laravel and Drupal;
- Python: Django, Flask, and FastAPI;
- Ruby: Rails;
- Java/Kotlin: Spring;
- Go: existing HTTP framework detectors;
- Rust: Axum, Actix, and Rocket;
- C#: ASP.NET;
- Swift: Vapor;
- JavaScript/TypeScript/TSX: Express, NestJS, React Router, Vue Router, and
  file-system routing;
- declarative packs: Play and Drupal routing configuration; and
- enterprise domain-fact detectors.

Migration is fact-family-specific. A file may activate one framework route
family without activating unrelated ORM, messaging, or job facts. The
enterprise detector therefore declares separate policies for each domain
family instead of using one broad “framework present” switch.

The migration removes equivalent pack-local activation scans after their
policies are covered. Framework-specific construct parsing stays in the
existing pack files.

## Performance design

The Laravel delivery performs one import traversal and one route-call
traversal. It does not run one import lookup per call or construct regular
expressions from source-controlled names.

The shared delivery collects evidence once and passes immutable references to
policies and parsers. Import maps and project manifest evidence use normalized
hash-map or set lookups. Pack migration must remove redundant
`body.contains(...)` activation scans when the same identity is available in
the evidence set.

Performance acceptance is:

- asymptotically linear collection in syntax-tree nodes plus emitted facts;
- no per-call whole-file scan;
- the existing framework fact limit remains enforced; and
- the repository's framework resolution scale/performance tests remain within
  their current ceilings.

Focused benchmarks will compare pre-change and post-change extraction for a
route-heavy corpus. Any measurable regression beyond ordinary benchmark noise
must be explained before the shared migration is accepted.

## Error and fallback behavior

- Invalid UTF-8 follows the existing lossy/empty-source behavior and must not
  panic.
- A malformed or incomplete import produces no direct identity evidence.
- An unsupported dynamic construct produces no exact framework fact.
- Syntax recovery nodes may contribute facts only when receiver identity and
  all required static fields remain unambiguous.
- Missing project evidence disables framework facts whose policy requires it;
  it does not disable generic extraction.
- Evidence collection respects existing per-file fact limits and uses
  saturating or checked position conversion consistent with current packs.
- Failure in one pack's policy does not activate a fallback heuristic.

## Testing strategy

### Laravel regression tests

Tests will be added before implementation and must initially fail for the
current detector. Fixtures cover:

- the canonical Laravel facade import;
- an aliased Laravel facade import;
- the fully qualified facade receiver;
- a wrong `Acme\Routing\Route` import inside a `routes` directory;
- an unimported `Route` inside a `routes` directory;
- a local class named `Route`;
- a dynamic handler;
- a variable static method;
- supported string and controller/action-array handlers;
- `match`, `resource`, and prefix/group behavior; and
- an unrelated prefix receiver.

Negative cases assert the absence of Laravel facts, not the absence of generic
PHP nodes or calls.

### Shared-mechanism tests

The evidence module receives table-driven unit tests for:

- direct versus supporting evidence;
- all-of and any-of policy clauses;
- canonical identity matching;
- framework and fact-family isolation; and
- deterministic evidence ordering.

Each existing pack gains or retains:

- at least one positive exact-evidence fixture;
- an alias or equivalent identity-resolution fixture where the language
  supports aliases;
- one wrong-framework near match;
- one missing-evidence near match; and
- static-shape negatives for facts that claim exact targets.

Cross-pack tests verify that evidence for one framework cannot activate
another and that generic path conventions do not activate code-driven packs.
Declarative and file-system routing tests verify their explicit configuration
and project-evidence policies.

Existing integration, serialization, limit, and scale tests remain green.

## Delivery and review sequence

1. Add failing Laravel adversarial tests.
2. Implement PHP import/alias and scoped-call receiver resolution.
3. Verify Laravel positives, negatives, integration tests, and focused
   extraction performance.
4. Commit the Laravel fix as an independently reviewable change.
5. Add the internal evidence model and policy unit tests.
6. Introduce the detection context and project-evidence input.
7. Migrate packs in language-sized commits, removing superseded activation
   scans as each pack moves.
8. Run the full Compass test suite and framework scale/performance checks.
9. Run `graphify update .` from the outer repository after all code changes.

## Acceptance criteria

The work is complete when:

- the Laravel false positives described above are regression-tested and
  eliminated;
- valid canonical, aliased, and fully qualified Laravel routes still resolve;
- every framework fact emitted through `frameworks/mod.rs` has passed a shared
  activation policy;
- no code-driven pack activates from path convention alone;
- framework-owned declarative artifacts use explicit configuration contracts;
- file-system routers require project framework evidence;
- the public framework-fact schema is unchanged;
- relevant unit, integration, limit, and performance tests pass; and
- `graphify update .` completes successfully.
