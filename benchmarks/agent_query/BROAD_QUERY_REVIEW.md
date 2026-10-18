# Source-fact broad-query review

`broad_query_review.py` implements the developer-side scoring and integrity
checks for Plan 029. It does not run either product, install a competitor, infer
ground truth from grep, or qualify a source judgment by matching prose. Use the [frozen capture adapter](BROAD_QUERY_CAPTURE.md), which reuses
`runner.run_bounded`, to collect commands and retain their exact argument vectors,
exit/limit/timing fields and raw stdout/stderr in immutable artifact records.

```bash
python3 -m benchmarks.agent_query.broad_query_review --self-test
python3 -m benchmarks.agent_query.broad_query_review \
  --validate-registration benchmarks/agent_query/broad_query_registration.json
python3 -m benchmarks.agent_query.broad_query_review \
  --score /external/evidence/input.json --output /external/evidence/report.json
python3 -m benchmarks.agent_query.broad_query_review --check /external/evidence/report.json
```

`--self-test` tests synthetic scorer contracts. Registration validation checks
shape, exact identities, source-file pins and internally coherent task oracles;
it accepts explicitly pending independent review. Neither command establishes
real-repository correctness. `--score` writes a new diagnostic report and exits
zero even when qualification is incomplete. `--check` recomputes the entire
assessment, rejects changed/fabricated/stale metrics, and exits zero only when
every registered qualification gate passes. Ineligible reports exit one;
malformed inputs or artifact-integrity failures exit two. Existing reports are
never overwritten.

## Review and trust boundary

### Source and mechanism exposure lineage

A new heldout capture requires `registration.exposureLineage` with schema
`compass.broad-query-exposure-lineage/1`. A legacy registration without it stays
shape-valid and can produce an ineligible diagnostic report. It cannot launch a
new heldout capture or qualify through the current assessor. Explicit legacy
development captures retain their separate independent source-review gate.

The inline lineage is part of the exact hashed source registration:

- `frozenAt` and `historyCompleteThrough` equal the registration freeze;
  `historyComplete` explicitly attests completeness, with an `author` identity.
- `independentReview` has `status: confirmed`, `reviewer`, `reviewerType`
  (`human`/`ai`), `sourceAware: true`, `independentOfAuthor: true`, and UTC
  `reviewedAt` no later than the freeze. The reviewer differs from both the
  history and source authors. This audit is source-aware; it does not require
  hiding historical tool identities.
- `repositoryAliases` contains explicit pairs of HTTPS repository URLs for
  renames/mirrors. `mechanismLinks` contains pairs of stable mechanism IDs for
  shared mechanisms, including mechanisms appearing in different repositories.
  Both links are transitive. IDs use lowercase ASCII letters/digits and
  `._:/-`; renaming an ID does not establish a new mechanism.
- `history` contains up to 8,192 events with unique `id`, UTC `observedAt` no
  later than the independent review, `repositoryUrl`, full `commit`,
  `mechanisms` (up to 32 IDs), `evidenceSha256`, and `kind`: one of
  `source_development`, `graph_build`, `query_capture`, `answer_review`, or
  `implementation_tuning`. Include all known prior exposure, not only exact
  question repeats. Ordinary source-first oracle authoring for the new frozen
  experiment is separate from prior development exposure.

Every registered task declares `split: development|confirmation` and one to 32
stable `mechanisms`. A heldout registration contains only confirmation tasks.
A confirmation task fails if any recorded history touches its repository
(across commits), the identical commit under another URL, or a linked mechanism.
Repository IDs and question wording do
not enter that decision. HTTPS origins normalize default ports, trailing dots,
`.git`/trailing slashes and GitHub host/path case; other renames require explicit
aliases. GitHub URLs must identify a repository root, not a tree/file page.
Escaped/ambiguous URLs fail. Aliased registered repositories or two URLs at the
same commit cannot count as two independent sources. No automatic semantic-equivalence claim is
made for mechanism IDs.

Capture inputs supply `exposureEvidence`, mapping each distinct historical
digest to an absolute `{path, sha256}` reference. Exactly the registered digest
set is required. Before any subprocess, the adapter checks every evidence hash
and the lineage; each input is at most 64 MiB, with a 512 MiB aggregate history
limit. It copies these bytes into the external bundle and rechecks originals
before publishing completion. The review input carries the same mapping with
contained references; the assessor verifies it again. Original, lookup and union
panels retain identical lineage and mapped task split/mechanism assignments.

