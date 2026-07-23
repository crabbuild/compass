# The Compass graph model

Compass stores project knowledge as entities connected by directed,
attributed relationships. This page develops the model from a small example to
the node-link JSON and multigraph details used by advanced queries.

> **Who this page is for:** anyone interpreting Compass output or writing
> CompassQL and graph integrations.
>
> **You will learn:** nodes, stable IDs, labels, directed relationships,
> attributes, parallel edges, hyperedges, communities, snapshots, and common
> interpretation mistakes.
>
> **Prerequisites:** [How Compass works](how-it-works.md) is helpful but not
> required.
>
> **Reading time:** 10–12 minutes.

## Start with two nouns and a verb

The smallest useful graph statement is:

```text
checkout() --CALLS--> authorize_payment()
```

`checkout()` and `authorize_payment()` are **nodes**. `CALLS` is the
**relationship type**. The arrow gives the relationship a direction.

The graph can add evidence:

```text
checkout() --CALLS [EXTRACTED, app.py:L18]--> authorize_payment()
```

That evidence is stored as attributes on the node or edge.

## Nodes

A node represents one identifiable project entity. Depending on input and
extractor, that may be:

- a source file or module;
- a function, method, class, trait, interface, type, or variable;
- a configuration key or project manifest item;
- a database table, column, function, or relation;
- a document section or semantic concept;
- an image, media segment, or external integration entity;
- a structural placeholder that resolution may later connect or replace.

Every serialized node has a stable string `id` within the graph document:

```json
{
  "id": "src/payments.py::authorize_payment",
  "label": "authorize_payment()",
  "file_type": "Function",
  "source_file": "src/payments.py",
  "source_location": "L12"
}
```

The exact ID construction is an implementation and compatibility concern.
Consumers should treat IDs as opaque strings, not parse undocumented segments
out of them.

### ID versus label

- `id` is the stable graph identity used by edges and exact consumers.
- `label` is the human-facing display name.

Labels can repeat. Two files may each define `Config`, and several methods may
be called `run()`. A query interface may accept labels for convenience, but
automation should return and persist the full string ID when it needs one
particular node.

CompassQL's `id(n)` is different: it returns a snapshot-local integer index.
Use `n.id` for a portable stable Compass ID.

## Relationships

A serialized relationship has a source, target, and attributes:

```json
{
  "source": "src/checkout.py::checkout",
  "target": "src/payments.py::authorize_payment",
  "relation": "calls",
  "confidence": "EXTRACTED",
  "context": "call"
}
```

Direction is part of the meaning:

```text
caller --CALLS--> callee
child  --INHERITS--> parent
file   --IMPORTS_FROM--> module
scope  --CONTAINS--> member
```

When looking for callers, you usually traverse incoming `CALLS` relationships.
When looking for downstream calls, you traverse outgoing ones.

### Relationship vocabulary

Relation names depend on the extractor and input type. Common families include:

| Family | Examples | Typical meaning |
| --- | --- | --- |
| Structure | `contains`, `declares`, `member_of` | Ownership or hierarchy |
| Dependency | `imports_from`, `uses`, `references` | One entity relies on another |
| Execution | `calls`, `dispatches` | Potential control transfer |
| Type | `inherits`, `implements`, `mixes_in` | Type-system relationship |
| Configuration | `configures`, `depends_on` | Manifest or deployment linkage |
| Semantic | `relates_to`, source-specific relations | Provider-extracted conceptual linkage |

Do not assume every relation implies runtime execution. `imports_from` and
`references` express different kinds of dependency from `calls`.

## Attributes

Nodes and edges retain open-ended JSON attributes. Common attributes include:

- `label`;
- `file_type`;
- `source_file`;
- `source_location`;
- `relation`;
- `confidence`;
- `context`;
- `community`;
- language or extractor-specific metadata.

Unknown attributes are retained by the graph document loader. This supports
compatible extensions without forcing every consumer to understand every
field.

Treat fields in the documented output contract as stable. Treat undocumented
attributes as extensible data: preserve them when round-tripping and tolerate
new ones.

## Directed multigraph semantics

The node-link document declares whether it is directed and whether it is a
multigraph:

```json
{
  "directed": true,
  "multigraph": true,
  "graph": {},
  "nodes": [],
  "links": []
}
```

A multigraph can keep more than one relationship between the same two nodes:

```text
barrel.ts --IMPORTS_FROM--> module.ts
barrel.ts --RE_EXPORTS----> module.ts
```

Those edges share endpoints but carry different meanings. A consumer that
collapses them into a single untyped connection loses information.

Some human renderers list the neighboring node once while preserving the
parallel-edge contribution to degree. Exact CompassQL patterns and graph JSON
can inspect distinct relationships.

## Missing endpoints and graph loading

