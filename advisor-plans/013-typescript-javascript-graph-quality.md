# Plan 013: Make TypeScript and JavaScript code graphs best in class

> **Executor instructions**: Treat this as a multi-PR quality program, not one
> large feature branch. Execute the phases in order and stop at every gate.
> Production must never publish the legacy and universal TypeScript/JavaScript
> paths together. Graphify and other competitors are diagnostic comparators,
> never product, test, fixture, runtime, or fallback dependencies. Update this
> plan and `advisor-plans/README.md` only after the corresponding evidence has
> landed and passed review.
>
> **Drift check (run before each phase)**:
> `git diff --stat a8a6a80aa7547f89cae97f127b7bd44512bcfaee..HEAD -- vendor/compass-tree-sitter-language-pack crates/compass-files crates/compass-languages crates/compass-resolve crates/compass-graph crates/compass-program crates/compass-core crates/compass-cli tests/qualification benchmarks/performance scripts docs PERFORMANCE.md COMPATIBILITY.md MIGRATION.md CHANGELOG.md Cargo.toml Cargo.lock package.json package-lock.json`
> Reconcile renamed files, adapter versions, evidence schemas, and already
> shipped behavior before continuing. Do not mechanically apply stale line
> numbers.

## Status

- **Priority**: P1
- **Effort**: XL, delivered as independently reviewable L/M phases
- **Risk**: HIGH
- **Depends on**: no implementation prerequisite; the final release gate should
  consume plan 005 or an equivalent exact-commit qualification gate
- **Category**: language architecture, correctness, resolution, qualification,
  performance, documentation
- **Planned at**: commit `a8a6a80aa7547f89cae97f127b7bd44512bcfaee`,
  2026-08-05

### Execution checkpoint (2026-08-05)

The first implementation slice is intentionally below the phase-5 production
cutover. It establishes independent measurement and removes one known
precision hazard while the larger universal-adapter program remains gated:

- Phase 0 now has a pinned TypeScript 5.9.3 compiler-API source oracle with
  bounded Node/Python bridging, deterministic UTF-8 byte ranges, malformed-file
  rejection, coverage validation, and Unicode regression tests. Repository-scale
  validation currently parses 307/307 admitted TS/JS files and emits 167,923
  bounded constructs.
- The Python qualification harness validates oracle schema/provider/compiler
  metadata, exact source ranges, incomplete coverage, deterministic digests,
  and preserves the existing non-TypeScript fixture contract. Its focused
  suite currently passes 146 tests.
- JSONC parsing is shared by project evidence and the resolver. A bounded,
  fail-closed subset accepts comments and trailing commas without evaluating
  JavaScript.
- The native JavaScript/TypeScript extractor now emits exact `references` for
  uniquely proven callback values passed through collections/arguments; it no
  longer labels an unknown receiving API as an `indirect_call`.
- The resolver now has a bounded `baseUrl`/`paths` slice for nearest
  `tsconfig*.json`/`jsconfig*.json`: JSONC, ordered fallback targets,
  JS-to-TS/JSX/MTS/CTS extension substitution, directory indexes, alias
  provenance, and same-depth config ambiguity are covered by resolver tests.
- The native fixture qualification passes with clean/warm/rebuild/relocated
  graph byte equality, and the workspace `--lib --bins` test and lint baselines
  pass. Existing qualification warnings about intentionally omitted partial
  fixture records remain unchanged and are not TypeScript/JavaScript failures.

These changes do not register TypeScript/JavaScript universal adapters, do not
invoke Node or TypeScript during normal Compass builds, and do not claim that
the full standards-correct module model or leadership gates are complete.

### Execution checkpoint (2026-08-06)

The next implementation slice strengthens project ownership, module identity,
and source-grounded interop while keeping the production universal cutover
closed:

- Recursive TypeScript configuration loading now accepts bounded `extends`
  chains (including package-style parents), preserves per-target path bases,
  carries `rootDirs`, `moduleSuffixes`, `allowJs`/`checkJs`, project references,
  module metadata, and reports cycles, missing parents, depth, and size limits
  as diagnostics. Referenced base configs are excluded from nearest-project
  selection, so an `extends`-only `tsconfig.base.json` cannot become a second
  same-depth project.
- Module resolution now chooses relative targets from the admitted inventory,
  including extension substitution, directory indexes, `moduleSuffixes`, and
  `rootDirs`; package manifests now handle importer-aware `exports` conditions,
  package `imports`, and bounded TypeScript `typesVersions` fallbacks. Ordered
  fallbacks remain ordered, mutually exclusive branches are not unioned, and
  duplicate/ambiguous packages remain unresolved.
- The native extractor records default, namespace, named, and type-only import
  bindings with exact anchors; emits JSX component occurrences as typed
  `references`; and publishes only source-grounded CommonJS `exports` edges for
  uniquely resolved local bindings. Unknown dynamic receivers still do not
  become calls.
- Project ownership now honors bounded `files`/`include`/`exclude` patterns,
  default dependency-tree exclusions, `typeRoots`, and ordered
  `customConditions`; excluded or ambiguous admitted files remain unresolved.
  Package condition selection consumes the selected project's custom condition
  order rather than applying a repository-wide guess.
- A qualification-only direct universal-evidence emitter now exists for the
  ECMAScript family. It emits source-grounded declarations, lexical scopes,
  value/type namespaces, imports/reexports, calls/constructors, base-type
  candidates, member access, decorators, JSX references, dynamic literal
  imports, and CommonJS exports. Adapter identity preserves parser dialect
  (`ts`, `tsx`, `js`, `jsx`, `mts`, `cts`, and related extensions) separately
  from semantic language.
- Regression coverage now includes same-directory config inheritance and
  cycles, relative named-import repointing, package `imports`, `exports`
  conditions, `typesVersions`, module suffixes/root directories, include /
  exclude ownership, `typeRoots`, custom conditions, import-kind metadata, JSX
  occurrences, CommonJS exports, and direct candidate evidence. Targeted
  language/resolver tests and resolver-wide tests pass; the full workspace
  baseline and independent oracle/fixture gates remain required before any
  phase exit.

This checkpoint is still below the Phase 2 universal-evidence and Phase 5
production-hard-cut gates. `include`/`exclude` ownership, complete Node16/
NodeNext/Bundler condition semantics, complete `typeRoots` package lookup,
compiler-trace differential qualification, broad syntax/capability coverage,
and direct TypeScript/JavaScript production adapter registration remain
planned work. The candidate emitter is deliberately not in
`UNIVERSAL_ADAPTERS`.

### Execution checkpoint (2026-08-06, identity slice)

The following Phase 1/2 contract slice is now source-grounded and validated:

- Universal evidence has an optional `SymbolNamespace` field on declarations
  and bindings (`value`, `type`, `namespace`, `value_and_type`). Existing
  adapters continue to omit it, and their stable IDs remain byte-for-byte
  compatible because the identity component is included only when explicitly
  emitted.
- Import and re-export bindings can carry `type_only`; validation rejects the
  marker on unrelated binding kinds and rejects a type-only value-space claim.
  `import type`, `export type`, namespace imports, normal dual-space imports,
  CommonJS bindings, and local re-exports are covered by direct candidate
  evidence tests.
- Candidate adapter versions advanced to 2 for the new semantic identity
  contract. The shared evidence schema major remains `/1`; fields are optional
  and omitted by legacy producers. The candidate path remains qualification
  only and is not registered in `UNIVERSAL_ADAPTERS`.
- Contract documentation now describes symbol-space identity and the
  validation rule. Focused language tests cover serialization/validation,
  legacy omission, deterministic IDs, and TS/JS identity assertions.
- The candidate traversal now preserves function/method parameter declarations
  in their lexical scope and gives anonymous/named `export default` forms a
  source-anchored `default` declaration/re-export instead of dropping the
  export or guessing a parameter as a function name.

This does not close syntax conformance, overload/signature identity, complete
Node16/NodeNext/Bundler resolution, compiler differential qualification, or the
production hard cut. Those remain explicit gates rather than inferred from the
candidate test fixture.

### Execution checkpoint (2026-08-06, module-mode and resolver-oracle slice)

The resolver now makes one important standards boundary explicit while keeping
the production universal cutover gated:

- Conditional `package.json` export objects are evaluated in manifest key order
  after the active condition set is computed. Compass no longer reorders a
  package author's `default`, `import`, `require`, `types`, or custom branches
  according to an internal preference list; the ordered fallback and
  ambiguity behavior remains source-inventory bounded.
- Explicit `moduleResolution` is honored for package maps. Node16, NodeNext,
  and Bundler may use `exports`/`imports`; Node10 falls back to legacy package
  fields and package subpaths; Classic leaves package-name imports unresolved
  unless the separate `paths` pass proves an alias. The selected rule and
  explicit mode are retained on the occurrence, and the older flattened
  workspace pass cannot overwrite a mode-specific decision.
- A qualification-only `typescript-resolution-oracle.mjs` now uses the pinned
  TypeScript 5.9.3 compiler API to resolve imports, exports, `import type`,
  `import =`, dynamic imports, and literal `require()` sites. It emits exact
  UTF-8 ranges, project ownership, module mode, target source/external/
  unresolved/ambiguous status, configuration/source digests, diagnostics, and
  bounded deterministic output. A focused test runs the oracle twice and
  cross-checks representative decisions against `tsc --traceResolution`.
- The independent benchmark suite now passes 147 tests. This is a measurement
  and resolver-correctness slice, not evidence that every advertised compiler
  mode or capability has reached the Phase 3 precision/recall gates.

### Execution checkpoint (2026-08-06, conservative receiver/member slice)

The qualification-only universal emitter now closes a precision hazard in
member attribution without widening the production cutover:

- Scope ownership is keyed by tree-sitter node identity rather than only the
  start byte. A root-level declaration beginning at byte zero can no longer
  shadow the module scope during receiver/type inference.
- Import and re-export bindings are pre-collected before semantic uses are
  emitted, matching ECMAScript import hoisting and making receiver inference
  independent of source order.
- Simple nominal receivers are tracked only when source evidence proves them:
  class/enum/namespace values, `new Class()`, typed variables, `this`, and
  namespace/named imports. Unique `Class.member` declarations receive exact
  declaration targets; overloaded/declaration-merged qualified names remain
  unresolved rather than selecting a traversal-order winner.
- Member calls no longer fall back to an unrelated same-named top-level
  function. Unknown/dynamic receivers remain unresolved. Imported namespace
  members preserve the module-symbol target shape (`module::member`) and
  retain the import binding as provenance.
- Calls now carry a bounded source argument count. When same-spelled local
  functions or members have fixed, source-visible arities, a unique arity
  match receives the exact declaration target; optional/default/rest or
  otherwise ambiguous signatures remain unresolved.
- The direct candidate fixture now covers `this`, `new`-initialized locals,
  exact method/field targets, dynamic-member negatives, namespace-import calls,
  and fixed-arity overloads. The focused candidate suite passes five tests after
  the change.

This remains a Phase 2/4 evidence slice: it does not claim flow-sensitive
assignment tracking, overload selection, hierarchy dispatch, compiler
differential recall, or production universal registration.

### Execution checkpoint (2026-08-06, JSX/super and typed-call slice)

The qualification-only emitter now adds two conservative attribution paths and
records source-proven call-shape evidence without widening the production cut:

- JSX member tags such as `<UI.Button />` use the nominal member resolver. A
  namespace import is published as `module::Button` with the import binding as
  provenance and the `Button` property anchor; dynamic or unresolved JSX
  receivers still produce no invented target.
- Import and re-export statements publish both the module-specifier occurrence
  and each exact binding occurrence, preserving compiler-oracle coverage for
  module identity and local alias anchors.
- Dot and literal computed members (`obj["field"]`) share the exact nominal
  receiver path; dynamic computed keys (`obj[key]`) are rejected before target
  publication rather than being treated as a same-spelled property.
- `super.member()` and `super.member` resolve only through one source-proven
  direct `extends` target. Ambiguous, malformed, or unresolved heritage does
  not create a base receiver, and imported bases retain their external module
  identity.
- Fixed class constructor arity is inferred from an explicit constructor (or
  zero when none exists). Calls with a proven incompatible fixed arity no
  longer select the declaration; optional/default/rest and incomplete
  signatures remain unresolved.
- Constructor declarations are published with a distinct `constructor` kind,
  preserving their source anchor while class construction remains targeted at
  the owning class declaration.
- Literal call arguments carry bounded `argument_types` evidence. When
  same-arity overloads have source-visible simple parameter types, a unique
  string/number/boolean/object/function or nominal match receives the exact
  declaration target. Type-mismatched or still-ambiguous overloads remain
  unresolved.
- The direct candidate suite still passes five tests and now covers namespace
  JSX members, `super` dispatch, fixed-arity negatives, and typed overload
  selection. The candidate adapter remains unregistered; compiler differential
  qualification, broad syntax coverage, and release scorecard gates are still
  required.
- Tree-sitter's dedicated `this` node is now treated as a nominal receiver,
  and private member identifiers (for example `this.#run()`) retain exact
  declaration targets. The candidate regression fixture covers both the
  receiver and private-member anchors; this closes a previously silent
  under-attribution case without guessing dynamic properties.
- An ignored developer-only differential test now runs the pinned TypeScript
  5.9.3 source oracle against a mixed TSX fixture and asserts byte-range
  coverage for declarations, imports, calls, construction, members, heritage,
  and JSX. The normal native test suite remains Node-free; qualification runs
  it explicitly with `-- --ignored`.

### Execution checkpoint (2026-08-06, unresolved-evidence and corpus slice)

The candidate path now preserves source constructs even when it cannot prove a
target, without weakening the graph resolver:

- Dynamic/ambiguous calls, nominally unknown members, unresolved type
  references, and unresolved heritage names emit occurrence-backed,
  target-less candidates. Their constraints omit lexical/qualified fallback
  and `allow_external`, so the shared resolver leaves them unresolved instead
  of inventing an external node or selecting a same-spelled declaration.
- The former local-variable `object.property` external fallback was removed.
  Namespace-import members retain `module::member` identity, while imported
  nominal types and `super` members retain `module::Type.member` identity.
  Qualified imported `extends` anchors and `super` calls have direct positive
  and negative coverage.
- Builtin-member evidence now uses receiver-specific static/instance
  allowlists. For example, `Array.from`, `new Map().get`, and `Set.add`
  remain supported while `new ArrayBuffer().isView` and
  `new WeakMap().values` remain unsupported; shadowed globals still cannot
  create external edges.
- Unparenthesized arrow parameters (`value => value`) are now emitted as
  parameter declarations with exact source anchors and fixed one-argument
  arity. This closes a common JavaScript/TypeScript callback shape without
  broadening dynamic-call inference.
- The shared resolver is covered by a regression proving that a target-less
  dynamic member and an arity-mismatched direct call stay `Unresolved`, while a
  source-proven nominal call resolves exactly.
- The pinned TypeScript 5.9.3 source oracle (script SHA-256
  `ac3c07e825505a42479e3dfa6024860cfbc0cdf2ffc6b8f1756e43727f20fb98`) now
  excludes the compiler API's pseudo-`const` type reference under `as const`.
  It parses all admitted files in both current release-gate corpora. On Zod
  commit `912f0f51b0ced654d0069741e7160834dca742ee`, 409/409 files parsed and
  the candidate covers 104,171/104,253 accepted source constructs (99.92%). On
  Axios commit `c3f553c740ebf3dff5e22dae24e9caaafafddd2d`, 236/236 files parsed
  and coverage is 40,522/40,522 (100.00%). These are source-occurrence recall
  measurements, not precision or target-identity results.
- The resolution oracle (compiler 5.9.3, script SHA-256
  `3d68c689338034c51b91c2f7863e01caa7e42898851b18916a32d3f09dafe500`) emits
  deterministic project/module decisions for 1,107 Zod and 698 Axios import
  sites. Zod has 548 source and 559 unresolved outcomes across 19 projects;
  Axios has 301 source and 397 unresolved outcomes in one project. The one
  Zod diagnostic is retained for adjudication; neither corpus is yet a
  precision-qualified release scorecard.

This closes a high-value evidence-loss and false-external-edge slice, but does
not close the mandatory precision/Wilson gates, per-capability accepted-sample
minimums, compiler target adjudication, Graphify/SCIP equivalent-scope audit,
framework matrix, performance gate, or production hard cut. The candidate
adapter remains unregistered in `UNIVERSAL_ADAPTERS`.

### Execution checkpoint (2026-08-06, target-adjudication and JavaScript-shape slice)

The candidate now has a separate checker-backed target oracle and several
same-file identity improvements. They remain qualification-only until the
accepted-sample and precision gates are frozen:

