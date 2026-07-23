# Compass PR Intelligence Design

**Status:** Approved design

**Date:** 2026-07-22

**Initial forge:** GitHub.com and GitHub Enterprise Server

**Analysis SLA:** Five minutes at the declared scale tier

## Goal

Compass PR Intelligence gives a reviewer one reproducible answer to:

- What changed architecturally?
- What can break locally and in registered downstream repositories?
- Which owners should review the change, and why?
- Which tests are required, recommended, or missing?
- Which concurrent pull requests interact with this change?
- Why is the change risky?
- Which findings are deterministic enough to block merging?

The initial product is reviewer-first. Advisory intelligence never blocks merging. A separate selective gate blocks only deterministic architecture-policy violations, proven contract breaks, or explicitly required tests that did not run successfully.

The analysis does not require an organization graph for the pull request repository. Local analysis requires the referenced Git objects and a valid Compass extraction configuration; Compass never fetches objects implicitly. When registered downstream graphs are available, the same report includes cross-repository consumers with explicit freshness and completeness. Compass waits for local, downstream, ownership, and test-impact analysis before publishing one completed report. Required tests may continue running after analysis; their gate remains pending until results bound to the exact merge revision arrive.

## Non-goals

The initial release does not:

- Modify source code.
- Automatically request reviewers.
- Reorder a merge queue.
- Execute arbitrary code from a pull request.
- Use an opaque machine-learning score as a gate.
- Treat inferred relationships as deterministic evidence.
- Rebuild an entire organization during each pull request.
- Claim organization-wide completeness when downstream evidence is unavailable.

## Product decisions

1. The core is an evidence-first semantic change engine.
2. CompassQL supplies organization-specific policies and risk definitions.
3. Historical and statistical signals are later advisory enrichments.
4. Advisory risk and deterministic gates are separate outputs.
5. The first release analyzes the local repository plus registered downstream consumers.
6. The analysis SLA is five minutes; test execution has its own CI lifetime.
7. CLI, MCP, CI, and forge delivery consume one typed result.
8. GitHub is the first forge, but forge details do not leak into analysis semantics.

## Standalone product boundary

Compass owns every PR Intelligence contract: repository identity, report schemas, command syntax, Model Context Protocol (MCP) tools, storage paths, configuration, release cadence, and deprecation policy.

PR Intelligence tests assert Compass behavior directly. Compass-specific runtime code, fixtures, schemas, migrations, and release automation live in the Compass repository. Standard interchange formats such as Git, SARIF, JUnit, LCOV, Cobertura, JaCoCo, and MCP remain product-neutral inputs and outputs.

Compass versions its public schemas and migrations independently. A breaking command, tool, or report change follows Compass's own deprecation and schema-version policy.

## Report contract

The primary output is `compass.pr_intelligence.report/1`. Its identity includes the repository and pull request identities; the merge-base, pull request head, target head, and synthetic merge-result identities; the graph schema version; the extractor version and configuration digest; the policy-pack digest; and the evidence-manifest digest.

Identical inputs produce byte-equivalent canonical findings. Presentation metadata such as timestamps and durations is kept outside the canonical finding set.

The report contains:

1. **Architecture delta:** changed entities, relationships, contracts, schemas, configuration, and architecture structure.
2. **Impact:** direct and transitive consumers with witness paths.
3. **Review routing:** direct, contract, downstream, and policy owners.
4. **Verification plan:** required, recommended, and suggested tests plus coverage gaps.
5. **Concurrent overlap:** interactions with open or queued pull requests.
6. **Advisory risk:** an explainable risk band and independent factors.
7. **Deterministic gates:** pass, fail, indeterminate, or error for gate-enabled rules.
8. **Provenance and completeness:** revisions, evidence sources, freshness, graph coverage, and failures.

Every claim carries its type and stable fingerprint, a human-readable statement, source and target entity identities, an optional witness path, the source revision, the evidence source and digest, confidence, completeness, freshness, and remediation.

