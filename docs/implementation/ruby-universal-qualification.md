# Ruby universal qualification

Plan 019 is implemented through a single Ruby evidence path and is promoted
to `Qualified` by the checked-in universal-evidence release decision. The
producer identity is `compass.ruby` (producer version 1, evidence schema v2);
the complete quality-audit results and deterministic performance evidence are
retained below for review.

## Production contract

The Ruby emitter in `compass-languages` publishes bounded evidence for:

- nested and reopened classes/modules (Ruby modules are graph `trait` nodes);
- lexical constant ownership, superclass facts, and exact `include`,
  `prepend`, and `extend` occurrences;
- instance (`Owner#method`) and singleton (`Owner.method`) method spaces;
- parameters, fields, literal attributes, construction, receiver-qualified
  calls, bare calls, `super`, literal imports/autoloads, and literal aliases;
- conservative diagnostics for dynamic dispatch, dynamic loading, evaluation,
  nonliteral aliases, malformed syntax, and resource limits.

The pipeline uses an explicit 8 MiB worker stack for deep Ruby DSL trees;
`super` owner lookup is frame-local and reopened-type hierarchy checks avoid a
per-call temporary candidate set. These are bounded hardening measures, not a
quality-audit waiver.

The resolver uses the same method-space codec, coalesces reopened Ruby type
nodes by exact graph identity, leaves duplicate method definitions ambiguous,
and rejects cross-language terminal-name matches. Rails routes are emitted by
the universal `rails-ruby` pack from AST calls plus validated Ruby occurrences;
the pack has one registered project expansion hook and does not use a
line-oriented or regular-expression detector.

## Independent qualification command

The checked-in entry point is qualification-only and has no runtime or test
dependency on Graphify:

```bash
python3 scripts/qualify_ruby_universal.py --mode fixture
python3 scripts/qualify_ruby_universal.py --mode pinned \
  --repository rails=/Volumes/Workspace/Github/rails/rails
python3 scripts/qualify_ruby_universal.py --mode quality-audit \
  --audit-manifest /path/to/ruby-audit.json \
  --graph /path/to/graph.json \
  --corpus /Volumes/Workspace/Github/rails/rails
```

### Build a bounded, source-grounded audit population from Compass graphs
```bash
python3 scripts/build_ruby_quality_audit.py \
  --corpus rails=/Volumes/Workspace/Github/rails/rails=/path/to/rails/graph.json \
  --output /path/to/ruby-audit.json
python3 benchmarks/performance/harness.py audit \
  --manifest /path/to/ruby-audit.json \
  --graph /Volumes/Workspace/Github \
  --corpus /Volumes/Workspace/Github
```

Fixture mode runs `scripts/ruby_source_oracle.rb` twice and requires byte-
identical canonical JSON, exact inventory digests, and bounded UTF-8/error
handling. Pinned mode requires every manifest checkout to be clean and at the
declared commit; missing Discourse and RuboCop checkouts are an intentional
failure rather than an implicit clone. The pinned manifest is
`tests/qualification/ruby-universal-repositories.toml`.

Performance mode consumes a prebuilt binary and a temporary copy of a checkout:

```bash
python3 scripts/qualify_ruby_universal.py --mode performance \
  --root /path/to/ruby/checkout --compass /path/to/compass --samples 5
```

It records cold, warm, fact-neutral, semantic-edit, and restore timings plus
graph hashes and changed/reused file counts. RSS is explicitly non-blocking.
The command never modifies the supplied checkout.

## Verification record

The following checks pass in the implementation checkout (Cargo artifacts use
`/Volumes/Workspace/crabbuild-target/compass-ruby-universal`):

```text
python3 -m unittest scripts.tests.test_ruby_source_oracle -v
python3 -m unittest benchmarks.performance.tests.test_correctness.CorrectnessTests.test_ruby_ripper_provider_is_pinned_byte_deterministic_and_typed
cargo test -p compass-languages --test ruby_universal_conformance --locked
cargo test -p compass-languages --locked
cargo test -p compass-resolve --test universal_resolution ruby --locked
cargo test -p compass-resolve --test php_ruby_jvm_routes rails --locked
cargo test -p compass-core --lib fact_digest_match_requires_all_cached_source_facts --locked
cargo test -p compass-files --lib build_guard --locked
./scripts/qualify_code_graph_v1.sh --fixtures-only
sh scripts/check_product_boundary.sh
PROJECT_ROOT=/Volumes/Workspace/Github/compass-ruby-parser-root \
TSLP_OFFLINE=1 \
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-ruby-universal \
cargo clippy -p compass-languages -p compass-resolve -p compass-core \
  --lib --bins --locked -- -D warnings
```