- `typescript-target-oracle.mjs` (schema
  `compass.typescript-target-oracle/1`, provider `typescript_checker_api_5_9_3`,
  script SHA-256
  `9f992c678d32c7083e69b104785fdf3c91e77bbb0e65bcb8b4543c39a23ba41a`) uses a
  bounded synthetic TypeScript 5.9.3 program to adjudicate source declaration
  targets for calls, construction, members, heritage, type references, and
  JSX. The ignored Rust harness reports exact local-target recall, wrong
  targets, unresolved outcomes, and local false positives by capability; it
  does not read Compass or Graphify output.
- JavaScript direct-call matching now respects JavaScript's omitted/extra
  argument semantics while preserving ambiguity as unresolved. Class-member
  scopes no longer shadow lexical helpers, so a same-spelled static member
  cannot steal an unqualified helper call.
- Object-literal `pair` declarations, object-variable receivers, assignment
  properties, and anonymous TypeScript signature scopes are source-anchored.
  This improves structural property targets and keeps repeated generic type
  parameters isolated instead of collapsing them into one interface scope.
- Dynamic/parenthesized/computed callees remain targetless when proof is not
  available; JavaScript `class_heritage` and TypeScript `import = require()`
  now retain exact module/base evidence. The candidate still is not registered
  in `UNIVERSAL_ADAPTERS`.
- Initial target adjudication is intentionally a failing diagnostic, not a
  release gate. On Axios, the candidate hit 2,923/3,167 same-file source
  targets (92.30% local recall) with 17 wrong-target cases and 564 local
  positives not matching the oracle outcome. On Zod it hit 6,717/9,883
  (67.96%) with 25 wrong-target cases and 1,314 local positives not matching
  the oracle. The
  largest gaps are project/flow-sensitive member receivers, function-valued
  variables, and framework/library declaration identity; these numbers are
  precisely why the target, precision, and cross-file gates remain open.

The target oracle currently qualifies as an adjudication instrument, not a
release claim: project-aware module identity, cross-file target precision,
framework strata, accepted-sample Wilson intervals, Graphify/SCIP equivalent
scope, performance, and production hard-cut gates remain open.

### Execution checkpoint (2026-08-06, scope/flow/type/call quality slice)

The qualification-only emitter now covers several high-impact target gaps
without widening the production adapter registry:

- Lexical block, switch, catch, and loop scopes are explicit; `var` bindings
  hoist to the nearest function/module scope while lexical bindings remain
  block-local. Class methods, constructors, and properties are excluded from
  unqualified helper lookup, preventing member names from shadowing real
  lexical calls.
- Structural object receivers retain source order. When a JavaScript object is
  reassigned, member resolution chooses the latest preceding source-backed
  property declaration for that receiver; nominal class/interface members
  continue to use scope and ambiguity proof rather than source-order guesses.
- Conditional `infer` bindings and mapped/object-type keys have dedicated
  type scopes. Repeated `K` keys, `infer Intersection`, and `infer Last` now
  resolve to their exact source declarations instead of collapsing into one
  alias scope. Interface/namespace declaration merging is handled as one
  type-space nominal for qualified member lookup.
- Callable-shaped parameters and aliases (`factory: (...) => T`,
  `typeof Ctor`, `SchemaClass<T>`, and constructor-valued locals) are retained
  as call/construction targets even when no local arity is available. Classes
  without an explicit constructor receive their source-language default
  constructor behavior. Constructor parameter properties resolve through
  inherited `this` members, and TypeScript heritage expressions can resolve a
  source-proven value-space constructor such as a mixin `Parent` variable.
- Inherited member lookup follows a proven local `extends` chain and checks
  both class members and constructor parameter-property identities. New direct
  regressions cover mapped/conditional type scopes and inherited parameter
  properties in `crates/compass-languages/tests/`.

The pinned source-oracle occurrence coverage remains 104,171/104,253 (99.92%)
for Zod and 40,522/40,522 (100.00%) for Axios. The latest checker target
adjudication is:

| Corpus | Exact local targets | Expected local | Missing | Wrong | Local positives | False positives |
|---|---:|---:|---:|---:|---:|---:|
| Axios | 3,119 | 3,167 | 48 | 0 | 3,370 | 251 |
| Zod | 7,548 | 9,883 | 2,335 | 0 | 8,727 | 1,179 |

By capability, Zod now reaches 452/452 heritage targets and 5,198/5,198
type-reference targets; Axios reaches 1,067/1,067 type-reference targets and
1,514/1,545 direct call targets. These are same-file checker-oracle
adjudication measurements, not release precision: the candidate still has
unresolved project/flow/framework members and a non-zero local false-positive
count. Graphify, SCIP, Wilson intervals, accepted-sample minimums, framework
matrix, performance, and the production hard cut remain open gates. The
candidate emitter is still deliberately absent from `UNIVERSAL_ADAPTERS`.

### Verification checkpoint (2026-08-06, repository gates)

The implementation and qualification changes were rechecked against the
repository-wide gates from the command table:

- `scripts/check_product_boundary.sh` passed with no Graphify/runtime boundary
  findings.
- `./scripts/qualify_code_graph_v1.sh --fixtures-only` passed. The exact
  production fixture graph remained byte-equivalent across clean, warm,
  rebuild, and relocated runs (`sha256:f13848fefa81b79c70ed9b50081c1cab8024f1ce84030064c8eb2d154ba4c160`),
  with 1,565 semantic invariants and 980 coverage records.
- `cargo fmt --all -- --check`, workspace clippy, workspace library/binary
  tests, focused universal evidence/resolution tests, and the independent
  Python qualification suite (147 tests) passed.
- `npm run typecheck:js`, `npm run test:js`, and
  `node scripts/check_viewer_assets.mjs` passed (142 viewer tests, 117 VS Code
  tests, and 76 browser tests). The generated viewer assets matched their
  deterministic manifest.
- `git diff --check` passed and no generated graph, `.compass` state, or
  qualification corpus was added to the worktree.

These are repository and candidate-quality checkpoints only. The mandatory
Plan 013 scorecard is still open: the current same-file target diagnostics
remain below the precision/recall thresholds, accepted-sample/Wilson evidence
is not frozen, and Graphify/SCIP equivalent-scope, framework, performance,
compiler-tier, and production hard-cut gates have not been run. Keep the
candidate adapter out of `UNIVERSAL_ADAPTERS` until those gates pass.

### Execution checkpoint (2026-08-06, typed-call and boundedness follow-up)

The retained follow-up is deliberately narrower than the attempted
destructured-flow prototype:

- Source-compatible call arguments now preserve exact same-file targets for
  object literals, arrays whose element type is source-proven, and unqualified
  versus qualified nominal type spellings. Primitive, callable, qualified
  conflicts, optional/rest, and ambiguous signatures still fail closed. The
  direct candidate regression suite has 16 passing tests.
- `this` is recognized as a dedicated tree-sitter callee for source-proven
  class construction/calls, while dynamic member receivers remain unresolved.
  The ignored target harness uses a fresh parser engine per large corpus file
  to keep the qualification process bounded against the vendored grammar's
  deep-stack behavior; production parser/cache behavior is unchanged.
- A bounded shallow destructured inline-object propagation slice now stores
  only capped, string-only property types and handles
  `props: { data: T }` / `const { data: local } = props`. Nested patterns,
  spreads, computed keys, and reassignment flow remain unresolved. The direct
  regression passes (16 candidate tests total), and the large Zod run stayed
  bounded with no wrong target.

The stable checker adjudication remains zero wrong local targets:

| Corpus | Exact local targets | Expected local | Missing | Wrong | Local positives | False positives |
|---|---:|---:|---:|---:|---:|---:|
| Axios | 3,152 | 3,167 | 15 | 0 | 3,403 | 251 |
| Zod | 7,764 | 9,883 | 2,119 | 0 | 9,016 | 1,252 |

Zod now reaches 845/2,695 member targets (up from 840), 452/452 heritage,
201/201 construction, 12/12 JSX, and
5,198/5,198 type-reference targets; Axios reaches 1,544/1,545 direct calls,
489/503 members, 39/39 construction, and 1,067/1,067 type references. These
are diagnostic same-file results, not release precision: project-aware flow,
destructuring, framework strata, accepted-sample Wilson intervals, Graphify/
SCIP equivalent-scope comparison, performance, and the production hard cut
remain open.

### Execution checkpoint (2026-08-06, structural utility/generic call slice)

The next bounded resolution slice is now implemented and regression-tested:

- Utility-wrapped parameters (`Pick`, `Omit`, `Partial`, `Required`, and
  `Readonly`) compare only source-proven property sets. A `Pick<T, K>` call is
  accepted when the argument's unique source type contains the selected keys;
  unrelated nominal types remain unresolved. Inline object parameter shapes
  compare required properties against source-emitted members, including array
  element object shapes. Optional keys, computed/index signatures, spreads,
  and unknown structural aliases remain conservative.
- Generic callable parameters retain only a bounded side table of direct
  `extends` constraints. A constrained array generic such as
  `U extends [T, ...T[]]` now accepts a source-proven array; primitive and
  incompatible constraints remain rejected. The metadata is kept outside the
  recursive declaration frame, preserving default-stack behavior on large
  files and preventing a large-Zod stack overflow.
- The direct candidate suite has 18 passing tests, including a constrained
  generic call regression. The target harness remains developer-only and
  ignored; no candidate adapter was added to `UNIVERSAL_ADAPTERS`.

Updated pinned checker adjudication (oracle SHA unchanged) remains zero wrong
local targets:

| Corpus | Exact local targets | Expected local | Missing | Wrong | Local positives | False positives |
|---|---:|---:|---:|---:|---:|---:|
| Axios | 3,152 | 3,167 | 15 | 0 | 3,403 | 251 |
| Zod | 8,009 | 9,883 | 1,874 | 0 | 9,263 | 1,254 |

The Zod call stratum is now 1,063/1,325 (up from 1,057 in the prior
checkpoint); members remain 1,083/2,695, heritage 452/452, construction
201/201, JSX 12/12, and type references 5,198/5,198. Axios remains
1,544/1,545 direct calls, 489/503 members, 39/39 construction, and 1,067/1,067
type references. These are diagnostic same-file results, not a release or
leadership claim: precision/recall thresholds, accepted-sample Wilson bounds,
framework/compiler tiers, Graphify/SCIP equivalent-scope comparison,
performance, and the production hard cut remain open.

### Execution checkpoint (2026-08-06, JavaScript prototype and wrapped-object slice)

The next bounded JavaScript/TypeScript resolution slice is now implemented and
measured without widening the production adapter registry:

- Same-file function constructors now participate in nominal instance-member
  resolution only when source evidence proves constructor intent: a `new Ctor`
  use or an explicit/aliased `Ctor.prototype` write. `Ctor.prototype.method`
  function expressions, `const proto = Ctor.prototype` aliases, and
  `new Ctor().method` share one source-backed prototype identity. Constructor
  assignments such as `this._pairs = []` become the canonical instance-field
  declaration, and prototype-method `this._pairs` reads can target that
  declaration.
- Prototype instance writes through `this` are deliberately not published as
  prototype members; dynamic/computed keys, unknown prototype receivers, and
  unproven function `this` values remain unresolved. This keeps the new
  attribution path fail-closed even when a JavaScript checker cannot prove a
  target for a prototype call.
- Structural object receivers now unwrap bounded runtime-neutral TypeScript
  wrappers (`as const`, `satisfies`, parenthesized expressions, and type
  assertions). Members of `const values = { ... } as const` therefore retain
  exact property declaration anchors while arbitrary dynamic objects remain
  unsupported. A direct regression covers the `as const` path alongside the
  prototype constructor/alias and negative dynamic-member cases.
- The focused candidate suite has 19 passing tests. Pinned source coverage is
  unchanged at 104,171/104,253 (99.92%) for Zod and 40,522/40,522 (100.00%)
  for Axios because this slice improves target identity rather than source
  occurrence discovery.

The current checker-target adjudication (oracle SHA unchanged) is:

| Corpus | Exact local targets | Expected local | Missing | Wrong | Local positives | False positives |
|---|---:|---:|---:|---:|---:|---:|
| Axios | 3,153 | 3,167 | 14 | 0 | 3,408 | 255 |
| Zod | 8,013 | 9,883 | 1,870 | 0 | 9,267 | 1,254 |

Zod now reaches 1,087/2,695 members, 1,063/1,325 calls, 452/452 heritage,
201/201 construction, 12/12 JSX, and 5,198/5,198 type references. Axios
reaches 490/503 members, 1,544/1,545 calls, 13/13 heritage, 39/39
construction, and 1,067/1,067 type references. Some newly source-proven
prototype members are counted as checker false positives because the pinned
checker oracle reports those JavaScript accesses as unresolved; they require
accepted-sample/manual adjudication before a precision claim. These remain
diagnostic same-file results, not a release or leadership claim. Precision and
recall thresholds, Wilson bounds, framework/compiler tiers, Graphify/SCIP
equivalent-scope comparison, performance, and the production hard cut remain
open, and the candidate emitter stays absent from `UNIVERSAL_ADAPTERS`.

### Execution checkpoint (2026-08-06, interface inheritance and optional-callable slice)

The next source-proven TypeScript slice is implemented and measured without
registering the candidate as a production universal adapter:

- Interface declarations now participate in the bounded `extends` hierarchy
  index used by member lookup. A member inherited through one local interface
  edge can therefore retain the exact base-interface declaration anchor;
  multiple/ambiguous or imported-only paths still fail closed.
- Optional callable properties preserve one nominal type from a property-only
  union such as `Callback | undefined` when every other union arm is a
  primitive/top/literal. The call resolver uses that declaration metadata only
  for a unique property target, including the inherited-member path; unions
  with multiple nominal arms, variables, aliases, and dynamic receivers remain
  unresolved.
- The candidate regression suite now has 33 passing tests, including direct
  interface inheritance and inherited optional-callable property calls.

Pinned checker adjudication remains zero-wrong local target resolution:

| Corpus | Exact local targets | Expected local | Missing | Wrong | Local positives | False positives |
|---|---:|---:|---:|---:|---:|---:|
| Axios | 3,153 | 3,167 | 14 | 0 | 3,408 | 255 |
| Zod | 8,359 | 9,883 | 1,524 | 0 | 9,617 | 1,258 |

Zod strata are members 1,335/2,695, calls 1,161/1,325, heritage 452/452,
construction 201/201, JSX 12/12, and type references 5,198/5,198. Axios is
members 490/503, calls 1,544/1,545, heritage 13/13, construction 39/39, and
type references 1,067/1,067. These are qualification diagnostics: the
candidate emits unresolved entries alongside some local positives, so the
false-positive column is not an accepted precision estimate.

Independent source-occurrence coverage remains 104,171/104,253 (99.92%) for
Zod and 40,522/40,522 (100.00%) for Axios. Zod's source differential requires
`RUST_MIN_STACK=33554432` because the large corpus can overflow the default
test stack; this is a developer diagnostic setting, not a production runtime
requirement. The real-corpus target harness uses the same isolated-engine
qualification setup and remains ignored by default.

The next actionable phases are deliberately still open:

1. **Project target closure:** add a read-only project index for imported
   aliases, package exports, declaration merging, generic instantiation, and
   mapped/indexed shapes; preserve explicit unresolved/ambiguous results and
   measure each stratum on Zod, Axios, and a third framework-heavy corpus.
2. **Accepted precision gate:** hand-label a stratified sample of local,
   external, unresolved, dynamic, and ambiguous calls/members; compute Wilson
   intervals and set release thresholds before enabling any adapter profile.
3. **Framework/compiler tiers:** qualify React/JSX, Node, Express, Jest,
   Next/Vite, decorators, `const enum`, namespaces, and modern TS 5.x syntax;
   compare tree-sitter-only, optional compiler-oracle, and Graphify/SCIP
   equivalent scopes with pinned versions and reproducible commands.
4. **Performance and publication hard cut:** benchmark cold/incremental runs,
   memory, cancellation, and bounded failure behavior; register the candidate
   only after source recall, target precision, determinism, compatibility, and
   product-boundary checks pass together.

### Execution checkpoint (2026-08-06, callable/generic-flow and inline-anchor slices)

The candidate-only implementation now covers several additional source-proven
paths while remaining deliberately absent from `UNIVERSAL_ADAPTERS`:

- Callable property signatures such as `getter: () => T` preserve their direct
  return type, including generic constraints, so a chain like
  `this._def.getter()._parse()` reaches the exact constrained declaration.
- Generic member receivers retain bounded type arguments through nested member
  chains and explicit type assertions. Assertions to `any`, `unknown`, and
  `never` still erase the receiver rather than recovering a guessed target.
- String-literal discriminants (`value.kind === "ready"`) narrow only a direct
  positive branch and only when one union constituent owns the matching literal
  property. Nullable logical wrappers (`member || null`/`member ?? null`) are
  unwrapped for the same source-proven flow; negative, ambiguous, and dynamic
  cases remain unresolved.
