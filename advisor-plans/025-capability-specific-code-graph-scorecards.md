# Plan 025: Gate code-graph quality with capability-specific scorecards

> **Executor instructions**: Deliver this program as a sequence of small,
> reviewable PRs. Read this plan completely, then read `AGENTS.md`,
> `COMPATIBILITY.md`, `docs/design/principles.md`,
> `docs/design/code-graph-v1-qualification.md`, and
> `docs/reference/universal-semantic-evidence.md` before changing source. Run
> every phase gate and confirm its expected result before starting the next
> phase. Do not replace an existing evidence contract, execute code from a
> qualification repository, add Graphify as a runtime/test/configuration
> dependency, or turn a missing denominator into a numeric zero.
>
> **Drift check (run before every phase)**:
>
> ```bash
> git diff --stat 5bf544c8..HEAD -- \
>   benchmarks/performance/compass benchmarks/performance/tests \
>   crates/compass-query/src/relevance.rs \
>   crates/compass-query/tests/relevance_qualification.rs \
>   crates/compass-query/tests/fixtures/relevance \
>   scripts/code_graph_v1_oracle.py scripts/check_code_graph_topology.py \
>   scripts/qualify_code_graph_v1.sh scripts/qualify_query_relevance.py \
>   scripts/qualify_react_frontend_graph.py scripts/tests \
>   tests/qualification docs/design PERFORMANCE.md advisor-plans
> ```
>
> If an in-scope file changed, compare the live implementation with “Current
> state” below. Mechanically update paths when ownership is unchanged. STOP if
> an existing schema version, audit judgment meaning, query metric definition,
> or qualification threshold changed.

## Status

- **Status**: TODO
- **Priority**: P1
- **Effort**: XL; six reviewable phases
- **Risk**: MEDIUM
- **Depends on**: no implementation prerequisite; reuse the existing source
  quality audit, code-graph topology report, React scorecard, and query
  relevance qualification. The final release gate should consume Plan 005 or
  an equivalent exact-commit production gate.
- **Category**: correctness, tests, direction, documentation
- **Planned at**: commit `5bf544c8`, 2026-08-29

## Decision

Compass will not publish or gate on one numeric “graph quality” score. It will
publish one status for each semantic capability, and three separate evidence
lenses within that capability:

1. **Fidelity** — did Compass publish the source-proven nodes, edges,
   occurrences, directions, multiplicity, and targets without fabrication?
2. **Structural usefulness** — does the published subgraph preserve the
   independently expected connections and paths without fragmentation or
   invented bridges?
3. **Agent utility** — can a bounded agent query retrieve and traverse the
   facts needed for a reviewed task, with acceptable ranking, context cost,
   and false-positive behavior?

A capability passes only when every lens marked `required` for that capability
passes. The report may expose an overall boolean and lists of failed or
under-sampled capability identities, but it must never calculate an average,
weighted sum, star rating, or percentage that can hide one failing capability.

## Why this matters

Node, edge, and community totals reward volume, not truth or usefulness. A
graph can gain thousands of isolated syntax nodes while becoming worse for an
agent tracing a FastAPI dependency, and a dense graph can be worse if its new
edges are invented. Compass already measures source fidelity, topology, and
agent-query relevance, but those systems report at different grains and cannot
answer “did route dependency resolution improve without regressing calls or
imports?” This plan adds an evidence-preserving coordination layer that answers
that question without weakening the existing contracts or changing the public
`compass.graph/1` schema.

## Current state

### Existing evidence contracts to preserve

- `benchmarks/performance/compass/audit.py:47-57` owns the generic independent
  edge audit. Its current schemas are `compass.quality-audit/2` and
  `compass.quality-audit-result/2`; its production gates require 2,000 accepted
  records overall, 100 per capability, 99.5% overall precision, 99% per-
  capability precision, and 95% per-capability recall.
- `benchmarks/performance/compass/audit.py:1462-1523` reports strata for corpus,
  language, relation, bare capability name, confidence, and target cluster.
  The release gate at `audit.py:1579-1619` correctly evaluates the composite
  `(producer, framework_pack, capability)` identity. The new report must retain
  that composite identity instead of merging same-named capabilities from
  different producers or framework packs.
- `scripts/qualify_react_frontend_graph.py:764-794` already performs
  deterministic one-to-one capability matching. One candidate occurrence can
  satisfy only one oracle record, so duplicated expectations expose
  multiplicity loss. This is the exemplar matching policy.
- `scripts/qualify_react_frontend_graph.py:854-879` emits
  `compass.react-frontend-scorecard/1` with per-capability expected, candidate,
  matched, false-positive, false-negative, precision, recall, and Wilson
  counts. Its existing `aggregate` is compatibility data; the new coordinator
  must not use that aggregate as a global quality score.
