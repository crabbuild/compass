# Compass Semantic Delta Engine Design

**Status:** Approved design

**Date:** 2026-07-24

**Scope:** Deterministic semantic change interpretation shared by reviewers,
CI compatibility gates, and architecture exploration

## Goal

Compass must explain how a program's meaning changed between two immutable Git
revisions, not merely which source lines or graph records changed.

The semantic delta must answer:

1. What behavior or contract changed?
2. What may break?
3. What code, systems, and tests are affected?
4. How strong and complete is the supporting evidence?

One deterministic engine produces a canonical, versioned report. PR reviewer
text, CI gates, architecture exploration, MCP, SARIF, and optional AI prose are
projections of that report. They do not independently reinterpret raw graph
changes.

## Decisions

1. Deterministic evidence is authoritative.
2. Optional AI may explain typed findings but cannot create findings, alter
   confidence, decide compatibility, assign gate results, or hide uncertainty.
3. The first product serves all three audiences through one engine:
   - concise PR review;
   - deterministic CI compatibility policy;
   - detailed architecture and dependency exploration.
4. Existing immutable history realizations and typed Prolly diffs remain the
   source of raw change evidence.
5. Existing `compass diff` behavior remains backward compatible. Semantic
   interpretation is selected explicitly with `--semantic`.
6. Compatibility, evidence strength, completeness, impact, and advisory risk
   are separate dimensions.
7. A semantic or impact claim without immutable supporting evidence is invalid.
8. Missing evidence never becomes a successful deterministic result.
9. Checkout-root and temporary-worktree identities must be normalized before
   semantic interpretation can support trusted dependency findings.
10. The first implementation remains repository-local. Cross-repository
    consumers reuse the same contracts in a later phase.

## Non-goals

The initial engine does not:

- replace `git diff` as a source patch viewer;
- prove arbitrary behavioral equivalence;
- claim compiler-grade semantics for capabilities backed only by syntax;
- infer organization-wide impact without registered downstream evidence;
- allow AI prose to affect CI;
- implement forge delivery, ownership, test execution, and cross-repository
  catalogs in the first slice;
- treat every implementation hash change as a known behavioral change;
- treat line movement, formatting, analysis churn, or temporary path churn as
  program meaning.

## Current foundation

Compass already provides most of the required substrate:

- exact immutable history realizations keyed by Git commit and build profile;
- comparable extraction fingerprints;
- streaming typed Prolly diffs;
- stable symbol identities where supported;
- `signature_hash`, `implementation_hash`, and `source_hash`;
- provider-neutral Program IR;
- deterministic function summaries containing calls, reads, writes, effects,
  errors, evidence, and capability coverage;
- reverse-call indexes;
- evidence provenance and capability-specific completeness;
- raw semantic, textual, location, analysis, and metadata diff categories.

The missing layer correlates these independent records into typed semantic
findings, compatibility decisions, impact witnesses, and human-scale change
stories.

## Product boundary

A new storage-independent interpretation package, `compass-delta`, owns the
semantic model and classifiers.

Dependency direction:

```text
compass-history ── raw GraphChange stream ──┐
compass-ir / compass-analysis ──────────────┼─> compass-delta
compass-model ──────────────────────────────┘         │
                                                      ├─> compass-cli
                                                      ├─> future PR intelligence
                                                      ├─> MCP
                                                      └─> architecture explorer
```

`compass-delta` may consume history's public diff interface, Program IR,
function summaries, graph model types, and optional source-hunk mappings. It
does not parse CLI flags, render terminal output, execute policies, call model
providers, or mutate history.

A shared operation in `compass-core` resolves revisions and comparable
realizations, opens the raw change stream, invokes `compass-delta`, evaluates
an optional trusted CompassQL policy over the completed deterministic
findings, and constructs the canonical report. CLI, MCP, CI, and future forge
adapters only parse transport input and render projections of that operation.

## Data flow

```text
Old immutable snapshot       New immutable snapshot
          │                           │
          └────── raw typed diff ─────┘
                         │
              Identity normalization
                         │
                Entity alignment
                         │
                Evidence fact set
                         │
              Semantic interpretation
                         │
              Contract classification
                         │
                 Story correlation
                         │
                  Impact analysis
                         │
              Canonical semantic report
               ┌─────────┼─────────┐
               │         │         │
           Reviewer     CI gate   Explorer
```

