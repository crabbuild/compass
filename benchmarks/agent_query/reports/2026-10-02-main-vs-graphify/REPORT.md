# Compass and Graphify evaluation on real repositories

Evaluated on October 2, 2026 for an engineering tooling decision. Compass main is pinned at [b7b7fa819b127464566430cc454fd2085d5d5b62](https://github.com/crabbuild/compass/commit/b7b7fa819b127464566430cc454fd2085d5d5b62) (0.4.1); Graphify is the official [graphifyy 0.9.74 package](https://pypi.org/project/graphifyy/0.9.74/). This is a fresh measured comparison on the same read-only public source revisions, not a reuse of earlier performance claims.

**The results do not establish a universal winner.** Compass retains more of the independently checked declarations and exact call occurrences. Graphify builds the Zod corpus faster. Natural-language retrieval, supported output features, graph completeness, and storage mode need separate consideration; the tables below preserve those distinctions.

## Scope and fairness

The five language panels use Cobra (Go), Flask (Python), Gson (Java), Zod (TypeScript), and the Axum package (Rust). These are selected development repositories and existing source-reviewed questions. They are not a blinded or randomly sampled evaluation of all projects or languages.

| Repository | Language | Source commit | Tracked files | Tracked MiB |
| --- | --- | --- | --- | --- |
| cobra | Go | adbc8813901b | 66 | 0.67 |
| flask | Python | d73fa1cdcbd8 | 236 | 1.78 |
| gson | Java | 15ca7360379c | 313 | 2.23 |
| zod | TypeScript | d2b135cfb7a3 | 739 | 14.80 |
| axum | Rust | af1345b53a25 | 85 | 0.82 |

Both products use their code-only, community-clustered profiles without model credentials. Compass uses default low inference and is measured with JSON and SQLite storage separately; SQLite is its current default storage mode. Graphify is measured with its base-install Louvain backend and a separate environment with the native Leiden backend (`graspologic-native` 1.3.1). The native backend was checked to be callable before its panel. Extraction defaults and graph contracts differ: identical flags do not imply identical file selection, entity kinds, relationships, or clustering. Compass also inventories and can structurally extract non-code files. Graph size is therefore not an accuracy metric.

| Reported fresh code scope | Compass headline file count | Graphify selected code files |
| --- | --- | --- |
| cobra | 37 | 37 |
| flask | 92 | 92 |
| gson | 273 | 273 |
| zod | 564 | 564 |
| axum | 60 | 61 |

These headline counts agree for four corpora; Axum differs by one selected file. Compass’s headline count does not include all its structural document work: its Axum graph reports 60 extracted Rust files plus 23 extracted Markdown files, while Graphify represents Cargo.toml and skips document extraction. The native per-language extraction counts are retained in [summary.json](data/summary.json). These are end-to-end product profiles, not an equal-work parser-speed experiment. Native structural coverage of non-code formats is not used as a declaration-recall advantage.

There are three fresh artifact roots and three unchanged rebuilds for each repository and configuration: 120 measured builds in total. Base Compass JSON and Graphify builds alternate order. SQLite follows those builds; the native Leiden arm follows the base panel. Build and query subprocesses are serial. The machine is an Apple M2 Max, 12 logical CPUs, 32 GiB RAM, macOS 26.5.2, with substantial unrelated workloads active; source-audit and report preparation also overlapped portions of this shared-host run. Per-build host load, CPU time, process-tree samples, commands, outputs and failures are retained. The wall-time comparisons are diagnostic shared-host measurements, not a controlled release-performance qualification.

Fresh builds mean fresh tool artifacts, with ordinary OS and runtime caches left in place. Unchanged rebuilds reuse the same output root. No source-edited incremental or watcher run was performed. Query trials start fresh CLI processes, while ordinary OS and tool caches may warm. They do not measure persistent MCP server latency.

## Source checked graph quality

The call oracle was written from pinned source before graph generation. A positive requires unique source and target declarations with exact file, declaration line and name; the directed `calls` relationship must retain the exact occurrence line. Five reviewed reversed-call controls must be absent. This measures selected call-occurrence recall and those controls, not whole-graph precision.

| Graph producer | Exact call occurrences | Negative controls | Matched declarations in two censuses |
| --- | --- | --- | --- |
| Compass JSON | 21/22 | 5/5 | 827/827 |
| Graphify base | 18/22 | 5/5 | 700/827 |
| Graphify native Leiden | 18/22 | 5/5 | 700/827 |

| Repository | Compass exact calls | Graphify base exact calls | Graphify native Leiden exact calls |
| --- | --- | --- | --- |
| cobra | 6/6 | 5/6 | 5/6 |
| flask | 4/4 | 4/4 | 4/4 |
| gson | 6/6 | 4/6 | 4/6 |
| zod | 2/3 | 2/3 | 2/3 |
| axum | 3/3 | 3/3 | 3/3 |

| Independent source census | Compass | Graphify base | Graphify native Leiden |
| --- | --- | --- | --- |
| flask function | 389/389 | 363/389 | 363/389 |
| flask class | 53/53 | 52/53 | 52/53 |
| cobra function | 270/270 | 270/270 | 270/270 |
| cobra type | 17/17 | 15/17 | 15/17 |
| cobra field | 98/98 | 0/98 | 0/98 |

The censuses cover all 24 `src/flask` Python files using CPython AST and all 19 tracked non-test Cobra Go files using Go’s standard parser. Names and exact declaration starts must match uniquely. Named Go fields also require direct containment by the exact source type. The Python census includes overload stubs, implementations, property setters and nested declarations; it does not count distinct runtime functions. Graphify’s 27 unmatched Python declarations include 9 overload stubs, 9 corresponding implementations merged onto earlier declarations, 7 setters, a nested view function and a nested class. Its two unmatched Go types are the Completion and CompletionFunc aliases. Function/class owner identity, full signatures, field-access binding and runtime dispatch are not established by these census matches. Graphify omits optional kind/type on many AST nodes; a documented scorer amendment allows missing kind for source-proved fields while retaining the exact source/owner requirements.

Concrete call gaps: Compass lacks `fromJSONSchema -> convertSchema` at Zod line 943. Graphify lacks `convertSchema -> convertBaseSchema` at line 814, retains only one of the two reviewed `InitDefaultHelpCmd -> Find` occurrences in Cobra, and collapses the two relevant Gson `toJson` overload identities onto an earlier same-named declaration. The two Zod misses are different facts. Neither tool invented any of the five selected negative calls. The checked-in suite prose incorrectly locates Zod’s return call at line 952; the independently verified occurrence is line 943. The required query names were kept unchanged.

Compass’s published schema declares a directed multigraph. Both clustered Graphify arms declare an undirected simple graph, though serialization restores the retained edge’s source/target direction. This can preserve a valid pair while losing repeated occurrences or distinct same-endpoint relationships. The Cobra occurrence and Gson overload checks show concrete losses; the graph flags alone are not an accuracy score.

## Query answer recall and interface coverage

Each fixed question is run three times on all four configurations, for 1,020 captured query workflows. Scores count distinct questions that pass all three trials. Repetitions are not independent accuracy examples. The 50 structured tasks use each tool’s closest documented operation and default output form; the 25 query questions use identical text and an 800-token first-page budget without continuations. The ten ask tasks use identical text and a 2,000-token budget, reusing source facts from the structured panel.

| Panel | Compass JSON | Compass SQLite | Graphify base | Graphify native Leiden |
| --- | --- | --- | --- | --- |
| structured | 48/50 | 48/50 | 44/50 | 44/50 |
| natural | 22/25 | 23/25 | 18/25 | 18/25 |
| ask | 10/10 | 10/10 | 10/10 | 10/10 |

**These are required-fact text-recall scores, not answer precision.** A required name can appear in a source excerpt, a broad neighborhood, or a path that does not prove the requested call. Extra statements are not exhaustively adjudicated. The five source-excerpt questions also measure a supported-output capability: Graphify explain does not return the requested declaration text. Those failures should not be described as five incorrect graph bindings.

| Structured panel excluding source-excerpt capability | Compass JSON | Compass SQLite | Graphify base | Graphify native Leiden |
| --- | --- | --- | --- | --- |
| 45 questions | 43/45 | 43/45 | 44/45 | 44/45 |

A post-capture directional audit checks the ten caller/callee tasks against 19 requirements from 18 distinct facts in the original before-build source oracle. It requires the displayed call direction and exact occurrence, plus graph support from unique exact declarations. This was added after seeing a text-score false pass, so it is explicitly not a blinded pre-registered score. The Gson caller task now requires both reviewed overload calls, a stricter requirement than the original name-only score. Extra output remains unjudged.

| Source-supported caller/callee requirements | Compass JSON | Compass SQLite | Graphify base | Graphify native Leiden |
| --- | --- | --- | --- | --- |
| 10 tasks | 10/10 | 10/10 | 8/10 | 8/10 |

Graphify’s Zod answer lists `convertBaseSchema` as an incoming caller at line 431 when the task asks for `convertSchema`’s callees. The original text scorer credits the name, but the reviewed outgoing call at line 814 is absent. Its Gson answer shows the real line-642 call, but omits line 724 and merges the overload declaration identities. Both native Leiden and base Graphify have these gaps; Compass answers all ten selected directional requirements. This does not establish complete answer precision.

| Repository | Structured Compass / Graphify base | Natural query Compass / Graphify base | Ask Compass / Graphify base |
| --- | --- | --- | --- |
| cobra | 9/10 / 9/10 | 4/5 / 2/5 | 2/2 / 2/2 |
| flask | 10/10 / 9/10 | 5/5 / 2/5 | 2/2 / 2/2 |
| gson | 10/10 / 9/10 | 4/5 / 4/5 | 2/2 / 2/2 |
| zod | 10/10 / 9/10 | 4/5 / 5/5 | 2/2 / 2/2 |
| axum | 9/10 / 8/10 | 5/5 / 5/5 | 2/2 / 2/2 |

Compass fails two structured retrieval tasks on every trial: Cobra’s broad resolve/run question omits Find, and Axum’s bounded impact answer omits route_endpoint. Those facts exist in its graph, but the returned answer does not satisfy the question. Graphify’s six structured failures are the five source-excerpt tasks and an Axum file-to-file path lookup that exits 1. Excluding the five source-excerpt capabilities leaves Graphify ahead on the original name-recall proxy, 44/45 versus Compass 43/45; the source-direction audit is a separate, stricter result.

Timed-out query workflows: 1. Questions with differing pass/fail results between trials: 1. These observations remain in the denominator. A timeout on the busy host does not by itself identify a root cause; successful retries do not erase the failed trial.

## Indexing time and CPU cost

Times below are three-sample medians in seconds. No outlier was discarded. Ranges and all sample values remain in [summary.json](data/summary.json) and [builds.jsonl](data/builds.jsonl). CPU time is accumulated user plus system time for the measured child workload and can exceed wall time when workers run in parallel.

| Repository | Compass JSON fresh | Compass SQLite fresh | Graphify base fresh | Graphify native Leiden fresh |
| --- | --- | --- | --- | --- |
| cobra | 2.102 | 3.101 | 4.164 | 3.556 |
| flask | 3.565 | 3.822 | 5.809 | 6.484 |
| gson | 6.079 | 10.838 | 7.100 | 20.034 |
| zod | 34.275 | 62.853 | 15.249 | 21.208 |
| axum | 6.930 | 13.909 | 9.436 | 5.083 |

| Fresh-build observed range, seconds | Compass JSON | Graphify base |
| --- | --- | --- |
| cobra | 1.675–2.407 | 1.911–5.592 |
| flask | 3.120–5.472 | 4.130–8.911 |
| gson | 4.685–7.335 | 5.978–8.525 |
| zod | 32.386–55.198 | 13.136–18.190 |
| axum | 4.698–13.503 | 8.722–9.442 |

| Repository | Compass JSON unchanged | Compass SQLite unchanged | Graphify base unchanged | Graphify native Leiden unchanged |
| --- | --- | --- | --- | --- |
| cobra | 0.294 | 0.330 | 1.831 | 2.093 |
| flask | 0.637 | 0.596 | 2.991 | 3.994 |
| gson | 1.263 | 1.092 | 3.302 | 11.272 |
| zod | 2.480 | 2.466 | 5.419 | 10.046 |
| axum | 0.855 | 0.771 | 7.352 | 3.081 |

| Repository | Compass JSON CPU | Compass SQLite CPU | Graphify base CPU | Graphify native Leiden CPU |
| --- | --- | --- | --- | --- |
| cobra | 0.901 | 1.231 | 3.680 | 3.743 |
| flask | 1.981 | 2.265 | 5.278 | 5.264 |
| gson | 5.220 | 7.606 | 7.944 | 8.531 |
| zod | 24.978 | 34.166 | 16.236 | 16.894 |
| axum | 2.275 | 2.812 | 4.793 | 4.092 |

## Query latency and output cost

Latency is the median of each distinct question’s median over three complete CLI workflows, including permitted continuation pages. It mixes task kinds, so it must not replace the per-question observations. First-trial medians are also retained in [summary.json](data/summary.json). Output tokens are `ceil(stdout UTF-8 bytes / 4)`, not model-tokenizer counts; stderr warnings are retained separately. Output size alone is not efficiency when the answer is incomplete.

| Panel | Compass JSON ms | Compass SQLite ms | Graphify base ms | Graphify native Leiden ms |
| --- | --- | --- | --- | --- |
| structured | 1121 | 522 | 1177 | 1220 |
| natural | 737 | 617 | 1331 | 1255 |
| ask | 402 | 44 | 584 | 562 |

For a less misleading speed comparison, the next table uses only the identical subset of questions passing every trial on every configuration; structured caller/callee tasks must also pass the added source-direction check. Natural and ask success still use the required-fact text proxy. Subset sizes and question identities are retained in [summary.json](data/summary.json).

| Common successful questions | Count | Compass JSON ms | Compass SQLite ms | Graphify base ms | Graphify native Leiden ms |
| --- | --- | --- | --- | --- | --- |
| structured | 40 | 1058 | 220 | 1190 | 1304 |
| natural | 16 | 1464 | 708 | 1104 | 1236 |
| ask | 10 | 402 | 44 | 584 | 562 |

| Panel | Compass JSON mean tokens | Compass SQLite mean tokens | Graphify base mean tokens | Graphify native Leiden mean tokens |
| --- | --- | --- | --- | --- |
| structured | 424 | 424 | 512 | 512 |
| natural | 537 | 547 | 758 | 759 |
| ask | 251 | 251 | 1519 | 1522 |

## Memory and artifact completeness

The primary resource counter is isolated `RUSAGE_CHILDREN.ru_maxrss`: the largest child-process peak, not the sum of concurrent workers. The supplementary process-tree figure sums descendant RSS sampled approximately every 100 ms. It excludes the measurement helper but counts shared pages multiple times and can miss short peaks. Neither is a measurement of unique physical RAM. Calling either tool universally more memory efficient from the single-process counter would be misleading.

| Repository | Compass JSON child / tree MiB | Compass SQLite child / tree MiB | Graphify base child / tree MiB | Graphify native Leiden child / tree MiB |
| --- | --- | --- | --- | --- |
| cobra | 85.8 / 79.7 | 81.8 / 80.0 | 60.9 / 644.8 | 62.8 / 623.8 |
| flask | 123.4 / 117.2 | 122.6 / 113.0 | 100.9 / 613.6 | 103.0 / 652.1 |
| gson | 297.7 / 291.9 | 302.6 / 297.4 | 115.5 / 684.0 | 109.4 / 687.3 |
| zod | 864.2 / 843.2 | 779.6 / 751.5 | 339.8 / 794.1 | 325.5 / 794.3 |
| axum | 139.5 / 128.1 | 133.8 / 127.6 | 64.6 / 612.8 | 67.2 / 637.5 |

| Repository and graph | Nodes | Relationships | JSON MiB | Publication omissions |
| --- | --- | --- | --- | --- |
| cobra Compass JSON | 758 | 3923 | 3.89 | None reported |
| cobra Graphify base | 708 | 2515 | 1.01 | None reported |
| flask Compass JSON | 3038 | 3987 | 6.92 | None reported |
| flask Graphify base | 2141 | 3444 | 1.85 | None reported |
| gson Compass JSON | 5425 | 21579 | 26.42 | partial graph published after quarantining 0 nodes and 2 edges with 0 identity collisions; 0 examples omitted by the diagnostic cap |
| gson Graphify base | 4601 | 16372 | 9.65 | None reported |
| zod Compass JSON | 34964 | 70006 | 86.20 | partial graph published after quarantining 0 nodes and 44 edges with 0 identity collisions; 0 examples omitted by the diagnostic cap |
| zod Graphify base | 5898 | 9561 | 5.31 | None reported |
| axum Compass JSON | 2687 | 7918 | 9.30 | None reported |
| axum Graphify base | 1730 | 3423 | 1.64 | None reported |

Reported omissions are failures of completeness even when extraction exits zero. Unsupported/binary inventory statuses and other diagnostics are retained in [summary.json](data/summary.json); a lack of Graphify diagnostics does not prove complete source coverage. Graph JSON sizes exclude caches, databases, other sidecars and historical snapshots.

## Rebuild consistency

Within each configuration and repository, all three fresh graphs have identical raw JSON digests. All 30 Compass fresh/unchanged pairs retain identical bytes. All 30 Graphify pairs change bytes. A record-level comparison of trial zero shows that Cobra, Flask, Gson and Axum keep the same node identities and edges while some _origin metadata changes. Zod loses four external import nodes (next, rollup, zod4 and zod/v4) and five dynamic_import relationships on an unchanged rebuild, and community assignments change in both Graphify backends. Removed records are retained in [graph-transition-audit.json](data/graph-transition-audit.json). This is a reproducible fresh-to-rebuild difference, not evidence of random output on the three fresh runs.

The selected call/declaration audits and query trials use the retained post-rebuild graph. The observed Graphify transition does not change any of the selected 22 direct calls or the Go/Python declaration census identities. No correctness claim is made about the removed external-import targets beyond the observed loss of those published records.

## Limits and decision guidance

For workflows that require named fields, overload identities, occurrence multiplicity, source excerpts and explicitly typed graph contracts, this panel supports preferring Compass with its default SQLite storage, while checking its partial-publication warnings and the Zod call gap. Graphify base has faster fresh builds than Compass SQLite on Gson, Zod and Axum; Compass JSON has faster fresh builds than Graphify base on four of the five corpora, but has substantially more expensive Zod query loading. These configurations cannot be mixed into a single best-case claim. Graphify is a credible choice when indexing cost or graph exploration dominates. Query-interface scores and latency should be weighed from the appropriate rows rather than pooled into one winner.

This run cannot certify either product for arbitrary production impact analysis. It does not independently estimate precision over all published edges, validate all inheritance/import/alias/dynamic-dispatch targets, assess ideal community partitions or god-object diagnosis, test C/C++/C#/Ruby/PHP/other languages, or establish large-monorepo scalability. The corpora and questions are known development inputs, and the busy host prevents a controlled performance ranking. A tooling choice for a different language or a critical dependency workflow needs equivalent source checks on that target.

## Verification and evidence publication

The pinned Rust 1.97.1 release build completed with `cargo build -p compass-cli --bin compass --release --locked`, using a dedicated Cargo target on the mounted workspace volume. Compilation took 26m 41s on the busy host; this is not an installation comparison. The 45 runner/edge-audit/process tests and `scripts/check_product_boundary.sh` passed. The completed run verified all five clean corpus revisions and source digests, both Graphify package source trees, the benchmark helper identities, all 120 frozen graph hashes and all 1,020 query graph references.

No Compass runtime source changed. The full Rust workspace baseline and eight-repository promotable performance suite were not run: this is a diagnostic external comparison, not a Rust change or release-performance qualification. Graphify setup automatically refreshed an existing Claude skill; the prior skill/version were restored and refresh was disabled for measurements. The globally installed Graphify package was not upgraded.

This repository publishes a path-normalized copy of the report, observations, oracles, source audits, raw stdout/stderr and method snapshots. The original external archive is named `20261002-main-b7b7fa81`. Graphs, databases, caches, binaries, virtual environments and external checkouts remain outside the repository. Their recorded digests do not make those files downloadable from this PR.

### Review the evidence

| File | Contents |
| --- | --- |
| [summary.json](data/summary.json) | Scores, timing/resource aggregates, every query failure, publication omissions and rebuild differences |
| [builds.jsonl](data/builds.jsonl) | All 120 builds: argument vectors, CPU/wall time, child/tree RSS, host load and graph digests |
| [queries.jsonl](data/queries.jsonl) | All 1,020 workflows: question, configuration, trial, argument vector, exit, timeout, score and graph digest |
| [source-call-oracle.json](data/source-call-oracle.json) / [audit](data/source-call-audit.jsonl) | Before-build public-source call witnesses and all 81 positive/negative observations |
| [source-census.json](data/source-census.json) / [audit](data/source-census-audit.jsonl) | The 827 Go/Python declarations and all 2,481 producer/declaration observations |
| [directional-query-audit.json](data/directional-query-audit.json) | Post-capture requirements, matching answer lines and all 120 task/trial observations |
| [captures.jsonl](data/captures.jsonl) / [streams.jsonl](data/streams.jsonl) | Every build/query stdout and stderr, deduplicated by normalized UTF-8 content |
| [registration.json](data/registration.json) and adjacent amendments | Corpus/tool pins, budgets, original protocol and declared amendments |
| [tool-identities.json](data/tool-identities.json) / [completion-verification.json](data/completion-verification.json) | Measured executable/package/helper identities and final source/artifact verification |
| [publication.json](publication.json) | Original input hashes, normalization rules and hashes of the published copy |

For a raw query answer, select `captures.jsonl` entries with paths `queries/PANEL/REPOSITORY/VARIANT/trial-N/QUESTION.TOOL.ATTEMPT.stdout`, then join `streamId` to `streams.jsonl` `id`. A workflow may include continuation attempts. Stderr uses the matching `.stderr` suffix. `streams.jsonl` content retains ANSI escapes and source text as JSON strings.

Local executable, checkout, corpus, artifact, volume and skill paths are replaced with `${COMPASS_BINARY}`, `${COMPASS_CHECKOUT}`, `${CORPUS_ROOT}`, `${EVALUATION_ROOT}`, `${WORKSPACE_VOLUME}`, `${GRAPHIFY_SKILL_ROOT}` or `${USER_HOME}`. Recorded original stream/file hashes refer to the unnormalized archive bytes; a stream ID hashes its published normalized UTF-8 content. The archive manifest is retained separately as [evidence-manifest.json](data/evidence-manifest.json). Paths have changed; scores, measurements, source anchors, diagnostics and omissions have not.

### Replay the method

The [evaluation orchestrator](method/evaluate.py.txt), [native Leiden arm](method/supplement.py.txt), [call auditor](method/source_audit.py.txt), [census](method/census.py.txt), [Go parser](method/go_census.go.txt), [directional auditor](method/directional_audit.py.txt), [transition auditor](method/transitions.py.txt), [report generator](method/summarize.py.txt) and [verifier](method/verify_evidence.py.txt) are source snapshots of this run, with local paths replaced. They are archival text, not installed Compass commands. Original script hashes and the published snapshot hashes are recorded separately.

To replay, use a clean Compass checkout at the measured commit, the pinned public corpus revisions, and a new external artifact directory. Copy the snapshots there with `.txt` removed, configure the literal path placeholders in `evaluate.py`, and recreate the two isolated Graphify environments from the recorded package versions. Build Compass with its pinned toolchain and a dedicated mounted-volume Cargo target. The base Graphify environment uses Python 3.12.13; the native environment additionally installs `graspologic-native` 1.3.1.

Capture registration and source oracles before any graph builds (`evaluate.py register`, `source_audit.py register`, `census.py register`), then record identities and run builds, the native arm and queries. Afterward run the source, census, directional and transition auditors, generate the report and verify retained evidence. The snapshots' explicit registrations for added arms and the original amendments should be reviewed before reproducing them. They do not replace the supported [agent query harness](../../README.md) or the [promotable performance protocol](../../../performance/README.md).

No failed question, build, timeout or publication omission was removed from the reported denominator. Repetitions remain repeated observations, not additional independent accuracy examples.