Finding fingerprints use `cmpprv1:<lowercase-sha256>`. The digest covers the fingerprint schema, finding type, rule or classifier version, stable entity identities, ordered relationship identities in the witness, and canonical scalar evidence. It excludes source line numbers, timestamps, display formatting, and transient graph indexes.

### Completeness

The top-level report and every cross-repository finding use these states:

- `local_exact`
- `downstream_complete`
- `downstream_partial`
- `downstream_unavailable`

Completeness also lists every registered repository considered, its graph revision, freshness, authorization outcome, and failure state. An unavailable or unauthorized graph is not interpreted as proof that it has no affected consumers.

Downstream evidence is fresh only when its graph revision matches the registered default-branch head observed in the immutable evidence manifest. A graph behind that head may still produce advisory findings, but it cannot satisfy a deterministic rule that requires complete downstream evidence. Installations may impose a stricter maximum graph age; they may not relax the exact-head requirement for deterministic downstream gates.

## Revision semantics

One pull request analysis captures four immutable revision identities. Comparing the merge base with the pull request head explains the author's change intent. Comparing the target head with the synthetic merge evaluates the actual candidate merge after the target branch has advanced.

The change-request source resolves full commit object IDs before analysis begins. The evidence manifest is then immutable. A force push or target-branch update creates a new analysis identity; it cannot mutate an in-flight report.

If Git cannot construct the synthetic merge because of a textual conflict, Compass reports that conflict, omits merge-result graph conclusions, and marks merge-result-dependent gates indeterminate. It does not analyze the pull request head as though it were the merge result.

Merge queues are first-class analysis targets. A `merge_group` event is analyzed against the exact temporary group revision, including pull requests ahead of the current item.

## Architecture

The shared PR Intelligence operation is the external seam. CLI, MCP, CI, and forge delivery are adapters. They may parse transport input and render the typed report, but they may not calculate findings, risk, or gates.

### Change-request source module

This deep module owns repository identity, pull request identity, exact revisions, diff hunks, file status and renames, review state, checks, pagination, rate limits, and forge error semantics.

The initial adapters are:

- GitHub App or Actions event input for CI.
- GitHub CLI for local interactive use.
- Local Git for explicit revision analysis.

GitHub adapters support GitHub.com and configured GitHub Enterprise Server hostnames. Repository identity includes forge kind, hostname, owner, and repository name. GitLab and Bitbucket justify the seam later as additional adapters. The low-level process runner remains an internal implementation detail rather than the forge interface.

### Snapshot module

This module reuses the graph-selection and immutable snapshot model shared with CompassQL and versioned graphs. It loads or builds graph snapshots keyed by repository, commit, extractor version, extractor configuration, and graph schema.

The module supplies exact snapshots for merge base, pull request head, target head, synthetic merge result, and registered downstream revisions. It retains each history activity guard through artifact validation and reconstruction. The operation then owns the reconstructed snapshots and verifies their identities again before report persistence.

### Semantic delta module

This module maps changed hunks to graph entities and classifies entity and relationship changes. It owns cross-revision identity alignment, rename and move inference, contract classifiers, and architecture-delta normalization.

### Relation semantics module

This module gives graph relationships typed impact behavior. It owns traversal direction, contract significance, permitted depth, confidence treatment, and witness rendering for calls, imports, inheritance, events, schemas, configuration, data, packages, and deployment relationships.

Raw relation strings and traversal assumptions do not leak into PR callers. The module earns depth by concentrating behavior currently repeated in affected traversal, graph analysis, policies, and PR logic.

### Impact module

This module accepts a semantic change set and returns local and downstream impacts with bounded witness paths. It composes relation semantics, registered consumer indexes, criticality, and graph confidence.

### Ownership and test-evidence modules

Ownership and test evidence vary independently and therefore have real seams with multiple adapters.

Ownership adapters include graph or catalog ownership, target-branch CODEOWNERS, repository metadata, policy ownership, and historical contribution evidence.

Test adapters include per-test runtime coverage, static graph relationships, build-target relationships, configured test rules, CI check results, and advisory historical evidence.

