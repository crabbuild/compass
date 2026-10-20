# Broad-query source qualification setup

## Current status

The checked-in panels are now exposed development data, not untouched
confirmation sets. All sixteen proposed expansion repositories also appeared
in an earlier development run at the same commits. Historical source-review
records and frozen registrations below remain audit lineage; they do not
establish an independent current comparison or a win over Graphify.

Before a new confirmation run, obtain independent source and exposure-history
review, select genuinely untouched sources, and establish sufficient prospective
power under the registered gates. Follow the
[exposure protocol](BROAD_QUERY_REVIEW.md#source-and-mechanism-exposure-lineage).
No accepted comparative answer/graph judgments or qualifying superiority report
are included here. The setup chronology below records the original proposals.

## Original source-panel setup

This is the source-only panel for Plan 029 phase 5. It contains 160 questions
across eight pinned public libraries: ten broad questions, two exact controls,
two directed controls, two two-hop questions, two ambiguity controls and two
unsupported-subject controls per repository. The selected languages are Python,
Go, Rust and TypeScript, with two repositories each. Java is outside this panel.

`broad_query_corpus.py` contains authored questions, identities, behavioral
statements and call witnesses. `broad_query_sources.json` fixes the eight source
commits. `broad_query_oracle.py` verifies clean pinned checkouts, hashes every
tracked source file in the declared extension scope, checks declarations and
witnesses, then publishes a registration without overwriting an earlier freeze.
It never invokes Compass, Graphify, a model, an embedding service or a provider.
Python declarations also receive AST owner/body checks. The other languages use
bounded declaration selectors; those checks do not prove semantic correctness.

The checked-in registrations are candidate held-out panels, not qualification
results. `broad_query_registration.json` retains the initial freeze, and v2 adds
alternate relevant implementations found during independent source review. The
v3 candidate freezes the connectivity-only answer rubric described below.
`broad_query_registration_reviewed.json` binds those source facts to two actual,
independent AI reviews covering all 160 judgments, with review hashes and revision
lineage. Historical generator/corpus bytes remain archived externally; the
current generator reproduces the v3 source candidate. The original candidates
retain their pending-review metadata. The reviewed envelope still sets
`qualificationEligible` to false because capture, graph, balanced-cluster and
power gates are incomplete.

No output from either tool on these eight repositories was consulted to select
the repositories, write questions or choose facts. The source author had seen
prior Flask/Compass development results; those are disclosed development data.
Before selection, repository URLs and names were searched in existing agent-query
registrations and prior qualification reports; none of the eight was present.
Known Flask/Compass questions and their paraphrases remain development evidence
and are excluded here.

## Source scope and independence

The inventory includes all tracked files with the listed source extensions,
including tests and examples. Both graph-build profiles must include this exact
scope. In particular, the httprouter ambiguity controls explicitly include test
source. A tool's default omission of an oracle source must appear at stage 1 of
the failure ledger; it cannot be hidden by dropping the task or changing its
expected answer after capture.

Broad questions concern distinct behaviors inspected in the implementation;
questions covering different facets of one method select different essential
facts. Related facets and call-chain/exact controls nevertheless share a cluster.
The freezer conservatively merges clusters when tasks share a relevant declaration
identity, retains each authored cluster and records overlap pairs. There are 160
questions, not 160 statistically independent observations. Report
repository and behavior-cluster uncertainty, and retain the all-answerable-task
denominator alongside any graph-conditional diagnostic.

This is a convenience sample of relatively small libraries. It does not establish
quality on large applications, unsupported languages, framework integration,
model-backed comprehension or arbitrary repositories. A source statement or
identity may be incomplete until independently reviewed. Unsupported controls
are bounded capability judgments about these pinned packages; a failed grep is
not ground truth for absence. Search is a way to locate candidate evidence. The
reviewer must inspect implementation scope and distinguish absent capabilities
from differently named or delegated implementations.

## Freeze and verify without tool outputs

Run from the Compass workspace. Keep checkouts beneath the mounted qualification
source root and git captures/generated experiment files beneath an external
artifact root. Substitute those two paths below; do not write into the inputs.

```bash
python3 -m unittest benchmarks.agent_query.broad_query_oracle_tests -v
python3 -m benchmarks.agent_query.broad_query_oracle \
  --source-root "$QUALIFICATION_SOURCE_ROOT" \
  --artifacts "$QUALIFICATION_ARTIFACT_ROOT/source-checks" \
  --frozen-at "$FROZEN_AT" \
  --verify benchmarks/agent_query/broad_query_registration_candidate_v3.json
python3 -m benchmarks.agent_query.broad_query_review \
  --validate-registration benchmarks/agent_query/broad_query_registration_candidate_v3.json
```

Set `FROZEN_AT` to the registration's exact `frozenAt` value for verification. A new freeze uses
`--output` with a new path and an explicit UTC timestamp. The source reader fails
on dirty/revised inputs, escaping paths, over-limit files, missing witnesses or
unbalanced task counts. Byte-identical regeneration proves reproducibility of
these authored judgments; it does not prove that they are correct. Keep the
registration SHA and bounded source-check git captures in the external audit
record. Never replace a frozen oracle in response to a tool failure.

## Independent review before capture

A reviewer independent of the source author must inspect all 160 task judgments
against the pinned source, without seeing either tool's results. A second AI
review is acceptable when honestly identified as AI; it must never be described
as human review. Divide task IDs among reviewers if needed and retain each
reviewer's actual coverage, notes, accepted/rejected facts, proposed additional
relevant identities, uncertainty and source anchors. Reviewing snippets alone or
validating JSON shape is insufficient. Check call receiver/import binding and
source-to-target direction, not just the presence of a callee name.

Keep the original registration immutable. Save review records externally and
publish a separately hashed reviewed revision with explicit linkage if facts
change. A confirmed record requires `reviewerType: ai|human`, a reviewer identity
different from `sourceReview.author`, `sourceAware: true`,
`independentOfAuthor: true`, `toolLabelsBlinded: true` and `status: confirmed`.
Each reviewer records which tasks they reviewed; a partial review does not
confirm the entire panel. Any post-output oracle correction becomes a disclosed
new revision/development diagnostic, never a silent held-out success.

## Freeze the experiment separately before capture

Resolve Graphify's latest official release again when qualification starts. On
2026-10-16 the latest release was v0.9.91, published 2026-10-14, source commit
`10130e79ed9caf8f60d998073e097531eaa1a395`. Its release branch is `v8`; the
repository's divergent main branch must not be substituted. Read-only comparison
of the isolated installed distribution `graphifyy==0.9.91` with that checkout
matched all 95 Python source files. The older user-installed executable is not
the comparator. Full machine-specific module/dependency hashes belong in the
external comparator-identity record, not in product dependencies.

The code-only, credential-free native arm and any optional model-backed arm are
different capabilities. Label them separately, with actual workflow/commands,
model/provider versions and costs when applicable. The present setup has run
neither arm on these repositories. Installing/importing Graphify must never
become a Compass runtime, fallback, native-test or CI requirement.

Before either tool runs, freeze a separate experiment manifest referring to the
reviewed oracle SHA and recording:

- Tool release/source/binary/package/dependency hashes; parser/adapter versions;
  exact argv and nonsecret child environment; all graph-builder settings and
  source inventory/profile digests. Refresh, query logs, credential discovery and
  model/network calls stay disabled for the native offline arm. Isolate child
  HOME, platform configuration/data/cache directories and temporary directories.
- A native-default arm and a common-budget arm. Choose a common tokenizer or a
  named UTF-8 byte proxy, exact budget/overflow treatment and follow-up allowance
  before collection. Native token estimates do not establish equivalent budgets.
  Overflow is an explicit failure/result, not an empty answer or silent clipping.
- All 160 task IDs and fixed question text, all required comparisons, excluded
  development data and failure-ledger definitions. Preserve raw bounded output,
  exit status, timeout, output-limit status and exact ordered candidates/anchors.
- Fixed timeout/output/build limits, randomization seed, interleaving order,
  repetitions, cold/warm definition, and measurements for p50/p95 latency and RSS.
  Timing one product cold and the other warm is not a paired cost comparison.

Build graphs into external artifact directories, verify source cleanliness and
hashes before and after, and never permit assistant-skill writes into source
inputs. The first-page evaluation uses source facts, not either graph as oracle.
Retain failures at all stages: source fact in declared scope; graph fact correct;
candidate admitted; top-five placement; first-page essential facts delivered.
Graph-missing cases stay in end-to-end denominators.

All current directed/path questions ask for identity/connectivity, so their facts
freeze `requireOccurrences: false`. A supported answer must still name the exact
source/target, relationship kind and direction. If it prints source sites, those
sites must be correct. The source `relation.occurrences` remain diagnostic
line-level witnesses and must never be silently copied into a returned answer.
They do not form an exhaustive multiplicity oracle: the occurrence supplement
records two `two_digits` calls on one source line, for seven calls over six
witness lines. Future questions explicitly asking sites/counts must freeze
`requireOccurrences: true` and a suitable occurrence contract before capture.

## Required gates still outside this source panel

The setup contains no product capture, no win claim and no accepted graph
precision panel. The small set of directed witnesses here is not a substitute
for at least 2,000 independently accepted graph judgments. Pre-register separate
source-first stratified graph sampling, include ambiguity/negative cases, inspect
identity/direction/multiplicity/occurrences/provenance, and report recall by
language/relation as well as precision. Required precision is at least .995 with
Wilson 95% lower bound at least .99. Do not count repeated task witnesses as
independent graph judgments.

Plan 029's proposed improvement criterion is at least +10 percentage points in
first-page essential-fact success, with paired 95% lower bound above zero. Source
claim precision and exact controls must meet the -2 percentage point
non-inferiority margin. Compute clustered uncertainty and actual power. Eight
repository clusters and sixteen exact controls may be insufficient; report
inconclusive and expand a newly source-frozen panel rather than narrowing to a
favorable sample or treating missing measurements as zero. Additional graph,
uncertainty and cost inputs must satisfy `broad_query_review.py --check` before
any qualification claim; source-registration validation alone cannot do so.

## Prospective control expansion

`broad_query_power_registration.json` freezes a design proposal before outputs;
it does not pretend that expanded task oracles already exist. The current joint
finite-risk bound needs at least 183 independent all-tie control clusters to
exclude a two-point regression. Equal allocation across eight repositories
rounds this to 184. The weaker one-sided-only floor of 149 is not the implemented
joint-bound criterion. Thirty-two nominal initial controls cannot meet it.

The proposal targets at least 192 new, disjoint exact/directed control clusters
across sixteen untouched source candidates, four repositories per language and
twelve controls per repository. Exact identities are intentional controls;
duplicate bodies, aliases, paraphrases and correlated facets cannot manufacture
independence. Candidate commits were resolved through the official repository
API. Source inventories and questions are authored in source-only batches;
their independent judgments and revisions must all be bound before combining
the registration. No expanded tool outputs may be collected before that freeze.

Unchanged Compass functions establish command conformance only; they do not
establish zero discordance or equality against Graphify. The earlier proposal's
conditional equality premise is withdrawn by the linked pre-output power
revision. With no definite wins and adverse probability .005, 192 independent
controls have about 38.2% probability of passing the finite bound, below both
80% and 90% prospective power targets. Actual labels can share library/retrieval
failure modes and yield fewer independent units. Sample-size assumptions must
come from declared development comparisons and include design effects; the
expanded panel remains exploratory while that gate is unmet. The correction is
frozen in `broad_query_power_registration_v2.json`; the original proposal remains
immutable as lineage. Broad source-claim
precision NI remains a separate comparison and may pass with positive paired
precision lift at smaller N. The 800-unit power row is an illustrative scenario,
not a mandatory sample size or evidence of empirical power. If source capacity or
development estimates require more units, expand with a linked source-only
revision before outputs; do not lower quality gates or stop at favorable results.

## Expanded source freeze

`broad_query_registration_expanded.json` combines the preserved original160,
fourteen additional broad questions and192 new exact/directed controls. Its
`metricCohorts` select94 broad labels in80 declaration-overlap clusters, ten per
original repository, and192 new controls, twelve per additional repository.
All366 tasks remain in the capture matrix; original controls, multihop,
ambiguity and unsupported subjects retain their diagnostic role. Related
declarations and shared repository mechanisms can still correlate; these source
cluster counts are conditional analysis units, not evidence of independent trials.

`broad_query_expansion_review_manifest.json` links the five immutable source
batches and six independent AI review records. The original160 review lineage
remains intact. `broad_query_registration_expanded_review_binding.json` links
the final combined-file rechecks covering all366 source tasks and24 source pins.
`broad_query_expansion_corrections.json` records pre-output
corrections and excluded timestamp drafts. No product output informed those
labels. Independent reviewers know the Compass implementation; their review is
independent of authorship and heldout outputs and is explicitly AI review.

The source combiner verifies candidate/review hashes and explicit accepted task
IDs. It rejects count-only review summaries, uncertain rows, missing coverage,
inconsistent pins and escaped artifact paths. It always retains
`qualificationEligible:false`; combining source labels cannot establish graph
quality, statistical power or measured improvement.

```bash
python3 -m benchmarks.agent_query.broad_query_review \
  --validate-registration benchmarks/agent_query/broad_query_registration_expanded.json
python3 -m unittest benchmarks.agent_query.broad_query_oracle_tests \
  benchmarks.agent_query.broad_query_combine_tests -v
```

To publish a new linked combination from the preserved external batch/review
directory, select a fresh output filename:

```bash
python3 -m benchmarks.agent_query.broad_query_combine \
  --artifacts /path/to/external/source-batches-and-reviews \
  --manifest benchmarks/agent_query/broad_query_expansion_review_manifest.json \
  --output /path/to/new-source-registration.json
```

The original eight broad corpora alone define `graphQualificationRepositories`.
The sixteen added repositories support query-control diagnostics and NI only.
Keep every graph ledger, but do not turn source task witnesses or added control
graphs into graph-superiority evidence. `broad_query_build_spec.json` is now an
explicitly exposed development template. Its Graphify release/commit declaration
matches the executed0.9.92runtime; immutable historical build specifications retain
their original bytes and the old declaration mismatch remains recorded. This
metadata correction did not establish a fresh split. Current capture/review code
now requires frozen exposure lineage for new heldout runs, with independently
reviewed history, explicit task splits/mechanisms and verified historical
artifacts. See the [exposure protocol](BROAD_QUERY_REVIEW.md#source-and-mechanism-exposure-lineage).
The linked proposals have not acquired accepted lineage or become untouched
through that implementation; historical source/task identities remain exposed.
New confirmation sources and independent review remain required. The template
prepares native build commands and isolation requirements; builds/captures wait for
the separate binary, environment, settings, budgets and assessor/normalizer
freeze. Source labels and a passing shape validator do not authorize a
qualification claim.