- Inline object-type indexes now retain every declared property, even primitive
  or union annotations. The exact object-type byte range wins over duplicate
  object-literal spellings, which closes real-world precision accesses such as
  Zod's `options?.precision`.
- Static callable aliases and local variable aliases preserve callable identity
  and declared return types, including overloaded local factories. A unique
  declared method wins over constructor-bound property duplicates only when all
  competing matches are those bound properties; unrelated duplicates remain
  ambiguous.

The focused candidate regression suite now has 43 passing tests. The pinned
checker target adjudication (oracle SHA and corpus realization unchanged) is:

| Corpus | Exact local targets | Expected local | Missing | Wrong | Local positives | False positives |
|---|---:|---:|---:|---:|---:|---:|
| Axios | 3,153 | 3,167 | 14 | 0 | 3,408 | 255 |
| Zod | 8,432 | 9,883 | 1,451 | 0 | 9,690 | 1,258 |

Zod strata are members 1,372/2,695, calls 1,197/1,325, heritage 452/452,
construction 201/201, JSX 12/12, and type references 5,198/5,198. Axios is
members 490/503, calls 1,544/1,545, heritage 13/13, construction 39/39, and
type references 1,067/1,067. The independent source-occurrence differentials
remain 104,171/104,253 (99.92%) for Zod and 40,522/40,522 (100.00%) for Axios.
Both corpora still report zero wrong local targets; Zod's false-positive count
is unchanged from the prior accepted diagnostic baseline, and the target
harness now prints bounded examples for manual adjudication. These numbers are
not a precision claim: accepted-sample labels, Wilson bounds, compiler and
framework tiers, and equivalent Graphify/SCIP runs are still required.

The repository fixture qualification also passed with the explicit per-checkout
Cargo target directory. Clean, warm, rebuilt, restored, and alternate-checkout
production artifacts were byte-identical; the qualification summary reported
28 edge kinds, 45 node kinds, 57 languages, 27 flows, 980 coverage records, and
the stable graph digest
`sha256:f13848fefa81b79c70ed9b50081c1cab8024f1ce84030064c8eb2d154ba4c160`.
The focused 43-test candidate suite, package Clippy gate, formatting check,
product-boundary check, and 147-test benchmark suite pass. The broader
`compass-languages --tests` run still exposes the pre-existing registry
expectation mismatch (`Rust` reports 15 capabilities while that test expects
13); no registry change was made as part of this slice.

The next actionable phases remain open:

1. **Project target closure:** add a read-only project index for imported
   aliases, package exports, declaration merging, generic instantiation, and
   mapped/indexed shapes; preserve explicit unresolved/ambiguous results and
   measure each stratum on Zod, Axios, and a third framework-heavy corpus.
2. **Accepted precision gate:** hand-label a stratified sample of local,
   external, unresolved, dynamic, and ambiguous calls/members; compute Wilson
   intervals and set release thresholds before enabling any adapter profile.
3. **Framework/compiler tiers:** qualify React/JSX, Node, Express, Jest,
   Next/Vite, decorators, `const enum`, namespaces, and modern TS 5.x syntax;
   compare tree-sitter-only, optional compiler-oracle, and Graphify/SCIP
   equivalent scopes with pinned versions and reproducible commands.
4. **Performance and publication hard cut:** benchmark cold/incremental runs,
   memory, cancellation, and bounded failure behavior; register the candidate
   only after source recall, target precision, determinism, compatibility, and
   product-boundary checks pass together.

### Execution checkpoint (2026-08-06, target-report and scorecard slice)

The qualification path now preserves enough evidence to turn the checker
diagnostic into a reviewed, machine-checkable scorecard without treating the
checker as Compass runtime behavior:

- The ignored target differential accepts `COMPASS_TS_TARGET_REPORT` and
  atomically writes `compass.typescript-target-adjudication/1`. The report pins
  the checker metadata and script digest, records exact source/target byte
  ranges, preserves every candidate observation, emits automatic outcomes by
  capability, and is bounded to 128 MiB. Normal native tests and builds do not
  write it.
- `benchmarks/performance/compass/typescript_scorecard.py` validates the
  reviewed `compass.typescript-target-scorecard/1` contract. Every record must
  have an explicit `accepted`/`source_oracle` pool, a manually entered
  `judgmentSource: "manual"`, and a judgment (with a review reason for every
  non-correct label); missing labels, unsafe paths, duplicate or unsorted IDs,
  stale provider metadata, and undeclared strata fail closed. Automatic checker
  outcomes may be retained as context but can never become scorecard labels.
- The scorecard computes deterministic precision, 95% Wilson bounds, recall,
  target-cluster concentration, corpus/relation/capability strata, critical
  semantic violations, and separate production versus leadership thresholds.
  Leadership mode additionally requires equivalent-scope, adjudicated Graphify
  and SCIP TypeScript comparator entries. Diagnostic mode cannot be eligible
  for a public quality claim.
- `python3 benchmarks/performance/harness.py typescript-scorecard` exposes the
  evaluator as a reproducible developer command. A synthetic one-file target
  report was generated successfully with exact observations; no real-corpus
  labels or competitor result has been promoted into the scorecard.

This closes the measurement contract but not the quality gate. The four-corpus
release sample, hand-labeled accepted records, Graphify/SCIP equivalent-scope
results, framework strata, and production hard cut remain open. The candidate
adapter remains absent from `UNIVERSAL_ADAPTERS`.

### Execution checkpoint (2026-08-06, universal module-index and re-export slice)

The qualification-only resolver now closes the first project-wide target gap
without reintroducing terminal-name matching or a filesystem search:

- `UniversalResolutionIndex` indexes TypeScript/JavaScript declarations by
  normalized repository-relative module path and export spelling. Relative
  imports use the importer directory, extension substitution, and directory
  `index` aliases; `.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`, `.mjs`, and
  `.cjs` realizations retain their source anchors.
- Local default/local-export aliases and bounded cross-file re-export aliases
  retain exact declaration slots. Re-export chains are followed through the
  in-memory evidence table with a depth bound; cycles and missing hops remain
  unresolved. `export *` never forwards `default` implicitly.
- TypeScript/JavaScript interop is admitted only after an exact module/export
  path proves the target. Imported member calls first resolve the exported
  owner, then select the exact owner member; duplicate same-path realizations
  across either semantic family stay ambiguous.
- Six focused resolver regressions cover relative extension substitution,
  default imports and members, cross-file re-export aliases, exact TS/JS
  interop, terminal-name collision negatives, duplicate-module ambiguity, and
  dynamic-member preservation. The candidate suite passes with deterministic
  IDs and occurrence anchors.

This slice is still below the Phase 3/4 and production hard-cut gates. The
universal index does not yet consume the full project resolver for `paths`,
`rootDirs`, package `exports`/`imports`, `typesVersions`, or all Node16/
NodeNext/Bundler conditions; declaration merging, generic instantiation,
mapped/indexed shapes, framework tiers, compiler differential recall, and the
accepted precision/leadership scorecard remain open. The candidate adapter is
still not registered in `UNIVERSAL_ADAPTERS`.

### Execution checkpoint (2026-08-06, project-resolver handoff slice)

The qualification-only TypeScript/JavaScript universal path now consumes the
same bounded project decisions that Compass already applies to its shared
resolver. Before universal evidence is materialized, the resolver retains a
read-only buffer of admitted `imports_from` decisions and applies the existing
project rules for `compilerOptions.paths`, `baseUrl`, `rootDirs`, package
`exports`/`imports`, `typesVersions`, workspace packages, and importer-aware
conditions. The universal index receives only those exact target files, keyed
by normalized importer plus the raw module specifier; target module keys are
bounded and duplicate realizations remain ambiguous rather than being guessed.

Universal construction/import edges that use this handoff publish the explicit
`project-module-binding` rule, while member calls retain `member-binding` and
the original source occurrence/provenance. No terminal-name matching,
unbounded filesystem search, Graphify runtime, compiler runtime, or network
dependency was added. The shared-resolution regression proves a `paths` alias
from `app/consumer.ts` to `src/api.ts` reaches both `Widget` construction and
`Widget.run()` with exact source-backed targets. The owned and borrowed merge
paths both pass the same project-edge handoff.

This closes the first universal/project-module integration gap, not the Plan
013 release gate. Declaration merging, generic instantiation, mapped/indexed
shapes, framework/compiler strata, hand-labeled precision and recall, and the
production hard cut remain open. The candidate adapter remains absent from
`UNIVERSAL_ADAPTERS` until those gates and the exact-release qualification
complete.

### Execution checkpoint (2026-08-06, imported declaration-merging slice)

The TypeScript candidate path now preserves the source binding when a value is
annotated with an imported nominal type (`value: ImportedType`). The receiver
proof no longer collapses to a bare qualified spelling, so member calls and
accesses carry the imported binding identity and a qualified target such as
`module::Config.inspect`. This covers type-only imports as well as ordinary
value-and-type imports and keeps unresolved/dynamic receivers conservative.

The shared TypeScript resolver now widens only its internal exported-owner
lookup to include type-space declarations (`interface`, `namespace`, and
aliases); final member filtering still admits only the callable/value member
target kinds. Consequently, two source declarations of one interface merge
their distinct members, while duplicate same-spelled members remain
ambiguous and publish no invented call. Borrowed and owned merge paths are
covered by the cross-file regression, with candidate-level assertions for
binding identity and member qualification.

This advances declaration merging and imported nominal receiver quality but is
not a full TypeScript type system. Generic instantiation beyond bounded source
shapes, mapped/indexed conditional types, framework tiers, compiler
differential recall, and the production hard cut remain open. The candidate
adapter remains absent from `UNIVERSAL_ADAPTERS` until the mandatory quality
and exact-release qualification gates complete.

### Execution checkpoint (2026-08-06, bounded generic member-chain slice)

The candidate adapter now publishes a bounded declaration-shape fragment in
the existing `DeclarationFact.signature` field: generic class/interface
parameter order and direct property nominal types. Imported generic receiver
arguments are canonicalized to their proven module-qualified identities and
remain attached to member constraints (for example,
`Box<../lib/item::Item>.item.inspect`). This preserves the existing evidence
schema and avoids turning a terminal member spelling into an authoritative
target.

The shared resolver follows a generic member-value chain only when every hop
is source-backed and unique: it resolves the imported `Box`, substitutes the
explicit `Item` argument for the property type parameter `T`, resolves the
admitted `Item` module, and finally selects the exact `inspect` declaration.
Ambiguous declarations, missing property type shapes, structural/complex
types, and unsupported generic forms remain unresolved or follow the existing
explicit external policy. A cross-module `Box<Item>` regression now proves
the final call and member access target the exact `Item.inspect` node with
`member-binding`; the full 137-test universal resolver suite and 44-test
TypeScript candidate suite pass.

This is a bounded generic propagation slice, not compiler-equivalent
TypeScript. Nested mapped/conditional/indexed types, overload-aware generic
substitution, framework tiers, compiler differential recall, and the
production hard cut remain open. The candidate adapter remains absent from
`UNIVERSAL_ADAPTERS` until the mandatory quality and exact-release
qualification gates complete.

### Execution checkpoint (2026-08-06, recursive nested generic member-chain slice)

The generic member-chain resolver now carries a bounded concrete type context
for each owner hop instead of reusing only the root instantiation. Nested
nominal arguments are recursively canonicalized in the existing
`DeclarationFact.signature`/qualified-path evidence (for example,
`Box<../lib/types::Wrapper<../lib/item::Item>>`), then propagated through
`Box<T>.item` to `Wrapper<U>.value` before resolving the final `Item.inspect`
member. This keeps the mechanism source-backed and deterministic without
introducing a compiler or runtime type-system dependency.

Three resolver regressions cover the positive cross-module chain, duplicate
`Item.inspect` ambiguity, and a primitive `Box<string>` negative. The
universal resolver suite is now 140 tests and the TypeScript candidate suite
remains 44 tests. Unsupported structural, mapped, conditional, indexed,
overload-dependent, or over-limit shapes still fail closed; framework tiers,
compiler differential recall, project-corpus qualification, and the production
hard cut remain open. The candidate adapter remains absent from
`UNIVERSAL_ADAPTERS` until the mandatory quality and exact-release
qualification gates complete.

### Execution checkpoint (2026-08-06, generic alias and indexed-member slice)

The shared TypeScript/JavaScript resolver now follows two additional
source-backed declaration shapes without widening terminal-name matching:

- Generic type aliases retain a bounded alias target shape (for example,
  `Alias<T> = Box<T>`). Alias parameters are substituted recursively, imported
  aliases are resolved through the alias module's exact import binding, and
  alias cycles or unsupported targets remain unresolved.
- Interface/type-alias index signatures publish a compact value-type shape.
  Imported receivers preserve an indexed marker (`Shape<Item>[].inspect`) so
  the resolver can select the unique source-backed value type for
  `shape[key].inspect()`; generic `Shape<T>` index values carry the concrete
  `T` context through the access.

The slice adds positive regressions for an object alias, an alias to an
imported generic nominal, ordinary and generic imported index signatures, plus
ambiguity and primitive negative cases. The universal resolver suite is now
146 tests and the TypeScript candidate suite remains 44 tests. The behavior is
bounded by the existing member/export candidate limits and type-shape byte and
depth limits; mapped/conditional/indexed access beyond a direct index value,
overload-dependent substitutions, framework tiers, compiler differential
recall, project-corpus qualification, and the production hard cut remain
open. The candidate adapter remains absent from `UNIVERSAL_ADAPTERS` until the
mandatory quality and exact-release qualification gates complete.

### Execution checkpoint (2026-08-06, bounded local reassignment slice)

The qualification-only ECMAScript candidate now carries a small, explicit
flow-sensitive receiver fact set for local TypeScript and JavaScript variables:

- A source-proven constructor or typed call result assigned in the variable's
  binding scope is recorded with its exact source order. A later member use
  selects the latest preceding fact, so a straight-line `new First()` followed
  by `current = new Second(); current.run()` targets only `Second.run`.
- The same rule works inside a function body and across the universal resolver
  publication path. The source fact keeps imported/local receiver identity and
  does not reopen terminal-name matching.
- Branch/loop/try/with assignments, compound assignments, and unsupported
  values create a bounded barrier. Older receiver facts cannot leak across the
  barrier, so ambiguous or dynamically reassigned values remain unresolved.

The slice adds TypeScript and JavaScript positive regressions, use-site ordering,
branch/unknown/compound negative cases, and a resolver-level exact-target
assertion. The TypeScript/JavaScript candidate suite is now 52 tests and the
universal resolver suite is now 147 tests. Alias escape, `eval`, proxy/dynamic
property mutation, interprocedural flow, and compiler differential recall remain
open Phase 4 work; the candidate adapter remains absent from
`UNIVERSAL_ADAPTERS` until the mandatory quality and exact-release gates pass.

### Execution checkpoint (2026-08-06, homomorphic mapped-alias slice)

The qualification-only TypeScript path now preserves a narrow, source-proven
nominal identity for homomorphic mapped aliases without treating structural
assignability as universal:

- A mapped alias whose single object shape is exactly
  `{ [K in keyof Item]: Item[K] }` publishes `Item` as its bounded nominal
  source. The same rule substitutes an explicit generic argument for
  `type Copy<T> = { [K in keyof T]: T[K] }`, including canonical module-local
  identities produced by the existing generic receiver path.
- The helper is deliberately strict: nested/additional object members,
  key-remapping, computed transforms, unsupported value shapes, and ambiguous
  canonical declarations do not produce a receiver target. Ordinary index
  signatures keep their existing indexed-value behavior.
- A direct TypeScript candidate regression covers non-generic and generic
  aliases plus a key-remapped negative. A cross-file resolver regression proves
  `Copy<Item>` reaches the exact imported `Item.inspect` declaration through the
  existing `member-binding` publication rule. The focused candidate suite is
  now 56 tests and the universal resolver suite is now 148 tests.

This remains a bounded evidence slice, not compiler-equivalent mapped-type
assignability: modifier-rich mapped types, conditional/infer transforms,
indexed access beyond direct value shapes, alias escape, dynamic mutation,
framework tiers, compiler differential recall, and the production hard cut
remain open. The candidate adapter stays out of `UNIVERSAL_ADAPTERS` until the
mandatory precision, determinism, qualification, and exact-release gates pass.

### Execution checkpoint (2026-08-06, bounded sequence-index slice)

The qualification-only TypeScript/JavaScript path now carries source-proven
element receivers through common array and tuple indexing:

- Postfix arrays (Item[]), Array<Item>, and ReadonlyArray<Item> preserve
  the nominal element receiver for literal or dynamic homogeneous indexes.
  Generic properties such as Box<T>.values: T[] substitute the concrete
  Box<Item> argument before resolving values[0].inspect().