The checker verifies the records supplied to it. It cannot discover omitted
experiments, authenticate reviewer identities, infer undeclared semantic reuse,
or turn an evidence digest into a true exposure claim. Independent history
completeness/source review is therefore a separate requirement, not a generated
pass label. Obtain independent split/history approval before materializing new
confirmation graphs. The final capture registration binds history through its
own freeze; preserve the earlier source-only review and build chronology
separately. The adapter operates on prebuilt graphs and cannot retrospectively
prove that ordering. A passing exposure check alone proves none of the
query-quality, statistical-power or graph-fidelity gates.

### Required original and lookup panels

`broad_query_panels.py` binds the linked declaration-lookup correction without
discarding its original named-behavior questions. Both canonical review reports
must pass their existing gates. A passing lookup report cannot substitute for a
missing or failing original report. The current source-author proposal contains
352 questions per panel, with 208 shared questions and 496 distinct questions.
Those drafts still require independent source/stratum review and prospective
power planning; these counts establish no quality result or statistical power.

Before collecting, freeze a `compass.broad-query-panels/1` definition with:

- `frozenAt`: a UTC timestamp after both source registrations and before capture;
- `assessorSha256`: `panel` and `canonical` SHA-256 values for
  `broad_query_panels.py` and `broad_query_review.py`;
- `registrations`: `original` and `lookup` contained `{path, sha256}` references
  to the complete, frozen source registrations;
- `lookupPairs`: explicit `{original, lookup}` task-ID pairs for every replacement.

The binder preserves shared questions/facts, source inventories, source clusters,
cost arms, gate policy, graph scope and mapped metric cohorts. Lookup variants
must request their declaration identity. Source or rubric corrections require a
linked pre-output revision; the binder does not approve them automatically.

Capture the union once in one isolated artifact root. Its source registration
must contain the exact union of both panels and the same capture plan, sources
and arms. After output review, project that union review input in place:

```bash
python3 -m benchmarks.agent_query.broad_query_panels \
  --project /external/bundle/review-input.json \
  --definition /external/bundle/panels-definition.json
```

This writes `panels-original-input.json`, `panels-lookup-input.json` and their
run/review documents, then a `panels-projection.json` completion receipt. Existing
files are never overwritten. All projection documents remain in the union root
so its frozen home/cache/temp environment remains valid. Projection adds no
accepted source/output judgments and does not establish qualification.

Score both projected inputs with the canonical `broad_query_review --score`
command. A `compass.broad-query-panel-review-input/1` document then binds a
`definition` reference and `reports.original` / `reports.lookup` references:

```bash
python3 -m benchmarks.agent_query.broad_query_panels \
  --score /external/bundle/panel-input.json \
  --output /external/bundle/panel-report.json
python3 -m benchmarks.agent_query.broad_query_panels \
  --check /external/bundle/panel-report.json
```

Both reports are recomputed through the canonical checker. Tool, graph, arm and
environment captures must agree. Shared answers, normalization judgments and
costs must be identical; different task/tool/arm keys are charged separately even
when their stdout bytes happen to match. Query costs are deduplicated by that
key, and graph-build costs are listed once per repository/tool. The aggregate
report uses `compass.broad-query-panel-review/1`; `--check` exits 0 only when both
panels pass, 1 for an ineligible comparison and 2 for malformed or stale input.
As with the canonical checker, this verifies integrity and arithmetic, not the
authenticity of reviewer attestations or the truth of source judgments.

Source meaning and answer meaning require independent review. A source author
freezes the questions, acceptable declarations, essential facts and expected
ambiguity/no-support outcomes before collection. A second independent reviewer
confirms source judgments and reviews anonymized output. This reviewer may be
human or AI, with its type and source-aware method disclosed. A reviewed typed
normalization maps actual output spans to graph-node identities and source
facts; it must distinguish retrieval rank from display/context order. The
normalizer is an evaluation adapter, not a tool-specific answer generator.

The checker verifies pinned bytes, spans, unique graph identities, explicit
relations and sites, complete paired matrices, scorer arithmetic, costs and
statistical gates. It cannot authenticate a named reviewer, establish that an
attestation is truthful, or infer that a source line proves an arbitrary
semantic claim. A complete set of fabricated reviewer attestations is outside
its verification boundary. Reviewers must never label AI review as human
review. The report states that boundary explicitly.