### Overlap module

This module builds compact pull request footprints, finds candidate interactions through inverted indexes, and performs exact combined-snapshot analysis for selected pairs or merge groups.

### Decision module

This module evaluates CompassQL policies, compatibility classifiers, required-test rules, advisory risk factors, evidence sufficiency, and gate eligibility. It is the only module that assigns risk bands or gate states.

### Report module

This module constructs the canonical typed report, stable fingerprints, human explanations, and delivery projections. Text, JSON, JSONL, SARIF, MCP structured content, and forge checks are generated from the same report.

## Analysis flow

1. Resolve the pull request and full revisions.
2. Capture the immutable evidence manifest.
3. Load or build local and downstream snapshots.
4. Map changed hunks to entities.
5. Compute semantic intent and merge-result deltas.
6. Run impact, ownership, test, overlap, and policy analyses concurrently.
7. Derive advisory risk and deterministic gate states.
8. Persist the canonical report.
9. Publish delivery projections.

No adapter may reopen a graph or fetch mutable pull request state after the evidence manifest is captured.

## Semantic architecture delta

Compass begins with changed hunks instead of changed filenames. Each hunk maps to the smallest enclosing entities in the base and result snapshots.

Entity alignment proceeds conservatively:

1. Exact stable Compass identity.
2. Exact language-native identity such as a fully qualified or SCIP symbol.
3. Container and structural fingerprints for a probable move or rename.
4. Otherwise, separate deletion and addition.

Only the first two classes are exact. Probable matches are labeled and remain advisory.

The semantic delta classifies:

- Entity added, removed, modified, renamed, or moved.
- Relationship added or removed.
- Public signature, visibility, or contract shape changed.
- Dependency direction changed.
- Cross-community or cross-owner dependency introduced.
- Cycle introduced or resolved.
- Critical abstraction gained or lost dependants.
- Contract producer and consumer changed compatibly or incompatibly.

Initial contract classifiers cover:

- Language-level public types, functions, methods, and traits.
- HTTP routes and request or response shapes when present in the graph.
- RPC operations and messages when present.
- Events, topics, and schema-registry subjects.
- Database tables, columns, views, routines, and migrations.
- Configuration keys and environment variables.
- Package coordinates and version constraints.
- Infrastructure resources and externally referenced outputs.

A classifier that cannot prove compatibility returns `possible_break`, never `proven_break`.

## Impact analysis

Traversal follows relation semantics rather than one universal direction:

- Changed functions affect callers.
- Changed interfaces affect implementations and consumers.
- Changed events affect publishers and subscribers.
- Changed tables affect queries, jobs, interfaces, and reports.
- Changed configuration affects readers and deployments.
- Changed packages affect importers and downstream repositories.

Each finding retains the shortest useful witness. The implementation removes duplicate and subsumed paths, but never removes the only witness for a distinct owner, repository, contract, or gate.

Path confidence is the weakest relationship confidence on the retained witness. Any inferred or ambiguous hop makes the path advisory.

Cross-repository traversal uses stable external identities such as package coordinates, route operations, event subjects, database objects, and schema subjects. Display labels are not cross-repository identity.

A deterministic downstream contract-break gate requires:

- An exact changed contract identity.
- A classifier-proven incompatible change.
- A registered consumer using the exact affected element.
- Fresh and complete evidence for that consumer.
- An exact merge-result snapshot.

## Ownership

Compass distinguishes:

- **Direct owners:** own changed files or entities.
- **Contract owners:** own changed public contracts.
- **Affected owners:** own proven downstream consumers.
- **Policy owners:** own triggered architecture rules.
- **Suggested specialists:** have advisory historical familiarity.

Evidence precedence is:

1. Explicit graph or catalog ownership.
2. Target-branch CODEOWNERS.
3. Repository and team metadata.
4. Policy ownership.
5. Historical contribution evidence.

Target-branch CODEOWNERS is authoritative because proposed ownership changes in the pull request must not weaken review routing.

