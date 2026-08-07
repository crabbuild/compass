# TypeScript/JavaScript qualification oracles

These Node-side tools are independent qualification inputs. They are not
Compass runtime dependencies, are not invoked by normal Cargo tests, and must
not be copied into the product boundary.

The pinned provider is TypeScript 5.9.3. Both tools emit deterministic JSON,
exact UTF-8 byte ranges, source/config digests, bounded diagnostics, and an
explicit parsed/rejected-file count:

- `typescript-source-oracle.mjs` records source constructs for declarations,
  imports/reexports, calls, construction, members, bases, type references, and
  JSX. It does not select graph targets.
- `typescript-resolution-oracle.mjs` records compiler-API import/reexport,
  dynamic-import, `import =`, and literal-`require()` decisions, including
  module mode, project ownership, source/external/unresolved/ambiguous outcome,
  and the exact target file where available.
- `typescript-target-oracle.mjs` uses the pinned checker to adjudicate exact
  source declaration targets for calls, construction, members, heritage, type
  references, and JSX. It is deliberately a qualification-only target oracle;
  its bounded synthetic program is not a substitute for project-aware
  Node16/NodeNext/Bundler resolution.

Run them against an external, read-only corpus after `npm ci`:

```bash
node benchmarks/performance/oracles/typescript-source-oracle.mjs \
  --root /Volumes/Workspace/Github/<owner>/<repository>
node benchmarks/performance/oracles/typescript-resolution-oracle.mjs \
  --root /Volumes/Workspace/Github/<owner>/<repository>
```

The ignored Rust differential harness compares the source oracle's accepted
construct ranges with the test-only Compass candidate emitter:

```bash
COMPASS_TS_QUALIFICATION_ROOT=/Volumes/Workspace/Github/<owner>/<repository> \
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 \
cargo test -p compass-languages --test typescript_corpus_differential \
  --locked -- --ignored --nocapture
```

Coverage is source-occurrence recall only. It does not prove target precision,
resolution correctness, framework semantics, Graphify parity, or a production
hard cut. A corpus must be pinned by commit, licensed, configuration-complete,
and independently adjudicated before it can enter the Plan 013 release gate.

Run target adjudication explicitly (it is ignored so normal native tests remain
Node-free):

```bash
COMPASS_TS_QUALIFICATION_ROOT=/Volumes/Workspace/Github/<owner>/<repository> \
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 \
cargo test -p compass-languages --test typescript_target_differential \
  --locked -- --ignored --nocapture
```

The harness reports exact local-target recall, wrong-target cases, unresolved
cases, and local false positives by capability. These figures are
adjudication evidence, not release thresholds until accepted samples, Wilson
intervals, and framework/competitor strata are frozen in Plan 013.
