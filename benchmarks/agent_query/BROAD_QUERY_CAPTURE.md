# Frozen native query capture

The explicit version2 developer entry point is
`python3 -m benchmarks.agent_query.broad_query_v2`. It connects collection,
normalization, printed-coverage/source scoring, immutable panel assessment,
report publication and recomputation. The older `broad_query_capture`,
`broad_query_review` and `broad_query_panels` commands remain protocol1-only;
neither path converts or silently mixes versions. A new comparison still needs
independent source/oracle acceptance, exposure review, prospective power and
the latest comparator verification before its pre-output freeze. Software
self-tests and source integrity do not supply those judgments.

```bash
python3 -m benchmarks.agent_query.broad_query_v2 --self-test
python3 -m benchmarks.agent_query.broad_query_v2 --capture /external/capture-input.json --output /external/new-capture
python3 -m benchmarks.agent_query.broad_query_v2 --score /external/new-capture/review-input.json --output /external/new-capture/report-reviewed.json
python3 -m benchmarks.agent_query.broad_query_v2 --check /external/new-capture/report-reviewed.json
```

Capture/diagnostic scoring exits0 after coherent publication. Check exits0 only
when every union/original/lookup qualification gate passes,1 for a recomputed
ineligible report and2 for invalid/stale/mixed evidence. Reports must be new
siblings of their input; paths are never overwritten. CLI status includes a
bounded failure preview and total failure count; the complete ledger is in the
report. Native Compass runtime, commands and graph formats are unchanged.

`broad_query_actors_v2.py` freezes all non-test Python sources under
`benchmarks/agent_query` and `benchmarks/performance/compass`, including imported
helpers and graph auditors. Its protocol descriptor binds the complete ordered
inventory and the actual version2 capture-entry, normalizer, assessor, panel and
reporter roles. Added, removed
or changed source files require a new pre-output freeze. Source discovery is
bounded by 16,384 entries, 512 directories and 512 Python files; retained code
is bounded by 64 MiB. Original and retained bytes are rechecked using the shared
bounded artifact reader and existing no-clobber bundle. Runtime callers must use
their own code root, never a root supplied by captured metadata. These integrity
components do not authenticate the execution or an independent source review.

`broad_query_lineage_v2.py` stages finite, immutable review selections. It binds
every original task/tool/arm to exactly one `initialNormalizedRef` and one
`reviewedNormalizedRef`, plus the original run, raw stdout, source registration
and frozen actor digests. The initial reference must equal the original run's
reference, including its path. Initial normalization precedes run publication
and therefore never contains a circular run digest. A reviewed normalization
may correct suggestion IDs, owners and grouping without rewriting that run.
Transitive amendments, missing or reordered rows and mixed versions are rejected.

The protocol2 `judgments` index has exactly `schema`, `runSha256` and
`observations`. Each ordered observation has exactly `task`, `tool`, `arm` and
`judgmentRef`. That reference selects a separate bounded judgments2 document
containing the same task/tool/arm, `runSha256`,
`reviewedNormalizedSha256` and the full reviewed claim/occurrence labels.
The existing 64 MiB JSON limit applies to each document; the shared 2 GiB and
65,536-artifact bundle limits still apply to the entire experiment. Embedding
all row labels in the global index is rejected.

The loader reads raw streams, normalizations and judgments one observation at a
time. Consumers must exhaust its single traversal, discard each large body
after scoring, and call its final `recheck()` before publication. A late row
failure, partial traversal or final integrity failure prevents publication.
Rejected rows retain raw bytes and unknown assertion counts, with no ranked or
coverage prefix. This component checks lineage and partitions; source truth,
native costs, reviewer independence and qualification need the full assessor.
The lineage loader itself supplies no qualification or command dispatch.

