# Compact query output qualification

This change makes human-readable answers smaller while preserving typed query
records, source anchors, direction, multiplicity and uncertainty. Native search
and traversal remain offline and do not depend on a comparison tool.

| Requirement | Implementation | Verification |
| --- | --- | --- |
| Compact defaults with full audit access | `compass-output` shares compact status and source-located ledgers; CLI `--verbose`/`--evidence` adds status and record evidence | Renderer and CLI answer-equivalence tests; MCP structured-content test |
| Answer-sized one-hop cost | Suppress duplicate entity inventories and empty sections | One-hop regression requires fewer than 100 estimated tokens with both source locations |
| Short non-answers | Cap compact no-match/candidate text at 199 approximate tokens | Empty/ambiguous renderer and CLI fixtures; MCP points to full structured records |
| Strict budget with continuation | Four UTF-8 bytes per approximate token, including newline and continuation; whole typed entries or explicit required budget | Unicode reconstruction and page-budget tests; finite-command interface panel |
| Read remaining output without repeating actions | Immutable `compass.saved-output/1` records and additive `compass output` reader | Digest/corruption, retention, redirected-path, machine-output and init publication tests |
| Faster repeated native queries | Complete-response cache bound to verified graph, Program IR, profiles, complete request and semantic mode | Cache-hit execution counter, reopen/invalidation/corruption tests and registered optimized cold/warm panel |
| Preserve correctness while reducing average cost | Replay unchanged source-reviewed questions over frozen public-repository graphs | Fixed 25-question natural suite and 50-question standard suite; per-row verdict comparison |

The cache excludes truncated executions and source excerpts. Profiling and
caller-supplied cancellation paths execute natively. Cache failures fall back to
native execution. Storage has entry, payload, page and scan limits; neither
cache rewrites graphs or historical realizations.

Finite budgets reject streaming events, watch, serve and REPL before starting.
Interactive init requires `--yes`. Oversized JSON/JSONL fails without emitting a
partial machine record and points to the completed saved output. Saved records
are work-directory-local and can be evicted; this is a disposable continuation
facility, not archival storage.

Token measurements concern CLI stdout and use the existing harness's UTF-8
byte estimate. MCP text is compact, while full structured content remains
available and still contributes to clients that consume both. The five public
source trees and their source-reviewed text oracles are a focused recall panel;
they do not independently establish precision or population-wide accuracy.

Exact-only lookup and source-bearing exploration remain native indexed queries;
the response cache does not promise acceleration for those paths.

The [recorded optimized replay](../../benchmarks/agent_query/compact_query_output_review.json)
keeps every native oracle verdict: 49/50 standard questions at 363.76 mean
estimated tokens versus the comparator's 549.74, and 24/25 natural questions at
547.44 versus 757.44. The [performance reference](../../PERFORMANCE.md#compact-query-text-and-complete-response-caching)
records both remaining failures, slightly larger natural answers, cold-run
regressions and the partial Gson graph that intentionally remains uncached.
All repeated raw JSON answers match across both optimized binaries.