The full `compass-resolve` package suite has two unrelated pre-existing
TypeScript fixture failures (`typescript_candidate_resolves_relative_and_default_imports_across_files`
and `typescript_workspace_package_exports_follow_nodenext_reexports`); all
Ruby and Rails tests in that suite pass.

The complete audit gates are green in the pinned three-corpus report. The
release decision promotes Ruby only at the version-1 producer identity shown
above; passing a future producer or capability change requires a new decision.

The pinned source-oracle run on 2026-08-17 completed deterministically with
Ruby 4.0.6 / revision `03b6d3f8898a28604fe6cb00eae3226b821168f4`:

| Corpus | Ruby files scanned/parsed | Inventory SHA-256 |
| --- | ---: | --- |
| Rails `cc7d47f4` | 3,486 / 3,486 | `2edb8b395bcc18014bf8fcd33c4cb3bc23c6a57b140ee8becbc647684fc76dad` |
| Discourse `699ad465` | 10,921 / 10,921 | `b556ae34d65b7cfdfd374a7a86f6a253aecdfd216366176d9bc8e6b61e724020` |
| RuboCop `c034d8b6` | 1,759 / 1,759 | `6ce27ad3a6785409d2e551046db6719800648d1d8a6e2e5ff5a283702f63d6e0` |

Discourse and RuboCop graph captures use read-only Ruby-only projections of the
pinned checkouts because their full non-Ruby Markdown trees exceed the current
bounded parser resource envelope; the source inventory remains pinned to the
same Git commits. This is an explicit qualification limitation, not a claim
that the full mixed-language graphs passed.

The generated audit population contains 89,981 accepted relationships across
the three corpora, with 100% observed precision, a 99.9957% Wilson lower bound,
98.5567% source-oracle recall, and zero critical violations. Every fixed
qualification gate passes. The release decision records this audited producer
as `Qualified`.

| Capability | Accepted | Recall | Status |
| --- | ---: | ---: | --- |
| calls | 30,182 | 98.4777% | pass |
| construction | 23,384 | 97.8068% | pass |
| ownership | 34,254 | 99.1200% | pass |
| traits | 2,161 | 98.7659% | pass |

The machine-readable result is produced by
`benchmarks/performance/harness.py audit`; no Graphify facts are used as truth.
The release decision records this audited producer as `Qualified`.

The current real-repository captures (cold, no build time) are:

| Projection | Files | Nodes | Edges | Cold |
| --- | ---: | ---: | ---: | ---: |
| Rails `cc7d47f4` | 4,967 | 95,462 | 158,272 | 222.7 s |
| Discourse `699ad465` (Ruby-only) | 11,199 | 104,033 | 187,412 | 247.2 s |
| RuboCop `c034d8b6` (Ruby-only) | 1,759 | 22,030 | 32,972 | 43.8 s |

On a 305-file Rails subtree, five unchanged updates reuse all 305 files in
0.1959–0.2045 s (median 0.1963 s); a one-file fact-neutral edit extracts one
file and publishes a file-only delta in 9.2387 s, and restore is byte-identical.
On the full Rails checkout, the five-warm-sample report records a 267.941 s
cold graph, a 2.9616 s unchanged-warm median, a 165.412 s fact-neutral edit,
a 249.001 s semantic edit, and a 239.309 s restore with an exact cold hash
match. The pinned Discourse Ruby-only five-warm-sample run also restores
byte-for-byte (cold 211.296 s, warm median 3.017 s with samples from 2.996–3.214
s, fact-neutral 145.529 s, semantic 226.129 s, restore 226.958 s). RSS remains
non-blocking. Ruby therefore remains `Qualified` under the release decision,
while the bounded unresolved-dynamic behavior described above stays explicit.

The fact-neutral delta also preserves unchanged files' extraction status,
parser-recovery diagnostics, and per-file coverage while refreshing the edited
file. The strict fixture qualification now verifies clean, warm, forced,
fact-neutral restore, and relocated-checkout graphs byte-for-byte.