The engine distinguishes three levels:

```text
Evidence fact -> Semantic finding -> Impact finding
```

For example:

```text
Evidence fact
  Parameter `retry` was removed from `execute`.

Semantic finding
  The public callable contract became source-incompatible.

Impact finding
  Twelve callers still pass `retry`; eight are exact and four unresolved.
```

## Inputs and comparability

An operation accepts:

- old and new full Git object IDs;
- old and new realization IDs;
- normalized build profiles and extraction fingerprints;
- the typed node, edge, hyperedge, program-fact, and program-summary changes;
- access to old and new reverse dependency indexes;
- capability coverage and registered evidence;
- optional repository-relative Git hunk locations.

Normal operation requires comparable profiles. Existing `--fingerprint` and
`--allow-profile-mismatch` behavior remains available, but a profile-mismatched
semantic report is advisory. It cannot produce deterministic compatibility
gates because differences may come from extraction configuration.

Git hunks locate and display changes. They do not determine semantic meaning.

## Identity normalization and alignment

### Checkout-root independence

All symbol, module, and relationship identities used by the engine must be
independent of:

- the user's checkout directory;
- detached history worktree directories;
- temporary directory names;
- process IDs;
- timestamps;
- platform path separators.

Repository-local entities use logical repository-relative paths or
language-native identities. External entities use stable coordinates such as
package names, route operations, event subjects, database object identities, or
infrastructure resource addresses.

If an input identity contains a checkout or temporary-worktree prefix, the
engine records `unstable_identity`, excludes it from deterministic dependency
and gate decisions, lowers the affected capability to partial, and explains
the remediation.

### Cross-revision alignment

Entities align in this order:

1. exact stable Compass identity;
2. exact language-native or SCIP identity;
3. exact contract coordinate;
4. conservative structural move or rename candidate;
5. otherwise, independent removal and addition.

The first three are exact. Structural matches are `probable` and remain
advisory. A probable match cannot support a proven compatibility break.

Moves and renames retain their semantic identity only when exact evidence
supports that identity. Locations never participate in finding fingerprints.

## Canonical report

The public schema is:

```text
compass.semantic_delta.report/1
```

Conceptually:

```json
{
  "schema": "compass.semantic_delta.report",
  "schema_version": 1,
  "comparison": {},
  "summary": {},
  "stories": [],
  "findings": [],
  "impacts": [],
  "tests": [],
  "gates": [],
  "completeness": {},
  "provenance": {},
  "narrative": null
}
```

### Comparison identity

The comparison records:

- old and new commits;
- old and new realization IDs;
- old and new extraction fingerprints;
- profile-mismatch state;
- semantic engine version;
- relation-semantics registry version;
- classifier versions;
- optional policy digest;
- configured impact limits.

### Semantic finding

Every finding contains:

- stable finding fingerprint;
- finding kind and classifier version;
- exact or probable subject identity;
- structured before and after values;
- compatibility classification;
- evidence strength;
- capability completeness;
- repository-relative display locations;
- immutable evidence references;
- optional story membership;
- deterministic remediation when available.

### Impact finding

Every impact contains:

- stable impact fingerprint;
- source finding fingerprint;
- affected entity;
- impact category;
- old-graph, new-graph, or combined origin;
- shortest retained witness path;
- distance;
- weakest evidence strength on the witness;
- capability completeness;
- whether the impact can participate in a gate.

### Fingerprints

Finding fingerprints cover:

- fingerprint schema version;
- classifier and rule versions;
- finding kind;
- stable subject and contract identities;
- canonical structured before and after values;
- stable relationship identities required to establish the finding.

They exclude:

- source line and column numbers;
- timestamps;
- display labels;
- renderer formatting;
- optional AI prose;
- temporary paths;
- transient graph indexes.

Change-story fingerprints are derived from the sorted member finding
fingerprints and the story classifier version.

## Semantic finding taxonomy

### Entity lifecycle

- entity added or removed;
- symbol, type, module, package, route, schema, or resource moved;
- exact or probable rename;
- entity split or merged;
- public symbol exposed or hidden.

### Callable contracts

- parameter added, removed, renamed, or reordered;
- required parameter becomes optional or optional becomes required;
- default value changed;
- parameter type widened or narrowed;
- return type changed;
- sync/async contract changed;
- generic bounds changed;
- visibility changed;
- declared error contract changed.