- Fixed tuples preserve a literal numeric element ([Item, string][0]) and
  generic tuple properties substitute the owner argument before selection.
  Out-of-range, optional/rest, and dynamic tuple indexes fail closed rather
  than selecting a union member by position.
- Imported chains retain a bounded values[0]/pair[0] path marker for the
  shared resolver. The resolver distinguishes genuine index-signature metadata
  from generic interface signatures, expands indexed intermediate properties,
  and keeps exact member-binding provenance for all three imported array/
  tuple calls in the regression fixture.

The slice adds six direct candidate regressions and one cross-file resolver
regression; the focused candidate suite is now 62 tests and the universal
resolver suite is now 149 tests. Arbitrary structural indexed access,
conditional/mapped modifier semantics, alias escape, dynamic mutation,
framework tiers, compiler differential recall, and the production hard cut
remain open. The candidate adapter remains absent from UNIVERSAL_ADAPTERS.

### Execution checkpoint (2026-08-06, bounded generic callable-return slice)

The qualification-only TypeScript/JavaScript path now preserves a
source-proven generic callable's returned receiver when the call arguments
uniquely determine its type parameters:

- Direct `T` returns infer from typed or constructor arguments, and explicit
  call-site arguments such as `identity<Item>(...)` are canonicalized through
  the existing local type identity rules.
- Inference recurses through matching postfix arrays and generic containers,
  so `collect(new Item())[0].inspect()` and
  `box(new Item()).value.inspect()` retain the exact `Item.inspect` target.
  Array and tuple return shapes remain bounded by the existing type-shape
  limits.
- Missing inference and conflicting arguments fail closed. Contextual
  overload inference, conditional/structural assignability, imported
  callable return shapes, and dynamic `eval`/proxy mutation are not treated
  as proven receivers.

The slice adds five direct candidate regressions and one resolver regression;
the focused candidate suite is now 67 tests and the universal resolver suite
is now 150 tests. The production candidate adapter remains absent from
`UNIVERSAL_ADAPTERS`; accepted precision/Wilson gates, cross-file callable
return evidence, framework/compiler tiers, equivalent Graphify/SCIP scope,
and the production hard cut remain open.

### Execution checkpoint (2026-08-06, bounded utility/conditional receiver slice)

The qualification-only TypeScript/JavaScript path now normalizes a small,
source-spelled set of standard utility wrappers after generic substitution:

- `NonNullable<T | undefined>` removes only non-nominal/nullish union arms
  when exactly one nominal receiver remains. `Awaited<Promise<T>>` and
  `Awaited<PromiseLike<T>>` unwrap bounded promise layers, while
  `Partial<T>`, `Required<T>`, and `Readonly<T>` preserve the underlying
  nominal declaration identity.
- The same normalization runs in the shared cross-file member resolver, so
  generic properties such as `Box<T>.nullable: NonNullable<T|undefined>`
  retain the imported `Item.inspect` target after `Box<Item>` substitution.
- Multiple nominal union arms, arbitrary structural wrappers, and unsupported
  conditional forms fail closed. Top-level union substitution is recursive but
  bounded by the existing shape/depth limits.

The slice adds three direct candidate regressions and expands the imported
array/tuple resolver fixture to six exact utility/indexed member calls; the
focused candidate suite is now 70 tests and the universal resolver suite is
now 150 tests. Imported callable-return evidence, richer conditional/mapped
modifier semantics, accepted precision/Wilson gates, framework/compiler tiers,
equivalent Graphify/SCIP scope, and the production hard cut remain open. The
candidate adapter remains absent from `UNIVERSAL_ADAPTERS`.

### Execution checkpoint (2026-08-06, imported callable-return evidence slice)

The qualification-only TypeScript/JavaScript path now carries bounded
source-proven call-result evidence across project files:

- Imported direct calls publish a length-limited `#call<...>` path marker with
  only known argument shapes. Unknown arguments, oversized markers, and
  dynamic call results remain unresolved; the marker is never eligible for a
  fabricated external target.
- Callable declarations publish fixed parameter and return metadata alongside
  their existing generic parameter prefix. The shared resolver uses that
  metadata for direct nominal returns and bounded generic inference through
  direct parameters, postfix arrays, and matching generic containers.
- Imported calls such as `make(value).inspect()`,
  `identity(value).inspect()`, and `box(value).value.inspect()` now resolve
  to the exact `Item.inspect` declaration when the source evidence is unique.
  Duplicate exported declarations remain unresolved instead of selecting a
  first candidate.

The slice adds one direct candidate regression and two cross-file resolver
regressions; the focused candidate suite is now 71 tests and the universal
resolver suite is now 152 tests. Imported callable member-method returns,
explicit generic-call arguments without inferable value arguments, richer
conditional/mapped modifier semantics, accepted precision/Wilson gates,
framework/compiler tiers, equivalent Graphify/SCIP scope, and the production
hard cut remain open. The candidate adapter remains absent from
`UNIVERSAL_ADAPTERS`.

### Execution checkpoint (2026-08-06, imported callable-member and explicit-generic slice)

Imported call-result evidence now covers one additional source-backed boundary:

- Static or instance-like callable members such as
  `Factory.make(value).inspect()` carry the call marker on the member segment,
  then resolve the unique callable member signature before continuing the
  nominal receiver chain.
- Explicit generic call arguments are preserved in a bounded `#types<...>`
  marker. They can prove a generic return even when a value argument is
  unknown, while conflicting inferred value types and arity mismatches remain
  unresolved.
- Multiple imported owners or multiple callable member declarations fail
  closed before their return contexts are merged, preserving ambiguity rather
  than collapsing distinct owner paths onto one target.

The slice adds one candidate regression and two cross-file resolver
regressions; the focused candidate suite is now 72 tests and the universal
resolver suite is now 154 tests. Conditional/mapped modifier semantics,
framework/compiler tiers, accepted precision/Wilson gates, equivalent
Graphify/SCIP scope, imported callable properties, alias escape/eval/proxy
flow, overload-aware generic substitution beyond bounded source shapes, and
the production hard cut remain open. The candidate adapter remains absent
from `UNIVERSAL_ADAPTERS`.

### Execution checkpoint (2026-08-06, callable-property and typed-value slice)

The qualification-only TypeScript/JavaScript path now preserves another
source-proven callable boundary across project files:

- Function-valued object properties publish bounded callable signatures with
  fixed parameter and return shapes, including generic arrow properties. A
  directly annotated value import also publishes its nominal `|type:` shape so
  member traversal can expand the referenced interface without guessing.
- Structural object-literal properties are indexed under their exact source
  variable owner in the shared resolver. Imported calls such as
  `api.make(value).inspect()` and `api.identity<Item>(value).inspect()` now
  resolve both the callable property and the returned `Item.inspect` member
  with `member-binding` provenance.
- Nominally typed value imports such as `declare const typed: TypedApi` now
  resolve their direct callable member target before external fallback. Duplicate
  callable properties remain unresolved, and unknown/ambiguous owners are not
  collapsed into a terminal-name match.

The slice adds one direct candidate regression and two cross-file resolver
regressions; the focused candidate suite is now 73 tests and the universal
resolver suite is now 156 tests. The exact fixture qualification remains
byte-stable at Compass revision `0d051868` with graph digest
`sha256:f13848fefa81b79c70ed9b50081c1cab8024f1ce84030064c8eb2d154ba4c160`.
Conditional/mapped modifier semantics, framework/compiler tiers, accepted
precision/Wilson gates, equivalent Graphify/SCIP scope, alias escape/eval/
proxy flow, overload-aware generic substitution beyond bounded source shapes,
and the production hard cut remain open. The candidate adapter remains absent
from `UNIVERSAL_ADAPTERS`.

### Execution checkpoint (2026-08-06, bounded imported-overload selection slice)

The resolver now uses source-published callable parameter shapes to select a
unique imported TypeScript/JavaScript overload when the call site carries an
exact, bounded argument shape:

- Direct imported functions, static members, and callable member chains can
  match fixed parameters, imported type aliases, bounded array/generic
  containers, and explicit generic arguments. Relative module-qualified types
  are normalized against the declaration's source module before comparison.
- Duplicate same-shape overloads, competing exact matches, arity mismatches,
  unknown arguments without explicit generic evidence, and unsupported
  assignability remain unresolved. Source-backed rejection cannot fall through
  to lexical or terminal-name guesses, while JavaScript method signatures that
  publish parameters without a return marker retain their existing resolution.
- The overload matcher is bounded by the existing candidate and type-shape
  limits; it emits no new external or deferred targets and preserves existing
  member-binding provenance for unique matches.

The focused candidate suite remains at 73 tests and the universal resolver
suite is now 158 tests, including one positive imported-overload fixture and
one ambiguity/mismatch/unknown negative fixture. Conditional/mapped modifier
semantics, richer overload-aware generic substitution, alias escape/eval/proxy
flow, framework/compiler tiers, accepted precision/Wilson gates, equivalent
Graphify/SCIP scope, and the production hard cut remain open. The candidate
adapter remains absent from `UNIVERSAL_ADAPTERS`. Fixture qualification stays
byte-stable at Compass revision `24f937360cc80d6b62b633f2fdd2a367eb6529c3`,
with 57 languages, 980 coverage records, 1,565 invariants, 27 flows, and
graph digest `sha256:f13848fefa81b79c70ed9b50081c1cab8024f1ce84030064c8eb2d154ba4c160`.

### Execution checkpoint (2026-08-07, bounded JavaScript alias-escape slice)

The qualification-only ECMAScript candidate now carries source-proven local
receiver identity through safe straight-line aliases:

- `const alias = current` preserves the latest constructor, call-result, or
  imported nominal receiver, including a cross-file callable-return chain.
- Passing a tracked value to a callable argument, returning it, writing a
  member, capturing it in a closure, entering `with`, invoking global `eval`,
  or constructing an unsupported `Proxy` records an explicit flow barrier.
  Barriers prevent stale exact member targets and dynamic/escaping cases remain
  unresolved instead of becoming terminal-name calls.
- Dynamic barriers are bounded by the existing traversal, per-variable fact,
  visible-binding, and argument limits. Persistent barriers are used for
  closure/dynamic-scope invalidation so later assignments cannot accidentally
  restore a receiver whose binding remains observable through the escape.

The focused candidate suite is now 75 tests and the universal resolver suite
remains 158 tests; the overload fixture also proves an imported call-result can
be aliased before its returned member is resolved. The candidate adapter remains
absent from `UNIVERSAL_ADAPTERS`. Conditional/mapped modifier semantics, richer
interprocedural flow, framework/compiler tiers, accepted precision/Wilson gates,
equivalent Graphify/SCIP scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, callable-value references and `in` narrowing)

The qualification-only ECMAScript candidate now preserves source-proven
callable values as references when they flow through ordinary JavaScript and
TypeScript containers or unknown APIs:

- Callable local variables and bounded alias chains (`const alias2 = alias`)
  publish exact `references` candidates for direct arguments, array/object
  values, and later uses. The candidate does not invent `indirect_call`; an
  alias remains a reference to its own source binding until a versioned API or
  framework contract proves invocation semantics.
- Callable aliases are classified through a small fixed point bounded by the
  existing inline-property budget. Member aliases require one unique local
  source declaration; imported member shapes remain unresolved at the
  per-file boundary. Non-callable values, conditional mixtures, duplicate
  owners, and dynamic containers remain unclassified.
- Positive literal `in` guards narrow a union receiver only when exactly one
  source constituent owns the guarded property. Shared-property, unknown-key,
  imported, and unsupported guard forms remain unresolved; the parser range and
  occurrence are still preserved.

The focused candidate suite is now 77 tests and the universal resolver suite is
159 tests, including resolver publication checks for callable references with
no indirect-call edge and candidate checks for unique/ambiguous `in` guards.
The candidate adapter remains absent from `UNIVERSAL_ADAPTERS`. Conditional and
mapped modifier semantics beyond the bounded utility/union rules, richer
interprocedural flow, framework/compiler tiers, accepted precision/Wilson gates,
equivalent Graphify/SCIP scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, bounded utility receiver projections)

The qualification-only TypeScript candidate now models a narrow, source-backed
subset of utility receivers while preserving fail-closed behavior:

- `Pick<LocalType, "member">` and `Omit<LocalType, "member">` project exact
  local members for receiver lookup. Unknown key expressions, imported bases,
  nested utility projections, oversized shapes, and duplicate/unsupported key
  encodings remain unresolved rather than inventing a property target.
- `Exclude<A | B, Filter>` and `Extract<A | B, Filter>` narrow to one nominal
  source constituent only when the bounded union/filter comparison proves a
  unique owner. Multiple surviving owners, non-nominal members, and unsupported
  assignability remain unresolved.
- Projection metadata is internal to candidate lookup; published declarations,
  source anchors, occurrence ranges, and member direction remain unchanged.

The focused candidate suite is now 79 tests and the universal resolver suite
remains 159 tests. The candidate adapter remains absent from
`UNIVERSAL_ADAPTERS`. Conditional and mapped modifier semantics beyond these
bounded utility/union rules, richer interprocedural flow, framework/compiler
tiers, accepted precision/Wilson gates, equivalent Graphify/SCIP scope, and the
production hard cut remain open.

### Execution checkpoint (2026-08-07, bounded conditional and mapped-modifier receivers)

The qualification-only TypeScript candidate now evaluates a narrow,
source-proven subset of conditional and mapped receiver types:

- Generic aliases of the form `T extends U ? A : B` substitute bounded concrete
  type arguments before branch selection. A branch is selected only when one
  local nominal owner proves the `extends` relation (or a local nominal type
  proves the bounded `object`/`unknown` check). Distributed unions, `any`,
  unresolved structural checks, nested conditionals, and unsupported branches
  remain unresolved.
- Homomorphic mapped aliases with `+/-readonly` and `+/-?` modifiers preserve
  their source nominal owner for member lookup. Key remapping, extra structural
  members, nested mapped shapes, and other unsupported transformations remain
  fail-closed.
- Conditional parsing is bounded and quote/depth-aware; source ranges,
  occurrence multiplicity, and published declaration identities are unchanged.

The focused candidate suite is now 81 tests and the universal resolver suite
remains 160 tests. The candidate adapter remains absent from
`UNIVERSAL_ADAPTERS`. Broader distributive conditional semantics, indexed/keyof
evaluation, richer interprocedural flow, framework/compiler tiers,
accepted precision/Wilson gates, equivalent Graphify/SCIP scope, and the
production hard cut remain open.

### Execution checkpoint (2026-08-07, project-aware compiler source-oracle slice)

The independent TypeScript/JavaScript source oracle now honors the compiler's
bounded project model before measuring source recall:

- It discovers `tsconfig*.json` and `jsconfig*.json`, parses their include/
  exclude/file selections, follows in-root project references, and guards
  project count, reference depth, project-file references, source bytes, facts,
  and diagnostics with explicit limits. Cycles are visited once and do not
  recurse indefinitely.
- The payload records project scopes, configuration/source digests, a
  `projectMode` (`project`, `fallback`, or `tree`), deterministic diagnostics,
  and exact UTF-8 ranges. Parser/configuration failures remain rejected or
  diagnosed coverage; an entirely invalid configuration set falls back to the
  bounded source tree rather than silently reporting an empty project.
- The Python inventory validator accepts and validates the optional project and
  diagnostics sections while preserving the existing v1 source-construct
  contract. A regression fixture covers include/exclude boundaries, project
  references, syntax rejection, deterministic repeated output, and Unicode
  byte ranges.

The source oracle remains qualification-only and emits source constructs, not
targets; its output is still a deterministic JSON payload rather than the final
JSONL evidence stream described by Phase 0. The candidate adapter remains
absent from `UNIVERSAL_ADAPTERS`. Project-manifest corpora, compiler-backed
scope/declaration inventories, broader conditional/keyof semantics, framework
tiers, accepted precision/Wilson gates, equivalent Graphify/SCIP scope, and the
production hard cut remain open.

### Execution checkpoint (2026-08-07, bounded literal indexed-access slice)

The qualification-only TypeScript candidate and resolver now follow a bounded
literal indexed-access type alias when the source proves a unique nominal
property:

- Local aliases such as `type Nested = Item["nested"]` preserve the selected
  member's declaration identity and let downstream calls resolve to the exact
  source method. Imported generic aliases such as `NestedOf<T> = T["nested"]`
  substitute an explicit cross-file nominal argument and resolve the selected
  member through the project export index.
- Computed keys, dynamic generic keys, union projections with competing owners,
  duplicate members, imported candidate-side projections, and unsupported
  structural shapes remain unresolved. Numeric access continues through the
  existing bounded array/tuple path; the new object projection accepts only
  quoted string keys.
