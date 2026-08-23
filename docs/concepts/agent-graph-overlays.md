# Agent Graph Overlays

An Agent Graph Overlay lets an authorized agent add evidence-backed knowledge
without rewriting Compass's source-derived Base Graph. Compass verifies every
submitted citation, publishes successful assertions as `GROUNDED`, and composes
an immutable overlay revision with one exact Base Generation.

`GROUNDED` describes verification state, not structural confidence and not
semantic certainty. It means Compass recomputed the cited bytes, identifiers,
records, or prior assertion and found that the citation supports the requested
effect under a named policy. Structural `inferred` continues to mean a
relationship resolved from source facts.

## Two immutable planes

```text
Base Generation + Overlay Revision + composition profile = Effective Graph
```

The Base Graph remains `compass.graph/1`, build-owned, and immutable. The
overlay stores agent-owned assertions, Challenges, and Retractions. An
Effective Graph is a derived read view with a digest that binds all three
inputs. A consumer must use that effective identity for query caches, task
context, and rendered output. A rendered profile is pinned: switching between
`augment` and `curated` requires composing and loading a new Effective Graph,
not filtering the same client-side identity.

The `augment` profile retains challenged Base facts and reports the Challenge.
The stronger `curated` profile may omit explicitly masked Base facts and
reports direct and cascaded omissions. Neither profile edits `graph.json`.

## Lifecycle

- Create derives a stable Assertion ID from repository, overlay, owner, fact
  class, and caller-selected assertion key.
- Replace preserves the Assertion ID and requires the exact current assertion
  digest.
- Retract removes the active contribution while preserving immutable history.
- Challenge records evidence-backed disagreement with a Base fact. A mask is a
  separately authorized curated-view effect, not deletion.
- Rebase retains only exact identities and canonical record digests. Changed
  evidence requires a new grounded replacement or an explicit Retraction.

Every batch binds the Base Generation, expected Overlay Revision, principal,
permissions, limits, and idempotency key. Activation uses compare-and-swap, so
concurrent writers cannot silently lose an update.

## Grounding evidence

Version 1 accepts closed evidence forms for source spans, exact Base facts,
exact directed Base paths, prior assertions at an exact immutable revision,
and bounded snapshot artifacts. Topology assertions require a verified source
span. Evidence order does not affect certificate identity.

Compass issues the certificate; a request has no field through which it can
self-declare `GROUNDED`. Summaries are agent-authored text and must be treated
as untrusted when rendered.

## Storage and audit

Git repositories use a dedicated database below the Git common directory.
Non-Git use requires an explicit absolute state root. Overlay objects and
revision manifests are immutable; only the small active selector changes.
Published historical Compass realizations are never rewritten.

Each prepared publication receives a bounded `compass.agent-graph.audit/1`
record. Audit records can contain the trusted principal, adapter, digested
session/request identifiers, and bounded model ID. The contract has no prompt,
chain-of-thought, credential, token payload, or source-excerpt field. A bounded
audit read returns at most 64 records and reports truncation.

Garbage collection is two-step: a bounded dry-run computes exact reachability,
then an adapter-confirmed quiescent grant permits deletion. Active and pinned
revision ancestry is retained.

## Related pages

- [Enhance a graph with an agent](../guides/enhancing-a-graph-with-an-agent.md)
- [Provenance](provenance.md)
- [Graph model](graph-model.md)
- [Technical design](../implementation/grounded-agent-graph-overlay-technical-design.md)

Next, follow the guide to inspect the exact Base Generation before constructing
a change batch.