Development registrations, pending or non-independent reviews, unjudged assertions,
unreviewed candidates, post-output positive oracle additions, missing graph
qualification, and underpowered comparisons remain claim-ineligible. Correcting
an incomplete oracle requires preserving the original result and conducting a
new confirmation; a result cannot silently change its frozen gold labels.

## Source registration contract

### Exposed development calibration

Version-2 registrations may explicitly opt into
`compass.broad-query-development-calibration/1` to collect a new diagnostic run
after the source author has seen earlier outputs. This is permanently
superiority-ineligible. It does not turn an exposed question into confirmation,
regrade an old report, approve new source labels, or establish statistical power.
Unprofiled development and heldout captures retain the output-blind authoring
requirement.

The `developmentCalibration` envelope has exactly these fields:

```json
{
  "schema": "compass.broad-query-development-calibration/1",
  "purpose": "paired_development_calibration",
  "priorOutputExposure": true,
  "priorRegistrationSha256": "<64 lowercase hexadecimal characters>",
  "priorRunSha256": "<different 64-character digest>",
  "selection": [{"task": "current-id", "priorTask": "historical-id"}],
  "selectionRationale": "Predeclared corpus, denominator and excluded-task scope."
}
```

Require `designation: development`, `qualificationEligible: false`, a named
`sourceReview.author`, and `sourceReview.blindToOutputs: false`. Unknown profile
versions/fields, version-1 opt-ins and conflicting eligibility flags fail.
Selection covers every current task exactly once, with no historical task
reused. It need not include every historical task. Disclose the selection
denominator and exclusions; selection is prospective only for the new run,
and is neither newly blinded nor necessarily representative.

Capture input and review manifest contain `calibrationEvidence`, mapping exactly
the two digests to hash-bound artifact references (absolute inputs, contained
bundle references). The historical run must use the supported capture schema
and bind that exact original registration digest. Its recorded chronology must
satisfy `prior.frozenAt < priorRun.startedAt <= priorRun.completedAt < frozenAt`.
Every selected historical question has exactly one observation for each of the
two tools and two arms. Current/historical question, kind and expected disposition
must match exactly, as must repository URL, commit and complete pinned file
inventory. IDs can change. New oracle annotations may differ only after a new
independent source review; those annotations never replace old outcomes.

The existing independent source-review requirements still apply. Additionally,
the reviewer attests `priorOutputsObserved: false` for substantive prior answers,
normalizations and per-task scores in the selected corpus. Metadata-only joins
are not answer observation. Freeze the review's UTC `reviewedAt <= frozenAt`
and `sourceDefinitionSha256`: SHA-256 of UTF-8
`json.dumps({"repositories": repositories, "tasks": tasks}, sort_keys=True,
separators=(",", ":"), allow_nan=False)` using Python's default ASCII escaping.
This binds exact prospective facts/groups, rather than transferring approval
from a historical oracle. Disclose reviewer familiarity and method. Hashes do
not authenticate independence or truthful blinding; labels removed after the
reviewer knew an output's identity do not establish tool-label blindness.
New answers still require independent source-aware, anonymized output review.

The adapter verifies and copies the two metadata documents before any process,
rechecks original paths and retained copies after provisional scoring, and
publishes no `COMPLETE` receipt on drift. Each document is bounded to 64 MiB,
128 MiB in aggregate. It does not recursively follow historical graph, answer,
normalization or report references, or accept calibration chains as the parent.
Keep the original report bundles and outcomes unchanged at their recorded
locations; cite them separately as historical provenance. This metadata pair
does not establish complete exposure history or independently authenticated
chronology. Do not give reviewers embedded answer payloads under a metadata-only
label. The assessor independently verifies the pair and selection, retains
diagnostic failures/metrics, and always rejects a superiority claim.
New runs also bind a contained `assessor` reference to the frozen scoring-code
digest. This reference is required for this profile. Canonical reassessment
verifies its bytes and frozen identity; legacy runs without the profile retain
their original no-reference behavior. Publication rechecks both retained capture
and assessor code copies, as well as their original paths, after provisional
scoring and immediately before completion.

Original/lookup panels preserve equal profile metadata excluding `selection`.
Shared tasks have identical mappings; every panel's mapping must be the exact
union mapping restricted to that panel's task IDs, independent of list order.
The two evidence digests and checked bytes remain bound through projection and
recomputation. New lookup wording is unsupported by this narrow profile unless
that exact question already exists in the cited historical registration.