- Type substitution is bounded to the existing type-shape limit and preserves
  the original indexed suffix, so generic aliases cannot silently widen into a
  terminal-name match. Candidate and resolver evidence retain existing source
  anchors, direction, multiplicity, and deterministic ordering.

The focused TypeScript candidate suite is now 83 tests and the universal
resolver suite is now 162 tests. The candidate adapter remains absent from
`UNIVERSAL_ADAPTERS`. Broader `keyof`/mapped/indexed evaluation, distributive
conditional semantics, project-manifest corpora, compiler-backed scope and
declaration inventories, framework tiers, accepted precision/Wilson gates,
equivalent Graphify/SCIP scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, bounded generic indexed and `keyof` identity slice)

The qualification-only TypeScript candidate and resolver now preserve two
additional source-proven type shapes:

- Generic aliases such as `NestedOf<T> = T["nested"]` substitute the explicit
  nominal argument before indexed-member lookup in both same-file candidate
  evidence and cross-file resolver evidence. The indexed suffix is preserved
  under the existing shape bound, so arbitrary computed keys still fail closed.
- `Pick<Base, keyof Base>` is recognized as a local identity projection, and
  the imported generic form follows the same identity through the project
  export index. `Omit<Base, keyof Base>` remains empty for member lookup. A
  literal or competing/structural key space is not widened by this slice;
  duplicate owners, imported candidate-side projections, and ambiguous unions
  remain unresolved.
- The normalized type-shape encoding keeps a separator after the `keyof`
  keyword, preventing a compact `keyofT` representation from being confused
  with an ordinary identifier. The parser still treats identifiers such as
  `keyofItem` as ordinary names.

The focused TypeScript candidate suite is now 86 tests and the universal
resolver suite is now 162 tests. The candidate adapter remains absent from
`UNIVERSAL_ADAPTERS`. Full `keyof` value-space evaluation, arbitrary
mapped/indexed projections, distributive conditional semantics,
project-manifest corpora, compiler-backed scope and declaration inventories,
framework tiers, accepted precision/Wilson gates, equivalent Graphify/SCIP
scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, source-oracle JSONL coverage stream)

The independent TypeScript/JavaScript source oracle now has a bounded,
record-oriented JSONL mode used by the Python audit inventory:

- `--jsonl` emits a deterministic header, project records, one parsed/rejected
  file record for every scanned file, diagnostics, source constructs, and a
  footer carrying counts and source/config digests. The parser rejects missing,
  duplicated, unknown, malformed, truncated, or inconsistent records instead
  of lowering the recall denominator.
- The existing single-document JSON mode remains available for direct oracle
  inspection and compatibility fixtures. Normal Compass builds still never
  invoke Node.js, TypeScript, or this developer-only oracle.
- Unicode byte ranges, project ownership, invalid-config fallback, parser
  rejection, deterministic ordering, and repeated output remain covered by the
  correctness suite; the audit path now validates the stream before building
  its source inventory.

The source oracle is still source-occurrence evidence rather than target truth.
Compiler-backed target adjudication, four release corpora, accepted
precision/Wilson gates, framework tiers, equivalent Graphify/SCIP scope, and
the production hard cut remain open.

### Execution checkpoint (2026-08-07, typed source-oracle fact stream)

The JSONL source-oracle contract now publishes bounded typed records in
addition to the compatibility construct inventory:

- `scope` records carry deterministic source-local IDs, lexical kind, exact
  UTF-8 range, owner identity, and a validated parent scope. Duplicate IDs,
  missing parents, path escapes, and range overflow fail closed.
- `declaration` records carry source-backed kind/name/qualified identity,
  explicit value/type/namespace space, exact name range, and bounded callable
  parameter metadata (total/minimum/rest). Non-callable declarations use an
  explicit null parameter shape rather than implying a callable signature.
- `call` records carry the exact callee anchor plus full call-expression range,
  direct/member/computed/dynamic target kind, owner, relation, argument count,
  spread and optional-call flags. The range validator proves that the callee
  lies inside the full call expression and inside its source file.
- Header/footer counts cover every typed record under
  `compass.typescript-source-oracle-jsonl/3`; repeated output is byte-identical,
  and the Python audit reconstructs the legacy inventory only after validating
  the typed stream.

This closes the source-oracle scope/declaration/call stream contract, but not
target adjudication or semantic quality: compiler-backed target truth, four
release corpora, accepted precision/Wilson gates, framework tiers, equivalent
Graphify/SCIP scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, typed relationship fact stream)

The qualification-only TypeScript/JavaScript JSONL oracle now promotes the
remaining source relationships into typed records instead of relying only on
the compatibility construct projection:

- `import` and `reexport` records retain module specifiers, named/default/
  namespace/local binding identity, type-only provenance, enclosing statement
  ranges, and exact source anchors.
- `construction` records separate `new` expressions from ordinary `call`
  records while preserving full invocation ranges, target classification,
  argument count, spread, and owner identity.
- `base` records cover `extends` and `implements` expressions with exact
  heritage-clause containment; `member` records cover property and bounded
  literal-element accesses with read/write and optional-access metadata.
- `reference` records cover source identifiers, type references, and JSX tag
  anchors. Every new record is bounded, sorted deterministically, tied to a
  parsed source file, and checked for enclosing-range and count consistency by
  the Python audit validator.

The JSONL contract is now
`compass.typescript-source-oracle-jsonl/3`; the single-document schema and
legacy construct inventory remain compatible. This closes the source-oracle
relationship fact inventory, but compiler-backed target truth, four release
corpora, accepted precision/Wilson gates, framework tiers, equivalent
Graphify/SCIP scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, imported literal utility projections)

The shared TypeScript resolver now carries source-proven literal `Pick`/`Omit`
projections through imported type aliases:

- A simple member path rooted at a type alias now enters the bounded alias-chain
  resolver, so non-generic aliases such as `Picked = Pick<Item, "enabled">`
  are expanded instead of being treated as terminal external owners.
- Literal key sets, including bounded unions of string/quoted-template keys,
  select only the projected member. `Pick` excludes unlisted members and `Omit`
  excludes listed members; every surviving target keeps the normal
  `member-binding`/source provenance path.
- Imported bases and re-exported aliases remain source-inventory bounded.
  Dynamic, structural, competing, and unsupported key spaces do not enter this
  branch and remain unresolved rather than widening to a nominal base.

The universal resolver suite is now 164 tests, including positive and negative
cross-file `Pick`/`Omit` projection calls. This is still below the full
value-space `keyof`, arbitrary mapped/indexed, compiler-target, corpus,
precision/Wilson, framework, comparator, and production-hard-cut gates.

### Execution checkpoint (2026-08-07, bounded inline structural index receivers)

The native TypeScript/JavaScript candidate path now carries one additional
source-proven structural shape across dynamic member access:

- Inline index signatures on parameters, variables, and properties cache their
  bounded value type (`{ [key: string]: Item }`) at the binding identity, so a
  dynamic `shape[key].inspect()` can resolve to `Item` only when that value type
  resolves to one source-backed nominal owner.
- Imported interface and type-alias index signatures retain their existing
  declaration-signature path; a regression fixture proves both forms resolve
  the same cross-file member target with `member-binding` provenance.
- Primitive, ambiguous, mapped, malformed, oversized, and otherwise
  source-unproven value shapes remain unresolved. No terminal-name fallback or
  arbitrary structural member selection is added.

The universal resolver suite is now 166 tests, including the inline structural
positive/negative case and the imported structural type-alias regression. This
closes only the bounded inline index-receiver slice; broader arbitrary
structural/mapped/indexed semantics, compiler-target truth, release corpora,
precision/Wilson, framework tiers, equivalent Graphify/SCIP scope, and the
production hard cut remain open.

### Execution checkpoint (2026-08-07, immutable `this` alias closure slice)

The native JavaScript flow path now preserves a narrow, source-proven receiver
identity for immutable aliases captured by a closure:

- `const token = this` (and equivalent `const` aliases to nominal receivers)
  can retain the enclosing class receiver through a later closure call, so
  `token.subscribe()`/similar members can resolve to the class declaration.
- Mutable aliases, structural object aliases, and dynamic escape paths remain
  barriers and fail closed. The implementation does not infer a receiver from
  a spelling or choose among competing structural members.
- A focused candidate regression covers the positive class pattern while the
  existing mutable-closure negative remains intact.

On the read-only Axios target-adjudication diagnostic, exact local target
matches improved from 3,014/3,167 to 3,019/3,167 (missing 148, wrong 0); this
is a diagnostic signal rather than a release precision/recall claim. Broader
interprocedural flow, conditional structural exports, compiler-target truth,
release corpora, precision/Wilson, framework tiers, equivalent Graphify/SCIP
scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, property-scoped immutable receiver flow)

The JavaScript flow path now distinguishes receiver identity from property
mutation for source-proven immutable aliases:

- `const` aliases to nominal receivers and exact object-literal receivers can
  cross a closure without widening to unresolved solely because an unrelated
  property is written.
- Member writes are tracked by `(binding, property)` under the existing
  bounded limit. A write to `token._listeners` does not erase a proven
  `token.subscribe` receiver, while `token.subscribe = replacement` remains a
  fail-closed barrier. Dynamic keys, mutable aliases, calls/returns, and other
  unsupported escapes retain the conservative whole-binding barrier.
- Candidate regressions cover both class `this` aliases and structural object
  aliases; the existing mutable and overwrite negatives remain unresolved.

On the read-only Axios target-adjudication diagnostic, exact local target
matches improved from 3,023/3,167 to 3,045/3,167 (122 missing, 0 wrong). The
member stratum is 382/503 and the call stratum is 1,544/1,545. This remains
diagnostic evidence rather than a release precision/recall claim. Conditional
structural exports, compiler-target truth, release corpora, precision/Wilson,
framework tiers, equivalent Graphify/SCIP scope, and the production hard cut
remain open.

### Execution checkpoint (2026-08-07, inline structural assignment receivers)

The native JavaScript flow path now preserves a bounded, source-qualified
identity for immutable aliases initialized from a plain assignment expression
whose value is an exact object literal:

- `const state = (this[key] = { inspect() {} })` publishes the object members
  under `state`, then carries that identity through later closures and
  property-scoped mutation barriers. Computed member keys remain opaque; the
  alias is tied to the local declaration, not to a guessed `key` spelling.
- Separate inline assignments in one callable retain separate qualified
  identities, so `first.inspect()` cannot select the later `second.inspect()`
  declaration merely because both objects share a property name.
- Object spreads, queried-member overwrites, mutable aliases, unsupported
  non-declarator assignment shapes, and dynamic escapes remain unresolved or
  fail closed. Direct object-literal behavior is unchanged.

The focused candidate suite remains 88 tests, including inline assignment
positive, unrelated-write, overwrite, spread, and distinct-object regressions;
the universal resolver suite remains 166 tests. On the read-only Axios
target-adjudication diagnostic, exact local target matches improved from
3,045/3,167 to 3,046/3,167 (121 missing, 0 wrong); members are 383/503 and
calls remain 1,544/1,545. This is diagnostic evidence rather than a release
precision/recall claim. Conditional structural exports, compiler-target truth,
release corpora, precision/Wilson, framework tiers, equivalent Graphify/SCIP
scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, chained inline structural assignments)

The same source-qualified inline receiver slice now accepts a bounded chain of
plain assignments when every link ends in one spread-free object literal:

- `const state = (this[key] = this[key] = { inspect() {} })` keeps the local
  declaration as the structural identity, so nested assignment syntax does not
  collapse separate objects or force a spelling-based receiver guess.
- Compound links, dynamic/non-declarator assignments, object spreads, queried
  overwrites, mutable aliases, and unsupported escapes remain unresolved or
  fail closed. The chain is depth-bounded and checks each operator directly
  from source bytes.
- A regression covers the positive chained receiver and a compound-assignment
  negative. The fixture qualification gate remains byte-identical (57
  languages, 980 coverage records, 1,565 invariants, 27 flows, 24 negatives,
  25 diagnostics, 28 edge kinds, and 45 node kinds).

The focused candidate suite remains 88 tests and the universal resolver suite
remains 166 tests. On the read-only Axios target-adjudication diagnostic, exact
local target matches improved from 3,046/3,167 to 3,047/3,167 (120 missing,
0 wrong); members are 384/503 and calls remain 1,544/1,545. This is diagnostic
evidence rather than a release precision/recall claim. Conditional structural
exports, compiler-target truth, release corpora, precision/Wilson, framework
tiers, equivalent Graphify/SCIP scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, nominal member-write and escape recovery)

The JavaScript flow path now preserves exact member identity for a stricter
nominal-only slice:

- Plain `=` writes on a source-proven nominal receiver publish the exact member
  declaration. Structural object writes, computed/dynamic keys, and compound
  assignments continue to use mutation barriers and fail closed.
- An immutable nominal alias can retain its receiver identity after an unknown
  call or other escape. This recovers class-member reads/writes without
  treating an escaped structural object as stable; nested member receivers are
  explicitly excluded from the fallback to prevent outer-owner substitution.
- Regressions cover direct and static nominal writes, compound-write negatives,
  unknown-call escape reads, and a nested-receiver wrong-target negative.

The focused candidate suite is now 89 tests and the universal resolver suite
remains 166 tests. On the read-only Axios target-adjudication diagnostic, exact
local target matches improved from 3,047/3,167 to 3,051/3,167 (116 missing,
0 wrong); members are 388/503 and calls remain 1,544/1,545. The fixture gate
remains byte-identical with 57 languages, 980 coverage records, 1,565
invariants, 27 flows, 24 negatives, 25 diagnostics, 28 edge kinds, and 45
node kinds. This is diagnostic evidence rather than a release precision/recall
claim. Conditional structural exports, compiler-target truth, release corpora,
precision/Wilson, framework tiers, equivalent Graphify/SCIP scope, and the
production hard cut remain open.

### Execution checkpoint (2026-08-07, CommonJS object-export bridge)

The universal TypeScript/JavaScript candidate now publishes bounded CommonJS
object-export evidence:

- `module.exports = { ... }` emits a source-backed default module reexport and
  exact named reexports for spread-free object properties. Shorthand/value
  properties retain the local declaration target; methods and literal
  properties retain their exact object-property declaration.
- Computed/dynamic keys and object spreads do not create guessed named
  bindings. The default module fact remains available, while incomplete
  property sets stay unresolved rather than widening to every visible name.
- Cross-file resolver coverage proves named `require()` bindings reach the
  exact exported function and method declarations. The candidate suite is now
  90 tests and the universal resolver suite 167 tests.
- Fixture qualification remains byte-identical (57 languages, 980 records,
  1,565 invariants, 27 flows, 24 negatives, 25 diagnostics, 28 edge kinds,
  and 45 node kinds). The Axios checker diagnostic remains 3,051/3,167 exact
  local targets, 116 missing, and 0 wrong; this bridge is project-level
  reexport evidence and is intentionally not presented as same-file recall.

Conditional structural exports beyond the bounded spread-free form,
compiler-target truth, release corpora, precision/Wilson, framework tiers,
equivalent Graphify/SCIP scope, and the production hard cut remain open.

### Execution checkpoint (2026-08-07, CommonJS require binding and namespace slice)

The candidate and resolver now preserve the two CommonJS import shapes that
were previously conflated:

- A direct binding such as `const api = require("./api")` is published as a
  source-anchored `module::*` namespace binding. Direct namespace members use
  the exact provider export slot; a callable `module.exports = fn` default is
  selected only for a direct callable require, while object-valued defaults do
  not become callable by assumption.
- Object destructuring preserves the source export key, including aliases and
  bounded static string/number keys (`run: execute` becomes `module::run`, not
  `module::execute`). Nested, rest, computed, array, malformed, and indirect
  require patterns remain unresolved rather than being flattened into guessed
  imports. Assignment defaults are accepted only when their bound value is a
  direct identifier.
- Namespace export-slot lookup prefers an exact source-backed re-export alias
  when a CommonJS object property and a same-named declaration coexist; this
  avoids publishing both the property declaration and the actual exported
  target. Ordinary ES namespace members continue through the same exact
  module/export path, while direct calls through non-callable ES namespaces
  remain unresolved.
- The candidate adapter versions advanced to 3 for the binding identity
  change. Focused coverage is now 91 candidate tests and 168 universal
  resolver tests, including aliased named requires, namespace member calls,
  callable CommonJS defaults, and dynamic/nested negative patterns.
- At exact HEAD `46fbbbb71f59c72b07e7c4d6c6c9e704cbb301dc`, the fixture-only
  qualification passes with the existing 57-language/980-record/1,565-
  invariant contract, graph digest
  `sha256:f13848fefa81b79c70ed9b50081c1cab8024f1ce84030064c8eb2d154ba4c160`,
  and clean/warm/rebuild/restored/alternate-checkout byte equality. The
  ignored Axios checker differential remains zero-wrong at 3,051/3,167 exact
  local targets (116 missing; 226 false positives in the accepted local
  candidate set); the remaining misses are same-file structural-flow cases,
  not CommonJS export-slot failures.

