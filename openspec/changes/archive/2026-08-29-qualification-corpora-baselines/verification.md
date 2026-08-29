# Verification: qualification corpora, baselines, and budgets

## Result

PASS for C-013's deterministic corpus, bounded harness, current-engine baseline,
and precommitted budget-ratification contract. This evidence does not approve a
SurrealDB license profile or assert that a future engine satisfies any budget.

## Requirement evidence

- The complete research budget table is ratified unchanged. The source phase
  plan and exact research section are pinned by SHA-256, and C-012 measurements
  are explicitly excluded from the decision basis.
- `qualification-medium` has exactly 100,000 nodes and 250,000 edges;
  `qualification-large` has exactly 1,000,000 nodes and 2,500,000 edges.
  Full logical iteration reproduces the pinned node and edge digests for both.
- Graph materialization atomically publishes one streamed strict `compass.graph/1`
  document with stable
  identities, directed calls, an explicit parallel edge, exact evidence, and
  deterministic order. Its synthetic file content digest is profile-specific;
  the stable file ID intentionally continues to represent the common source path.
  Oversized or invalid profiles fail closed.
- The semantic descriptor pins the existing code-graph qualification sources and
  requires identity, direction, multiplicity, confidence, evidence, ambiguity,
  negative cases, limits, ordering, and pagination. Its four count claims are
  recomputed from the pinned semantic source in the normal test suite.
- The raw denominator scans graph arrays incrementally, has explicit byte/node/
  edge/depth/result/time limits, preserves direction and parallel multiplicity,
  checks deadlines during full node scans, bounds the task document to 1 MiB and
  100 tasks, validates declared limits against effective CLI values, emits an
  explicit status for every operation, validates task shape, and reports limit
  or input failures with exit status 2 rather than an empty result.
- Exactly 30 unique tasks are balanced six each across search, callers, callees,
  impact, and path. All pass the bounded raw oracle against the exact medium graph;
  the timing-free result set is retained and continuously checked against every
  expected status, node, edge-count, and path-endpoint assertion.
- The shipped 14-case skill trigger corpus is pinned by SHA-256 and partitioned
  exactly into two umbrella invocations and twelve focused boundary prompts, two
  for each focused skill, with zero tolerated regression or ambiguity.
- One documented current-engine runner emits binary and runtime-pinned gzip size,
  cold start, five samples per query workload, nearest-rank p95, peak RSS, graph
  and binary hashes, host/tool versions, computed source-state identity, and its
  directly measured raw traversal denominator using repository-relative paths.
- Before measuring, the runner verifies the supplied graph's exact bytes, file
  SHA-256, node/edge counts, and complete logical digests against generator
  metadata and the pinned profile catalog. It explicitly requires
  `qualification-medium` and fails closed on any mismatch or other profile.
- Every Git, binary, Rust, and Cargo metadata subprocess plus source-patch capture
  and every measured workload use the existing measured-process helper with
  explicit time and stdout/stderr limits.
- Every discarded query warmup also uses the same bounded measured-process helper,
  and search pins its supported `--max-candidates 256` bound explicitly.
- Source identity covers the tracked binary diff plus all 638 existing tracked
  or non-ignored untracked files across root manifests/toolchain, every workspace
  crate, and both vendored path dependencies. The 612-file tracked and 26-file
  untracked subsets have separate counts, byte totals, and digests; ignored
  ambient files are excluded under explicit file/byte ceilings.
- The path workload's selected endpoints have an observed exact three-hop
  shortest path. Because the legacy command exposes no depth flag, its fixed
  100,000-node/250,000-edge graph and 120-second process timeout are recorded as
  independent bounds rather than inventing unsupported CLI arguments.
- `manifest-v1.json` is a closed checksum inventory of every retained benchmark
  source, corpus, evidence document, and regression below its directory.