Compass recommends the smallest team-oriented reviewer set that covers material ownership domains. It returns the uncovered domains when no such set exists. Historical activity never overrides declared ownership and never gates merging.

## Test impact

Test evidence tiers are:

1. Per-test runtime coverage bound to a source revision.
2. Static test relationships in the graph.
3. Build-target relationships.
4. Configured test rules.
5. Naming, co-change, or historical failure association.

Results are grouped as:

- **Required:** demanded by a versioned rule.
- **Recommended:** supported by exact coverage, static, or build evidence.
- **Suggested:** supported only by heuristic or historical evidence.
- **Coverage gap:** affected critical behavior has no known covering test.

Aggregate coverage formats cannot prove which individual test covers a line. Compass records aggregate coverage as verification evidence but does not manufacture per-test identity.

For large suites, Compass recommends a minimal set through weighted set coverage over affected entities. Contract and critical-path coverage receive priority. The report states the proportion of exact and inferred affected evidence covered by the recommendation.

A missing-required-tests gate is valid only when:

- A versioned target-branch rule explicitly requires a suite or target.
- Compass can resolve that requirement to a stable CI check or test identity.
- The execution result belongs to the exact merge revision.
- The result is successful.

Pending, skipped, cancelled, stale, or unavailable execution is not passing. The gate remains pending, indeterminate, or error according to the rule's evidence policy.

Initial evidence formats are JUnit execution results, LCOV, Cobertura, JaCoCo, build-system test targets, and forge check results.

## Pull request overlap

Community intersection remains a weak candidate signal, not a merge-safety conclusion.

Compass classifies:

- `text_conflict`
- `entity_collision`
- `contract_consumer_interaction`
- `dependency_intersection`
- `deletion_reference_interaction`
- `emergent_policy_violation`
- `verification_overlap`

Every interaction includes both pull request identities, shared entities or witness paths, directionality, confidence, completeness, and a coordination recommendation.

The overlap implementation:

1. Builds one compact footprint per pull request.
2. Indexes entities, contracts, owners, policies, and critical paths.
3. Selects candidate pairs through index intersection.
4. Runs exact combined-snapshot analysis only for candidate pairs, explicitly requested sets, and merge groups.
5. Caches results by target revision and ordered pull request heads.

Pairwise predictions are advisory. Emergent policy or contract findings become gate-eligible only on the exact merge-group revision.

## Advisory risk

Risk is an explanation derived from independent factors:

- **Contract severity:** internal, compatible public, possible break, or proven break.
- **Propagation:** crossed entity, ownership, repository, and runtime domains.
- **Criticality:** explicit catalog tier, policy tags, sensitivity, and production evidence.
- **Verification gap:** missing or weak coverage of affected behavior.
- **Concurrent exposure:** exact and semantic interactions with other changes.
- **Uncertainty:** ambiguous identities, inferred paths, stale graphs, and extraction gaps.

Propagation is not raw node count. Crossing a repository, ownership, deployment, or data-sensitivity domain is more material than touching many private helpers in one module.

The default risk bands are:

- **Critical:** proven breaking change to a critical contract, or a high-criticality path with no effective verification.
- **High:** material public change, multi-repository propagation, or strong semantic overlap.
- **Moderate:** bounded internal impact with identifiable owners and verification.
- **Low:** isolated, well-verified change with complete evidence.

Organization policy may raise a factor or band but may not relabel inferred evidence as exact. Uncertainty may raise risk or lower confidence; it may not lower risk.

Every high or critical factor has an explanation tree with concrete evidence. LLMs may summarize the typed report but cannot create factors, change severity, or decide gates.

## Deterministic gates

Advisory intelligence and deterministic gates are separate forge checks.

A finding is gate-eligible only when it has:

- A gate-enabled, versioned rule.
- Exact local and merge-result snapshots.
- A stable fingerprint.
- Exact evidence and a retained witness.
- Sufficient freshness and completeness for that rule.
- A reproducible remediation message.

Gate states are:

- `pass`
- `fail`
- `indeterminate`
- `error`
- `pending`, used only while an exact required check is still executing