- `scripts/code_graph_v1_oracle.py:1205-1231` emits
  `compass.code-graph-qualification-summary/1`. It records assertion counts,
  route resolutions, coverage, graph digest, and byte comparisons.
- `docs/design/code-graph-v1-qualification.md:112-135` explicitly says raw
  totals are coverage signals, not connectivity proof. The separate
  `compass.code-graph-topology-report/1` reports typed endpoint pairs,
  edge-bearing and isolated nodes, weak components, cross-file and
  cross-community edges, self-loops, total communities, and singleton
  communities. It also states that a larger third-party graph is not truth.
- `crates/compass-query/src/relevance.rs:418-467` owns
  `compass.query-qualification/1`. Its typed metrics include Success@1,
  MRR@10, recall@5/20, precision@10, nDCG@10, intent macro-F1, entity match,
  ambiguity recall, edge kind/direction precision and recall, path acceptance,
  no-answer precision, false-positive rate, latency, and bounded work. Undefined
  metrics use `{value: null, diagnostic: ...}` at `relevance.rs:410-415`.
- `tests/qualification/python-framework-repositories.toml:12-26` already pins
  FastAPI and the FastAPI full-stack template with `http_routes`,
  `dependency_injection`, `security`, `data_modeling`, and `persistence`
  capability labels. Reuse these read-only corpora; do not introduce OpenClaw
  into this program.

### Current fragmentation

The generic audit is edge-centric and cannot score a node-only fact or an
agent task. The topology report is graph-wide or relationship-wide and cannot
say which semantic capability produced a fragment. The React scorer has good
capability accounting but a framework-specific schema. Query relevance has a
strong task metric vocabulary but no mapping from reviewed query IDs to code-
graph capabilities. The qualification shell invokes these gates independently
at `scripts/qualify_code_graph_v1.sh:372-421`, so a reviewer must manually
correlate several outputs.

## Technical design

### 1. Capability is the unit of decision

A capability describes an agent-useful semantic fact, not a language, node
kind, relationship kind, or framework label. The stable identity is:

```text
capability id + producer + optional framework pack + capability-definition version
```

For example, `web.route-dependency/python/fastapi/v1` is distinct from both
`web.route-handler/python/fastapi/v1` and a future Java route-dependency
implementation. Report ordering is lexicographic by these fields. A
capability-level status is the logical conjunction of its required slices and
required lenses, never an arithmetic roll-up.

Use these initial capability IDs:

| Capability | Source-proven fact | Why an agent needs it | Initial corpus |
|---|---|---|---|
| `symbol.definition` | exact declaration node identity, kind, and source occurrence | locate the authoritative implementation before traversal | code-graph-v1 fixtures plus qualified language corpora |
| `module.import-target` | import occurrence to exact internal/external module target | discover ownership and module coupling | code-graph-v1 fixtures plus qualified language corpora |
| `symbol.call-target` | call occurrence to exact callable target | callers, callees, impact, execution context | code-graph-v1 fixtures plus qualified language corpora |
| `type.inheritance-target` | type declaration to exact base type | type hierarchy and inherited behavior | code-graph-v1 fixtures |
| `web.route-handler` | route declaration to exact handler with handler-stage evidence | locate an endpoint implementation | FastAPI, FastAPI template, React router fixtures |
| `web.route-dependency` | route/dependency occurrence to exact dependency or security target and stage | explain auth/DI and dependency impact | FastAPI and FastAPI template |
| `ui.render-target` | JSX/template render occurrence to exact component target | trace page composition and frontend impact | existing React fixtures and pinned corpora |

Keep `data.persistence`, `test.linkage`, `config.consumption`, and
`architecture.community-navigation` observational until each has both an
independent source/task oracle and a reviewed candidate-population rule.
Community count, modularity, and singleton count may be diagnostics for an
existing capability, but “more” or “fewer” communities is never inherently a
pass condition. Gate a community property only when a reviewed task or an
independent expected grouping proves what should be co-located or bridged.

### 2. Policy is checked in; evidence remains external to policy

Add `tests/qualification/code-graph-capability-policy.json` with strict schema
`compass.code-graph-capability-policy/1`. The policy defines meaning,
applicability, sampling, and thresholds. It must not contain observed results
or silently generated expectations.

Target shape:

```json
{
  "schema": "compass.code-graph-capability-policy/1",
  "limits": {
    "maxCapabilities": 128,
    "maxSlicesPerCapability": 256,
    "maxInputBytes": 268435456,
    "maxRecords": 2000000,
    "maxDiagnostics": 1000
  },
  "capabilities": [
    {
      "id": "web.route-dependency",
      "version": 1,
      "description": "Resolve source-proven route dependencies and security dependencies",
      "releaseRequired": true,
      "factShapes": ["node", "directedEdge", "boundedPath"],
      "selectors": [
        {
          "producer": "python",
          "frameworkPack": "fastapi",
          "candidateNodeKinds": ["route"],
          "candidateRelations": ["routes_to", "depends_on"],
          "candidateStages": ["dependency", "security"]
        }
      ],
      "lenses": {
        "fidelity": {
          "mode": "required",
          "minimumAccepted": 100,
          "minimumRecallCandidates": 1,
          "minimumCorpora": 2,
          "minimumPrecision": 0.99,
          "minimumRecall": 0.95,
          "zeroTolerance": [
            "fabricated_target",
            "fabricated_occurrence",
            "wrong_direction",
            "unsafe_local_substitution"
          ]
        },
        "structuralUsefulness": {
          "mode": "required",
          "minimumExpectedEndpointConnectionRecall": 0.95,
          "minimumExpectedPathRecall": 0.95
        },
        "agentUtility": {
          "mode": "observational",
          "minimumReviewedTasks": 20,
          "minimumCorpora": 2
        }
      }
    }
  ]
}
```

Allowed v1 fact shapes are `node`, `directedEdge`, and `boundedPath`. A
capability can require more than one: for example, route dependency quality
requires the route/dependency endpoints, the directed relationship, and a
bounded route-to-transitive-dependency path. Score each fact shape separately;
do not average a strong node result with a weak edge result. The exact node,
relationship, and stage strings must be confirmed against the live graph
vocabulary before committing policy. Do not broaden a selector to make a score
pass. A candidate selector defines the complete population over which false
positives are counted; selector drift is a reviewed contract change.

Policy validation must reject unknown fields, duplicate or unsorted
capabilities, unknown lens modes, non-finite/out-of-range thresholds, empty
release-required selectors, unknown zero-tolerance rules, unsafe paths, and
unknown major versions. `version: 1` versions the capability definition, not
the Compass product and not the public graph contract.

### 3. Normalize evidence through adapters, not migrations

Add `benchmarks/performance/compass/capability_scorecard.py`. It is a pure,
bounded qualification module that accepts normalized records and produces a
canonical report. Add small adapters for:

- `compass.quality-audit-result/2` plus its verified audit records;
- `compass.react-frontend-scorecard/1` plus its source-oracle facts;
- `compass.code-graph-qualification-summary/1` and
  `compass.code-graph-topology-report/1`;
- a new query capability-slice report described below.

Do not change the meaning or version of any input schema. Every adapter records
the input schema, SHA-256 digest, source identity, and any information it could
not map. An unsupported schema major or a lossy required mapping fails closed.
An optional unavailable lens produces `not_applicable` only when policy says it
does not apply; otherwise it produces `insufficient_evidence`.

The normalized node fact key is:

```text
(corpus, producer, framework_pack, capability_id,
 node_identity, node_kind, source_file, start_byte, end_byte,
 provenance_identity)
```

The normalized edge fact key is:

```text
(corpus, producer, framework_pack, capability_id,
 source_identity, target_identity, relationship, direction,
 source_file, start_byte, end_byte, provenance_identity)
```

Matching is deterministic and one-to-one. Sort truth and candidate facts by
the complete key, then consume at most one candidate occurrence for each truth
occurrence. Preserve separate counts for `ambiguous`, `unresolved`,
`unsupported`, and `represented_elsewhere`; never drop them. A truly dynamic
construct may be excluded from a recall denominator only when the independent
oracle records a reviewed exclusion reason.

### 4. Measure three lenses without blending them

#### Fidelity lens

At minimum emit the following counts and ratios for each applicable fact shape:

- expected, candidates, matched, false positives, and false negatives,
  separately for nodes, directed edges, and bounded paths;
- accepted and source-oracle sample counts;
- precision, recall, F1, and two-sided 95% Wilson precision lower bound;
- exact-target, direction, occurrence-anchor, multiplicity, and provenance
  accuracy;
- ambiguous, unresolved, unsupported, represented-elsewhere, and critical
  judgment counts;
- distinct corpora, source files, producers, framework packs, and target
  clusters.

Use `null` plus a diagnostic for undefined ratios. Zero fabricated targets,
fabricated occurrences, cross-language matches, wrong-direction edges, and
unsafe local substitutions are hard gates where policy marks them zero
tolerance.

#### Structural-usefulness lens