## Verification performed

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s benchmarks/qualification/tests -v`: 30 passed, including full medium/large digest regeneration, graph/profile mismatch rejection, output-cap rejection, malformed retained-evidence rejection, mandatory/strict expected-result contracts, unknown-field and empty-suite rejection, declared-limit and task-input bounds, explicit empty status, strict graph identities, duplicate-edge rejection, atomic JSON output, and interrupted graph-publication recovery.
- Full metadata-only generation for both profiles: exact pinned node/edge counts
  and SHA-256 digests matched.
- All retained benchmark JSON parsed successfully.
- `openspec validate qualification-corpora-baselines --strict`: passed.
- `git diff --check` for the change surface: passed.
- `sh scripts/check_product_boundary.sh`: passed.
- `cargo build -p compass-cli --release --locked`: passed while producing the
  retained current-engine baseline binary.
- Full documented baseline runner: passed with five samples for all six process
  workloads and a directly measured PASS result for all 30 raw tasks.
- `compass update .`: passed; indexed 713 files into 121,261 nodes and 285,681
  edges (68 unsupported edges omitted explicitly, zero nodes omitted, zero
  identity collisions quarantined).

The workspace Rust clippy/test baseline was green immediately before C-013 and
remains applicable because this change adds no Rust source, dependency, manifest,
lockfile, shipped product behavior, or public compatibility surface.

## Qualification status

Deterministic artifact refinement passed using the repository's established
fallback record because the installed artifact-refiner adapter lacks its
referenced canonical controllers and schema assets. The first isolated review
correctly blocked on baseline reproducibility and raised six additional boundedness
and evidence concerns. All findings were resolved or, for KBD-required `.refiner/`
state, dispositioned against the explicit persistence contract. The second review
passed without critical findings; its remaining four warnings and two suggestions
were also resolved. A final fresh-context review and graph refresh are recorded
before archive. A third review caught two inaccurate reproduction/evidence claims
and two remaining runner bounds; those findings were corrected and the complete
baseline was re-measured before the next review.
The fourth review passed without critical findings and identified three remaining
audit warnings plus one input-validation suggestion. Those were resolved with
source-complete provenance, explicit KBD QA-state ownership, and clean malformed
input failures before another full baseline run and review.
The fifth review passed without critical findings and identified two provenance/
evidence warnings plus two input-integrity suggestions. The source inventory now
uses Git's tracked and non-ignored-untracked sets explicitly, the refinement date
matches the final evidence date, expected-result field types are validated before
execution, and duplicate edge identities fail closed. A complete five-sample
baseline was recorded again after these corrections.
The sixth review passed without critical findings and identified one remaining
fail-closed warning plus two publication/documentation suggestions. Every task now
requires an expected-evidence object, the raw and baseline JSON outputs publish
atomically through one portable helper, and the proposal now states the runner's
repository-contained work-directory contract accurately.
The seventh review passed without critical findings and identified three final
publication/immutability/bounds warnings plus three consistency suggestions.
Graph materialization now streams to an atomic sibling, future baselines are
added under new versions rather than replacing history, search and warmups are
explicitly bounded through the shared process helper, binary compression streams,
and the retained host records the Python/zlib pair governing gzip size. The full
baseline was re-measured after these corrections.
The eighth review passed without critical findings and identified two raw-contract
warnings plus two boundedness/scope suggestions. All operations now publish
explicit status, declared task limits are enforced, the task input has byte/count
ceilings, and the current-engine runner rejects non-medium profiles before work.
The timing-free oracle and full baseline were regenerated afterward.
The ninth review passed without critical findings and identified two final
fail-closed warnings plus two strictness/audit suggestions. Expected contracts now
reject unknown keys, empty task suites cannot report PASS, graph identity/name
fields require non-empty strings, and the normalized interpreter label is tied
explicitly to its measured executable name and `host.python`. The full baseline
was recorded again after the per-record validation change.
The concluding tenth isolated review passed with zero critical findings, two
warnings, and two suggestions; the anti-theater gate passed. Its retained
follow-up findings cover deeply nested task JSON error normalization, a scripted
oracle-promotion convenience, impact's necessarily non-empty source-inclusive
status semantics, and streaming rather than post-process output caps. None changes
the frozen corpora, measured evidence, or acceptance result, and the KBD contract
permits archive on PASS while preserving noncritical findings in the review record.