`broad_query_execution_v2.py` checks each retained command against the frozen
question, tool, graph, arm, isolated environment and measurement wrapper. It
recomputes byte costs and peak RSS from the original streams. The v2 shared
RSS parser checks decimal length before integer conversion and applies the
positive range up to 10^12 bytes after GNU KiB scaling. Missing, duplicate,
zero, malformed and out-of-range measurements stay unavailable. Its line
whitespace cannot consume another measurement line. Legacy v1 RSS parsing is
unchanged. Native failures, exact declared no-match
outcomes and byte-budget overruns retain their original costs without clipping.
`broad_query_observation_v2.py` connects that check to initial/reviewed coverage
and task scoring using one observation budget and the shared bundle counters.
Row limits and typed assertion collisions keep an explicit failure with unknown
full precision, while source or artifact integrity failures abort assessment.
Initial aggregate occurrence bounds are checked before allocating pending
labels. A nonrejected initial partition is checked even when the reviewed
normalization is rejected. Initial and reviewed failures remain separate:
reviewed material can supply diagnostics, but cannot erase an original parser
or limit rejection or promote that observation to success/formal precision.
Native capture truncation likewise leaves full precision, assertion count and
false-empty status unknown, even if an independent reviewer labels every
retained-prefix assertion. Known prefix judgments stay diagnostic; byte/time
costs and the failed observation remain present.
These layers require the full assessor to supply verified graph/source inputs
and do not authenticate execution, reviewer attestations or qualification.

The internal `broad_query_assess_v2.assess_file_v2` connects the entire matrix to
the existing source/graph/tool and qualification gates. Its own runtime source
root supplies the actor check. It rechecks the actual review-input file and
original/retained inputs before returning. Current Git HEAD and cleanliness
are checked before and after; bounded temporary Git captures are created
alongside the external evidence directory and removed. Git optional locks,
fsmonitor and untracked caching are disabled for these checks. This verifies current
source state and supplied build records, not authentic historical execution.

Capture2 runs must retain `captureInput`, a reference to the original validated
capture-input2 document. Its plan, original registration and source roots must
match the retained run. Each tool additionally retains an ordered `dependencies`
list of `{original, artifact}` records matching every file in its frozen
environment inventory. `original` is an absolute hash-bound input reference;
`artifact` is its contained retained copy. Original executables, launchers,
dependencies, release evidence, graph inputs, build streams and history metadata
are checked before and after assessment. Original inputs share a 65,536-file /
2 GiB bound; dependencies additionally share the existing 512 MiB bound.
Unique input/artifact count excesses reject before any payload read. Reads use
the remaining aggregate byte allowance; an oversized read may inspect one
sentinel byte but never hashes an excess payload. Known files are bounded by
their verified original length. Dependencies are bounded before original reads,
then retained copies are bounded by the verified original length/digest.
Release-reference presence as well as its digest must match the pre-output plan.

The shared bounded subprocess runner treats process-group permission errors as
cleanup failures unless the child has been reaped and a separate group probe
confirms the group is absent. A live or inaccessible group cannot produce a
successful capture through that recovery path.

Protocol2 summaries retain unknown precision at the aggregate, repository and
language levels. Unknown false-empty values are not averaged away. Complete
paired inputs reuse the existing repository/cluster weights, bootstrap and
finite-risk bounds exactly. Any unknown required matrix precision, including
in the default arm, makes formal precision comparison unavailable; no row is
dropped, imputed to zero or replaced with its diagnostic partial fraction.
The existing source review, exposure, task balance, prospective power,
independent graph qualification and subgroup gates remain required. This
assessor is invoked by the explicit version2 entry; the version1 path stays separate.

`broad_query_collect_v2.collect_file_v2` internally connects collection to that
assessor. It preflights the frozen source review, complete actor inventory,
executables/dependencies, graph/build evidence, isolated environment and command
matrix before querying.
Truthfully exposed development calibration retains the existing explicit
exception to author output-blindness only after calibration history, source
definition and independent output-blind reviewer checks. It stays permanently
ineligible for superiority; non-calibration author blindness remains required.
It executes every task/tool/arm once in canonical order,
with no retries or free follow-ups. Raw outputs and costs remain present for
native failures, exact declared no-match outcomes, parser/line/JSON rejections,
missing RSS and byte-budget overflow. Initial normalizations and each separate
judgment document remain explicitly pending independent review. The provisional
report must retain the whole matrix and cannot establish superiority.

The collector retains the original invocation file's exact bytes in
`captureInputInvocation: {original, artifact}`, separately from the canonical
validated `captureInput` snapshot. Both the collector and assessor bind that
file to the validated object and recheck its original path and retained copy.
The internal object-only `collect_v2` helper is diagnostic: missing invocation
file custody is an explicit qualification failure.