Compute topology only over the capability-selected candidate edges plus graph
nodes named by the independent truth set. Emit:

- expected endpoint nodes present/missing and non-isolated;
- expected endpoint connection recall;
- expected direct-edge and expected bounded-path recall;
- unique typed endpoint pairs and occurrence multiplicity;
- weak components, largest-component fraction, cross-file edges, self-loops,
  and isolated expected nodes;
- cross-community edges and singleton communities as diagnostics;
- exact-evidence-only versions of every applicable metric.

Do not gate on density, component count, modularity, community count, or
cross-community rate without an oracle expectation. Different capabilities
have legitimately different topology: imports can span subsystems, containment
forms trees, and route dependencies form shallow directed chains. The policy
may gate expected path/connection recall because those expectations are facts;
it may not demand arbitrary global connectedness.

#### Agent-utility lens

Add `tests/qualification/query-capability-map.json` with schema
`compass.query-capability-map/1`. It maps existing reviewed query IDs to one or
more capability IDs and declares which retrieved nodes, edges, directions, or
paths prove task success. Do not add fields to
`compass.query-judgments/1` solely for this scorecard.

Extend the qualification test harness, not the runtime query API, to filter the
existing judgment corpus and observations by capability. Emit
`compass.query-capability-slices/1`, reusing the typed metric calculations in
`compass-query::relevance::score` for each deterministic slice. Each slice
records reviewed task count and corpus count in addition to:

- Success@1, MRR@10, recall@5/20, precision@10, and nDCG@10;
- edge precision/recall and edge-kind/direction precision/recall;
- path acceptance and mean accepted path rank;
- no-answer precision and false-positive rate;
- context bytes or returned-node/edge counts, latency, and bounded work.

Seed reviewed tasks in six user-facing families: locate implementation, find
callers/importers, trace a bounded dependency path, explain a route's DI/auth
chain, assess change impact, and correctly return no answer for an ambiguous or
unsupported relationship. Keep agent utility `observational` until a
capability has at least 20 reviewed tasks across at least two corpora. Promotion
to `required` is a separate policy review backed by a checked-in baseline; do
not invent initial thresholds in the implementation PR.

### 5. Report a status lattice, not a score

Emit canonical JSON with schema `compass.code-graph-capability-scorecard/1`:

```json
{
  "schema": "compass.code-graph-capability-scorecard/1",
  "policySha256": "...",
  "inputs": [{"schema": "...", "sha256": "..."}],
  "passed": false,
  "failedRequiredCapabilities": ["web.route-dependency/python/fastapi/v1"],
  "insufficientRequiredCapabilities": [],
  "capabilities": [
    {
      "identity": {
        "id": "web.route-dependency",
        "producer": "python",
        "frameworkPack": "fastapi",
        "version": 1
      },
      "status": "fail",
      "fidelity": {"status": "pass", "metrics": {}},
      "structuralUsefulness": {"status": "fail", "metrics": {}},
      "agentUtility": {"status": "observational", "metrics": {}}
    }
  ],
  "diagnostics": []
}
```

Allowed lens statuses are `pass`, `fail`, `insufficient_evidence`,
`not_applicable`, and `observational`. Rules:

- a required lens below a threshold is `fail`;
- a required lens below its sample minimum is `insufficient_evidence`, not
  `fail` and not `pass`;
- an observational lens never changes the capability status;
- a required capability is `pass` only when all required lenses and slices
  pass;
- the report-level `passed` is true only when all release-required
  capabilities pass;
- any critical zero-tolerance finding fails its owning capability regardless
  of sample size;
- there is no `overallScore`, numeric aggregate, or ranking of capabilities.

### 6. Execution flow and ownership

```text
independent source oracles ──> existing audit/React adapters ──> fidelity
published compass.graph/1 ───> capability selector + topology ─> structure
reviewed query judgments ────> capability query slices ────────> agent utility
                                      │
checked-in capability policy ─────────┴──> canonical scorecard + exit status
```

Keep all orchestration qualification-only:

- `benchmarks/performance/compass/capability_scorecard.py` owns validation,
  normalization, metric calculation, and report assembly.
- `scripts/qualify_code_graph_capabilities.py` owns bounded CLI I/O, adapters,
  canonical output, and exit status. Its stable qualification interface is
  `--policy PATH --claim-mode conformance|qualification --evidence PATH`
  (repeatable), optional `--graph PATH`, and `--output PATH`. Every evidence
  file must identify one supported schema; `--graph` is required whenever a
  structural lens must be recomputed.
- `crates/compass-query/tests/relevance_qualification.rs` owns test-only query
  slice generation using production scoring logic.