Missing evidence never becomes `pass`. The default treatment of `indeterminate` is advisory. Regulated installations may configure individual rules to fail closed. That decision is versioned with the policy and appears in the report.

Initial deterministic rule families are:

- New CompassQL architecture-policy violation.
- Proven incompatible contract change with an exact registered consumer.
- Explicit required test or check missing or unsuccessful on the merge revision.

## Reviewer experience

The forge publishes:

- **Compass / PR Intelligence:** the completed advisory report.
- **Compass / Deterministic Gates:** gate state only.

The summary order is:

1. Risk, gate state, and completeness.
2. Architecture changes.
3. Affected local and downstream systems.
4. Recommended owners.
5. Required and recommended tests.
6. Coverage gaps.
7. Overlapping pull requests.
8. Gate findings.

Source annotations are reserved for actionable findings. Full evidence is expandable and includes witness paths, confidence, freshness, and remediation.

Publishing is idempotent. A rerun updates the same check and managed comment. Stable fingerprints divide findings into new, resolved, and unchanged.

The initial CLI extends the existing namespace:

```bash
compass prs 123 --format text
compass prs 123 --format json
compass prs --overlap
compass prs 123 --explain <finding-fingerprint>
```

MCP exposes typed `analyze_pr`, `list_pr_overlaps`, and `explain_pr_finding` operations. Each operation returns structured content derived from the canonical report.

The canonical report is persisted before forge publishing. Failed delivery retries reuse the persisted report and do not rerun analysis.

## Five-minute execution model

The SLA applies to completed analysis, including test selection, not to the runtime of required tests.

The implementation uses:

- Content-addressed graph snapshots.
- Versioned-graph subtree hashes to skip unchanged data.
- Precomputed registered downstream contract-consumer indexes.
- Pre-ingested ownership, coverage, build-target, and test evidence.
- Concurrent analyses over one immutable evidence manifest.
- Candidate filtering before combined pull request analysis.
- Bounded traversal, memory, concurrency, and stage deadlines.

The scale tier records repository count, total graph nodes and relationships, open pull request count, registered downstream count, cache state, and hardware. Compass does not advertise a five-minute SLA without that envelope.

At the deadline, outstanding optional evidence is cancelled. Compass may publish with explicit downstream incompleteness only when local semantic and merge-result analysis is exact. If exact local analysis is unavailable, it publishes an operation error without a risk band.

## Failure model

Failures are classified as:

- **Operation error:** exact local analysis cannot be trusted.
- **Incomplete evidence:** optional downstream, ownership, or test evidence is unavailable.
- **Indeterminate gate:** a rule requires unavailable evidence.
- **Publishing error:** a persisted report could not be delivered.

Snapshot mismatch, graph corruption, extractor mismatch, and mixed revisions are operation errors. A single optional downstream timeout is incomplete evidence. A required downstream graph timeout is also an indeterminate gate for rules that require it.

All errors use stable codes, causal messages, affected evidence sources, and remediation. Execution failures never become empty successful results.

## Security and authorization

Pull request content is untrusted.

- Policies and CODEOWNERS come from the target revision.
- Analyzers never execute repository code.
- Executable plugins or provider configuration introduced by the pull request are ignored.
- Parsers use strict size, recursion, memory, and time limits.
- Forge credentials are unavailable to source analyzers.
- Imported graphs and evidence are schema-validated before composition.
- Caches are tenant-scoped and content-addressed.
- Cross-repository paths, owners, and source locations are filtered through repository authorization.
- A reviewer without downstream access sees a redacted affected-repository fact, not protected source details.

The report records redaction and authorization outcomes as completeness evidence.

## Observability

Operational metadata includes:

- Stage duration and cancellation.
- Cache hit, miss, and eviction.
- Snapshot identities.
- Evidence freshness.
- Extraction coverage and unresolved references.
- Registered downstream success and failure.
- Exact versus inferred finding counts.
- Test-impact coverage.
- Publisher retries.

Operational metadata never changes a risk band invisibly. Any signal used by the decision module appears as report evidence.