The graph model ensures every edge endpoint can be indexed. When loading a
compatible document, the implementation can create a minimal endpoint node for
an edge whose referenced ID is not already in the node list. Validation and
command-specific loaders still reject malformed or unsafe inputs according to
their contracts.

Graph loading is guarded by:

- a `.json` extension on normal query paths;
- a size cap;
- valid JSON;
- indexable endpoints;
- directed/multigraph handling;
- cache signatures before a binary cache is reused.

This protects query commands from treating arbitrary, oversized, or stale
cache content as a valid graph.

## Provenance and confidence

Relationships can carry:

- `EXTRACTED` — directly supported by parser/source evidence;
- `INFERRED` — resolved from evidence across scopes, files, or systems;
- `AMBIGUOUS` — several plausible interpretations remain.

Provenance qualifies the evidence, not the importance of the relationship. An
inferred link can be very useful; an extracted link can still describe code
that is dead or unreachable at runtime.

Read [Provenance and confidence](provenance.md) for a decision framework.

## Communities

Community detection assigns densely connected nodes to groups. A node can
carry a numeric community identifier, and reports can associate a human-readable
label with a group.

```text
Community 2
├── LoginHandler
├── SessionStore
├── verify_token()
└── AuthMiddleware
```

Community IDs are properties of a particular graph snapshot and clustering
result. Do not store business logic that assumes “community 2 always means
authentication.”

Use communities to:

- choose an entry point for architecture reading;
- identify subsystem boundaries;
- find surprising cross-community dependencies;
- divide a large review into coherent regions.

## God nodes and degree

Degree counts incident relationships. In a directed graph, Compass can consider
incoming plus outgoing edges for hub analysis.

```text
low degree                     high degree
----------                     -----------
LeafParser --USES--> Token     AppContext
                                 ^  ^  ^  ^
                                 |  |  |  |
                              many dependents
```

A god node is highly connected relative to the graph. It can reveal a critical
hub or a design smell, but it can also be a legitimate shared abstraction.
Compass's report filters some common built-in noise; it does not make a final
architectural judgment.

## Hyperedges

Some facts involve more than a source and target. For example, a call may bind
a caller, a target, an argument position, and a type constraint. Compass
extraction and historical realizations can retain hyperedges for such
multi-part facts.

The ordinary node-link `links` array remains the main query and visualization
surface. Do not assume that reconstructing only pairwise edges captures every
authoritative historical fact; history export preserves the full artifact set
according to its format contract.

## Graph snapshots

A graph is meaningful only together with the inputs and configuration that
produced it.

For the working tree:

```text
snapshot identity ~= files seen + build options + extractor/analyzer versions
```

For versioned history, Compass makes that explicit:

```text
realization = exact commit + extraction fingerprint + canonical artifact roots
```

The same commit can have multiple realizations—for example, a code-only profile
and a semantic profile. They are not silently treated as equivalent.

## What queries return

Different query surfaces expose different projections:

- `query` returns a focused, budgeted neighborhood;
- `explain` renders one node's incoming and outgoing neighborhood;
- `path` returns a shortest known connection;
- `affected` uses a compact projection with identity, location, and relevant
  relation fields;
- CompassQL returns typed tabular values, paths, nodes, relationships, maps, or
  lists under explicit limits.

A projection is not a second graph truth. It is a task-specific view over a
saved snapshot.

## Common interpretation mistakes

### “Connected” means “executes”

Not necessarily. An import, reference, containment edge, or semantic relation
does not imply a runtime call.

### “Extracted” means “correct at runtime”

It means direct static evidence exists. Dead code and conditionally loaded code
can still be extracted.

### “Inferred” means “made up”

It means a resolver connected structural evidence that was not expressed as
one direct relation in one file.

### “Shortest path” means “most important path”

It means fewest known hops under the command's traversal semantics. Importance
and runtime frequency are separate questions.

### “Affected” means “must edit”

It means a node is in the incoming impact neighborhood for configured relation
families and depth.

### “Community” means “official module”

It is a connectivity-derived cluster and can change with the graph.

## A compact reading checklist

When Compass returns an edge, ask:

```text
1. What are the exact source and target IDs?
2. Which direction does the relation point?
3. What does the relation type claim?
4. What provenance/confidence is attached?
5. Which source locations support it?
6. Is this current-tree or exact-revision data?
7. Is the result complete or a budgeted projection?
```

Those seven questions prevent most over-interpretation.

## Related pages

- [How Compass works](how-it-works.md)
- [Provenance and confidence](provenance.md)
- [CompassQL concepts](compassql.md)
- [Output reference](../reference/outputs.md)

**Next step:** read [Provenance and confidence](provenance.md) to learn how to
weigh graph evidence during investigation and review.