- `scripts/qualify_code_graph_v1.sh` invokes the new fixture gate only after it
  is deterministic and fast enough for the existing CI job.
- No normal Compass command, graph build, MCP path, or runtime query depends on
  Python or the scorecard files.

## Compatibility and safety constraints

- Do not change `compass.graph/1`, `compass.quality-audit/2`,
  `compass.quality-audit-result/2`, `compass.react-frontend-scorecard/1`,
  `compass.code-graph-topology-report/1`, `compass.query-judgments/1`, or
  `compass.query-qualification/1`.
- The two new contracts start at major version 1. Do not bump any Compass crate,
  package, product, graph, producer, or framework-pack version.
- Do not add Graphify output, code, packages, configuration, test fixtures, or
  fallback behavior. Comparative findings may inform hypotheses but never
  supply truth.
- Use only pinned, read-only qualification repositories beneath
  `/Volumes/Workspace/Github`. Never modify, update, reset, clean, import, or
  execute their code.
- Use canonical UTF-8 JSON, sorted object-derived collections, stable decimal
  calculations, and atomic output writes. Equivalent inputs must be byte
  identical.
- Enforce the policy's byte, record, capability, slice, path-depth, diagnostic,
  and subprocess-output limits before allocation or traversal. A limit is a
  typed failure, never an empty result.
- Preserve provenance and occurrence multiplicity. Do not choose the first
  target when resolution is ambiguous.

## Commands the executor will need

Before any Cargo command, verify `/Volumes/Workspace` is mounted and use a
checkout-specific target directory exactly as required by `AGENTS.md`.

| Purpose | Command | Expected on success |
|---|---|---|
| Python unit tests | `python3 -m unittest benchmarks.performance.tests.test_capability_scorecard scripts.tests.test_code_graph_capability_qualification` | exit 0; all new tests pass |
| Query relevance tests | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-capability-scorecards cargo test -p compass-query --test relevance_qualification --locked` | exit 0; existing and capability-slice cases pass |
| Fixture qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-capability-scorecards ./scripts/qualify_code_graph_v1.sh --fixtures-only` | exit 0; capability scorecard stage passes and repeated report bytes match |
| Query qualification | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-capability-scorecards python3 scripts/qualify_query_relevance.py` | exit 0; global and capability-slice reports validate |
| Product boundary | `sh scripts/check_product_boundary.sh` | exit 0; no Graphify/product-boundary violations |
| Python syntax | `python3 -m py_compile benchmarks/performance/compass/capability_scorecard.py scripts/qualify_code_graph_capabilities.py` | exit 0 |
| Shell syntax | `bash -n scripts/qualify_code_graph_v1.sh` | exit 0 |
| Format | `cargo fmt --all -- --check` | exit 0, no diff |
| Native baseline | `CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-capability-scorecards cargo clippy --workspace --lib --bins --locked -- -D warnings && CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-capability-scorecards cargo test --workspace --lib --bins --locked` | exit 0 |

## Scope

**In scope** (add or modify only as required by the relevant phase):

- `benchmarks/performance/compass/capability_scorecard.py` (new)
- `benchmarks/performance/tests/test_capability_scorecard.py` (new)
- `scripts/qualify_code_graph_capabilities.py` (new)
- `scripts/tests/test_code_graph_capability_qualification.py` (new)
- `tests/qualification/code-graph-capability-policy.json` (new)
- `tests/qualification/code-graph-capability-fixture.json` (new independent
  conformance facts and candidates)
- `tests/qualification/query-capability-map.json` (new)
- `crates/compass-query/tests/relevance_qualification.rs`
- `crates/compass-query/tests/fixtures/relevance/` reviewed fixture files
- `scripts/qualify_query_relevance.py`
- `scripts/qualify_code_graph_v1.sh`
- `docs/design/code-graph-capability-scorecards.md` (new)
- `docs/design/code-graph-v1-qualification.md`
- `PERFORMANCE.md`
- `advisor-plans/README.md` and this plan when recording execution status

**Out of scope**:

- graph extraction or resolution behavior; use a separate focused PR when a
  scorecard exposes a product defect;
- changes to any existing public schema or version number;
- a dashboard, hosted service, database, network dependency, model call, or
  credential requirement;
- Graphify integration or treating another graph implementation as truth;
- OpenClaw qualification;
- arbitrary density, node-count, edge-count, modularity, or community-count
  targets;
- automatic threshold weakening, baseline rewriting, or accepting generated
  expectations from the graph under test;
- release claims for under-sampled capabilities.

## Git workflow and PR slicing

- Branch: `advisor/025-capability-scorecards`
- Use conventional commits matching the repository, for example
  `test(graph): add capability scorecard policy`.
- Commit by phase. Do not mix extraction/resolver fixes into qualification PRs.
- Do not push or open a PR unless the operator explicitly instructs it.

Recommended review sequence:

1. policy/schema and pure scorer;
2. existing evidence adapters and fixture scorecards;
3. initial seven capability definitions and reviewed truth cases;
4. FastAPI/React pinned-corpus evidence;
5. query capability slices and agent-utility observations;
6. CI/release integration and documentation.

## Execution plan

### Phase 1: Freeze the scorecard contract and conformance behavior

Create the strict policy and report data model in
`benchmarks/performance/compass/capability_scorecard.py`. Implement bounded JSON
loading, schema validation, composite capability identity, status-lattice
evaluation, canonical serialization, and input digests. Create a deliberately
small conformance fixture covering:

- a fully passing capability;
- an accuracy threshold failure;
- an under-sampled required lens;
- a zero-tolerance failure below the normal sample minimum;
- observational and not-applicable lenses;
- same-named capabilities under two framework packs;
- undefined metrics represented as `null` with diagnostics;
- unknown major, unknown field, duplicate identity, invalid threshold, and
  limit failures;
- byte-identical output after shuffled equivalent input.

The conformance fixture proves code paths only. Mark it ineligible for a
production quality claim.

**Verify**:

```bash
python3 -m unittest benchmarks.performance.tests.test_capability_scorecard
```

Expected: exit 0; every status, rejection, limit, and determinism case passes.

### Phase 2: Adapt existing fidelity and topology evidence

Implement adapters in `scripts/qualify_code_graph_capabilities.py`. Each
adapter must validate the input schema before reading fields, calculate and
publish the input digest, and either preserve all required semantics or reject
the mapping. Reuse the React scorer's deterministic one-to-one occurrence
matching; do not re-match by display name or unordered `any(...)` logic.

For generic audit records, expose the complete `(producer, framework_pack,
capability)` identity, not only `strata.capability`. For topology, recompute the
capability-selected subgraph from the canonical graph and independent truth;
do not infer a capability from whole-graph averages. Add adversarial tests for
a connected false edge, a correct repeated occurrence, a missing expected
bridge, an ambiguous target, an isolated expected node, and a selector that
would otherwise omit a false positive.

**Verify**:

```bash
python3 -m unittest \
  benchmarks.performance.tests.test_capability_scorecard \
  scripts.tests.test_code_graph_capability_qualification
