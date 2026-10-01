# Python calls and impact qualification

Compass captured **2,550 / 2,550 explicit source calls (100%)** in ten Dify
production service classes and accounted for all published direct connections
of five widely used classes. This measures source-call capture, not runtime
dispatch or exact target resolution.

Input: [Dify at `1a918ea407990ecd2156af2614932db8c9c43091`](https://github.com/langgenius/dify/tree/1a918ea407990ecd2156af2614932db8c9c43091/api).
The checkout was read-only. Extraction used native code-only mode, maximum
inference, and excluded `tests/**`. The random seed was `20260930`; selection
was from all 175 source service classes with at least five method-body calls.
Missing or ambiguous graph classes fail the gate rather than being replaced.

## Call capture

| Service | Source calls | Captured | All-exact edge sites | Inferred edge sites |
| --- | ---: | ---: | ---: | ---: |
| MigrationPackageService | 24 | 24 | 8 | 15 |
| InstalledAppConversationService | 17 | 17 | 16 | 1 |
| ModelLoadBalancingService | 143 | 143 | 32 | 82 |
| OAuthServerService | 30 | 30 | 30 | 0 |
| RagPipelineService | 479 | 479 | 77 | 342 |
| SkillManagementService | 1488 | 1488 | 443 | 846 |
| WorkflowAppLogQueryService | 15 | 15 | 5 | 8 |
| WorkflowCommentService | 169 | 169 | 33 | 121 |
| WorkflowDraftVariableService | 159 | 159 | 28 | 122 |
| WorkspaceProvisioningService | 26 | 26 | 26 | 0 |

There were 698 source sites with all-exact target evidence and 1,537 with
inferred edge evidence. These are separate measurements; neither is a claim
of 100% resolved-target accuracy. Remaining invocations stay in the source
inventory. Default callee queries omit inferred edges while reporting unresolved
counts; `--include-heuristic` exposes the optional inferred layer.

The source oracle uses Python AST invocation ranges, including nested call
expressions, while excluding nested named function/class bodies from their
enclosing method. Native class queries aggregate owned methods and initializers,
retain actual method endpoints, and disclose their source-call inventories.
All ten responses retained their query bounds without truncation.

The source-first sample caught a rejected typed factory receiver in
`ModelLoadBalancingService`. The native evidence validator now accepts
source-backed Python nominal aliases as factory receivers, while rejecting
other binding kinds and non-Python aliases. The rebuilt graph had zero
extractor failures.

## Direct impact audit

| Symbol | Known directional contacts | Direct dependents | Outgoing context | Excluded |
| --- | ---: | ---: | ---: | ---: |
| machinery.context.RequestContext | 1389 | 1381 | 6 | 2 |
| models.model.App | 1289 | 1243 | 45 | 1 |
| models.account.Account | 1265 | 1221 | 43 | 1 |
| fields.base.ResponseModel | 663 | 660 | 2 | 1 |
| models.dataset.Dataset | 583 | 556 | 26 | 1 |

For every symbol, the retained direct dependents, outgoing context and
exclusions account for every known `(direction, neighbor)` contact. Parallel
occurrences remain graph relationships but do not inflate affected-node counts.
Outgoing context and containment are not invented affected dependents.
The audit uses depth one, at most 2,000 nodes and 10,000 edges, and independently
checks published adjacency. A capped path ledger does not cap the summary.

Five source locations per symbol were inspected, covering imports, parameter
annotations, return contracts, inheritance and query/model references. Examples:

- RequestContext: [controller import](https://github.com/langgenius/dify/blob/1a918ea407990ecd2156af2614932db8c9c43091/api/controllers/console/flask_admission.py#L27).
- App: [typed tool arguments](https://github.com/langgenius/dify/blob/1a918ea407990ecd2156af2614932db8c9c43091/api/core/mcp/server/streamable_http.py#L264).
- Account: [workspace controller import](https://github.com/langgenius/dify/blob/1a918ea407990ecd2156af2614932db8c9c43091/api/controllers/console/workspace/models.py#L35).
- ResponseModel: [inherited response](https://github.com/langgenius/dify/blob/1a918ea407990ecd2156af2614932db8c9c43091/api/controllers/web/human_input_form.py#L70).
- Dataset: [query reference](https://github.com/langgenius/dify/blob/1a918ea407990ecd2156af2614932db8c9c43091/api/controllers/service_api/dataset/document.py#L410).

Native fixtures separately verify exception construction and bare catch
references, typed unit-of-work fields, injection, factory chains, mutation,
shadowing, ambiguous receivers and incorrect same-name recursive rewrites.

The publisher quarantined 15 invalid edge records, with zero omitted nodes
and zero identity collisions. Responses retain `IncompleteCoverage`; a complete
direct audit refers to the published graph, not to every possible source or
runtime dependency. Transitive completeness is bounded separately.

## Reproduce

Set `CARGO_TARGET_DIR` to the per-checkout external target required by
`AGENTS.md`, `DIFY_API` to the pinned checkout's `api/` directory, and
`QUALIFICATION_OUT` to an artifact directory outside the source checkout.

```bash
CARGO_TARGET_DIR="$CARGO_TARGET_DIR" cargo build --locked \
  -p compass-core --example republish_qualification \
  -p compass-query --example query_batch
"$CARGO_TARGET_DIR/debug/examples/republish_qualification" \
  "$DIFY_API" "$QUALIFICATION_OUT"
python3 scripts/qualify_python_service_calls.py \
  --repository "$DIFY_API" \
  --graph "$QUALIFICATION_OUT/compass-out/graph.json" \
  --compass "$CARGO_TARGET_DIR/debug/compass" \
  --batch-runner "$CARGO_TARGET_DIR/debug/examples/query_batch" \
  --report "$QUALIFICATION_OUT/services-report.json"
```

The rebuild helper reuses only validated compatible AST facts. The batch runner
executes the owning native query APIs on one validated graph; Python is solely
an offline qualification oracle and never supplies product graph facts.
Inputs, queries and outputs are bounded. Batch setup plus execution has a
twenty-minute bound because a cold large-graph search index can be expensive.
This qualification makes no latency, token-cost or competitor-performance claim.

## Fixture topology baseline

Source-backed deferred receiver nodes add 17 nodes to the fixture graph
(1,292 to 1,309). They remain explicitly inferred and attach to their callers.
The exact layer retains 56 cross-file edges and gains six exact edges
(956 to 962), six exact typed endpoint pairs (948 to 954), and two exact
edge-bearing nodes (918 to 920). Overall connected components fall to 236;
there are no self-loops.

The three exact-density floors are rebased to the larger node denominator
(42, 702 and 728 per thousand). Exact-only isolated nodes and components each
grow by 15, matching the added nodes without exact incident evidence. Their
ceilings become 389 and 528. Absolute exact-edge, cross-file-edge and endpoint
floors remain intact, as do all relationship-specific and semantic assertions.
This changes the expected inferred layer rather than claiming those contacts
as exact evidence.