Source witness text is one bounded UTF-8 line and may retain indentation tabs.
Newlines, carriage returns, NUL and escape controls remain invalid. Witness text
is preserved in the registration; other labels and identities still reject tabs.
Pinned file digests and source-line checks continue to bind the witness to its
source. Accepting indentation does not establish source meaning or independent
review.

Schema: `compass.broad-query-registration/1`. The repository/source-preparation
workflow may add descriptive metadata. The core fields are:

```json
{
  "schema": "compass.broad-query-registration/1",
  "designation": "heldout",
  "frozenAt": "2026-10-16T00:00:00Z",
  "qualificationEligible": false,
  "sourceReview": {"author": "source author", "blindToOutputs": true},
  "independentReview": {
    "status": "pending", "reviewer": null, "toolLabelsBlinded": true
  },
  "repositories": [{
    "id": "sample", "language": "python", "url": "https://example.invalid/sample",
    "commit": "FULL_GIT_OBJECT_ID",
    "files": [{"file": "src/store.py", "sha256": "HEX_SHA256"}]
  }],
  "tasks": [{
    "id": "save-task", "repository": "sample", "cluster": "save-mechanism",
    "kind": "broad", "question": "where is persistent state saved",
    "expected": "answer",
    "relevant": [{"identity": {
      "file": "src/store.py", "startLine": 10, "symbol": "save", "owner": "Store"
    }, "grade": 3}],
    "facts": [{
      "id": "save-fact", "essential": true,
      "identity": {"file": "src/store.py", "startLine": 10, "symbol": "save", "owner": "Store"},
      "statement": "Save delegates persistence to the writer.",
      "source": {"file": "src/store.py", "line": 12, "text": "return self.writer.flush()"}
    }]
  }]
}
```

The example uses placeholders and is not a valid qualification registration.
An identity has exactly `file`, `startLine`, `symbol`, and `owner`; the owner is
the reviewed lexical owner, or the empty string for a module-level declaration.
Paths are canonical repository-relative POSIX strings. Grades are 0–3;
grades 2–3 are relevant. Declaration file/start/name and owner must all agree
for answer scoring. Graph mapping uses the existing symmetric exact
file/start/terminal-name policy and requires a unique result. Source review
establishes lexical ownership; the normalizer cannot choose the graph node
whose name merely fits an expected answer.

Task kinds are `broad`, `exact`, `directed`, `multihop`, `ambiguous`, and
`unsupported`. Expected outcomes are `answer`, `ambiguous`, and `abstain`.
Answerable tasks need positive identities and at least one essential fact (or a
version-2 essential group, below).
Ambiguous tasks need at least two source-reviewed positive identities.
Unsupported tasks have empty `relevant`/`facts` arrays and a `rationale` scoped
to the registered source inventory. They mean “no supporting indexed evidence
in this scope”, not proof that a behavior cannot exist anywhere.

### Equivalent implementation witnesses

New prospective registrations may opt into
`compass.broad-query-registration/2`. Version 1 retains its original conjunctive
fact requirements and rejects `essentialFactGroups`; older assessors reject
version 2. Unknown registration and group versions fail explicitly. Existing
source registrations, captures and reports are not migrated or rescored in place.

A broad answerable task may add this versioned requirement:

```json
{
  "essentialFactGroups": {
    "schema": "compass.broad-query-essential-fact-groups/1",
    "groups": [{"id": "expired-lookup", "anyOf": [["get-expiry"], ["peek-expiry"]]}]
  }
}
```

Each alternative in `anyOf` is a complete witness set of fact IDs. For example,
`[["read", "validate"], ["alternate-read", "alternate-validate"]]` requires
both facts of one complete path; mixing `read` with `alternate-validate` fails.
Shared-prefix sets such as `[["read", "validate"], ["read", "reject"]]`
still require `read` and a complete alternative. Each referenced fact remains an
entry in `facts`, with its own exact
declaration identity, source witness and optional relationship/occurrences. Its
identity must be a frozen positive candidate. Group members have `essential:
false`; an individually essential fact cannot also be an alternative. A group
contains at least two alternatives with distinct sets of full declaration
identities (file, start line, symbol and lexical owner). Two methods in the same
class can qualify. Different complete paths through identical declaration sets
cannot be expressed by this contract; do not silently discard valid alternatives
to fit it. Group IDs are unique and distinct from fact IDs; members are known
and unique within each witness set. Facts may be shared between alternatives of
one group, but not between groups. Duplicate or subsumed witness sets fail.
The existing 64-fact bound applies; groups, alternatives and facts per set each
have a 64-entry bound, with at most 256 total fact references across alternatives
per task. The checker evaluates these sets directly without expanding combinations.
Empty groups/sets, extra group fields and unknown schemas fail. Exact, directed,
multihop, ambiguous and unsupported tasks cannot use groups.