```

Expected: exit 0; adapters reject incompatible/lossy inputs and all adversarial
facts produce the expected per-capability status.

### Phase 3: Seed seven reviewed capability definitions on fixtures

Add the seven initial capability policies from the table above. Reuse existing
semantic, topology, Python framework, and React source facts where they are
independent of the graph under test. Add only the missing reviewed facts needed
to cover exact targets, directions, occurrences, multiplicity, ambiguity,
unresolved constructs, and negative candidate populations.

Run the fixture qualification twice and compare canonical report bytes. Keep
under-sampled production thresholds explicit: a fixture can prove a capability
implementation path while reporting `insufficient_evidence`; it must not be
presented as a production pass. Add a separate `conformancePassed` result for
fixture behavior if needed, rather than lowering production sample floors.

**Verify**:

```bash
python3 scripts/qualify_code_graph_capabilities.py \
  --policy tests/qualification/code-graph-capability-policy.json \
  --claim-mode conformance \
  --evidence tests/qualification/code-graph-capability-fixture.json \
  --output /tmp/compass-capability-scorecard-a.json
python3 scripts/qualify_code_graph_capabilities.py \
  --policy tests/qualification/code-graph-capability-policy.json \
  --claim-mode conformance \
  --evidence tests/qualification/code-graph-capability-fixture.json \
  --output /tmp/compass-capability-scorecard-b.json
cmp /tmp/compass-capability-scorecard-a.json \
  /tmp/compass-capability-scorecard-b.json