Callable findings retain structured parameter information rather than only old
and new signature strings.

### Type contracts

- field or enum variant added or removed;
- field type, mutability, nullability, or visibility changed;
- base class, trait, or interface changed;
- required method added or removed;
- serialization shape changed;
- sealed/exhaustive contract changed.

### Behavior and effects

Derived from Program IR and deterministic summaries:

- call added or removed;
- resolved target changed;
- external interaction added or removed;
- read set changed;
- write set changed;
- mutation or side effect introduced or removed;
- await, concurrency, or blocking behavior changed;
- explicit return or exit behavior changed;
- error, throw, panic, or recovery path changed;
- transaction or resource lifetime changed;
- security-sensitive operation introduced or removed.

An implementation hash change without more specific supported evidence emits:

```text
implementation_changed
semantic_detail: indeterminate
```

The engine must not invent a behavioral explanation from a digest alone.

### Dependencies and architecture

- import or package dependency added or removed;
- dependency direction reversed;
- cross-module, cross-layer, cross-community, or cross-owner dependency
  introduced;
- architecture boundary bypassed;
- cycle introduced or resolved;
- critical abstraction gained or lost dependants;
- module cohesion materially changed.

Analysis-derived community or owner changes are supporting evidence, not
meaning by themselves. A community relabel does not become an architecture
finding.

### External contracts

The same finding model supports classifiers for:

- HTTP routes and request/response shapes;
- RPC operations and messages;
- events, topics, and payload schemas;
- database tables, columns, views, routines, and migrations;
- configuration keys and environment variables;
- package coordinates and constraints;
- infrastructure resources and externally referenced outputs.

Each family supplies an independent, versioned compatibility classifier.

## Compatibility model

Every semantic finding uses one compatibility classification:

- `proven_break`;
- `proven_compatible`;
- `possible_break`;
- `behavior_change`;
- `not_applicable`;
- `indeterminate`.

Examples:

```text
Remove a required public parameter      -> proven_break
Add an optional parameter               -> proven_compatible
Make a public symbol private            -> proven_break
Narrow an unresolved dynamic type       -> possible_break
Change a private implementation body    -> behavior_change
Change a body digest without enough IR  -> indeterminate
```

Compatibility is not severity. A proven break with no known consumers can
have low observed blast radius while still being a proven break. A compatible
change on a critical path can have high advisory risk.

A `proven_break` requires:

- exact subject identity;
- exact evidence for the changed contract;
- complete coverage for the relevant capability;
- a classifier that proves incompatibility for that language or contract
  family.

A classifier that cannot prove compatibility returns `possible_break` or
`indeterminate`; it never guesses.

## Evidence and completeness

These dimensions remain independent:

```text
identity:       exact | probable
evidence:       exact | inferred | ambiguous
coverage:       complete | partial | unavailable
compatibility:  proven_break | proven_compatible | possible_break |
                behavior_change | not_applicable | indeterminate
```

Coverage is capability-specific. One report may have complete source structure,
partial call resolution, and unavailable database-schema evidence.

Weak or missing evidence can lower confidence, make a gate indeterminate, or
raise advisory risk. It can never strengthen a claim.

## Change-story correlation

Independent findings that share subjects, evidence, call neighborhoods, or one
new subsystem are grouped into deterministic change stories.

Example:

```text
Batch failure handling introduced
  + RetryWithSmallerBatch added
  + BatchItemFailure added
  + split sync/async functions added
  + AsyncFunction imports batching
  + embedding paths gained retry and error behavior
```

Story formation uses bounded deterministic templates and sorted member
findings. It must remain useful without AI.

Optional AI may improve a story title or explanation. The original member
findings and evidence remain visible, and AI text does not change story
identity.

## Relation semantics

A versioned registry gives each relationship typed impact behavior. It owns:

- traversal direction;
- old-graph and new-graph behavior;
- contract significance;
- maximum useful depth;
- permitted evidence strengths;
- deterministic-gate eligibility;
- witness rendering;
- hub and boundary behavior.

Initial examples:

| Changed entity | Propagation direction |
|---|---|
| Function | Reverse calls toward callers |
| Interface or trait | Implementors and consumers |
| Module or package | Importers and downstream packages |
| Event or schema | Publishers and subscribers |
| Database object | Queries, jobs, APIs, and reports |
| Configuration | Readers and deployments |