Fixture qualification, compiler-source-oracle recall, release-corpus
precision/Wilson gates, framework tiers, equivalent Graphify/SCIP scope, and
the production hard cut remain open; this is an interop correctness slice, not
a best-in-class release claim.

### Execution checkpoint (2026-08-07, bounded object-flow and object-method slice)

The candidate now covers two additional source-grounded JavaScript receiver
shapes without relaxing its fail-closed flow policy:

- A direct straight-line local assignment such as `let value; value = { ... }`
  carries the exact object receiver into later member accesses. Object spreads,
  conditional assignments, unknown writes, and compound writes remain
  unresolved; the new regression pair proves both the positive and negative
  paths.
- Stable, spread-free object-literal methods now retain their object receiver
  for `this.member` calls. Object literals with spreads do not receive this
  recovery, so a potentially overridden member is not attributed by guesswork.
- The focused candidate suite is now 94 tests and the universal resolver suite
  remains 168 tests. On the pinned Axios checker diagnostic at the same corpus
  realization, exact local target matches improved to 3,052/3,167 (115
  missing, 0 wrong); member matches are 389/503 and calls remain 1,544/1,545.
  The recovered target is a same-file object-flow member in
  `axios/tests/unit/toFormData.test.js`; this is diagnostic evidence, not a
  release precision or leadership claim.

Fixture qualification, compiler-source-oracle recall, accepted-sample/Wilson
gates, framework tiers, equivalent Graphify/SCIP scope, and the production hard
cut remain open.

### Execution checkpoint (2026-08-07, primitive structural member-write slice)

The candidate now preserves the declared property identity for a narrow,
source-proven primitive write on a stable spread-free structural object, such as
`state.flag = true`. This is intentionally narrower than general mutation:
callable- or object-valued replacements, aliases, calls, spreads, dynamic and
compound writes, and conditional/ambiguous flow remain behind the existing
fail-closed barriers. The regression exercises both the exact `response.request`
read and the exact `internals.isCaptured` literal write found in real Axios
shapes.

The focused candidate suite is now 95 tests and the universal resolver suite
remains 168 tests. On the pinned Axios checker diagnostic at the same corpus
realization, exact local target matches improved to 3,053/3,167 (114 missing,
0 wrong; 390/503 member matches and 1,544/1,545 call matches). This is
diagnostic evidence only, not a release precision or leadership claim.

Fixture qualification, compiler-source-oracle recall, accepted-sample/Wilson
gates, framework tiers, equivalent Graphify/SCIP scope, and the production hard
cut remain open.

### Execution checkpoint (2026-08-07, path-aware JavaScript escape barriers)

Flow escape evidence now respects source evaluation order and bounded execution
context. A callable argument escape is anchored at the argument occurrence,
rather than the enclosing call, so a member read evaluated before a later object
argument remains eligible. Escape barriers also retain a bounded branch/function
path: a write or escape in one conditional arm or sibling callback no longer
poisons a mutually exclusive use, while an outer or same-function use remains
fail-closed. This recovers the Axios-shaped `response.request` read that occurs
in one callback while a sibling callback passes the same structural object.

The focused candidate suite remains 95 tests and the universal resolver suite
remains 168 tests. On the pinned Axios checker diagnostic at the same corpus
realization, exact local target matches improved to 3,054/3,167 (113 missing,
0 wrong; 391/503 member matches and 1,544/1,545 call matches). Negative
coverage confirms that a direct escape before a read and a nested escape called
before a read remain unresolved. This is diagnostic evidence only, not a
release precision or leadership claim.

Fixture qualification, compiler-source-oracle recall, accepted-sample/Wilson
gates, framework tiers, equivalent Graphify/SCIP scope, and the production hard
cut remain open.

### Execution checkpoint (2026-08-07, stable callable object assignments)

Stable, spread-free structural objects now retain an exact source-backed member
when a member assignment itself introduces a callable value, for example
`validators.run = function run() {}` or `validators.check = (value) => value`.
The recovery is declaration-anchored and requires a unique property declaration
for the receiver-qualified name; non-callable writes, later overwrites, dynamic
keys, spreads, aliases with unknown mutation, and ambiguous declarations remain
behind the ordinary fail-closed mutation barrier. This keeps the positive path
useful for callable registries without turning arbitrary property writes into
guessed targets.

The focused candidate suite is now 96 tests and the universal resolver suite
remains 168 tests. The exact pinned Axios checker differential remains at
3,054/3,167 expected local targets (113 missing, 0 wrong; 391/503 member
matches and 1,544/1,545 call matches, with 226 false positives in the accepted
local candidate set). The slice therefore adds covered source shapes and
regression evidence without claiming a corpus recall increase; the Axios
validator's earlier dynamic-key mutation correctly remains conservative.

Fixture qualification, compiler-source-oracle recall, accepted-sample/Wilson
gates, framework tiers, equivalent Graphify/SCIP scope, and the production hard
cut remain open.

### Execution checkpoint (2026-08-07, ES default object export ownership)

Spread-free ES default object literals now publish a source-backed `default`
value owner and retain their exact property declarations beneath the stable
`<module>.default` qualified identity. Cross-file default imports can therefore
resolve `value.member()` and `value.member` to the provider property or method
declaration. The adapter emits an explicit default reexport binding for the
synthetic owner. Default objects containing spreads remain outside this path;
their member resolution stays unresolved/external rather than assuming that a
listed property survives an override. The candidate and resolver tests cover
the positive source-backed members and the spread negative.

Named declaration exports now also publish explicit source bindings for direct
function, class, type, interface, enum, namespace, and variable declarations,
including merged declaration slots. Ordinary value imports are admitted only
through those explicit export bindings, while private declarations remain
available to type-owner expansion; an invalid named import therefore stays
external instead of selecting a same-file private helper.

The focused candidate suite is now 97 tests and the universal resolver suite is
now 169 tests. The pinned Axios per-file target differential remains
3,054/3,167 exact local targets (113 missing, 0 wrong; 391/503 member matches,
1,544/1,545 call matches, and 226 false positives); that differential is
intentionally per-file and does not exercise cross-file resolver publication,
so no Axios recall increase is claimed for this slice.

Fixture qualification, compiler-source-oracle recall, accepted-sample/Wilson
gates, framework tiers, equivalent Graphify/SCIP scope, and the production hard
cut remain open.

### Execution checkpoint (2026-08-07, ES wildcard barrel reexports)

The ECMAScript candidate now emits bounded `export * from "./module"` and
`export * as namespace from "./module"` bindings, including `export type *`.
The resolver follows wildcard reexports transitively through barrel chains,
resolves namespace-alias members through the provider module owner, and
fail-closes on duplicate exports and cyclic barrels. Reexport traversal remains
depth- and candidate-bounded; it never selects the first surviving target.

The focused candidate suite is now 98 tests and the universal resolver suite is
now 170 tests. New regressions assert exact transitive `run` resolution,
namespace-alias member resolution, duplicate wildcard ambiguity, and cyclic
wildcard non-resolution. The native production registry remains unchanged; this
is still a qualification-only universal path.

Fixture qualification, compiler-source-oracle recall, accepted-sample/Wilson
gates, framework tiers, equivalent Graphify/SCIP scope, and the production hard
cut remain open.

## Outcome

Compass should produce the most trustworthy TypeScript and JavaScript graph for
source understanding: exact source-backed declarations and relationships,
standards-correct project/module resolution, conservative ambiguity handling,
strong framework coverage, deterministic incremental behavior, and optional
compiler-grade enrichment without making Node.js or a compiler part of the
native product boundary.

Do not define success as “more edges than Graphify.” More edges can mean more
false calls. Define and publish a named, versioned quality scorecard. Compass may
claim leadership only for the scorecard, corpus, competitors, versions, and
date actually qualified. “Better than every code graph” is not a testable or
durable release claim.

## Why this matters

Compass already has a strong TypeScript/JavaScript foundation: exact occurrence
ranges, deterministic identities, imports and reexports, workspace package
exports, several framework route packs, template extraction, and no required
language-server runtime. A recent Zod comparison was also encouraging: Compass
produced 11,322 nodes and 14,921 edges in 1.47 seconds versus Graphify's 4,338
nodes and 5,928 edges in 4.92 seconds, with no dangling or exact-duplicate
records in the Compass graph.

Those counts do not establish superiority. The comparison had no independent
stratified audit, and 1,073 Graphify hypotheses remained unsupported after the
current mapping. The current implementation also has architectural ceilings:

- TypeScript, TSX, and JavaScript still use the established direct extraction
  path in `crates/compass-languages/src/engine.rs`; only Go, Java, Python, and
  Rust have registered universal evidence adapters.
- TypeScript parsing still uses byte-preserving masks for grammar gaps such as
  variance, `export type *`, and import-type syntax.
- project evidence now accepts bounded JSONC and records alias markers, while
  the resolver has only a conservative nearest-config `baseUrl`/`paths` slice;
  it does not yet implement the full TypeScript project and module-resolution
  contract.
- JavaScript workspace resolution understands relative imports, a useful
  subset of `package.json` exports, and the new alias slice, but not the full
  per-importer Node16, NodeNext, or Bundler decision model.
- direct member resolution relies on a shallow per-file type table.
- the direct extractor can label a function passed through an array, object, or
  argument as an `indirect_call` without evidence that the receiving API invokes
  it. That is a precision risk; the safe default is a reference.
- the independent source oracle in `benchmarks/performance` supports Python,
  not TypeScript/JavaScript. The existing production precision/recall gate
  therefore cannot yet substantiate a TypeScript/JavaScript quality claim.
- offline SCIP ingestion exists, but graph projection currently collects sites
  only for Java.

Graphify is useful as a gap detector here. The inspected Graphify implementation
has richer JSONC config inheritance, `baseUrl`/`paths`, workspaces, package
exports, nearest-config selection, extension probing, and barrel handling. Its
answers are hypotheses, not ground truth, and its source must not cross the
Compass product boundary.

## Product principles for this program

1. **Two explicit quality tiers**
   - **Native structural tier**: always available, Rust-native, local-first,
     bounded, deterministic, compiler-free, and independently qualified.
   - **Verified compiler tier**: optional user-supplied fresh SCIP or a future
     explicitly invoked bounded analyzer. It may enrich exact known sites, but
     may not silently replace, fabricate, or weaken native evidence.
2. **Parser dialect is not semantic identity**: TSX is a TypeScript dialect;
   JSX is a JavaScript dialect. Preserve the dialect as provenance while using
   stable semantic language families for resolution.
3. **Interop is evidence, not a name match**: TypeScript-to-JavaScript and
   TypeScript-to-TSX links require exact import/project/package evidence. Never
   reopen generic cross-language terminal-name matching.
4. **Ambiguity is data**: emit unresolved or ambiguous diagnostics when more
   than one target survives. Never select the first export, overload, package,
   or workspace candidate.
5. **Every relationship is occurrence-backed**: preserve direction,
   multiplicity, exact byte range, enclosing declaration, resolution rule, and
   provenance.
6. **Hard cut, not dual production**: construct and qualify universal evidence
   behind test-only seams, then switch production atomically and delete the
   replaced direct facts/resolvers.
7. **Framework knowledge cannot excuse core mistakes**: qualify language and
   project semantics before expanding framework heuristics.

## Definition of “surpass”

### Mandatory production gates

Use the repository's existing universal-evidence contract as the minimum ship
bar. For TypeScript and JavaScript separately, and for their approved interop
strata, require:

- at least 2,000 independently accepted sampled relationships;
- at least 400 accepted relationships per release-gate corpus;
- at least 100 accepted relationships for each required relation and adapter
  capability;
- no target cluster larger than 10% of accepted samples;
- observed precision at least 99.5% and a 95% Wilson lower bound at least 99%;
- per-capability precision at least 99% and source-oracle recall at least 95%;
- zero fabricated occurrences, unsafe local substitutions, or unproven
  cross-language matches;
- cold, warm-cache, repeated, relocated-root, and equivalent incremental builds
  produce the same canonical graph;
- every valid file in the pinned syntax corpus either parses without recovery
  or produces a reviewed, source-bounded diagnostic with no offset corruption.

### Leadership gates

Apply these stricter gates before a public “best-in-class” claim:

- observed precision at least 99.7%, Wilson lower bound at least 99.2%, and
  precision at least 99.5% in every published capability stratum;
- source-oracle recall at least 97% in the native tier and at least 99% for
  exact target-resolution strata in the verified compiler tier;
- at least 98% of non-rejected, non-ambiguous Graphify structural hypotheses
  are supported by Compass or explained by an adjudicated semantic difference;
- zero false framework route/handler edges in the release fixture matrix and
  exact expected multiplicity for every positive route case;
- no regression greater than 10% in median wall time or peak bounded counts
  from the frozen Compass baseline without an approved quality tradeoff;
- no competitor claim without equivalent source scope, ignores, generated-file
  policy, dependency availability, compiler configuration, and graph mapping.

Graphify coverage is only a diagnostic gate because one implementation cannot
be its competitor's truth oracle. Also publish comparisons with a current SCIP
TypeScript index and a documented CodeQL capability matrix where equivalent
facts exist. Do not compare Compass structural graphs to CodeQL taint/data-flow
results as if they were the same product contract.

## Current ownership and likely change surface

| Concern | Current evidence | Owner / likely files |
|---|---|---|
| language and dialect registry | TS/TSX/JS share generic extraction | `crates/compass-languages/src/registry.rs`, `engine.rs` |
| universal adapter registry | only Go/Java/Python/Rust | `crates/compass-languages/src/adapters.rs` |
| direct typed evidence | language switch has no TS/JS branch | `crates/compass-languages/src/evidence/build.rs`, `model.rs`, `validate.rs` |
| project config markers | strict JSON and partial alias collection | `crates/compass-languages/src/project_evidence.rs`, `json_config.rs` |
| module/package resolution | relative extensions, index, exports subset | `crates/compass-resolve/src/lib.rs` |
| members and calls | per-file `ts_type_table`, direct JS passes | `crates/compass-resolve/src/members.rs`, `lib.rs` |
| universal resolution | mature evidence resolver with language specializations | `crates/compass-resolve/src/evidence.rs` |
| frameworks and templates | broad route packs and SFC extraction | `crates/compass-languages/src/frameworks/typescript.rs`, `templates.rs`, `crates/compass-resolve/src/frameworks/typescript.rs` |
| compiler artifacts | fresh, bounded SCIP ingestion; Java projection only | `crates/compass-program`, `crates/compass-resolve/src/program.rs`, `crates/compass-core/tests/program_pipeline.rs` |
| independent audit | Python source oracle only | `benchmarks/performance/compass/occurrences.py`, `audit.py`, `benchmarks/performance/tests/` |
| release fixtures | useful TS routes/workspaces; weak real-repo release gating | `tests/qualification`, `scripts/qualify_code_graph_v1.sh` |

New modules should follow the live ownership after the drift check. Prefer
focused files such as `crates/compass-languages/src/evidence/typescript.rs`,
`crates/compass-languages/src/typescript_project.rs`, and
`crates/compass-resolve/src/typescript_modules.rs` over making the already large
`engine.rs`, `evidence/build.rs`, or resolver root larger.

### Current implementation anchors

These excerpts pin the architectural seams at the planning commit. Re-read the
live code before implementation.

`crates/compass-languages/src/evidence/build.rs` has no TS/JS universal branch:

```rust
match profile.language {
    "python" => state.extract_python(root)?,
    "go" => state.extract_go(root)?,
    "java" => state.extract_java(root)?,
    "rust" => state.extract_rust(root)?,
    _ => return Err(/* no direct universal extractor */),
}
```

`crates/compass-languages/src/project_evidence.rs` parses a bounded JSONC
configuration dialect and still records only partial markers:

```rust
let Some(root) = parse_jsonc(source) else { return; };
if options.contains_key("baseUrl") { /* record key */ }
if let Some(paths) = options.get("paths").and_then(Value::as_object) {
    collect_json_aliases(paths, &mut output.aliases);
}
```

`crates/compass-resolve/src/lib.rs` bounds workspace package manifests and
exports, which should be preserved, but flattens export condition candidates
into sets and accepts only a unique admitted target. Phase 3 replaces that
subset with importer/context-aware TypeScript semantics rather than weakening
its ambiguity behavior.

`crates/compass-resolve/src/program.rs` makes the optional SCIP seam explicit:

```rust
.filter_map(|extraction| extraction.semantic_evidence.as_ref())
.filter(|batch| batch.adapter.language == "java")
```

Phase 6 should generalize this exact-anchor contract only after TS/JS are
hard-cut universal adapters; it should not add a second TS/JS graph path.