Every retained file, including raw streams and provisional reports, counts
toward the shared artifact/byte bounds and is rechecked before completion. A
subprocess reserves capacity for both maximum-sized bounded streams plus one
maximum-sized JSON sidecar before launch (currently 96 MiB); this conservative
reservation may reject a near-capacity run before executing it. Actual retained
bytes, rather than the reservation, are charged to final storage. Source
HEAD/cleanliness and original/retained actors and inputs are checked before
queries, after collection and after provisional assessment. Mandatory integrity,
report or publication failures leave the directory without `COMPLETE.json`.
That marker confirms coherent pending-review capture only. This remains an
integration supplies coherent capture only; helper functions do not authorize
an experiment or establish independent source/answer qualification.

`broad_query_panels_v2` computes original, lookup and union comparisons over
one immutable union capture. The union source registration freezes
`comparisonPanels: {schema: "compass.broad-query-panels-binding/2",
definitionSha256: ...}`. Capture-input2 additionally declares `panels` with a
definition reference and exactly `original`/`lookup` registration references.
Original references are absolute; the run retains contained copies. The
definition has schema `compass.broad-query-panels/2`, `frozenAt`, the complete
`actorSha256`, `registrations` mapping those two roles to SHA256 digests, and
`lookupPairs` with original/lookup task IDs. Digest-only role bindings avoid a
circular union-registration hash. Both panel registrations precede the
definition, which precedes the union source freeze and actual collection.

All original broad tasks remain unchanged. Only named exact controls gain
canonical declaration lookup variants; source identities, clusters and shared
tasks must match exactly. The union contains the complete task union, with
the same sources, capture plan, cost arms, graph scope and policy. Each panel
keeps its own source-frozen cohorts/power and all existing qualification gates.
Combined eligibility requires original, lookup and union to pass. Missing
source-frozen panels makes version2 assessment ineligible. No run, initial
normalization, reviewed selection or raw stream is projected or rewritten.
Observations are scored once; shared query/build costs are charged once in the
union summary. Metadata/graph verification is sequential across panels, with
graph/source indexes released between them.

`broad_query_report_v2` internally publishes and recomputes the version2 report
wrapper. Its exact fields are `schema: "compass.broad-query-review/2"`, absolute
`inputPath`, `inputSha256`, complete `actorSha256`, and `assessment`. The
collector's provisional report uses this wrapper too. New reports are siblings
of their review input in the external artifact root. Encoding is bounded
incrementally, then original/retained source, input and actor guards run before
exclusive publication. Rechecking recomputes the complete matrix and panel
assessments; stored metrics, numeric types, eligibility, actors and original
input/report bytes cannot be substituted. Pending or underpowered reports
remain diagnostic. Public version2 dispatch uses only the explicit entry above.
Historical
protocol1 runs/reports retain their original schemas and arithmetic.

The staged components retain all printed occurrences in globally bounded
batches, exact byte-line coverage and typed source assertions. Initial
normalization always remains pending independent review. Rejection emits zero
normalized records with unknown actual assertion count; it never establishes
empty output. Incomplete semantic coverage or required judgments leaves full
precision unavailable. Diagnostic partial fractions cannot establish paired
precision noninferiority. These rules do not authenticate source truth or
reviewer independence. Historical protocol1 parsing, its 128-claim cap and its
scoring arithmetic remain unchanged; do not convert or regrade historical
captures. Freeze new actors for a new experiment when integration is complete.

`broad_query_capture.py` is developer evaluation tooling. It reuses
`runner.run_bounded`, executes explicit preinstalled tools against explicit
prebuilt graphs, and never builds, installs, downloads, or changes source.
The collection adapter supports Darwin/Linux through the existing POSIX bounded
subprocess backend and rejects other hosts explicitly. Its self-test uses local
fake runners and small public-format graph fixtures:

```bash
python3 -m benchmarks.agent_query.broad_query_capture --self-test
python3 -m benchmarks.agent_query.broad_query_capture \
  --capture /external/capture-input.json --output /external/new-capture-directory
python3 -m benchmarks.agent_query.broad_query_review \
  --check /external/new-capture-directory/report-provisional.json
```