```

Expected: all commands exit 0; `cmp` is silent. The conformance evidence file
contains both reviewed truth and deliberately adversarial candidates, so this
step does not require a production graph. Do not discover arbitrary repository
files.

### Phase 4: Qualify real FastAPI and React capability slices

Reuse the pinned FastAPI and FastAPI full-stack template revisions from
`tests/qualification/python-framework-repositories.toml` and the existing React
qualification manifest. Resolve each checkout beneath `/Volumes/Workspace/Github`
and verify its exact commit before reading. Treat every checkout as read-only.

Generate independent source inventories first, then the canonical Compass graph,
then per-capability candidate populations. At minimum report separate slices
for FastAPI route handlers, FastAPI route dependencies/security dependencies,
and React render targets. Require at least two corpora for any
release-required framework capability. Retain the existing generic-audit floors
of 100 accepted facts, 99% precision, and 95% recall; do not weaken them to fit
the available sample. An under-sampled slice is `insufficient_evidence` and
blocks a release-required capability without falsely claiming a regression.

Store generated graphs and reports under the checkout-specific
`/Volumes/Workspace/crabbuild-target` directory, never in the Compass tree or
qualification repository. Check in only manifests, reviewed expectation policy,
and small source snippets allowed by fixture licensing.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-capability-scorecards \
  ./scripts/qualify_code_graph_v1.sh \
  --repositories tests/qualification/code-graph-v1-repositories.toml
```

Expected: exact pinned revisions are reported; capability reports are
deterministic; every release-required slice is `pass` or the command exits
non-zero naming the precise failed/insufficient capability. Never relabel an
actual failure as insufficient evidence.

### Phase 5: Add reviewed agent-task capability slices

Create `tests/qualification/query-capability-map.json` and extend
`crates/compass-query/tests/relevance_qualification.rs` to score deterministic
subsets using the existing `score` function. The sidecar must reject unknown
query IDs, unknown capability IDs, duplicates, empty required slices, and
capability labels for queries whose expected nodes/edges/path do not support
that capability.

Add reviewed tasks for the six task families listed in the technical design,
including negative and ambiguity cases. Emit
`compass.query-capability-slices/1` from the qualification harness and consume
it in the scorecard adapter. Do not expose a new runtime CLI/API merely to make
the test harness convenient. Keep each capability's agent lens observational
until its reviewed denominator meets policy; record `null` and a diagnostic for
metrics with no applicable judgments.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-capability-scorecards \
  cargo test -p compass-query --test relevance_qualification --locked
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-capability-scorecards \
  python3 scripts/qualify_query_relevance.py
```

Expected: exit 0; existing global relevance metrics remain unchanged, every
query maps deterministically, and the capability-slice report validates.

### Phase 6: Integrate the stable gate and document threshold governance

After Phases 1–5 are stable and fixture runtime fits the existing CI envelope,
invoke capability qualification from `scripts/qualify_code_graph_v1.sh
--fixtures-only`. Reuse the existing `code-graph-v1-fixtures` CI job instead of
adding a parallel build. Update `docs/design/code-graph-v1-qualification.md`
and create `docs/design/code-graph-capability-scorecards.md` with:

- capability/lens/status definitions;
- why no global score exists;
- how candidate populations and independent truth are reviewed;
- how to add a capability, corpus, or agent task;
- how under-sampling and not-applicable metrics differ;
- the exact local fixture and pinned-corpus commands;
- threshold change governance and rollback.

Add a machine test ensuring an ordinary threshold update may keep or strengthen
a threshold but cannot weaken it silently. A deliberate weakening must change
an explicit policy-approval field and be isolated in its own PR with evidence;
never auto-update thresholds from observed output.

**Verify**:

```bash
CARGO_TARGET_DIR=/Volumes/Workspace/crabbuild-target/compass-capability-scorecards \
  ./scripts/qualify_code_graph_v1.sh --fixtures-only
