# Versioned History HTML Export Plan Audit

## Outcome

The original plan had a sound product direction but was not implementation-ready. It contained three release-blocking contradictions and omitted important performance, history-rewrite, comparison, accessibility, privacy, and browser-verification contracts. The implementation plan and design spec have been revised to close those gaps.

This audit scores plan coverage, not a shipped UI. Implementation quality must be re-audited after the viewer exists.

## Audit Health Score

| Dimension | Original | Revised plan | Key correction |
| --- | ---: | ---: | --- |
| Accessibility | 1/4 | 4/4 | Keyboard model, live state, canvas alternative, touch targets, WCAG browser test |
| Performance | 1/4 | 3/4 | Streamed archive, per-commit compression, lazy decode, worker, bounded LRU, overview mode |
| Responsive design | 1/4 | 4/4 | Explicit 760/520 px adaptations and 320 px acceptance test |
| Theming | 1/4 | 3/4 | Shared tokens, light/dark behavior, contrast tests |
| Anti-patterns | 3/4 | 4/4 | DAG rail becomes the signature; generic glass/card styling is prohibited |
| **Total** | **7/20 — Poor** | **18/20 — Excellent plan coverage** | **Implementation evidence still required** |

## Anti-Patterns Verdict

**Pass with corrections.** The commit rail and graph canvas are specific to Compass and do not inherently read as generic AI UI. The original plan did risk a generic three-pane dashboard and a second copy of the existing graph viewer. The revised plan makes the parent-DAG rail the visual signature, reuses Compass graph behavior, and explicitly rejects decorative glass blur, nested cards, gradient text, and metric-tile filler.

## Executive Summary

- Original issues: **3 P0, 9 P1, 3 P2**
- Release blockers: monolithic payload architecture, unsafe no-clobber semantics, incorrect SHA-based timeline/default selection
- Major risks: rewritten Git history, non-Compass diff logic, renderer divergence, large graphs, layout instability, privacy/XSS/CSP, and missing real-browser evidence
- The revised plan now has six implementation tasks plus an atomic-file prerequisite and a deferred enhancement backlog

## Detailed Findings

### P0 — One monolithic raw JSON archive contradicted corruption isolation and scalability

- **Location:** Original Task 2 renderer architecture
- **Impact:** One malformed record would prevent parsing the entire archive. Loading every full graph at startup would multiply memory use and freeze or crash large histories.
- **Recommendation applied:** Separate manifest from independently compressed/digested payloads; stream export; lazy-decode one commit; keep a three-entry LRU; isolate payload errors.

### P0 — Check-then-write did not guarantee no overwrite

- **Location:** Original `write_history_html`/CLI publication steps
- **Impact:** A concurrent process could create the target between `exists()` and atomic replacement, causing data loss despite the documented no-overwrite promise.
- **Recommendation applied:** Add `PreparedFile` with staged owner-only output and an atomic no-clobber publish primitive tested on Linux, macOS, and Windows.

### P0 — SHA sorting could not produce a timeline or identify the newest commit

- **Location:** Original deterministic-order test and `newestEntry` client fallback
- **Impact:** Lexicographic object IDs are unrelated to time or ancestry. The rail could be scrambled and invalid fragments could open an arbitrary graph.
- **Recommendation applied:** Reverse topological ordering over stored parent edges, deterministic lane assignment, materialized `HEAD` default, first-parent fallback, then deterministic leaf fallback.

### P1 — Rewritten Git history could make a valid immutable export fail

- **Location:** Original required `Repository::presentation` lookup
- **Impact:** Compass deliberately retains realizations after history rewrites, but the plan required Git subject/author/date to exist.
- **Recommendation applied:** Use machine-readable `git cat-file --batch-check`; missing objects produce optional presentation fields while stored SHA/parents/graph remain exportable.

### P1 — Browser-side ID comparison bypassed Compass diff comparability

- **Location:** Original `compareWithParent`
- **Impact:** It missed changed records and hyperedges and could present profile-induced differences as code changes.
- **Recommendation applied:** Precompute bounded Node/Edge/Hyperedge diffs through `HistoryStore::diff_records`; disable absent/incompatible parent comparisons with reasons.

### P1 — A second graph renderer would regress existing Compass behavior

- **Location:** Original new `history_html.rs` viewer
- **Impact:** Search, aggregation, accessibility, physics controls, node inspection, and parity behavior would drift from `html.rs`.
- **Recommendation applied:** Extract and reuse a shared `GraphViewModel` and canvas/control template while preserving existing static export parity.