No actual held-out captures belong in this checkout. The last command must
reject a provisional capture: completed collection is distinct from independent
source/output review, sufficient power, graph qualification, and a passing
comparative result. `COMPLETE.json` means the full matrix was captured and
provisional reporting succeeded; it does not mean the quality gates passed.
Failed runs leave an incomplete directory without that marker and preserve the
raw evidence. Output directories and files are never overwritten.

Compact Compass `SEED` lines count as displayed declaration citations only when
they print a source location and sufficient owner identity that resolves uniquely
in that graph. Unlocated seeds and graph-only owners do not become candidates.
The seed's exact/approximate label makes no behavioral claim. A seed followed by
the same `NODE` citation counts once; both printed spans remain in the review
queue. Repeated citations of the identical resolved identity do not make a
relationship endpoint ambiguous; conflicting or unresolved citations still do.
This is display order, not an inferred retrieval rank. A changed normalizer needs
a new frozen experiment or an explicitly separate diagnostic analysis; retained
captures and primary reports are never silently replaced.

## Freeze before execution

The input has schema `compass.broad-query-capture-input/1`, `registration`,
`sourceRoots` (repository ID to absolute source root), and `plan`. External
input references are `{path: absolutePath, sha256: exactByteDigest}`. Every
registered source file, binary, dependency file, and graph is bounded and
verified before execution and rechecked afterward. Sources must have the
registered Git HEAD and a clean status before and after. Artifacts must live
outside both the product checkout and every source checkout.
Heldout registrations additionally require the frozen
[source/mechanism exposure lineage](BROAD_QUERY_REVIEW.md#source-and-mechanism-exposure-lineage)
and an exact `exposureEvidence` digest-to-external-reference mapping. The adapter
rejects missing history review, evidence drift and known confirmation reuse
before invoking any subprocess. Captured review inputs retain the verified
history bytes under contained references; original history inputs are rechecked
before completion. This verifies supplied records, not undisclosed exposure.
Executable symlink invocation paths are preserved, including Python virtual
environment prefixes. Hashing follows their current target, and post-capture
verification reopens the declared path so retargeting cannot evade the check.

`plan` must equal `registration.capturePlan` exactly, and `plan.arms` must equal
`registration.arms`. Independent source review must already be confirmed and
source-first authoring attested, even when the panel is explicitly underpowered
and `qualificationEligible` is false. The explicit version-2
[exposed development calibration profile](BROAD_QUERY_REVIEW.md#exposed-development-calibration)
allows prior author exposure with a verified historical registration/run pair,
an exact selected-question mapping and new independent source review. Its
`calibrationEvidence` has exactly those two hash-bound references; original
paths, retained copies and current actor files are rechecked after provisional
scoring and before completion. Newly captured runs bind the copied scorer through
`run.assessor`; profile reassessment requires that reference and verifies its
source-frozen digest. Both retained code copies receive the final publication
recheck. It remains permanently claim-ineligible.
Source-only pending registrations cannot accidentally launch either tool.

Plan schema `compass.broad-query-capture-plan/1` contains:

- `timeoutSeconds`: integer 1–300 per command.
- `normalizerSha256` and `assessorSha256`: exact hashes of the capture and
  review Python modules, freezing parser and scoring code before collection.
  The checker rejects captured commands/settings/binary or dependency hashes
  that differ from this plan; reports from a changed assessor require a new
  explicitly labeled analysis revision, never silent primary promotion.
- `tools`: exactly `compass` and `graphify`. Each has `version`, full `commit`,
  `executable` reference, explicit `argvPrefix` array, `environment` reference,
  `nativeOffline: true`, and recorded `settings` with `scopePolicy`,
  `inferenceProfile`, `clustering`, and `queryInterface`. The environment is a
  JSON object with `files` containing hash-bound dependency references and
  optional version metadata. No environment variables or secrets are dumped.
  The Graphify record additionally has a `releaseVerification` reference using
  the review scorer's official-release contract. Python launchers may use an
  explicit interpreter executable and module/script arguments; no runtime
  installation or fallback is performed.
- `graphs`: one per repository/tool, each containing `repository`, `tool`,
  `artifact`, `sourceCommit`, `sourceCleanBeforeAndAfter: true`, `sourceInventory`,
  and `buildCapture`. Inventory JSON has `files` equal to the source registration
  inventory. The existing bounded successful build capture is retained; this
  adapter does not rebuild. Before/after source-clean build attestation is
  required, and current source verification cannot retrospectively prove a
  different build's cleanliness. Build JSON contains `argv`, `exit_code`,
  `timed_out`, `output_limited`, `wall_ms`, `stdout_bytes`, `stderr_bytes`,
  and hash-bound absolute `stdout`/`stderr` input references. The adapter copies
  the original JSON and both raw streams, then publishes a rebased build record.
  The assessor checks byte counts, the unchanged original fields/digests and
  the graph/inventory hashes against the frozen inputs. Success flags alone
  cannot stand in for a captured build.
- `arms`: exactly `{id: default}` and `{id: normalized, measure: utf8_bytes,
  budgetBytes: INTEGER, overflow: fail}`. Default invokes native `query QUESTION
  --graph GRAPH` with no renderer override. Normalized adds the native
  `--text-budget` or `--budget` using floor(bytes/4), while actual UTF-8 output
  bytes govern acceptance. Complete oversized output remains captured and the
  row fails; output is never clipped into compliance. Opt-in Compass ranked
  view is allowed only in an explicitly frozen normalized arm with
  `compassView: ranked, configuredComparison: true`; resulting reports identify
  the configured comparison, and cannot support a claim about native defaults.
  Equal native token inputs do not imply identical estimates: Graphify uses
  three Unicode characters per token. The separately frozen measured UTF-8
  ceiling applies to both tools without changing either native estimator.
- `environmentExecutable`: hash-bound system `env` executable. The runner uses
  `env -i` with `childEnvironment.inherit: false`, frozen PATH `/usr/bin:/bin`,
  and explicitly isolated `HOME`, all `XDG_*_HOME` paths, `APPDATA`,
  `LOCALAPPDATA`, and `TMPDIR` contained under the fresh artifact directory.
  `{artifactRoot}` is the sole path substitution. Unknown environment keys are
  rejected. `GRAPHIFY_NO_AUTO_REFRESH=1` and `GRAPHIFY_QUERY_LOG_DISABLE=1`
  explicitly keep query capture from refreshing the graph or writing query logs.
  Optional Python bytecode/user-site/hash-seed and locale/timezone settings are
  allowed only as frozen nonsecret values. No inherited model credentials or
  user configuration are passed to either query process.
- `measurement`: an explicit hash-bound system `time` executable and
  `style: darwin-l|gnu-v` for this host. Peak RSS is bytes (GNU kilobytes are
  multiplied by 1024). Missing or ambiguous RSS remains unavailable and blocks
  cost qualification. The wrapper is in every recorded argument vector.

Collection order is task ID, default then normalized, tool ID. Every execution
is a fresh isolated CLI process with an unflushed OS cache. This is not a cold
cache claim. There are no adaptive retries, diagnostic query subprocesses,
free follow-ups, or task exclusions. Static graph inspection is separately
identified and supplies graph-stage diagnostics only. Wall time includes the
same measurement wrapper for both tools; stdout/stderr and limit/exit status
are retained separately.

## Coherent Compass graph inputs

When a native graph has a sibling `source-query.json`, its graph entry must
freeze `sourceQueryArtifact: {path, sha256}` alongside `artifact`. The input
path must be that exact sibling, and the file must be regular, not a symlink.
The collector rejects an existing but undeclared sibling before executing any
process. Without this artifact, native graph-only queries lack source-body
recall; graph JSON alone does not represent the full current query index.

The collector retains each declared pair at
`graphs/<repository>-compass/{graph.json,source-query.json}`. Separate directories
prevent repositories from sharing the fixed sibling filename. Both query arms
receive that graph path. The sidecar is bounded at the native 64 MiB ceiling;
its schema must be `compass.source-query/1`, its profile one of the seven native
profiles1–7, and `graphDigest` must equal the exact graph-byte SHA-256. This
checks the envelope and binding; native Compass still validates the full
payload. A valid envelope does not establish source or graph quality.

The run retains the reference, and canonical assessment checks its hash against
the frozen plan and its location against the actual graph path. Original and
retained bytes, regular-file status and undeclared sibling absence are rechecked
before completion. An undeclared retained sibling cannot silently affect the
native query. Legacy graph-only captures retain their old layout and meaning;
they do not gain body-evidence coverage retrospectively. Freeze new actor
hashes and collect a new run to correct an omitted artifact. This optional field
applies only to Compass and introduces no dependency on the comparator.

## Native query exit outcomes

A tool may optionally declare this exact `queryExitContract` in its frozen
capture plan. Collection validates it before launching any subprocess, copies
it into `run.tools`, and canonical assessment checks equality with the plan:

```json
{
  "schema": "compass.broad-query-query-exits/1",
  "noMatchExitCode": 4,
  "noMatchStdoutLine": "No matching nodes found."
}
```

The source reviewer must verify the declared CLI outcome against the pinned
tool's implementation. This example describes Graphify 0.9.92's documented
query no-match outcome. The contract requires exactly these fields, a nonzero
exit code in 1–255, and a nonblank, control-free UTF-8 line of at most 4,096
bytes. Recognition requires that exact code and exactly the encoded line plus
one LF byte. No substring, trimming, extra output or CRLF substitution is used.
Unknown versions, extra fields and malformed contracts fail explicitly.

A recognized no-match is an abstention suggestion pending independent review.
It is still an incorrect answer to an answerable question. It cannot produce a
successful answered disposition. Timeouts, capture limits, output-budget
overruns and every other nonzero outcome remain failures in every denominator.
Raw exit status, stdout, stderr and costs are retained. Without this optional
contract, legacy interpretation is unchanged: every nonzero exit fails.

Freeze new actor hashes and a new experiment before using this contract.
Historical captures and their original scoring remain immutable; this is not
permission to regrade an earlier run under changed semantics. Independent
answer/source/graph review and all existing qualification gates still apply.

## Output normalization and review

The normalizer has no registration/oracle input. It reads only the actual
first response and its tool's graph. Actual printed file/start/name anchors
must map to a unique graph declaration; collisions remain unresolved. A graph
neighbor absent from text is never credited. Printed qualified owners are
preserved. Missing receiver owners (for example `.method()`) require source
review rather than guessed ownership. Printed edge arrows are retained;
actual optional ` at=file:Lnn[-Lnn]` citations are separated from target labels
and retained as `printedSite` in the review queue. They contain no invented
source text. Initial connectivity claims keep occurrence arrays empty pending
independent source-aware citation review, even when source/graph metadata has
sites. Malformed citations never yield automatic connectivity claims. Identical
site-free connectivity claims are coalesced for both tools; every printed edge
line, duplicate, citation and exact byte span remains separately queued. Distinct
relations or directions are retained separately. This does not establish
column-level or complete call multiplicity. Display order is not retrieval rank.
Missing rank identities or rank gaps prevent automatic retrieval metrics.
Unexposed admission remains `null`.

Every parsed declaration/relation and every unmatched text line has an exact
UTF-8 byte span/digest in `reviewQueue`. A declaration line asserts identity,
not an automatically inferred behavioral fact. No expected fact ID is assigned.
The initial typed claims are unjudged, and `normalizationComplete: false`
explicitly prevents qualification. An independent reviewer must inspect the
full raw response and pinned source, resolve every material ambiguity, preserve
unsupported claims, and check that no prose/context assertion has disappeared
from the precision denominator. AI review must be identified as AI review.
Tool labels can be blinded for review but distinctive output style can still
reveal the tool; disclose that limitation.

Publish reviewed typed normalizations, a new run sidecar referencing them,
review judgments bound to those exact hashes, and a new review-input manifest
as new files. Preserve the original raw captures, original run and provisional
report. Set `normalizationComplete: true` only after that independent completeness
review. Then use the review scorer's `--score` to produce a new report and
`--check` to recompute eligibility. Programmatic checks verify integrity and
registered arithmetic; they cannot authenticate reviewer attestations or
establish source semantics automatically.
