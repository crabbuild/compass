# Frozen native query capture

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
its schema must be `compass.source-query/1`, its profile one of the five native
profiles 1–5, and `graphDigest` must equal the exact graph-byte SHA-256. This
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