sh scripts/check_product_boundary.sh
bash -n scripts/qualify_code_graph_v1.sh
git diff --check
```

Expected: all commands exit 0; fixture qualification prints the capability
stage, product-boundary check finds no Graphify dependency, and no whitespace
errors exist.

Then run the repository baseline from the Commands table. Expected: all checks
exit 0. Record any environment-only skipped gate and its precise reason in the
PR; do not call the plan complete while a changed-surface gate is unrun without
reviewer approval.

## Test plan

### Pure scorer tests

- strict v1 policy and report validation;
- stable composite identity and lexicographic ordering;
- no numeric aggregate field;
- required-lens conjunction and release-required conjunction;
- threshold fail versus under-sampled `insufficient_evidence`;
- zero-tolerance failure regardless of population size;
- nullable undefined metrics with diagnostics;
- bounded inputs, records, capabilities, slices, paths, and diagnostics;
- canonical byte equivalence under shuffled inputs.

### Fidelity adapter tests

- exact target, direction, source anchor, provenance, and multiplicity;
- deterministic one-to-one matching for duplicate occurrences;
- false candidates remain in the precision denominator;
- ambiguous, unresolved, unsupported, and represented-elsewhere facts remain
  visible;
- same bare capability name under different producers/packs stays separate;
- unsupported schema major and lossy required mapping fail closed.

### Structural-usefulness tests

- expected connected edge and bounded path recovered;
- missing expected bridge and isolated expected endpoint detected;
- fabricated bridge does not improve a capability to pass;
- exact-evidence-only topology differs truthfully from all-evidence topology;
- repeated occurrences do not inflate unique typed endpoint pairs;
- singleton/community diagnostics never become an implicit density gate.

### Agent-utility tests

- all mapped query IDs exist exactly once in the judgment corpus;
- multi-capability task membership is explicit and deterministic;
- positive, no-answer, ambiguity, direction, and path tasks score correctly;
- existing global relevance report is byte/metric compatible;
- small slices remain observational or insufficient, never numeric zero by
  accident.

### Integration tests

- fixture run is offline and byte deterministic;
- FastAPI and React inputs are pinned and read-only;
- a single failed capability produces non-zero exit and names that identity;
- no other passing capability hides the failure;
- product boundary, Rust baseline, and existing qualification gates pass.

## Done criteria

- [ ] The checked-in policy defines at least the seven initial capability IDs and
      validates strictly as `compass.code-graph-capability-policy/1`.
- [ ] The canonical report validates as
      `compass.code-graph-capability-scorecard/1` and contains no numeric global
      score or aggregate that can hide a failure.
- [ ] Fidelity, structural usefulness, and agent utility remain separate,
      nullable, evidence-linked lenses for every capability.
- [ ] Candidate populations are explicit and deterministic one-to-one matching
      preserves occurrence multiplicity.
- [ ] Missing evidence is `insufficient_evidence`; policy-inapplicable evidence
      is `not_applicable`; neither is silently converted to zero or pass.
- [ ] FastAPI route-handler and route-dependency slices use the existing pinned
      real repositories, not OpenClaw, and do not modify those checkouts.
- [ ] Existing audit, React, topology, graph, and query schema versions are
      unchanged; no product/package/crate version is bumped.
- [ ] `./scripts/qualify_code_graph_v1.sh --fixtures-only` includes the stable
      scorecard gate and exits 0 twice with byte-identical scorecard output.
- [ ] The targeted Python and query tests, native Rust baseline, and product
      boundary gate all pass.
- [ ] Documentation explains how to add facts/tasks and why graph volume,
      density, and community count are not quality scores.
- [ ] `git status --short` shows no generated graph, qualification checkout,
      local build artifact, or unrelated modification.
- [ ] This plan's status and `advisor-plans/README.md` are updated with exact
      execution commits and any blocked capability identities.

## STOP conditions

Stop and report; do not improvise if:

- an existing input schema or metric meaning must change to implement an
  adapter;
- independent truth cannot distinguish the target, direction, occurrence, or
  candidate population for a proposed required capability;
- a proposed threshold is justified only by current Compass output or by
  Graphify output rather than reviewed source/task evidence;
- a candidate selector would need to omit known false positives to pass;
- FastAPI/React pinned revisions differ or a qualification checkout is dirty
  and no clean read-only checkout is available;
- qualification would require importing/executing repository code, installing
  repository dependencies, making a network request, or using credentials;
- `/Volumes/Workspace` is unavailable for any Cargo build or real-repository
  artifact;
- fixture data alone is being used to claim production qualification;
- a capability needs a public graph/query schema or version change;
- a required mapping is lossy, a limit is exceeded, or deterministic repeated
  runs differ;
- a phase verification fails twice after a reasonable local correction.

## Maintenance notes

- Treat a capability policy change like an API review: explain which truth and
  candidate populations changed and whether historical reports remain
  comparable.
- Add a new capability as observational first. Promote a lens to required only
  after independent facts/tasks meet minimum denominators across the required
  corpora.
- When a scorecard exposes an extraction or resolution defect, fix it in the
  owning crate with a lowest-layer regression test, then rerun the unchanged
  scorecard. Never patch the evaluator to accommodate the defect.
- Keep historical scorecards immutable and keyed by policy digest, graph
  digest, corpus revision, and analyzer identities.
- Reviewers should scrutinize candidate selectors, excluded recall facts,
  zero-tolerance judgments, and threshold changes before metric arithmetic.
- Community quality remains intentionally evidence-gated. A future
  `architecture.community-navigation` capability should be driven by reviewed
  co-location/bridge tasks and stability measurements, not an aesthetic target
  for the number or size of communities.

## Rollback

Remove the invocation from `scripts/qualify_code_graph_v1.sh` first, then
revert the query sidecar/slice output, adapters, scorer, policy, and docs as one
qualification-only series. Do not change or roll back the public graph, audit,
topology, React, or query contracts. Historical generated scorecards remain
identified by their policy/input digests and must not be rewritten.
