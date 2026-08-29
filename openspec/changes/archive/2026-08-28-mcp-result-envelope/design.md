# Design: MCP result envelope

## Envelope

The four core navigation tools return this closed, versioned shape:

```json
{
  "schema": "compass.code_context.v1",
  "repository": "sha256:source-tree",
  "generation": "sha256:generation",
  "freshness": {"status": "current"},
  "data": {"schema": "compass.query/1"},
  "evidence": {"records": 2, "anchored": 2},
  "confidence": {"exact": 2, "inferred": 0, "ambiguous": 0},
  "truncation": {"truncated": false, "next": null},
  "warnings": []
}
```

`resultType` deliberately does not appear inside `structuredContent`. It is the
MCP `CallToolResult` discriminator emitted by rmcp 3.1.4 and is `complete` for
these synchronous operations. `schema` is the Compass result-envelope version.

`repository` uses the exact realization's content-addressed source-tree digest.
`generation` uses that same realization's generation ID. Both identities come
from the query engine snapshot, so an atomic graph replacement cannot mix
envelope identity from one realization with data from another. Strict graph
validation rejects empty source-tree or generation identities before MCP
invocation, so successful envelopes satisfy the schema's non-empty identity
contract. Freshness is evidence-scoped: `stale` when the query
reports stale source evidence, `current` when returned file evidence was
checked without such a warning, and `unknown` when the result has no checked
file evidence. Compass does not invent an indexed timestamp or dirty-worktree
claim that the graph artifact does not record.

`data` is the prior `compass.query/1` response without field removal or
reinterpretation. Evidence and confidence are deterministic summaries of the
returned node/edge evidence. Warnings are the prior sorted diagnostics.
Truncation carries the existing bounded flag and reserves `next`; this version
sets `next` to null because these tools do not yet expose continuation tokens.

## Tool status

| Tool | Status after this change |
| --- | --- |
| `search_symbols` | `compass.code_context.v1` envelope + output schema |
| `get_callers` | `compass.code_context.v1` envelope + output schema |
| `get_callees` | `compass.code_context.v1` envelope + output schema |
| `get_impact` | `compass.code_context.v1` envelope + output schema |
| `explore_code` | Existing typed `compass.query/1`; envelope deferred |
| `get_node` | Existing typed `compass.query/1`; envelope deferred |
| natural `query_graph` | Existing typed `compass.query/1`; envelope deferred |
| traversal `query_graph` | Text mode deprecated since 0.4.0; typed natural routing remains |
| `get_neighbors` | Text result deprecated since 0.4.0 |
| `get_community` | Text result deprecated since 0.4.0 |
| `god_nodes` | Text result deprecated since 0.4.0 |
| `graph_stats` | Text result deprecated since 0.4.0 |
| `shortest_path` | Text result deprecated since 0.4.0; use `get_node` |
| `list_prs` | Text result deprecated since 0.4.0 |
| `get_pr_impact` | Text result deprecated since 0.4.0; use typed symbol impact where applicable |
| `triage_prs` | Text result deprecated since 0.4.0 |

No tool is renamed or removed. Discovery descriptions plus Compass metadata
mark every remaining text-only result deprecated without representing those
formats as stable machine schemas. No removal release is scheduled before a
typed replacement exists and receives its own compatibility review.

## Determinism and compatibility

Tool discovery remains in the existing fixed order. Golden fixtures cover the
four complete results and output schemas. Pre-envelope projection tests
asserts that each envelope's `data` is exactly the previous typed response, so
direction, multiplicity, ambiguity, evidence, bounds, and stable record order
cannot be lost by the wrapper. The multiplicity fixture includes two distinct
same-endpoint call occurrences with separate identities, source sites, and
evidence, and the schema suite validates every model enum and tagged detail
variant.

The caller's `maxResponseBytes` bound applies to the final encoded envelope,
not only its nested query payload. An envelope that would cross the explicit
bound fails with the existing query-response limit classification.
