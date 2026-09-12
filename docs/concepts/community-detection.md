# Community detection and quality

Compass communities are deterministic navigation partitions over a projected
view of the Base Graph. They are useful hypotheses about subsystem boundaries,
not source truth: node and relationship records remain authoritative even when
community membership changes between engine profiles.

## Production profile

Clustered typed graphs use this complete, versioned profile:

```text
algorithm   seeded-leiden-modularity/v1
topology    typed-evidence-undirected/v1
quality     community-quality/v1
selector    fixed-resolution/v1
seed        42
limits      community-limits/v1
resolution  1 by default, or the positive finite --resolution value
```

The topology preserves all Base Graph records unchanged. It creates a separate
undirected clustering projection in which calls and runtime wiring are strong,
imports and type relationships are medium, and containment, references, tests,
and documentation links are weak. Exact evidence contributes its full bounded
weight, inferred evidence contributes half, and ambiguous evidence is recorded
as omitted rather than used to invent affinity. Parallel occurrences are
deduplicated and capped per node-pair and relationship kind; aggregate pair and
total weights are also bounded.

Native deterministic Leiden local moving and connected refinement produce the
partition. Compass independently checks completeness and connectedness before
publication. It fails with a typed limit or partition error instead of
publishing a partial result.

## Resolution selection

`--resolution N` selects one fixed generalized-modularity resolution. Higher
values normally produce smaller communities; lower values normally produce
larger ones. Omitting the option currently uses the fixed value `1`.

Compass also implements a deterministic three-candidate selector over
`0.75 × base`, `base`, and `4/3 × base`. It compares every candidate at one
common quality resolution, filters invalid or materially worse partitions, and
then applies size, conductance, fragmentation, and digest tie-breaks. Compact
fixtures show that it improves ring-of-cliques and articulation cases and now
meets the compact timing gate. It remains qualification-only until the complete
pinned-corpus release matrix also passes latency, memory, stability, and
quality gates; omission of `--resolution` does not enable it.

## Incremental updates

An incremental update admits changed nodes and their complete prior
communities. Adjacent unchanged communities remain visible as frozen anchors,
so a local run does not lose external influence. Compass evaluates the merged
partition on the full topology. Removed nodes, excessive affected regions,
anchor merges, hub-policy changes, or quality regression cause a deterministic
full Leiden fallback. Historical materialization never depends on current-tree
assignments.

## Quality evidence

Clustered typed builds publish `community-quality.json` with schema
`compass.community-quality/1`. It is bound to the exact graph generation and
canonical `graph.json` digest and contains:

- the complete community profile and exact work limits;
- the selected candidate and any rejected candidate summaries;
- modularity, conductance, size, singleton, and connectedness measurements;
- relationship, strength, and confidence mixes;
- bounded node and edge witnesses plus exact omission counts; and
- a digest over the complete result.

Unknown schemas, unknown fields, profile mismatch, mutation, or graph mismatch
fail validation. Older graphs may legitimately omit this artifact; absence
means unavailable evidence, not a quality score of zero. Immutable history
stores the sidecar verbatim with its realization.

Community numeric IDs are graph-local. Stable member signatures reduce
unnecessary churn across updates, but detector, topology, resolution, or source
changes can legitimately change membership and IDs. Integrations that need
semantic identity should retain member IDs and the complete profile.

## Related pages

- [Graph model](graph-model.md)
- [Output reference](../reference/outputs.md)
- [Command reference](../reference/commands.md)
- [Technical design](../implementation/community-detection-quality-technical-design.md)
- [Qualification](../implementation/community-detection-quality-qualification.md)

**Next step:** inspect `community-quality.json` beside a clustered graph before
treating a community boundary as an architectural conclusion.
