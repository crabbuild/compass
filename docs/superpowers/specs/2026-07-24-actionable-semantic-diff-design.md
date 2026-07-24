# Actionable Semantic Diff for PR Reviewers

**Date:** 2026-07-24

**Status:** Approved in design discussion; pending written-spec review

**Implementation root:** `/Users/haipingfu/graphify/compass`

**Supersedes:** The public diff behavior in
`docs/superpowers/specs/2026-07-22-versioned-graph-gap-mitigations-design.md`

## Purpose

Replace Compass's record-oriented graph diff with an actionable semantic review
that answers three reviewer questions:

1. What behavior or contract changed?
2. What may break?
3. What code and tests are affected?

The existing diff correctly compares immutable history records, but its public
output exposes storage-level changes. A real cocoindex comparison emitted about
65 MB of JSON, including tens of thousands of analysis and metadata records.
Reviewers had to write `jq` queries to discover signature changes,
implementation changes, imports, calls, and affected symbols. Some dependency
edges also leaked temporary worktree paths, creating false architectural churn.

This design keeps the immutable history and Prolly diff foundation, but replaces
the public command, renderer, and JSON contract in one hard cutover. Compass has
not shipped a stable semantic-diff contract, so compatibility code, deprecation
flags, dual schemas, and legacy renderers are explicitly out of scope.

## Approved product decisions

- PR reviewers are the primary audience.
- The default report is actionable-first, not exhaustive.
- Deterministic evidence creates findings, compatibility, confidence, impact,
  verification status, and reviewer actions.
- An optional AI model may summarize completed findings but cannot create or
  alter them.
- Routine symbol churn, metadata, positional changes, and generated noise are
  collapsed.
- Missing evidence is explicit and never becomes a compatibility claim.
- The current `compass diff` behavior is removed rather than deprecated.
- The current raw JSON diff schema is removed rather than versioned alongside
  the replacement.
- Low-level record streaming remains an internal library capability for
  implementation and tests, not a public compatibility surface.
- Stable logical identity is a prerequisite. Absolute checkout paths and
  temporary history-worktree paths cannot participate in findings.

## Goals

1. Produce a concise report of likely breaks, behavior changes, affected
   consumers, dependency changes, and verification gaps.
2. Correlate multiple raw record changes into one reviewer-level finding.
3. Distinguish directly edited entities from derived impact.
4. Preserve exact evidence, witness paths, confidence, and capability-specific
   completeness for every claim.
5. Reuse immutable history, Program IR, function summaries, and reverse-call
   indexes without reconstructing or serializing a complete raw diff.
6. Give the CLI, JSON consumers, MCP, CI, and future PR delivery one canonical
   typed report.
7. Remain useful offline and without a model configured.

## Non-goals

- Preserving the old text output, JSON schema, option names, or raw-change
  ordering.
- Treating an implementation hash as an explanation of behavior.
- Proving compatibility for a language without an exact classifier and
  sufficient evidence.
- Inventing test gaps when test evidence is unavailable or incomplete.
- Assigning organization-specific risk bands or CI gate results. A later
  decision module may consume this report.
- Fetching Git objects, compiler artifacts, coverage, or downstream graphs
  implicitly.
- Using an AI model as a semantic classifier or source of evidence.

## Hard-cutover command contract

The public command becomes:

```text
compass diff OLD NEW
  [--format text|json]
  [--all]
  [--explain FINDING_ID]
  [--summarize]
  [--fingerprint SHA256]
```

Default `compass diff OLD NEW` renders the actionable semantic review.

The following legacy options are removed:

```text
--detailed
--topology-only
--include-locations
--include-analysis
--include-metadata
--allow-profile-mismatch
```

Unknown removed options fail with the new command usage. Documentation, help,
examples, tests, and the bundled Compass skill change in the same cutover.

`--all` expands compatible additions, lower-priority internal changes, probable
moves, and collapsed routine groups. It still does not expose storage metadata
or raw Prolly records.

`--explain FINDING_ID` renders one finding with all retained evidence,
capability coverage, source anchors, affected consumers, and witness paths.

`--summarize` adds an explicitly labeled generated summary. It never changes
the deterministic sections or command success when the model is unavailable.

`--fingerprint` selects the same extraction fingerprint at both commits. Normal
selection still requires semantically comparable normalized build profiles.
Cross-profile semantic interpretation is rejected; there is no
`--allow-profile-mismatch` escape hatch.

## Architecture

Add a reusable `compass-semantic-diff` crate:

```text
Git exact revisions and changed hunks
                 +
compass-history immutable comparable realizations
                 |
                 v
compass-semantic-diff
  identity -> correlation -> contract -> behavior
           -> relations -> impact -> verification
           -> ranking -> report
                 |
                 v
compass.semantic_diff.report/1
                 |
                 +-> CLI text
                 +-> JSON
                 +-> MCP / PR / CI adapters
                 `-> optional grounded summary