## Reference semantics

Treat these upstream specifications and tools as executor references, not
runtime dependencies:

- [TypeScript module reference](https://www.typescriptlang.org/docs/handbook/modules/reference)
  for Node16/NodeNext/Bundler resolution, package `exports`/`imports`, self-name
  imports, file formats, and condition selection;
- [TypeScript project references](https://www.typescriptlang.org/docs/handbook/project-references.html)
  and [`paths`](https://www.typescriptlang.org/tsconfig/paths.html);
- [SCIP protocol](https://github.com/scip-code/scip) and
  [scip-typescript](https://github.com/sourcegraph/scip-typescript) for optional
  compiler evidence and interoperability;
- [CodeQL supported languages and frameworks](https://codeql.github.com/docs/codeql-overview/supported-languages-and-frameworks/)
  as a contemporary framework-coverage checklist, not a structural truth set.

Pin every qualification tool version and record its digest. Re-run against the
current stable TypeScript release before a leadership claim because module and
syntax behavior changes over time.

## Commands and environment

All Cargo commands that compile must use this worktree's unique external target:

`CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923`

Verify `/Volumes/Workspace` is mounted and the directory is writable before the
first build. Stop rather than falling back to `target/`.

| Purpose | Command | Expected result |
|---|---|---|
| language tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 cargo test -p compass-languages --locked` | exit 0 |
| resolver tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 cargo test -p compass-resolve --locked` | exit 0 |
| program/core tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 cargo test -p compass-program -p compass-core --locked` | exit 0 |
| qualification unit tests | `python3 -m unittest benchmarks.performance.tests.test_audit benchmarks.performance.tests.test_correctness benchmarks.performance.tests.test_language_fixture_compare` | exit 0 |
| fixture qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 ./scripts/qualify_code_graph_v1.sh --fixtures-only` | all cases pass |
| JavaScript package checks | `npm ci && npm run typecheck:js && npm run test:js` | exit 0 |
| product boundary | `sh scripts/check_product_boundary.sh` | no Graphify/runtime boundary violation |
| format | `cargo fmt --all -- --check` | exit 0 |
| workspace lint | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 cargo clippy --workspace --lib --bins --locked -- -D warnings` | exit 0 |
| workspace tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 cargo test --workspace --lib --bins --locked` | exit 0 |

Each phase below adds a narrower verification command. Keep those commands in
the final PR description with the exact commit, tool versions, and corpus pins.

## Scope

**In scope**:

- TS, TSX, MTS, CTS, JavaScript, JSX, MJS, CJS, and declaration-file identity;
- parser conformance and exact source ranges;
- lexical scopes, TypeScript value/type/namespace identities, declaration
  merging, imports, exports, reexports, calls, construction, bases, members,
  receivers, ownership, and external references;
- JSONC project configs, inheritance, project references, workspace packages,
  and standards-correct module modes;
- conservative JS/TS interop and ambiguity diagnostics;
- current framework/template packs plus evidence-driven framework expansion;
- an independent TypeScript compiler-backed qualification oracle;
- optional, freshness-verified SCIP projection after native hard cut;
- deterministic, incremental, bounded, real-corpus qualification and public
  support documentation.

**Out of scope**:

- Graphify code, runtime, fixtures, configuration, or CI as a dependency;
- requiring Node.js, `tsserver`, TypeScript, SCIP, credentials, or network access
  for a normal Compass structural build;
- speculative terminal-name, fuzzy, or “first candidate” resolution;
- whole-program taint tracking, security queries, or general value/data-flow
  parity with CodeQL;
- a graph schema major-version change without a separate approved compatibility
  design and migration plan;
- automatic package installation or compiler invocation;
- framework heuristics that cannot emit exact source evidence and negative
  cases;
- a timeless “better than every code graph” marketing claim.

## Git and delivery workflow

- Program branch prefix: `advisor/013-typescript-javascript-graph-quality`.
- Use one branch/PR per phase; do not keep an XL branch alive across the program.
- Keep contracts/fixtures, implementation, qualification evidence, and docs
  separately reviewable where practical.
- Use repository-style commit messages such as `feat: emit universal typescript evidence`,
  `fix: resolve nodenext package imports exactly`, and
  `perf: bound typescript project indexing`.
- Do not push, publish, or open a PR unless instructed.
- A phase is not complete because its happy-path tests pass. Its negative,
  ambiguity, limit, cold/warm, and relocated-root cases must pass too.

## Phase map

```text
0. Scorecard + independent oracle + HEAD baseline
                         |
1. ECMAScript identity + typed project model
                         |
2. Parser conformance + test-only universal evidence emitter
                         |
3. Standards-correct module/package resolution
                         |
4. Scope, symbol, call, construction, and member resolution
                         |
5. Atomic production hard cut; remove replaced direct path
                         |
6. Optional verified compiler/SCIP enrichment
                         |
7. Framework/template migration and evidence-driven expansion
                         |
8. Real-corpus qualification, performance, release claim
                         |
9. Continuous conformance and regression maintenance
```

Phases 1–4 may expose test-only universal APIs, but the production registry must
remain on exactly one path until phase 5.

## Phase 0: Establish truth before changing extraction

### Context

The existing benchmark can count and map graphs, but only Python has an
independent source oracle. Re-running Compass against Graphify alone would
optimize toward Graphify's choices, including its mistakes. This phase creates
the measurement system that decides all later work.

### Actions

1. Define `typescript-javascript-code-graph-quality-v1` as a versioned manifest
   under `tests/qualification` or the live code-graph qualification location.
   Record corpus commit, license, source roots, ignores, generated-file policy,
   config entry points, TypeScript version, module mode, expected language
   family, and required capability strata.
2. Add a developer-only TypeScript compiler source oracle, preferably
   `benchmarks/performance/oracles/typescript-source-oracle.mjs`. It must:
   - load the pinned compiler from the repository lockfile;
   - build the same projects/file set as the manifest, including project
     references;
   - emit deterministic sorted JSONL for files, declarations, scopes, imports,
     reexports, calls, constructions, bases, references, members, and exact
     owner/source ranges;
   - translate TypeScript UTF-16 positions to source byte ranges and test
     Unicode/surrogate pairs explicitly;
   - report unsupported/incomplete facts and compiler diagnostics rather than
     silently dropping them;
   - enforce file, source-byte, fact, diagnostic, time, memory/output, and
     project-depth limits;
   - include compiler version, script digest, config digest, and source digest.
3. Integrate this oracle into `benchmarks/performance/compass/occurrences.py`,
   the audit pipeline, and its focused tests. A partially indexed project must
   fail the recall denominator instead of appearing to have perfect recall.
4. Freeze four statistically audited release corpora with complementary shapes:
   a type-heavy library (Zod), a mixed JS/TS library (Axios), a module/bundler
   workspace (Vite), and a decorator/framework workspace (NestJS or an
   equivalently licensed pinned corpus). Keep larger ecosystem repositories in
   an extended regression tier until audit cost is sustainable.
5. Rebuild current HEAD and compare it against the compiler oracle, the pinned
   Graphify CLI, and a pinned SCIP TypeScript output. Classify every miss into
   parser, identity, project config, module selection, reexport, scope,
   receiver/member, framework, mapping, competitor error, or true ambiguity.
6. Preserve the current Zod/Axios reports as historical diagnostic evidence,
   but do not use their raw edge counts as the phase baseline. Record a new
   exact-HEAD baseline with manual stratified audit and Wilson intervals.

### Verification

- Oracle tests cover LF/CRLF, Unicode, malformed projects, JSONC, project
  references, compiler diagnostics, output limits, timeout, deterministic
  ordering, and relocated roots.
- Two identical runs have byte-identical normalized oracle output.
- Removing one project/file from an oracle result causes coverage validation to
  fail.
- The audit produces precision, Wilson lower bound, source-oracle recall, and
  per-capability/per-corpus results rather than one aggregate count.

### Exit gate

Do not begin semantic optimization until the manifest, independent oracle,
baseline, miss taxonomy, and adjudication workflow are reviewed. The baseline
may be below the final gates; the measurement itself may not be incomplete.

## Phase 1: Model ECMAScript identity and projects explicitly

### Context

The registry currently treats `typescript`, `tsx`, and `javascript` as distinct
language names, while universal resolution deliberately forbids unsupported
cross-language matches. A safe hard cut therefore needs a clear distinction
between parser dialect, semantic language, and evidence-proven interop. Project
configuration also needs a typed model rather than framework activation markers.

### Actions

1. Add stable dialect metadata for TypeScript, TSX, MTS, CTS, JavaScript, JSX,
   MJS, and CJS. Normalize TSX/MTS/CTS to semantic language `typescript` and
   JSX/MJS/CJS to `javascript`; preserve the original dialect and module-format
   decision as provenance.
2. Define an explicit `EcmaScriptInterop`/equivalent resolution rule. It may
   cross TypeScript/JavaScript families only through an exact import, export,
   project `allowJs`/`checkJs`, package, or compiler-artifact binding. It must
   never enable global terminal-name matching.
3. Replace partial config markers with a versioned typed project model owned by
   `compass-languages` or the lowest project-evidence owner. Parse bounded JSONC
   using the repository's existing safe config primitives where possible.
4. Model nearest-config ownership, `extends` including arrays and packages,
   cycle/depth detection, `references`, `files`/`include`/`exclude`, `allowJs`,
   `checkJs`, `baseUrl`, `paths` fallback arrays, `rootDirs`, `typeRoots`,
   `module`, `moduleResolution`, `moduleSuffixes`, `resolveJsonModule`, and
   relevant custom conditions.
5. Build a bounded package/workspace inventory from source already admitted by
   Compass: nearest `package.json`, package name/type, workspace declarations,
   `main`/`module`/`types`, `exports`, `imports`, and project/package ownership.
   Do not read arbitrary paths outside the admitted workspace inventory.
6. Give the project model a schema/cache version and deterministic canonical
   ordering. Unknown semantics, duplicate projects/packages, cycles, or limits
   must be typed diagnostics, not empty configuration.

### Verification

- Focused tests cover TSX/JSX family identity, `.d.ts`, `.mts`/`.cts`, mixed
  `allowJs`, nearest config, nested packages, JSONC comments/trailing commas,
  multi-level and cyclic `extends`, path fallback arrays, project references,
  duplicate package names, symlink/path containment, and every configured limit.
- Equivalent relocated workspaces produce identical project identities and
  canonical model JSON.
- A TypeScript file cannot resolve an unrelated JavaScript symbol by name alone.

### Exit gate

The typed project model must be usable by both the legacy production path and
the test-only universal path without duplicating config semantics.

## Phase 2: Reach syntax conformance and emit universal evidence

### Context

`crates/compass-languages/src/evidence/build.rs` currently dispatches direct
universal extraction only for Python, Go, Java, and Rust. The TypeScript/JS
extractor must emit typed evidence directly from the parser tree; translating
legacy raw graph records is not an acceptable adapter.

### Actions

1. Create a pinned syntax-conformance corpus from the TypeScript compiler tests
   and small reviewable local fixtures. Cover current stable syntax and the
   repository's supported language-version policy.
2. Audit and update the vendored tree-sitter TypeScript/JavaScript grammars with
   provenance, license, version, and deterministic build changes. Keep runtime
   parsers statically linked.
3. Retain a byte-preserving parser mask only for a documented upstream grammar
   gap with exact original-to-parser offset equivalence, dedicated positive and
   negative tests, and an explicit deletion condition. Prefer eliminating the
   existing variance, type-star, and import-type masks when the grammar permits.
4. Add focused TypeScript and JavaScript universal evidence emitters. Emit
   exact declarations, scopes, bindings, occurrences, aliases, direct-base
   candidates, receiver/type evidence, and resolution candidates for:
   - functions, arrow/function expressions, overloads, methods, accessors,
     constructors, fields, properties, parameters, and destructuring;
   - classes, interfaces, type aliases, enums, namespaces/modules, namespace
     augmentation, declaration merging, generics, and ambient declarations;
   - separate value/type/namespace identities and `import type`/`export type`;
   - default/named/star exports, CommonJS imports/exports, dynamic literal
     imports, and reexports;
   - calls, optional calls, `new`, base types, decorators, type references,
     ownership, receiver evidence, and external references;
   - JSX/TSX component references with exact tag occurrence. Until a new graph
     relation is explicitly approved, publish these as `references` with typed
     JSX context rather than inventing `calls` or `renders`.
5. Emit parser-recovery diagnostics with exact affected ranges and evidence
   completeness. Never publish a construct outside a source-backed range just
   because the root parser recovered.
6. Expose the adapter only to focused tests/qualification at this phase. Do not
   add it to `UNIVERSAL_ADAPTERS` yet.

### Verification

- Add language tests beside `engine_edge_coverage.rs` and
  `universal_evidence.rs` for every capability, including duplicate names,
  shadowing, overloads, declaration merging, type/value namespace collisions,
  anonymous/default exports, Unicode, malformed input, repeated occurrences,
  and limit failures.
- Differentially compare compiler and tree-sitter source ranges on the syntax
  corpus. Valid files have no unexplained recovery and no byte drift.
- Universal evidence validates without consuming `RawNodeRecord`,
  `RawEdgeRecord`, or `RawCall`.
- Two runs produce byte-identical sorted evidence batches.

### Exit gate

Every capability claimed in the future adapter profiles has direct validated
evidence and a positive, ambiguity, negative, and boundary test. Do not register
a capability that is merely inferred later from a legacy graph edge.

## Phase 3: Implement standards-correct module and package resolution

### Context

The current resolver's relative extension/index behavior and package-exports
support are valuable, but TypeScript/JavaScript target quality depends on the
importer's project, module mode, package format, import kind, and condition set.
Graphify currently covers more of the config/workspace surface; this is the
largest clear competitive gap.

### Actions

1. Move JavaScript/TypeScript module logic from the resolver root into a bounded
   focused owner such as `typescript_modules.rs`. Feed it the typed project and
   package inventory from phase 1 and universal binding candidates from phase 2.
2. Implement the official per-importer decision model for Classic only if still
   supported, Node10 where required, Node16, NodeNext, and Bundler. Record the
   selected mode and rule on every resolved binding.
3. Resolve exact relative paths, allowed extension substitution, index files,
   `rootDirs`, config `paths` fallback arrays, `baseUrl`, package self-name,
   package `imports`, package `exports`, `typesVersions`, `types`/`main`/`module`,
   workspace packages, project references, and admitted dependency sources in
   their documented precedence.
4. Respect importer and target ESM/CJS format, import versus require context,
   type-only versus value context, active conditions/custom conditions, file
   extensions, and declaration/source substitution. Do not union mutually
   exclusive condition targets and then choose one later.
5. Support named/default/star exports, export assignment, CommonJS property
   exports, literal dynamic imports, and bounded barrel chains/cycles. Preserve
   every occurrence and its multiplicity.
6. Resolve only within Compass's admitted source/package inventory. A package
   name present in two workspaces, wildcard matching more than one target, an
   unsupported conditional branch, or a limit must remain ambiguous/unresolved
   with a diagnostic.
7. Preserve third-party targets as explicit external identities when source is
   unavailable. Never redirect an external import to a same-named local symbol.

### Verification

- Build a table-driven fixture suite from the official TypeScript module
  reference. Cover every mode/context pair, `package.json` type boundary,
  `.ts`/`.tsx`/`.mts`/`.cts`/`.js`/`.jsx`/`.mjs`/`.cjs`/`.d.ts`, package
  imports/exports/self-name, wildcards, condition order, paths fallbacks,
  rootDirs, workspaces, project references, barrel cycles, and duplicates.
- Add a differential developer test against `tsc --traceResolution` for pinned
  fixtures. Normalize traces into expected decisions; do not call the compiler
  in production tests or runtime.
- Assert target identity, direction, exact import/export range, resolution rule,
  ambiguity candidates, multiplicity, and deterministic ordering—not just edge
  counts.
- Extend existing workspace/reexport tests in
  `crates/compass-resolve/tests/universal_resolution.rs` without weakening their
  duplicate-package fail-closed behavior.

### Exit gate

The independent source oracle must show at least 99% precision and 97% recall in
the module/import/reexport strata before phase 5. Every unsupported official
case must have a named diagnostic and documented support status.

## Phase 4: Resolve symbols, calls, construction, and members conservatively

### Context

The current direct resolver has useful import-aware calls and typed members,
but its type table is shallow and indirect-call inference is too permissive for
a leadership precision target. The universal resolver already supplies strong
ambiguity, ownership, receiver, hierarchy, and resolution-rule primitives; add
TypeScript-specific evidence rather than rebuilding a parallel resolver.

### Actions

1. Model lexical scopes, hoisting/visibility, shadowing, closure capture,
   module/script scope, `this`, `super`, aliases, value/type/namespace separation,
   declaration merging, and overload groups using stable typed facts.
2. Resolve direct/imported/namespace/static/instance/constructor/callable-value
   calls, optional chaining, computed literal members, tagged templates where
   call semantics are exact, `new`, and `super` calls. Preserve unresolved
   dynamic calls rather than guessing.
3. Extend receiver evidence from parameters, locals, fields, constructor
   assignments, type assertions/narrowings that are source-proven, generic
   bounds, return types, and bounded chained call results. Use arity and exact
   type compatibility only when it uniquely selects an overload.
4. Model class/interface inheritance, implementation, overrides, mixins, and
   prototype/static members only when declaration or compiler evidence proves
   the link. Do not infer dynamic prototype graphs from matching names.
5. For JavaScript, add bounded flow-sensitive local assignment tracking where
   it proves a unique binding. Bail out on dynamic writes, alias escape,
   `eval`, `with`, unsupported proxies, or candidate-limit overflow.
6. Change the generic “function passed in an array/object/argument” result from
   `indirect_call` to an exact `references` edge. Only a versioned API/framework
   invocation contract may turn registration into an indirect call, and that
   contract must identify the callee API, argument position/shape, callback
   occurrence, handler target, and negative cases.
7. Attach an explicit resolution rule and confidence/provenance class to every
   resolved relationship. Preserve multiple call occurrences even when they
   share endpoints.

### Verification

- Add focused tests for nested/shadowed functions, recursion, overloads,
  declaration merging, imported aliases, namespace calls, constructors,
  optional chains, generic receivers, inheritance/overrides, field/return
  chains, JS reassignment, dynamic property negatives, callbacks, repeated
  sites, Unicode ranges, and ambiguous duplicate candidates.
- A callback stored or passed to an unknown API produces a reference and no
  indirect call. Known framework contracts produce the exact expected handler
  edge and no edge when the API/argument shape differs.
- The source oracle meets mandatory per-capability gates for calls,
  construction, members, bases, and references; no capability hides behind the
  aggregate result.
- Target-cluster and unsafe-substitution audit counters remain zero or below
  the mandatory contract as applicable.

### Exit gate

All universal TypeScript/JavaScript capabilities pass their mandatory minimum
sample counts, precision, Wilson, recall, and negative gates in test-only mode.

## Phase 5: Perform the atomic production hard cut

### Context

This is the compatibility-sensitive phase. It is a deletion and ownership
phase as much as a registration phase. Running both paths would create duplicate
or conflicting graphs and conceal semantic drift.

### Actions

1. Add separate `compass.typescript` and `compass.javascript` adapter profiles
   with exact capability lists and new adapter versions. Map TSX/MTS/CTS and
   JSX/MJS/CJS through their semantic family while preserving dialect evidence.
2. Switch production extraction to direct parser-tree universal evidence in one
   commit. Do not publish legacy raw and universal facts in the same build.
3. Route project-wide module, call, construction, base, reference, and member
   candidates through the shared evidence resolver plus the focused module
   resolver.
4. Delete or disable replaced TS/JS branches in `engine.rs`, `members.rs`, and
   resolver root, including redundant `ts_type_table`, JS-specific target maps,
   raw-call paths, and unsafe indirect-call inference. Retain only behavior with
   a clearly different ownership boundary, such as Program IR or project model.
5. Migrate existing route/template producers to consume or attach universal
   evidence without changing their published semantics. Phase 7 can deepen
   them after parity is proven.
6. Bump every affected extraction/cache/fingerprint version. Review graph JSON,
   stable IDs, history realization fingerprints, CLI/MCP output, and incremental
   manifests under `COMPATIBILITY.md`.
7. Add `MIGRATION.md` and `CHANGELOG.md` entries if users must rebuild cached
   graphs or stable identities necessarily change. Prefer identity preservation
   where semantic identity is unchanged, but never preserve an incorrect ID by
   fabricating equivalence.

### Verification

- Cold, warm, repeated, relocated, incremental edit/add/delete/rename, and
  project-config/package-manifest changes publish coherent equivalent graphs.
- Fixture comparison accounts for every legacy relation: preserved, corrected
  with reviewed contract change, or intentionally removed as unproven.
- Search confirms no production TS/JS dual extraction and no replaced direct
  resolver tables remain.
- Run language, resolver, graph, core, CLI contract, fixture qualification,
  product-boundary, format, lint, and workspace tests from the command table.

### Exit gate

The native production graph must pass all mandatory gates with no regression in
existing route/template fixtures. Roll back the cutover commit if the gate fails;
do not add a runtime fallback to the legacy path.

## Phase 6: Add optional verified compiler and SCIP enrichment

### Context

Native syntax/project evidence should remain the default. Compiler indexes can
raise exact target recall for overloads, generics, declaration merging, aliases,
dependencies, and JS/TS interop. Compass already has bounded SCIP decoding,
freshness manifests, source digests, Program evidence, and exact-anchor Java
projection; reuse that seam.

### Actions

1. Generalize program projection in `crates/compass-resolve/src/program.rs`
   from Java-only site collection to a language-neutral exact-anchor contract,
   then opt in hard-cut TypeScript/JavaScript with focused policy tests.
2. Join SCIP definition/reference/call facts only to an existing exact source
   file and byte range, declaration occurrence, call occurrence, construction
   occurrence, or import binding. Reject stale sources, missing manifests,
   conflicting local targets, wrong language/project identity, and offset drift.
3. Preserve provider/artifact identity, SCIP symbol, source digest, projection
   rule, and structural evidence provenance. An enriched edge must not erase an
   independently valid native relationship.
4. Require unanimous or explicitly modeled non-conflicting provider evidence
   for a local exact target. Runtime dispatch sets may remain multiple; do not
   collapse them into one convenient method.
5. Add TypeScript/JavaScript cases to `crates/compass-program/tests/scip.rs` and
   `crates/compass-core/tests/program_pipeline.rs`: definitions, references,
   overloads, JSX, JS/TS interop, Unicode, stale artifact, source edit, config
   edit, conflict, missing target, limit, cold/warm cache, and relocation.
6. Keep `scip-typescript` outside normal execution. A future analyzer command
   must be separately approved, explicit opt-in, argument-vector based,
   time/output/memory bounded, version-pinned, and never auto-install packages.

### Verification

- A native build remains byte-equivalent and succeeds when Node/SCIP/compiler
  binaries are absent.
- A fresh artifact adds only exact-anchor facts and raises the compiler-tier
  target-resolution recall to the leadership threshold.
- Stale, conflicting, truncated, oversized, or offset-mismatched artifacts fail
  explicitly or are ignored according to the existing typed contract; they
  never publish a plausible partial enrichment.
- Product-boundary and credential/network-free fixture qualification pass.

### Exit gate

Document the two quality tiers separately. Do not describe compiler-enriched
recall as native Compass recall.

## Phase 7: Migrate and deepen framework and template semantics

### Context

Compass already recognizes Express, Fastify, Hono, Nest, React Router, Next,
Remix, Vue Router, Nuxt, SvelteKit, Astro, Vite, and template scripts. Preserve
that lead while moving every relationship onto the exact evidence contract.

### Actions

1. Migrate existing packs atomically to universal descriptors/resolution while
   preserving route path, HTTP method, handler direction, source occurrence,
   multiplicity, aliasing, and negative behavior.
2. Define versioned invocation contracts for framework callback APIs. Examples
   include Express/Fastify/Hono route handlers, Nest decorators, and router
   configuration arrays. Each contract needs exact activation/import evidence,
   argument position/shape, callback semantics, and near-miss tests.
3. Preserve source-map/host mapping for Vue, Svelte, and Astro embedded scripts.
   Every declaration and relationship must point to exact host-file bytes; a
   generated virtual offset is not acceptable publication evidence.
4. Add framework support based on audited user value and missing capability
   strata, not checkbox parity. Candidate next packs are Angular routing and DI,
   Koa routing/middleware, React component/hook relationships, and high-value
   build/config entry points. Each is a separate focused PR after core quality.
5. Distinguish routing, registration, rendering/reference, dependency injection,
   and ordinary calls. Reuse existing graph relations where semantics fit;
   propose any new stable relation through the normal schema/compatibility
   process rather than encoding it in an arbitrary attribute.

### Verification

- Extend `crates/compass-resolve/tests/typescript_routes.rs` and template tests
  with imported/default/aliased handlers, arrays, nested routers, duplicate
  routes, computed/dynamic negatives, malformed templates, and exact host ranges.
- Require 100% expected precision and multiplicity on the reviewed framework
  fixture matrix before advertising a framework as supported.
- Run the compiler oracle where source semantics exist, but maintain
  Compass-owned framework truth fixtures for behavior the compiler cannot know.

### Exit gate

Every advertised framework has a documented capability/status, positive and
negative fixtures, exact route/handler evidence, limits, and real-corpus smoke
coverage. Unsupported dynamic behavior is named honestly.

## Phase 8: Qualify release leadership and performance

### Context

This phase turns implementation into a defensible claim. It must run against
the exact release candidate, not an earlier feature commit or a working tree.

### Actions

1. Promote the four audited corpora to pinned release gates after license and
   disk policy review. Store external read-only repositories beneath
   `/Volumes/Workspace/Github/<owner>/<repository>`; keep generated graphs and
   audit artifacts outside tracked sources.
2. Add extended regression corpora for a large monorepo, current TypeScript
   compiler syntax, React/Next, Angular, Vue/Nuxt, Svelte/SvelteKit, and Astro.
   These may be scheduled until they meet the reviewed release-gate standard.
3. Run native and compiler tiers through cold, warm, repeated, relocated,
   incremental edit/add/delete/rename, config/package change, malformed input,
   ambiguity, and all limit paths.
4. Publish machine-readable precision, Wilson interval, recall, per-capability,
   per-corpus, miss taxonomy, unresolved/ambiguous counts, parser recovery,
   time, source bytes, nodes/edges/occurrences, peak bounded counts, and tool
   versions. Include the exact Compass commit and hardware for performance.
5. Run Graphify, SCIP TypeScript, and any other approved comparator with
   equivalent scope/config. Store their versions and mappings. Manually
   adjudicate a stratified sample; do not turn unsupported competitor facts into
   automatic Compass bugs.
6. Update `PERFORMANCE.md`, the TypeScript/JavaScript support reference,
   `COMPATIBILITY.md`, `CHANGELOG.md`, and qualification documentation. Use a
   precise claim such as: “Compass leads the published TypeScript/JavaScript
   Code Graph Quality v1 scorecard as of YYYY-MM-DD.” Link the manifest,
   methodology, versions, and full results.
7. Wire the exact-commit qualification into the release gate. A release binary
   must be the binary audited; evidence from an ancestor commit is insufficient.

### Verification

- All mandatory and leadership gates pass on the exact release candidate.
- A deliberately injected wrong target, lost occurrence, stale SCIP artifact,
  missing corpus, unsupported skip, or different binary SHA makes the gate fail.
- Median and variability are reported; CI does not adopt a brittle wall-clock
  threshold from one developer machine.
- `git status --short` contains no generated graph, `.compass`, credentials,
  private sources, or external corpus files.
- Run every applicable command in the command table and `git diff --check`.

### Exit gate

Publish no leadership statement if any mandatory gate, per-capability minimum,
corpus minimum, exact-SHA check, or equivalent-configuration review fails.

## Phase 9: Keep support state of the art

1. Test the current stable and next TypeScript syntax/module behavior on a
   scheduled developer/CI lane. A next-version failure informs planning; it does
   not silently change the stable support contract.
2. Review vendored grammar provenance and compiler-oracle compatibility on every
   supported TypeScript upgrade.
3. Track quality by capability and miss reason. Set a zero budget for fabricated
   occurrences, unsafe substitutions, path escapes, nondeterminism, and critical
   precision regressions.
4. Refresh competitor versions and the public scorecard at least quarterly and
   before repeating a leadership claim. Keep historical results immutable.
5. Add the smallest reviewable positive, ambiguity, negative, and limit fixture
   for every new syntax, module rule, resolver behavior, or framework contract.
6. Never lower a threshold to absorb a regression. Fix the issue, narrow an
   overstated support claim, or version the qualification contract with review.

## Cross-phase test plan

- **Classification**: all extensions/dialects, shebangs where applicable,
  declaration files, scripts versus modules, ESM/CJS boundaries.
- **Parser**: current syntax corpus, recovery, masks, Unicode/CRLF, malformed
  constructs, source-byte preservation.
- **Evidence**: declarations, scopes, identities, imports/reexports, aliases,
  calls/construction, types/bases, members/receivers, ownership, references,
  decorators, JSX, exact anchors, multiplicity, provenance.
- **Projects/modules**: JSONC/extends/references, paths/baseUrl/rootDirs,
  workspaces, package exports/imports/self-name, conditions, Node16/NodeNext/
  Bundler, ESM/CJS, barrels, external identities, ambiguity and limits.
- **Resolution**: shadowing, overloads, declaration merging, JS flow, imported
  aliases, receiver/member chains, callbacks, dynamic negatives.
- **Frameworks/templates**: route/handler contracts, activation negatives,
  aliases, nested configs, embedded-source host anchors.
- **Compiler enrichment**: freshness, exact anchor, conflict, Unicode, stale,
  truncated/oversized, cold/warm, relocated and native-without-tool behavior.
- **Publication**: graph validation, deterministic order/IDs, cache fingerprints,
  incremental add/edit/delete/rename/config changes, coherent atomic output.
- **Qualification**: independent oracle completeness, manual stratification,
  Wilson intervals, target clusters, source-oracle recall, competitor mapping,
  exact release SHA, bounded performance.

## Done criteria

- [ ] TypeScript and JavaScript have direct parser-tree universal adapters; no
  production adapter translates legacy raw graph records.
- [ ] Parser dialect and semantic language family are distinct and exact JS/TS
  interop never enables generic cross-language matching.
- [ ] The typed project model covers the supported TypeScript config, workspace,
  and package contract with deterministic bounded failures.
- [ ] Module resolution matches the documented TypeScript decision model for
  every advertised mode and import context.
- [ ] Calls, construction, bases, members, references, and callbacks meet every
  mandatory per-capability quality gate.
- [ ] Unknown callback passing produces references, not invented indirect calls.
- [ ] The hard cut removed replaced TS/JS direct facts/resolvers and never dual
  publishes production graphs.
- [ ] Normal structural builds remain native, offline, compiler-free, and
  Graphify-free.
- [ ] Optional SCIP/compiler evidence is fresh, bounded, exact-anchor,
  provenance-preserving, and separately reported.
- [ ] Existing and newly advertised framework/template cases have exact positive
  and negative qualification.
- [ ] Cold/warm/repeated/relocated/incremental graphs are canonically equivalent.
- [ ] The exact release candidate passes mandatory and leadership scorecard
  gates on pinned, licensed, reviewable corpora.
- [ ] Performance and superiority claims cite reproducible versions, mappings,
  scope, results, hardware/date, and limitations.
- [ ] Format, targeted tests, workspace lint/tests, fixture qualification,
  product-boundary, docs, and diff checks pass.
- [ ] Plan 013 is marked `DONE` in `advisor-plans/README.md` only after all
  phases and the release gate complete.

## STOP conditions

Stop and report rather than weakening the design if:

- the source oracle is incomplete, shares Compass extraction logic, or cannot
  prove its file/range coverage;
- a parser workaround changes byte offsets or valid pinned syntax has
  unexplained recovery;
- resolution would need terminal-name matching, arbitrary filesystem search,
  first-candidate selection, or an unbounded barrel/project/package traversal;
- a normal build would require Node.js, a compiler, SCIP, credentials, network,
  runtime grammar downloads, or Graphify;
- the universal adapter cannot pass mandatory gates before production cutover;
- the hard cut requires dual production publication or an automatic legacy
  fallback;
- an existing framework route/template contract regresses without an approved
  compatibility decision;
- a graph schema major change or stable-ID break is required without an approved
  migration plan;
- competitor inputs cannot be configured equivalently enough for a defensible
  comparison;
- performance results are too noisy or incomplete for a public claim;
- `/Volumes/Workspace` is unavailable for a Cargo build or real-corpus checkout;
- pre-existing user changes cannot be preserved.

## Maintenance notes

- Adapter, project-model, evidence, cache, and qualification versions are
  contracts. Bump the narrowest version when meaning changes and reject unknown
  majors explicitly.
- Keep historical corpus realizations and published scorecards immutable. Add a
  new realization instead of rewriting results in place.
- Track current stable TypeScript support separately from preview syntax.
- Revalidate package/module semantics when TypeScript changes supported modes or
  conditions.
- Keep generated benchmark artifacts outside the repository and qualification
  repositories read-only.
- Prefer a smaller graph with exact unresolved diagnostics over a larger graph
  containing plausible but unproven calls.