Unknown relationships remain visible in the raw evidence but cannot propagate
impact until the registry defines their semantics.

## Impact analysis

For every eligible semantic finding:

1. start at the changed entity;
2. traverse both old and new reverse dependencies;
3. retain the shortest useful witness for each distinct affected entity;
4. remove duplicate and subsumed paths;
5. stop at configured boundaries, weak evidence, hubs, or depth;
6. separate exact, inferred, ambiguous, and unresolved impacts;
7. group display results by module, owner, community, contract, or repository.

The old graph finds consumers of removed contracts. The new graph finds
consumers of introduced contracts and behavior.

Impact categories are:

- `direct_consumer`;
- `transitive_consumer`;
- `implementation`;
- `data_consumer`;
- `deployment_consumer`;
- `downstream_repository`;
- `affected_test`;
- `possibly_affected`;
- `unresolved_consumer`.

Editing a function does not prove that every transitive caller changes
behavior. The engine reports a potential propagation path unless evidence
supports a stronger claim.

No witness means no impact claim.

### Blast-radius controls

Reviewer output:

- shows direct impacts first;
- groups transitive impacts;
- limits displayed examples while retaining exact counts;
- avoids expanding common utility hubs by default;
- retains separate witnesses for distinct owners, repositories, contracts, and
  gates.

Architecture exploration may request deeper or complete paths within configured
resource limits.

## Test impact

Tests are classified by evidence:

- `required`: exact per-test coverage or explicit trusted policy;
- `recommended`: exact static or build relationship;
- `suggested`: inferred or historical relationship.

Compass recommends a bounded minimal set that covers the greatest number of
affected exact entities, prioritizing public contracts and critical paths. It
reports the proportion of exact and inferred impact covered by the
recommendation.

Aggregate coverage may validate general execution but cannot establish
individual test identity.

## CI compatibility gates

CI evaluates the completed canonical report. It does not rerun classifiers.

Gate states are:

- `pass`: required evidence completed and no failure rule matched;
- `fail`: deterministic evidence matched an explicit policy;
- `warn`: advisory policy matched;
- `indeterminate`: evidence required by a policy is incomplete;
- `error`: the comparison or report cannot be trusted.

Missing evidence never becomes `pass`.

A policy may fail closed on `indeterminate`, but the output must say that the
policy failed because compatibility could not be established. It must not
relabel the semantic finding as `proven_break`.

Default Compass behavior is advisory. Enforcement is explicit:

```bash
compass diff main HEAD --semantic \
  --view ci \
  --policy .compass/compatibility.cql
```

A built-in gate supports simple adoption:

```bash
compass diff main HEAD --semantic --fail-on proven-break
```

Policies used in CI come from the trusted base revision or an administrator
configuration, never from an untrusted change head.

### Suppressions

Suppressions reference stable finding fingerprints:

```yaml
suppress:
  - finding: cmpsem:v1:abc123
    reason: "Intentional v3 API removal"
    expires: 2026-10-01
```

A suppression:

- does not remove the finding;
- appears in the report;
- requires a reason;
- may expire;
- cannot change evidence, compatibility, or completeness.

## CLI and projections

Existing raw behavior remains:

```bash
compass diff OLD NEW
```

Typed semantic interpretation is explicit:

```bash
compass diff OLD NEW --semantic
```

Supported projections:

```bash
# Concise reviewer report
compass diff main HEAD --semantic

# CI policy report
compass diff main HEAD --semantic \
  --view ci \
  --policy .compass/compatibility.cql

# Architecture detail
compass diff main HEAD --semantic --view architecture

# Canonical machine-readable report
compass diff main HEAD --semantic --format json

# Code-scanning annotations
compass diff main HEAD --semantic --format sarif

# Optional evidence-grounded explanation
compass diff main HEAD --semantic --summarize
```

`--semantic` conflicts with `--topology-only`. Raw `--detailed` continues to
mean raw record detail; semantic views provide their own structured explanation
and filters.

Initial semantic filters:

```text
--module PATH
--symbol ID
--relation NAME
--kind KIND
--confidence exact|inferred|ambiguous
--impact-depth N
```

### Reviewer projection

Reviewer output answers, in order:

1. what behavior changed;
2. what may break;
3. what is affected;
4. how certain Compass is.

Example:

```text
Semantic change summary
  Compatibility: no proven public API breaks
  Behavior: batch failures are now retried as smaller batches
  Impact: 3 production entry points, 8 tests
  Evidence: high confidence; call resolution partial in 2 modules

Change stories

  Batch failure isolation introduced
    RetryWithSmallerBatch and BatchItemFailure added.
    AsyncFunction now converts failed batches into per-item outcomes.
    LiteLLM and SentenceTransformer paths gained smaller-batch retry behavior.

    Compatibility
      Public signatures unchanged
      Runtime behavior changed

    Affected
      AsyncFunction._execute
      FunctionExecutor
      LiteLLMEmbedder._embed
      SentenceTransformerEmbedder._embed

Possible concerns
  Call resolution is incomplete for two dynamic dispatch sites.
  No proven break; manual review recommended.
```

Raw hashes are hidden by default. JSON and a finding-specific explanation expose
the complete evidence.

### CI projection

```text
Compass compatibility gate: PASS

  0 proven breaks
  2 behavior changes
  1 possible break
  0 suppressed failures
  8 required tests identified
  Evidence completeness: 94%

Advisory
  Dynamic call resolution was unavailable for two consumers.
```

SARIF annotations are reserved for actionable findings:

- proven or possible compatibility breaks;
- new policy violations;
- uncovered critical impacts;
- expired suppressions.

Analysis noise remains in the complete report instead of becoming hundreds of
inline annotations.

### Architecture projection

Architecture output organizes changes by boundary:

```text
Module dependencies
  + _internal/api -> _internal/batching
  + _internal/function -> _internal/batching

Call-flow changes
  + _run_split_async -> BatchItemFailure
  + _run_split_sync  -> BatchItemFailure

Data/effect changes
  LiteLLMEmbedder._embed
    + may raise RetryWithSmallerBatch
    + performs retry classification

Boundary changes
  New internal batching subsystem
  No new cross-package dependency
  No dependency cycle introduced
```

A later HTML explorer consumes the canonical report for before/after dependency
diagrams, expandable witnesses, symbol timelines, and commit-by-commit change
stories. The browser never recalculates findings.

## Optional AI narrative

`--summarize` receives only:

- canonical typed findings;
- structured before and after contracts;
- witness paths;
- bounded source snippets explicitly attached as evidence;
- completeness and uncertainty metadata.

It returns a non-authoritative attachment:

```json
{
  "authoritative": false,
  "report_digest": "sha256:...",
  "provider": "...",
  "model": "...",
  "text": "..."
}
```

Rules:

- AI prose never enters deterministic fingerprints or the canonical report
  digest.
- Every narrative paragraph cites finding fingerprints.
- A validator rejects references to findings absent from the report.
- AI failure leaves the deterministic report valid and complete.
- CI and policy evaluation ignore the narrative.
- Provider input is bounded, redacted, and explicitly enabled.
- Repository text is treated as untrusted data, not model instructions.

## Errors and uncertainty

Operation errors produce no semantic report:

- corrupt or invalid history realization;
- unknown revision;
- unapproved profile mismatch;
- invalid Program IR or analysis invariants;
- evidence associated with the wrong revision;
- report serialization or schema failure.

Incomplete capabilities produce a valid report with limitations:

- unsupported language capability;
- unresolved dynamic call;
- unavailable SCIP or compiler artifact;
- probable identity match;
- unknown relation semantics;
- unstable path-derived identity.

The affected finding becomes advisory or indeterminate. The report names the
capability, affected scope, reason, and remediation.

Optional summarization failure is reported separately and never invalidates the
deterministic result.

## Security and trust

- CI policies come from a trusted base revision or administrator source.
- Repository-relative paths are required in evidence and reports.
- Source snippets sent to providers are bounded and explicitly enabled.
- Credential-shaped values are redacted before provider input.
- Renderers escape hostile labels, paths, and source text.
- Imported evidence is schema-validated and size-bounded.
- Inferred or ambiguous evidence cannot support deterministic failure by
  default.
- Untrusted repository configuration cannot weaken organization policy.

## Performance and caching

Semantic work remains proportional to changed subtrees and reached impact
neighborhoods:

1. reuse Prolly structural sharing;
2. stream raw differences;
3. decode changed records only;
4. load reverse dependencies for changed entities only;
5. traverse bounded impact neighborhoods;
6. cache the canonical report;
7. run optional AI after deterministic completion.

The report cache identity includes:

- old and new realization IDs;
- semantic engine version;
- relation-semantics version;
- classifier versions;
- policy digest;
- impact configuration.

Performance invariants:

- equal roots return without semantic traversal;
- no full graph reconstruction is required;
- memory is bounded by configured findings and witnesses;
- aggregate counts remain exact when examples are bounded;
- CI reuses the report rather than reanalyzing;
- architecture expansion lazily requests deeper paths;
- cancelled or broken output stops traversal promptly.

## Rollout

### Phase 0: identity and noise hardening

- Normalize detached-history worktree paths.
- Guarantee checkout-root-independent module and edge identities.
- Detect unstable identities.
- Add replay tests proving repeated builds of the same commit produce identical
  semantic records.

### Phase 1: typed semantic report

- Add the versioned report schema.
- Classify entity lifecycle, signature, visibility, implementation, import,
  dependency, and call changes.
- Add deterministic compatibility classification.
- Render reviewer text and JSON.

### Phase 2: behavior and local impact

- Diff calls, reads, writes, awaits, effects, and errors.
- Add old/new reverse-dependency traversal.
- Retain witness paths.
- Group deterministic change stories.
- Recommend affected tests.
- Render the architecture projection.

### Phase 3: CI policy and delivery

- Evaluate CompassQL compatibility policies.
- Add gate states and stable exit codes.
- Add suppression lifecycle.
- Add SARIF.
- Expose the report through shared CLI and MCP operation adapters.

### Phase 4: narrative and exploration

- Add optional evidence-grounded summaries.
- Validate narrative finding references.
- Add the historical architecture explorer.
- Add registered cross-repository consumers without changing local finding
  semantics.

## Verification

### Deterministic fixtures

Fixtures prove:

- required public parameter removal is `proven_break`;
- optional parameter addition is `proven_compatible`;
- public-to-private visibility is `proven_break`;
- body-only change is `behavior_change`;
- digest-only evidence is `indeterminate`;
- formatting-only edit creates no semantic finding;
- exact move is not removal plus addition;
- probable rename remains advisory;
- import addition creates a dependency finding;
- dependency cycle introduction creates an architecture finding;
- added error path creates a behavior finding;
- unresolved dynamic calls produce incomplete impact.

### Property tests

- Reverse comparison swaps before/after and add/remove symmetrically.
- Identical inputs produce byte-identical deterministic reports.
- Location movement does not alter finding fingerprints.
- Finding and story ordering is deterministic.
- Weak evidence never raises confidence.
- Missing required evidence never produces `pass`.
- Optional AI output cannot change findings or gates.
- Suppressions never remove findings from the canonical report.
- Old/new witness traversal finds consumers of both removed and introduced
  contracts.

### Cross-language qualification

Equivalent contract and behavior fixtures cover Rust, Python,
TypeScript/JavaScript, Go, Java/Kotlin, C/C++, C#, and Swift. Each report states
actual capability coverage; it does not imply uniform precision.

### Adversarial qualification

Tests cover:

- temporary and absolute path leakage;
- hostile labels and source text;
- cyclic and oversized evidence;
- malformed Program IR;
- ambiguous identity matches;
- unknown relationships;
- huge hubs and path explosions;
- broken output writers and cancellation;
- untrusted policy changes;
- provider timeouts and malformed narrative references.

### Real-repository replay

The cocoindex batching change is a mandatory qualification:

```text
Expected
  New batching subsystem recognized.
  No existing signature break claimed.
  Seven implementation changes recognized.
  New batching dependencies grouped.
  Retry and error behavior linked to affected functions.
  Temporary worktree identities absent.
  Repeated builds produce identical reports.
```

## Acceptance criteria

The design is complete when:

1. A reviewer can understand behavior, compatibility, impact, and uncertainty
   without reading raw graph records.
2. CI fails only from explicit trusted policy over deterministic evidence.
3. Architecture users can inspect dependency, call-flow, and data-flow
   evolution with witness paths.
4. Every nontrivial semantic or impact claim traces to immutable evidence.
5. Unsupported analysis is incomplete rather than fabricated.
6. Existing raw `compass diff` behavior remains compatible.
7. Semantic output is deterministic without AI.
8. Optional AI improves readability but has no authority.
9. Repeated historical builds do not create path-derived semantic churn.
10. The same canonical report drives reviewer, CI, architecture, MCP, SARIF,
    and future forge projections.