### P1 — Large graphs had no rendering contract

- **Location:** Original full-snapshot viewer
- **Impact:** Current Compass HTML aggregates above 5,000 nodes; directly sending 50,000 nodes to `vis-network` is not usable.
- **Recommendation applied:** Embed the exact graph but precompute/open a community overview above 5,000 nodes; retain exact search and drill-down.

### P1 — Commit changes would visually “teleport”

- **Location:** Original `renderGraph` replacement
- **Impact:** Fresh physics layouts destroy a user’s spatial memory, undermining time travel.
- **Recommendation applied:** Preserve positions for stable node IDs, seed new nodes near surviving neighbors, retain surviving node selection, and destroy old network/listener state.

### P1 — Offline intent lacked an enforceable security boundary

- **Location:** Original “no `https://`” string assertion
- **Impact:** String checks can false-positive on license text and miss dynamic requests. Untrusted commit/node labels could become DOM injection.
- **Recommendation applied:** Strict CSP, inert compressed payload elements, structural ID validation, text-only DOM insertion, hostile-label browser tests, and request interception.

### P1 — The shareable file could leak an absolute local path

- **Location:** Original `repository.root().display().to_string()`
- **Impact:** Exporting and sharing the HTML disclosed the exporter’s filesystem/user path.
- **Recommendation applied:** Use repository basename or explicit `--title`; test that the temporary root never appears.

### P1 — Accessibility and responsive behavior were unplanned

- **Location:** Original three-pane viewer task
- **Impact:** No keyboard rail model, canvas alternative, mobile collapse, touch-target, live-status, reduced-motion, or contrast contract existed.
- **Standard:** WCAG 2.2 AA
- **Recommendation applied:** Explicit semantics and breakpoints plus Playwright/axe coverage at 320 px and reduced motion.

### P1 — Marker tests could not prove `file://` behavior

- **Location:** Original final verification
- **Impact:** Deep links, Back/Forward, CSP, workers, payload decode, and accessibility can fail in a browser while Rust substring tests pass.
- **Recommendation applied:** Add pinned Chromium acceptance tests over generated deterministic fixtures and run them in CI.

### P2 — Payload digest semantics were incomplete

- **Location:** Original manifest contract
- **Impact:** A stored digest provided no value unless the browser checked exact bytes before parsing.
- **Recommendation applied:** Verify uncompressed length and SHA-256 before JSON parsing with a local `file://` fallback. Clarify that v1 integrity is not authenticity.

### P2 — Manifest failure and payload failure were not distinguished

- **Location:** Original runtime error behavior
- **Impact:** “Other commits stay selectable” is possible for a payload error but not for an unreadable manifest.
- **Recommendation applied:** Manifest corruption is page-fatal with recovery guidance; payload corruption is commit-local.

### P2 — Visual language and theme behavior were underspecified

- **Location:** Original UI plan
- **Impact:** It could regress contrast or become a generic dashboard disconnected from Compass.
- **Recommendation applied:** Shared tokens, accessible light/dark themes, topology-lane visual identity, and anti-pattern constraints.

## Systemic Issues Corrected

- The original plan treated a small export and a repository-wide historical archive as the same rendering problem.
- It relied on current Git state even though Compass history is intentionally more durable than Git reachability.
- It specified offline behavior as content inspection rather than a browser-enforced/runtime-tested contract.
- It treated time travel as data replacement, without spatial continuity or semantic comparability.

## Positive Findings Preserved

- The CLI remains backward-compatible with revision-specific `graph-json` and `compass-out` exports.
- Preferred realizations are validated and alternate realizations stay out of v1.
- Full graph is the default; comparison remains opt-in.
- Merge parents are explicit.
- The export is a single portable HTML artifact.
- The plan retains TDD steps, focused contracts, frequent commit boundaries, and a final `graphify update .`.

## Recommended Execution Order

1. **P0 `/harden`** — Implement staged no-clobber publication, CSP, payload isolation, and rewritten-history fallback.
2. **P0 `/optimize`** — Implement streaming compression, lazy worker decode, bounded LRU, virtualization, and overview mode.
3. **P1 `/normalize`** — Extract and reuse the current graph view model/controls instead of duplicating them.
4. **P1 `/adapt`** — Complete keyboard, responsive, reduced-motion, theme, and canvas-alternative behavior.
5. **P2 `/polish`** — Validate layout continuity, copy, empty/error states, and the DAG rail’s visual clarity.

Re-run `/audit` after implementation to replace plan-coverage scores with measured UI evidence.