## Verification

The report interface is the primary test surface.

### Semantic delta

Fixtures cover signature changes, moves, renames, deletion and addition, relationship changes, public visibility, ambiguous matches, and stable fingerprints across supported language families.

### Revision integrity

Tests force target movement, pull request force-pushes, simultaneous reloads, cache reuse, and synthetic merge failures. No report may contain findings from mixed revisions.

### Impact

Tests cover relation direction, shortest useful witnesses, cycles, bounded traversal, confidence propagation, path subsumption, and stable cross-repository identities.

### Ownership

Tests cover target-branch CODEOWNERS precedence, catalog ownership, contract and downstream owners, authorization redaction, minimal team selection, and uncovered domains.

### Test evidence

Tests distinguish per-test and aggregate coverage, stale evidence, build targets, explicit required suites, pending and skipped checks, and unverified critical paths.

### Overlap

Fixtures cover every interaction class, candidate filtering, exact combined snapshots, merge groups, and policies that fail only after composition.

### Risk and gates

Property and mutation tests enforce:

- Uncertainty never lowers risk.
- Inferred evidence never gates.
- Incomplete evidence never passes a rule that requires it.
- Unrelated low-risk factors cannot hide a critical factor.
- Identical canonical evidence produces identical fingerprints.

### Security and resilience

Fuzzing covers hostile diffs, malformed graphs, oversized coverage, symlinks, invalid Unicode, cyclic evidence, decompression limits, timeouts, and malicious pull request configuration.

### End-to-end

Tests cover GitHub pull requests, forks, force pushes, target updates, check retries, idempotent comments, required checks, and `merge_group` events.

### Performance

Cold and warm benchmarks cover declared small, medium, large, and organization scale tiers. The release gate measures p50 and p95 wall time, peak memory, graph cache reuse, downstream fan-out, open-pull-request overlap count, cancellation latency, and report size.

## Rollout

This document is the umbrella design. It is intentionally not implemented through one monolithic plan. Delivery is decomposed into child design and implementation cycles:

1. **PR Intelligence foundation:** typed report, exact revision capture, evidence manifest, snapshot use, shared operation module, persistence, and typed CLI/MCP adapters.
2. **Semantic delta and local impact:** hunk mapping, identity alignment, relation semantics, contract classification, and witnesses.
3. **Ownership, tests, and decisions:** evidence adapters, reviewer routing, test planning, risk explanations, and deterministic rules.
4. **Downstream federation:** registered consumer indexes, authorization, freshness, completeness, and cross-repository witnesses.
5. **Overlap and merge groups:** footprints, candidate pairing, combined snapshots, and exact merge-queue evaluation.
6. **Historical calibration:** separately labeled advisory outcome evidence.

Each child receives its own approved specification and implementation plan. Child 1 is first because every later child consumes its report, revision, evidence, and operation interfaces. The interfaces are allowed to grow only through versioned additions proven necessary by a later child; child 1 must not prebuild speculative historical or multi-forge implementation.

PR Intelligence remains preview-only until stages 1 through 5 operate together. Preview commands and MCP tools use the versioned typed report from their first release.

Production rollout begins in shadow mode:

1. Publish advisory reports without gates.
2. Compare owners and tests with reviewer choices and runtime evidence.
3. Measure false positives, missed impacts, stale evidence, and latency.
4. Enable deterministic gates one rule at a time after rule-specific validation.
5. Retain a versioned per-rule emergency disable and full audit history.

## Success criteria

- p95 analysis completes within five minutes for the documented scale tier.
- Identical inputs produce byte-equivalent canonical findings.
- No deterministic gate uses inferred or incomplete evidence.
- Every high or critical factor has a human-readable witness.
- Every downstream conclusion reports completeness and freshness.
- Test recommendations report exact and inferred coverage separately.
- Force pushes replace stale findings without comment duplication.
- Reviewers can explain every recommended owner and test.
- Gate false positives are measured per rule before enforcement.
- The analysis result can be reproduced from its persisted evidence manifest.