Success still requires every individually essential fact. It also requires
all supported, recovered facts of at least one alternative in **every** group. Claim matching never
substitutes a group member's owner for another fact's owner; relationship
direction and occurrence requirements remain exact. Unjudged/unsupported claims
and execution limits retain their original failure behavior. No fact can satisfy
two groups, and alternatives add no independent tasks or source clusters.

The source author and independent reviewer must approve complete equivalence for
the question's requirement and quantifier, including branch qualifications and relevant alternative
implementations. The checker validates declared structure and evidence; it does
not infer behavioral equivalence from similar text or names. Group membership is
part of the hashed source task and registration, frozen before outputs. Any
oracle correction needs a linked prospective source review and new capture-plan/
assessor hashes. Adding a group after seeing answers cannot rehabilitate a
held-out result. Original required panels/outcomes remain retained.

The original/lookup panel binder preserves both registration versions and group
payloads, including through union projection. Scored rows with groups expose
`essentialFactGroupSchema`, `essentialFactGroups` (`id`, recovered member IDs,
zero-based `satisfiedAlternativeIndexes` in frozen input order and `satisfied`),
and `essentialFactGroupRecall`. All completed alternatives are reported. The per-witness source/graph ledger and
`sourceFactRecall` remain separate: recovering one of two equivalent witnesses
can satisfy a question with witness recall 0.5. Group recovery is not graph
precision, source-witness recall, independence or a qualification certificate.
Rows without groups retain their existing shape and scoring.

A fact optionally includes a typed relationship:

```json
{
  "relation": {
    "source": {"file": "src/store.py", "startLine": 10, "symbol": "save", "owner": "Store"},
    "target": {"file": "src/writer.py", "startLine": 30, "symbol": "flush", "owner": "Writer"},
    "kind": "calls", "direction": "source_to_target",
    "occurrences": [{"file": "src/store.py", "line": 12, "text": "return self.writer.flush()"}]
  }
}
```

Both endpoint identities, relation kind, direction and the complete registered
occurrence set must agree. Repeating one occurrence cannot recover a different
missing one. Multihop tasks register each essential directed hop as a separate
fact; endpoints plus a list of relation names do not prove an intermediate
route. Native undirected artifacts explicitly report an unavailable direction
contract rather than an invented reverse-call error.

## Evidence bundle contract

All artifact references are `{ "path": "relative/path", "sha256": "64 lower-case hex digits" }`.
They resolve under the input manifest's directory; symlink escapes, `..`,
absolute artifact paths, digest drift, duplicate JSON keys and excessive
input sizes fail closed. Local `sourceRoots` are explicit external checkout
paths and are not product configuration. Keep raw captures and source content
outside the source repository under the source owner's access policy.

Input schema `compass.broad-query-review-input/1` contains:

```json
{
  "schema": "compass.broad-query-review-input/1",
  "registration": {"path": "registration.json", "sha256": "HEX_SHA256"},
  "run": {"path": "run.json", "sha256": "HEX_SHA256"},
  "reviews": {"path": "reviews.json", "sha256": "HEX_SHA256"},
  "sourceRoots": {"sample": "/external/source/sample"},
  "graphQualification": [{
    "repository": "sample",
    "manifest": {"path": "graph-audit-manifest.json", "sha256": "HEX_SHA256"}
  }]
}
```

`graphQualification` supplies existing `compass.quality-audit/2` qualification
manifests. The checker calls the existing `run_audit` on the exact captured
Compass graph and source, preserving its 2,000-judgment, .995 precision, .99
Wilson lower-bound and diversity/capability/source-coverage gates. It does not
trust an `eligible: true` summary from another file. Symmetric source-based
graph comparisons remain independent of this Compass production gate; neither
graph counts nor reproducing all competitor hypotheses replaces those audits.
For a multi-corpus production audit, supply one entry with `manifest`, relative
`graphRoot` and explicit external `sourceRoot` instead of `repository`. Its
corpus names, source roots, commits and graph hashes must match the captured
panel exactly. This preserves the existing aggregate 2,000-total and
400-per-corpus floors rather than requiring 2,000 judgments separately for
every repository.

