# Enhance a graph with an agent

This guide creates and inspects an opt-in Agent Graph Overlay while leaving the
Base Graph byte-for-byte unchanged.

## Use it from an AI coding session

Install or refresh Compass's bundled skill, then reload the skill or start a new
assistant session:

```bash
compass install --project
```

The user can ask for the outcome without writing commands or batch JSON:

```text
Use Compass overlay overlay:review for this coding session. Query the current
graph first. Preserve only useful source-cited GROUNDED enhancements, show each
applied change, and pin the exact revision for later reads. Do not mask Base
facts.
```

The installed skill teaches the assistant to initialize the Base Generation,
draft the strict batch, request only the required local write capability, audit
the receipt, and rebase after source changes. Navigation stays read-only unless
the user explicitly asks to add, update, retract, challenge, or enhance overlay
knowledge. `GROUNDED` is awarded by Compass verification, not asserted by the
assistant.

## 1. Inspect the selected Base Generation

For a Git repository:

```bash
compass agent-graph status \
  --root . \
  --graph compass-out/graph.json \
  --overlay overlay:review \
  --format json
```

For a non-Git corpus, also pass an absolute `--state-root`. Compass rejects an
implicit non-Git storage location.

The returned `baseGeneration` must be copied into the request. Do not compute a
replacement identity from a label or path.

## 2. Prepare a strict change batch

Start from
[`fixtures/contracts/agent-graph/batch-v1.json`](../../fixtures/contracts/agent-graph/batch-v1.json).
Replace its exact Base Generation, source anchor, file digest, and excerpt
digest with values from the selected project. Requests cannot contain a
Grounding certificate or `GROUNDED` status.

Use `selector: new` with a durable `key:` for creation. For replacement, use
`selector: existing` with both the Assertion ID and current assertion digest.
Use Retraction for an agent-owned assertion and Challenge for a Base fact.

## 3. Apply with explicit local authority

```bash
compass agent-graph apply \
  --root . \
  --graph compass-out/graph.json \
  --overlay overlay:review \
  --request change-batch.json \
  --principal principal:local \
  --enable-writes \
  --format json
```

Mask operations additionally require `--allow-masks`. A successful receipt
contains the immutable revision, sequence, exact counts, and batch digest.
Retrying the same idempotency key and content returns the original receipt;
different content under the same key fails.

## 4. Read and query an exact Effective Graph

```bash
compass agent-graph query \
  --root . \
  --graph compass-out/graph.json \
  --overlay overlay:review \
  --revision REVISION_DIGEST \
  --profile augment \
  --cql 'MATCH (a)-[r]->(b) RETURN a, r, b'

compass agent-graph export \
  --root . \
  --graph compass-out/graph.json \
  --overlay overlay:review \
  --revision REVISION_DIGEST \
  --output review-effective.json \
  --format json
```

`export` confines its output beneath `--root`, writes atomically, and refuses to
replace any existing file, directory entry, or symbolic link. Task context
accepts the paired `--agent-overlay` and `--agent-revision` selectors; its
`agentKnowledge` section keeps `GROUNDED` separate from structural confidence.

To use an immutable historical Base Generation, replace `--graph` with the
exact history realization selector on every command:

```bash
compass agent-graph export \
  --root . \
  --realization REALIZATION_ID \
  --overlay overlay:review \
  --revision REVISION_DIGEST \
  --profile augment \
  --output historical-review.json \
  --format json
```

Compass opens the trusted graph and an offline detached checkout for that
realization. It does not update the preferred realization or any history root.
`--realization` is rejected with `--graph` or `--state-root`.

## 5. Inspect history, audit, or rebase

```bash
compass agent-graph history --overlay overlay:review --format json
compass agent-graph audit --revision REVISION_DIGEST --format json
compass agent-graph rebase-plan \
  --overlay overlay:review \
  --revision REVISION_DIGEST \
  --format json
```

After a source rebuild, an Effective Graph read for the old Base Generation
does not silently attach to the new graph. Review the plan, provide grounded
replacement or Retraction operations for every unresolved item, and submit a
strict `compass.agent-graph.rebase-commit/1` request.

## MCP operation

Agent Graph tools appear only when the server has a canonical project
allowlist. `apply_agent_graph` is omitted unless writes are explicitly enabled.
HTTP writes require a read credential and a distinct write-capability
credential; the request cannot choose its principal, permissions, mask
capability, expiry, or limits. `inspect_agent_graph` supports `effective`,
`query`, `audit`, history, diff, and rebase-plan reads.

## Related pages

- [Agent Graph Overlays](../concepts/agent-graph-overlays.md)
- [Task context](task-context.md)
- [Integrating Compass](integrating-compass.md)

Next, automate batch generation in the agent while keeping Compass's
verification and authorization adapter outside the model prompt.
