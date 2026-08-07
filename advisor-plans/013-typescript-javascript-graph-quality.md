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