The run schema `compass.broad-query-capture/1` contains:

- `registrationSha256` and UTC `startedAt`, after the source registration freeze.
- Exactly two `tools`, IDs `compass` and `graphify`, each with `version`,
  `commit`, `nativeOffline: true`, recorded `settings`, and artifact references
  `executable`, `environment`, and `normalizer`. Environment manifests must
  retain the installed distribution/dependency/package hashes; a Python
  launcher hash alone does not identify an installed competitor.
  Settings explicitly name `scopePolicy`, `inferenceProfile`, `clustering`, and
  `queryInterface`; missing settings prevent qualification. Record actual
  differences rather than inventing equivalent internal feature flags.
- Graphify also has `releaseVerification`, referring to a record with
  `version`, `commit`, UTC `checkedAt`, and
  `source: "https://github.com/Graphify-Labs/graphify/releases"`. Pin the latest
  release before collection, not divergent default-branch HEAD.
- One `graphs` entry per repository/tool: `repository`, `tool`, graph
  `artifact`, `sourceCommit`, `sourceCleanBeforeAndAfter: true`, and
  `sourceInventory`/`buildCapture` references. Both tools must receive the same
  inventory hash for a repository. Retain parser omissions separately.
  The successful bounded build record must explicitly have `exit_code: 0`,
  `timed_out: false`, and `output_limited: false`.
- Exactly two `arms`, frozen identically in the registration:
  `{"id":"default"}` and
  `{"id":"normalized","measure":"utf8_bytes","budgetBytes":8000,"overflow":"fail"}`.
  The byte budget is configurable before collection; it is a named proxy,
  not a tokenizer. Normalized first-page scoring permits one capture and no
  follow-ups. Overflow stays in the denominator as a failed execution; the
  checker retains the complete raw response rather than clipping it.
- One `observations` entry for every task/tool/arm, containing `task`, `tool`,
  `arm`, `normalized` reference, `captures` references (1–4), `condition`,
  and `peakRssBytes`. Conditions are `fresh_cli`, `warm_cli`, `warm_mcp`,
  `cold_cli`, or `unrecorded`. Missing measurements remain explicit and prevent
  complete qualification. Interleave/randomize timing collection and retain
  its order/setup metadata; this checker does not establish a latency win.

Each capture is the existing bounded runner's field record: `argv`,
`exit_code`, `timed_out`, `output_limited`, `wall_ms`, `stdout_bytes`,
`stderr_bytes`, plus `stdout`/`stderr` artifact references instead of inline
text. Follow-up bytes and latency are charged in full. Nonzero exits, limits,
timeouts and over-budget normalized responses fail task success while remaining
in all cost and task denominators.
Broad and unsupported tasks must pass the exact frozen question as one argument
in each measured command. A tool-specific oracle-informed question rewrite is
rejected. Exact/directed controls may use the closest documented typed operation
with their preregistered source subjects; do not count known-ID preparation as
successful natural-language retrieval.

## Typed response and judgments

Each normalized response has schema `compass.broad-query-normalized/1`, matching
`task`, `tool`, and `arm`, plus:

- `disposition`: `answered`, `ambiguous`, `abstain`, `error`, or `limit`.
- `rankingKind`: `retrieval`, `display`, or `unavailable`. A neighborhood's
  arbitrary display order is not a ranked retrieval result. Retrieval metrics
  are unavailable for display/context lists.
- `ranked`: candidate records `{identity,nodeId,firstPage,span?}`. A first-page
  candidate must carry a raw first-capture `span`; it must uniquely resolve to
  the recorded graph node at its exact declaration anchor. Ranks beyond the
  first page may be retained from the documented typed ranked result/trace.
- `admission`: an instrumented array of admitted graph node IDs, or `null` if
  the tool does not expose that stage. Missing instrumentation is unavailable,
  never invented as a false admission failure. A candidate with an observed
  rank must also be in an instrumented admitted set when one is supplied.
