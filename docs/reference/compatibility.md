# Compatibility and evolution

Compass was inspired by Graphify and was developed with a frozen Graphify
baseline as a behavioral oracle. It is a native product with an independently
evolving feature set.

> **Who this reference is for:** evaluators, migrators, integrators, and
> contributors deciding whether behavior is compatible or Compass-native.
>
> **You will learn:** the frozen baseline, certified command families,
> intentional normalization, native additions, platform scope, and how
> divergence is documented.
>
> **Prerequisites:** none.
>
> **Reading time:** 7–10 minutes.

> The authoritative ledger is [COMPATIBILITY.md](../../COMPATIBILITY.md). This
> page is a navigational explanation, not a replacement.

## Product relationship

```text
Graphify
  ideas + frozen behavioral evidence
          |
          v
Compass native Rust implementation
  compatible where certified
  intentionally normalized where documented
  Compass-native features beyond the baseline
  future independent evolution
```

Preferred positioning:

> Compass is a native, local-first knowledge graph engine inspired by Graphify,
> built in Rust, and evolving beyond it.

Avoid claiming either:

- “Compass has no relationship to Graphify”; or
- “Compass will always be only a drop-in Rust port.”

## Frozen oracle

The current canonical ledger records:

- Python baseline: Graphify `v0.9.20`;
- baseline commit: `edec9eabeceeae6aa2375eddb3835efa1a32c0a3`;
- native root: Compass repository;
- Python use: development/CI oracle only.

Released Compass binaries do not start Python or load tree-sitter grammars at
runtime.

## Certified families

The ledger groups evidence for:

- build;
- query;
- graph operations;
- service;
- project workflows;
- assistant setup.

Certification can include:

- CLI argument/exit snapshots;
- graph node/edge equivalence;
- complete output comparisons;
- official MCP client tests;
- native integration tests;
- release/corpus qualification.

Compatibility is asserted only where evidence exists.

## Approved normalization

The frozen Python query renderer has unstable set-iteration order for
equal-degree nodes. Compass uses stable node-ID tie ordering.

Differential qualification therefore compares:

- exact headers;
- complete line multisets for affected query output;
- order-independent node/edge graph records where Python file walk order is
  platform-dependent.

Attributes, multiplicity, ranking, traversal membership, and duplicate lines
remain part of the contract. This is a documented normalization, not permission
to ignore arbitrary diffs.

## Compass-native capabilities

Examples include:

- `compass query --cql` and the CompassQL compiler/executor;
- immutable versioned graph history and exact-revision queries/diffs;
- native product installation and assistant assets;
- performance/safety improvements that preserve or explicitly evolve
  contracts.

The frozen Python oracle does not need a matching flag for every native
feature.

## Command identity

`compass` is the authoritative shipped CLI. New scripts and documentation use:

```bash
compass update .
compass query "..."
compass history build HEAD
```

The internal compatibility frontend is test infrastructure, not a second
published product identity.

## Platform matrix

CI currently covers targets listed in the canonical ledger, including Linux,
macOS, and Windows architectures. Release packaging can be narrower than CI
compilation/testing.

Do not infer official release support from “it compiled locally.” Follow the
current [compatibility ledger](../../COMPATIBILITY.md) and release page.

## Assistant platforms

The native installer covers a broad matrix and exact project/global file-tree
behavior is tested. Use:

```bash
compass install --help
```

for the version-specific list.

## Migration

Read [MIGRATION.md](../../MIGRATION.md) for:

- executable and output-path changes;
- compatibility expectations;
- behavior intentionally retired or evolved;
- migration commands.

Do not add a silent incompatible change. Add:

1. compatibility ledger entry;
2. native and/or differential fixture;
3. migration note;
4. documentation/reference change;
5. release note when user-visible.

## Classifying a change

```text
Does the frozen baseline have this behavior?
  |
  +-- yes --> preserve and add differential evidence,
  |           or document intentional incompatibility
  |
  `-- no  --> define a Compass-native contract and tests
```

If the behavior is an accidental Python runtime artifact, normalize only with
explicit approval and evidence.

## Related pages

- [Canonical compatibility ledger](../../COMPATIBILITY.md)
- [Migration guide](../../MIGRATION.md)
- [Design principles](../design/principles.md)
- [Roadmap](../roadmap.md)

**Next step:** before depending on a behavior, find it in the canonical ledger
or a Compass-native contract and add a fixture if the evidence is missing.