```

Component responsibilities:

- `compass-history` owns immutable storage, comparable realization selection,
  authenticated record lookup, and raw Prolly traversal.
- `compass-ir` owns provider-neutral symbols, types, calls, reads, writes,
  awaits, errors, effects, evidence, and capability coverage.
- `compass-analysis` owns deterministic function summaries, summary digests,
  and reverse-call indexes.
- `compass-semantic-diff` owns cross-revision alignment, semantic
  interpretation, relation behavior, impact, verification, ranking, and the
  canonical report.
- `compass-cli` owns argument parsing and rendering only.
- Model adapters accept a completed report and return optional prose with
  finding references.

The semantic engine does not discover repositories, run Git, open stores,
render terminal output, or call providers. Its operation input contains exact
revision identities, comparable snapshot handles, a bounded source-delta
description, configured evidence adapters, and resource limits.

## Processing model

### Phase 1: Direct-change collection

The Git adapter computes exact file statuses, renames, and hunks between the
resolved commits without fetching. Hunks map to the smallest enclosing entities
in the old and new snapshots.

The engine streams node and edge differences and retains only:

- directly changed entities;
- exact or probable alignment candidates;
- relationships incident to those entities;
- bounded counters for collapsed routine changes;
- capability and identity limitations.

Analysis and metadata trees are not streamed wholesale. Program facts and
summaries are loaded by stable key for directly changed entities and later for
the bounded impact neighborhood.

Each semantic delta records its origin:

- `direct`: the entity's definition, contract, or implementation changed in
  edited source;
- `derived`: a relationship, summary, consumer, or verification result changed
  because of a direct delta.

### Phase 2: Correlation and interpretation

Related evidence is grouped by stable subject. A body hash change, two new
calls, a newly thrown error, and affected callers become one behavior finding,
not separate storage-record lines.

The engine then:

1. classifies contract compatibility;
2. compares behavior summaries;
3. interprets typed relationship changes;
4. traverses affected consumers with bounded witness paths;
5. evaluates configured test evidence;
6. deduplicates and subsumes redundant findings;
7. ranks actionable findings;
8. builds one canonical report.

Resource limits cap retained entities, relationships, witnesses, and findings.
Exceeding a safety limit returns an explicit resource-limit error; Compass does
not silently publish a partial successful report.

## Identity integrity and entity alignment

Identity validation runs before interpretation:

- source paths must be repository-logical and normalized;
- checkout roots, `.git/compass/tmp`, random worktree names, and absolute paths
  are forbidden in semantic IDs and relationship endpoints;
- logically equal builds from different directories must produce equal
  identities;
- rejected evidence becomes a capability limitation with its source, never a
  false addition or removal.

Entities align conservatively:

1. exact stable Compass identity;
2. exact language-native identity, such as a fully qualified or SCIP symbol;
3. container, signature, and structural fingerprints for a probable
   move/rename;
4. otherwise, separate removal and addition.

Only the first two are exact. Structural matches remain advisory and cannot
support a proven compatibility claim.

## Finding contract

The canonical schema identifier is:

```text
compass.semantic_diff.report/1
```

Conceptually:

```text
SemanticDiffReport
  schema
  comparison
  findings[]
  collapsed_groups[]
  completeness
  limitations[]
  generated_summary?

SemanticFinding
  id
  type
  subject
  origin
  headline
  explanation
  compatibility
  confidence
  review_priority
  before
  after
  affected_consumers[]
  witness_paths[]
  verification
  reviewer_action
  evidence[]
  completeness
```

Finding IDs use a stable digest over the finding schema, classifier version,
stable subject identity, finding type, before/after semantic values, and
retained relationship identities. They exclude line numbers, timestamps,
display formatting, random worktree paths, and model prose.

`review_priority` orders the report but is not an organization policy risk
band. A future decision module may combine semantic findings with criticality,
ownership, downstream, and policy evidence.

## Finding types

### Contract change

Parameters, return types, visibility, sync/async behavior, inheritance,
generic constraints, routes, schemas, configuration keys, and other consumed
shapes.

### Behavior change

Calls, reads, writes, external effects, awaited operations, returned errors,
thrown exceptions, or other supported function-summary facts changed.

### Dependency change

A module, package, service, schema, or infrastructure resource gained or lost a
dependency; dependency direction or cycle membership changed.

### Impact change

Callers, implementations, consumers, owners, or downstream contracts are
affected through retained witness paths.

### Verification gap

Complete test evidence proves that no mapped test covers a changed or affected
behavior, or exact required verification is stale or failing.

### Structural change

An entity moved or was probably renamed without a material contract or behavior
change. Structural findings are collapsed unless they alter public identity,
ownership, or an architectural boundary.

## Compatibility, confidence, and completeness

Compatibility and confidence are independent:

```text
compatibility:
  proven_break
  possible_break
  compatible
  behavioral
  not_applicable
  indeterminate