- `claims`: first-page fact assertions `{id,identity,relation?,span}`. Each span
  is `{startByte,endByte,sha256}` over exact first-capture stdout bytes, with
  `endByte` exclusive. Spans establish where the reviewed assertion appeared;
  they do not establish its meaning. An independent reviewer must also confirm
  complete normalization so omitted incorrect assertions cannot improve precision.

Judgments schema `compass.broad-query-judgments/1` has `runSha256` and
`observations`. Each row repeats `task`, `tool`, and `arm`, binds
`normalizedSha256`, and has an `independentReview` object. Confirmed independent
source-aware review uses `status: "confirmed"`, `reviewerType: "human"` or
`"ai"`, a nonempty `reviewer`, `independentOfAuthor: true`, `sourceAware: true`,
and `toolLabelsBlinded: true`. The source author and independent source reviewer
must be distinct. Pending review is allowed as a diagnostic input and cannot
pass qualification. Reports disclose AI review and never call it human review.

`claims` reviews contain `id`, `verdict: supported|unsupported|unjudged`,
`factId` (a frozen fact ID or null), and `reason`. A supported frozen fact
requires exact identity, relationship kind and direction agreement; a wrong owner or reversed
edge is unsupported even if the review labels it supported. A supported extra
assertion with null `factId` additionally needs a pinned `source` witness and
independent semantic review. Extra source files must be pinned before scoring.
Unknown fact IDs and duplicate semantic claims fail validation.

`candidates` reviews contain `identity` and `grade` for displayed candidates
outside the frozen positive/negative identities. Unknown returned candidates
remain unjudged until reviewed. Newly discovered positive identities are
reported as post-output oracle corrections and prevent a held-out claim.
Changing a previously frozen grade is rejected.

## Metrics and qualification

An answerable task succeeds only when its first page identifies a relevant
declaration, answers every essential source fact and predeclared version-2 group,
and contains no unsupported
or unjudged assertion/candidate. An ambiguity-only list cannot pass an
answerable task. A valid ambiguity answer explicitly retains at least two
source-reviewed possibilities. An unsupported task succeeds through explicit
abstention with no competing asserted candidates/facts.

The report retains the source → graph → admission → rank → first-page ledger
for every registered fact. Graph absence and missing instrumentation remain
different outcomes. End-to-end success includes every answerable task;
graph-present conditional statistics cannot hide extraction misses.

Success@5 and MRR@10 use frozen grade ≥2 relevant identities and documented
retrieval ranks. nDCG@10 uses gains `2^grade - 1` and `log2(rank + 1)` discounts.
Source-fact recall is recovered frozen facts divided by all frozen facts.
Claim precision uses supported/(supported+unsupported) reviewed claims;
unjudged assertions are reported separately and prevent qualification. Its
paired noninferiority score assigns an empty answer zero per-task precision
(`precisionWithEmptyZero`) to prevent improving precision by abstaining on
hard tasks. This score is distinct from pooled claim precision and is labeled
accordingly. False-empty rate is explicit abstention or no candidates/claims
on answerable tasks. All summaries retain repository, language and kind strata.

Only the normalized arm determines the registered query claim. The scorer uses
a deterministic hierarchical paired bootstrap: average paraphrases within
their source-task cluster, resample repositories, then resample source-task
clusters within each selected repository. Repositories have equal weight.
Seed and 1,000–10,000 resamples are frozen in `gatePolicy`. The result reports
the paired estimate, percentile 95% interval, repository/cluster counts, and
method. The reported interval is widened by a conservative exact finite-task
paired-risk bound: lower confidence for definite task-cluster wins minus upper
confidence for any task-cluster loss, with Bonferroni-adjusted Clopper–Pearson
bounds. Paraphrases remain one cluster; partially positive clusters receive no
definite-win credit. This bound assumes independently sampled source tasks
conditional on the fixed repository panel and requires equal independent
cluster counts per repository. It does not establish population repository
inference. In particular, 32 all-tie controls retain a nonzero upper bound on
unseen losses and cannot establish .02 noninferiority. A bootstrap's zero
empirical variance is never treated as certainty. Purposive corpus selection
still limits generalization even if the interval passes.

The proposed gates cannot be weakened through the manifest:

- At least eight repositories, four languages with at least two repositories
  each. A single panel follows 10 broad / 4 exact-or-directed / 2 multihop /
  2 ambiguous / 2 unsupported per repository. A heterogeneous source-first
  expansion may instead freeze `metricCohorts` and `samplingDesign`, retaining
  every original task and using balanced independent cluster counts and language
  allocation within each primary metric cohort.
