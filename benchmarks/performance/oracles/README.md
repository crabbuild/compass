# Independent qualification oracles

## TypeScript and JavaScript

These Node-side tools are independent qualification inputs. They are not
Compass runtime dependencies, are not invoked by normal Cargo tests, and must
not be copied into the product boundary.

The pinned provider is TypeScript 5.9.3. Both tools emit deterministic JSON,
exact UTF-8 byte ranges, source/config digests, bounded diagnostics, and an
explicit parsed/rejected-file count. Source paths, project references and
rejected-file lists use UTF-8 byte ordering, matching the Python consumer even
for mixed-case and supplementary Unicode filenames. Record the producer script
SHA when reusing oracle evidence; corrected ordering can change inventory
digests without changing typed source facts. The source oracle also supports a
record-oriented JSONL stream (`--jsonl`) with a header, project/file coverage,
diagnostic/construct records, and a checked footer; the audit pipeline consumes
that stream so a truncated or incomplete denominator fails closed. The source oracle discovers bounded
`tsconfig*.json`/`jsconfig*.json` projects, follows in-root project references
with cycle/depth limits, and selects the compiler's project file set. If every
discovered configuration is invalid it reports diagnostics and falls back to
the bounded source tree instead of silently returning an empty inventory.

The JSONL stream is schema `compass.typescript-source-oracle-jsonl/3`. In
addition to the legacy flattened `construct` records, it emits independently
validated `scope`, `declaration`, `call`, `construction`, `import`,
`reexport`, `base`, `member`, and `reference` records. Declarations retain
value/type/namespace identity and bounded parameter-shape metadata; scopes
retain deterministic parent IDs; calls and constructions retain exact target
anchors and full invocation ranges; imports/reexports retain module and
binding identity; bases, members, and references retain enclosing owners and
source ranges. Header/footer counts cover every typed record, and the Python
audit rejects missing parents, duplicate IDs, invalid ranges, unknown fields,
and count/digest mismatches.

- `typescript-source-oracle.mjs` records source constructs for declarations
  (including `using`/`await using` resource bindings), imports/reexports,
  calls, construction, members, bases, decorators, type references, JSX tags,
  JSX value/spread/child references, and TypeScript `import_type` queries. JSX
  prop names are not mistaken for
  value uses, while nested callback arguments remain source-grounded.
  Decorator factories are not duplicated as ordinary calls, while calls nested
  in decorator arguments remain visible. It does not select graph targets. Its
  payload/JSONL stream includes project scopes, configuration/source digests,
  diagnostics, and deterministic UTF-8 ranges.
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
  --root /Volumes/Workspace/Github/<owner>/<repository> --jsonl
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

To persist the deterministic target evidence for later manual labeling, set an
explicit report path. The report is atomically written only when this ignored
qualification test is run; it is never produced by normal Compass builds:

```bash
COMPASS_TS_QUALIFICATION_ROOT=/Volumes/Workspace/Github/<owner>/<repository> \
COMPASS_TS_TARGET_REPORT=/Volumes/Workspace/<run>/typescript-target-report.json \
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-6923 \
cargo test -p compass-languages --test typescript_target_differential \
  --locked -- --ignored --nocapture
```

The report schema is `compass.typescript-target-adjudication/1`. It includes the
pinned compiler/oracle metadata, exact source ranges, oracle target ranges,
candidate observations, automatic checker outcomes, and capability strata. It
does not contain manual judgments. A reviewed scorecard must add explicit
`accepted` and `source_oracle` pools, `judgmentSource: "manual"`, and a review
reason for every non-correct label under
`compass.typescript-target-scorecard/2`, then run:

```bash
python3 benchmarks/performance/harness.py typescript-scorecard \
  --scorecard /Volumes/Workspace/<run>/typescript-scorecard.json \
  --output /Volumes/Workspace/<run>/typescript-scorecard-result.json
```

The evaluator computes precision, 95% Wilson bounds, recall, target-cluster
concentration, per-corpus/relation/capability strata, critical semantic
violations, and the Plan 013 production or leadership gates. Diagnostic mode
is useful before labels are complete but can never become eligible for a public
quality claim.

## Go source candidates

`go-source-oracle.go` is an optional, standard-library-only qualification
helper. Its `compass.go-source-oracle/1` payload is consumed by provider
`go_stdlib_ast_types_v1` in the performance audit. Normal product execution,
Cargo tests, and ordinary Python fixture tests do not require Go. Build the
helper explicitly into an external directory; never run corpus tooling:

```bash
COMPASS_GO_ORACLE_BUILD_DIR=/Volumes/Workspace/crabbuild-target/compass-YOUR-WORKTREE-go-oracle
mkdir -p "$COMPASS_GO_ORACLE_BUILD_DIR"
GOCACHE="$COMPASS_GO_ORACLE_BUILD_DIR/cache" GOTOOLCHAIN=local GO111MODULE=off \
GOPROXY=off GOSUMDB=off GOWORK=off \
go build -o "$COMPASS_GO_ORACLE_BUILD_DIR/go-source-oracle" \
  benchmarks/performance/oracles/go-source-oracle.go

COMPASS_GO_SOURCE_ORACLE="$COMPASS_GO_ORACLE_BUILD_DIR/go-source-oracle" \
python3 -m unittest benchmarks.performance.tests.test_go_oracle
```

Use an `.exe` suffix on Windows. Replace `YOUR-WORKTREE` with this checkout's
name and follow the repository's mounted-volume policy. Set `COMPASS_GO_SOURCE_ORACLE`
to the absolute executable path when running the existing audit harness.
Missing runtime configuration fails explicitly instead of producing an empty
successful inventory. The helper never downloads packages, reads `go.mod` or
`go.work`, executes repository code, invokes build scripts, or runs `go list`.

The Python adapter supplies the complete selected file list, hashes and byte
sizes. The helper parses all selected source, including tests and conditional
files; it does not silently substitute a compiler project subset. Strict
payload validation checks the full file ledger, source digest, UTF-8 callee,
invocation and declaration ranges, local target references, status/count
ledgers, unknown fields and duplicate occurrences. Files rejected by the parser
remain explicit coverage failures. Source, records, diagnostics, subprocess
output and duration are bounded; limit failures publish no partial inventory.

Go's default compiler importer can invoke `go list -export`, so this helper
deliberately denies external imports during `go/types` checking. Any package
type error or rejected sibling source file suppresses every claimed concrete
target in that package. The typed payload distinguishes `local_concrete`,
`external`, `interface_dispatch`, `function_value`, `builtin`, `type_conversion`
and `unresolved_call_or_conversion`. Interface and function-value dispatch do
not acquire invented concrete targets. Proven conversions are excluded from
the flattened call pool; unresolved forms remain candidates for review.
Diagnostics contain fixed codes and locations rather than source literals.

The adapter feeds **unjudged source candidates** into the existing audit;
binding metadata never becomes an accepted graph judgment automatically. Pin
and retain the Go toolchain, helper executable/source, adapter and inventory
hashes before collecting audit evidence. Missing external type data and
all-source build-constraint conflicts remain visible in the type-status and
diagnostic ledgers. Manual source/graph adjudication, source coverage, target
diversity, precision/recall and the unchanged 2,000-total/400-per-corpus floors
still apply. Parsing 400 expressions does not establish 400 accepted facts.

Rust remains unsupported by the production audit provider registry. A separate
`syn` syntax census can estimate prospective review work; its unexpanded
macros, conditional items, constructor/call distinctions and trait dispatch
cannot certify complete semantic recall or satisfy promotion gates.