confidence:
  exact
  probable
  inferred
  unknown
```

Every finding also carries per-capability completeness, for example:

```text
signature: complete
implementation: complete
call_resolution: partial
effects: complete
test_mapping: unavailable
```

Compass never reduces this to one opaque confidence number.

Uncertainty may lower confidence or increase reviewer priority. It cannot
convert a possible break into a compatible change.

Initial deterministic rules include:

| Change | Classification when exact |
|---|---|
| Required parameter added | `proven_break` |
| Parameter removed or incompatibly reordered | `proven_break` |
| Optional parameter added | `compatible` |
| Default value changed | `behavioral` or `possible_break` |
| Return type incompatibly narrowed | `proven_break` |
| Public visibility reduced | `proven_break` |
| Sync changed to async or vice versa | `proven_break` |
| New externally observable error | `behavioral` or `possible_break` |
| External write or side effect added | `behavioral` |
| Dependency cycle introduced | `possible_break` |
| Exact resolved consumer affected | exact impact |
| Dynamic possible consumer affected | inferred impact |

Language adapters own signature and type-compatibility rules. The first exact
adapters cover Python, Rust, and TypeScript/JavaScript. Other languages report
the exact before/after signature when available but use `indeterminate`
compatibility until an adapter can prove more.

An implementation hash is only a change detector. If the hash changes without
sufficient Program IR, Compass reports an implementation change with incomplete
semantic coverage; it does not narrate the behavior.

## Relation semantics and bounded impact

Traversal follows relationship meaning rather than one universal graph
direction:

- changed functions affect callers;
- changed interfaces affect implementations and consumers;
- changed imports affect importing modules;
- changed events affect publishers and subscribers;
- changed data or schemas affect readers and writers;
- changed configuration affects readers and deployments;
- changed packages affect importers.

Direct exact consumers appear individually. Transitive consumers are grouped
by module, owner, repository, or architectural boundary after a
relation-specific bounded depth. Redundant paths are subsumed, but every
distinct reported consumer group retains at least one shortest useful witness.

The weakest relationship on a path determines path confidence. Any inferred or
ambiguous hop makes the resulting impact advisory.

## Verification semantics

Verification evidence is capability-specific:

- no test integration configured: `unknown`;
- test evidence present but incomplete: `partial`;
- exact mapping to passing current tests: `covered`;
- exact complete mapping with no covering test: `gap`;
- mapped required test stale, failing, or not run: its exact state.

Compass never turns unavailable coverage into a test-gap claim. Static test
relationships may recommend tests but remain distinct from runtime coverage.

The MVP provides basic static mapping from test entities, resolved calls, and
build relationships. Later adapters add per-test runtime coverage, build
targets, and minimal test-set selection.

## Actionable-first rendering

Text output begins with exact revisions and a bounded summary:

```text
Semantic review: 9057153 -> 71f9cc9
2 likely breaks · 3 behavior changes · 6 affected consumers · 1 test gap
```

Default ordering:

1. proven breaks;
2. possible breaks with exact affected consumers;
3. material behavior and side-effect changes;
4. cross-module or architectural dependency changes;
5. verification gaps;
6. compatible API additions;
7. routine internal churn and structural changes, collapsed.

Every expanded finding answers:

- what changed;
- why it matters;
- what is affected;
- how Compass knows;
- what evidence is incomplete;
- what the reviewer should do.

The default display budget is 20 actionable findings. Proven breaks are never
hidden by the budget. Additional findings are grouped and counted, and `--all`
expands them. JSON contains all retained findings subject to configured safety
limits.

Location-only, formatting-only, metadata, community assignment, and generated
artifact changes do not appear as reviewer findings. Relevant source anchors
remain attached to evidence and explanations.

## Optional grounded summary

`--summarize` sends only the completed typed report and bounded evidence details
to the configured model. The model response is separate from deterministic
findings and labeled as generated.

Validation requires every behavioral, compatibility, impact, or verification
claim in generated output to reference known finding IDs. Unknown IDs, altered
counts, altered compatibility, or altered verification states reject the
generated summary.

Model failure, timeout, or missing configuration does not affect deterministic
report generation. The command succeeds with an explicit summary limitation.

## Error behavior

- Unknown revisions fail before opening or creating history state.
- Missing Git objects fail without fetching.
- Missing realizations materialize through the existing exact-tree history
  resolver.
- Different normalized build profiles fail as non-comparable.
- A requested fingerprint must exist or be materializable at both revisions.
- Corrupt history fails without a partial semantic report.
- Missing Program IR reduces behavior completeness while preserving supported
  contract findings.
- Unstable identities exclude the affected evidence and emit a limitation.
- Probable moves and unresolved calls remain advisory.
- Resource-limit exhaustion is an operation error, not a truncated success.
- AI errors never erase or mutate deterministic findings.

## Delivery stages

### Stage 0: Identity correctness

- Normalize checkout and temporary-worktree paths.
- Reject absolute paths in semantic identities.
- Fix the unstable Astro/JavaScript dependency identities observed in
  cocoindex.
- Prove same-commit, different-directory semantic equality.

### Stage 1: Hard-cutover deterministic MVP

- Add `compass-semantic-diff` and `compass.semantic_diff.report/1`.
- Add exact Git hunk-to-entity mapping.
- Add entity alignment and identity limitations.
- Add Python, Rust, and TypeScript/JavaScript contract adapters.
- Compare Program IR behavior summaries where coverage exists.
- Interpret imports, calls, and dependency changes.
- Calculate bounded local impact and witness paths.
- Add basic static test mapping.
- Add ranking, collapsing, text, JSON, `--all`, and `--explain`.
- Replace the old CLI, JSON schema, renderer, help, tests, docs, and examples in
  the same change.
- Delete superseded renderer and classification code rather than leaving
  compatibility branches.

### Stage 2: Deeper impact and verification

- Expand effect, error, interface, event, schema, configuration, package, and
  infrastructure interpretation.
- Add dependency-cycle and architectural-boundary findings.
- Add ownership, build-target, and runtime test adapters.
- Add minimal recommended test sets.
- Add more language compatibility adapters.

### Stage 3: Grounded summaries and delivery adapters

- Add optional validated model summaries.
- Project the canonical report into MCP, GitHub checks/comments, SARIF, and CI
  policy inputs.

## Verification

### Identity and determinism

- Rebuild one commit in different absolute directories and temporary worktrees;
  the semantic report is empty.
- Temporary path names never enter subject, edge, evidence, or finding IDs.
- Repeated reports are byte-identical.
- Forward and reverse comparisons swap before/after values and added/removed
  semantics while re-evaluating directional compatibility.

### Contract classifiers

Fixtures cover required, optional, reordered, removed, renamed, variadic, and
defaulted parameters; return types; visibility; sync/async; inheritance; and
generic constraints.

Every language adapter has positive, negative, ambiguous, and unsupported
cases. Unsupported constructs return `indeterminate`.

### Behavior and impact

Fixtures cover added and removed calls, reads, writes, awaits, errors, external
effects, exact callers, unresolved callers, interface implementations,
dependency directions, cycles, and bounded path subsumption.

A body-only edit produces a behavior finding when supported and an incomplete
implementation finding otherwise.

### Verification evidence

Tests distinguish exact runtime coverage, static mapping, partial evidence,
stale evidence, missing integration, and complete evidence with no mapped
test. Only the last case produces a true gap.

### Safety properties

- Missing evidence never becomes `compatible`.
- Inferred evidence never becomes a proven break.
- A probable rename never becomes exact identity.
- Uncertainty never lowers reviewer priority through arithmetic averaging.
- A model cannot create a finding, change compatibility, or alter verification.
- Corrupt snapshots and resource limits fail closed.

### End-to-end qualification

Mandatory repositories include:

- a synthetic multi-language contract fixture;
- a body-only behavior fixture;
- a dependency-cycle fixture;
- the historical LevelDB cases;
- the two stored cocoindex revisions used during this investigation.

The cocoindex qualification must produce a bounded actionable report, identify
the batching-related behavior and dependency changes supported by evidence,
avoid temporary-worktree dependency churn, and avoid materializing the former
65 MB public raw diff.

## Acceptance criteria

The hard cutover is complete when:

1. `compass diff OLD NEW` emits only the new actionable semantic review.
2. The old raw text renderer, raw JSON schema, removed flags, and associated
   compatibility branches no longer exist.
3. A required public parameter produces an exact proven-break finding with
   affected callers.
4. A body-only edit reports changed behavior when Program IR supports it.
5. An unsupported behavior change is explicit and indeterminate.
6. A module dependency change uses logical identities and retained evidence.
7. Test gaps are emitted only from sufficiently complete test evidence.
8. Routine metadata, location, and analysis churn is absent from the reviewer
   report.
9. Every actionable claim has evidence and a source anchor or witness path.
10. Same-commit rebuilds from different paths produce zero semantic findings.
11. Text, JSON, and explain views derive from one canonical report.
12. The deterministic command works fully with no model configured.
13. Docs, help, bundled skill references, and examples describe only the new
    behavior.