- At least 80 independent broad source-task clusters; paraphrases do not add
  independent sample size.
- Broad first-page success improves by at least .10 observed, with paired
  95% lower bound above zero. This does not assert that the true gain is ≥.10.
- Exact/directed task success and answerable per-task claim-precision score
  have paired lower bounds strictly greater than −.02. A repository-level
  answerable-success regression worse than .10 requires independent investigation.
- Independent source-aware source/normalization review, complete provenance/judgments,
  common frozen cost arms, and existing independent graph qualification.
- A frozen `powerAnalysis`, including `targetPower` ≥.8,
  `expectedBroadDiscordance`, `expectedControlDiscordance`, `designEffect` ≥1,
  and `developmentEvidence` artifact reference. The approximate paired-binary
  planning floor uses `(z_.975 + z_power)^2 * (discordance - effect^2) /
  effect^2 * designEffect` for broad superiority and
  `(z_.975 + z_power)^2 * controlDiscordance / margin^2 * designEffect` for
  control noninferiority. These are prospective planning checks, not post-hoc
  “achieved power”. Independently review the assumptions and retained development
  evidence; the checker cannot establish their representativeness.

The initial 160-task registration is not a promise that these gates can pass.
In particular, a .02 noninferiority margin can require far more independent
controls. Plan any balanced expansion before looking at confirmation outputs,
retain failures, and report an inconclusive result when uncertainty or power
is inadequate. Do not reclassify development reruns as a fresh holdout.

## Occurrences, prospective cohorts and power

Relationship source witnesses always retain the frozen occurrences. The
answer rubric is separately predeclared with `fact.requireOccurrences`: true
requires exact printed line witnesses; false permits omitted sites for a
connectivity-only question while still requiring exact endpoints/kind/direction.
Any actually asserted site must be source-supported. No absent printed site is
backfilled from source or graph. The initial panel uses false only for questions
that ask identity/connectivity/path rather than sites/counts, accepted before
outputs by the source author and independent reviewers. File+line witnesses do
not establish column-distinct call multiplicity. Native graph qualification and
its occurrence/provenance gates remain separate.

`metricCohorts` has exactly `broadSuccess`, `exactDirected`, and
`claimPrecisionEmptyZero`, each with `taskIds`, `rationale`,
`minimumRepositories` (at least eight), and `minimumIndependentClusters`.
Broad success and precision use the same frozen broad tasks; control success
uses exact/directed tasks. Every captured task remains in diagnostics, even
when a new source-only control cohort is the prospective NI population.
`samplingDesign` explains selection, source dependence and original-task
retention. Paraphrases, shared implementations and repeated endpoints do not
create independent clusters.

The normal-planning floor above is diagnostic and cannot by itself establish
power of the actual acceptance rules. The scorer additionally computes an exact
finite-control pass probability from independent-cluster N and the predeclared
adverse probability, crediting no definite wins. At N=200, adverse probability
.005 gives only about .367 probability of passing that finite NI bound. An
assumed equality alternative of zero requires hash-bound pre-output evidence;
zero losses observed afterward cannot supply the assumption.

`powerAnalysis.acceptanceRuleSimulation` must reference a frozen
`compass.broad-query-power-simulation/1` artifact with `frozenAt`, an independently
confirmed `independentReview`, and `metrics` for all three comparison names.
Each metric has `assumptions`, `method`, hash-bound simulation `code` and
`results`, `independentClusters`, and `powerLowerBound`. The planned lower bound
must reach registered target power and the actual panel must reach planned
clusters. Simulations must model the actual hierarchical and finite-risk gates,
including per-task precision's nonbinary distribution. The checker verifies
these records and reviewer attestations; it does not authenticate reviewers or
assert that their prospective model represents a population. Missing planning
records keep the result ineligible rather than converting observed significance
into post-hoc power.

Graph-quality claim scope may be frozen in
`graphQualificationRepositories` (at least eight registered repositories,
including every broad comparison repository). Added strict-query control
repositories are explicitly query-only and receive no graph-quality superiority
claim. Reports enumerate both groups and audited graph hashes. Without an
explicit scope, every registered repository requires graph qualification.
The native audit retains its existing 2,000-total and 400-per-corpus floors;
eight audited broad corpora therefore need at least 3,200 accepted judgments.
