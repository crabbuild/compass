# Explore an unfamiliar codebase

This guide gives you a repeatable way to turn a large repository into a small
set of architectural hypotheses, implementation paths, and review targets.

> **Who this guide is for:** developers onboarding to a repository, reviewers,
> maintainers, and coding-assistant users.
>
> **You will learn:** how to move from a repository-wide report to communities,
> symbols, paths, impact, source verification, and a concise architecture note.
>
> **Prerequisites:** Compass installed and a successful `compass update .`.
>
> **Completion time:** 20–45 minutes for an initial survey.

## The investigation loop

Do not try to understand the whole graph at once. Use a narrowing loop:

```text
broad report
    |
    v
one subsystem or question
    |
    v
focused neighborhood
    |
    v
specific node and path
    |
    v
source verification
    |
    `---- refine the question ----+
```

Compass reduces the reading set; source code remains the final evidence for
implementation behavior.

## 1. Confirm freshness and scope

From the repository root:

```bash
compass update .
```

Confirm the output:

```bash
test -f compass-out/graph.json
test -f compass-out/GRAPH_REPORT.md
```

Before interpreting absence, check:

- the requested root;
- ignore and explicit exclude patterns;
- whether the build was code-only;
- whether generated or external sources exist outside the corpus;
- whether the language or format is represented in the compatibility ledger.

Record the current revision and dirty state in your investigation notes:

```bash
git rev-parse --short HEAD
git status --short
```

A working-tree graph can include uncommitted changes. If the question is about
an exact historical revision, use [versioned history](versioned-history.md)
instead.

## 2. Read the report as a map, not a verdict

Open:

```bash
sed -n '1,240p' compass-out/GRAPH_REPORT.md
```

Look for:

1. corpus size and diagnostics;
2. high-degree nodes;
3. communities and their representative entities;
4. cross-file or surprising connections;
5. suggested questions.

Write three provisional notes:

```text
Likely entry point:
Likely subsystem boundary:
One surprising dependency to verify:
```

These are hypotheses. A community is a connectivity-derived group; a god node
is highly connected. Neither is automatically the official architecture.

## 3. Ask a behavior-shaped question

Queries work better when they name a domain behavior than when they name a
generic programming word.

Good:

```bash
compass query "HTTP request authentication and session validation"
compass query "payment retry and idempotency"
compass query "database migration discovery and execution"
```

Too broad:

```bash
compass query "service"
compass query "manager"
compass query "data"
```

If the result is broad, add the boundary or action you care about:

```text
"authentication"                  broad
"API authentication middleware"   narrower
"API token verification failure"  behavior-shaped
```

Save useful output when comparing questions:

```bash
compass query "API token verification failure" > /tmp/compass-auth.txt
```

Do not treat temporary result text as a durable schema. For automation, use
CompassQL JSON/JSONL.

## 4. Pick and explain an anchor

Choose a concrete result—preferably a function, class, file, or configuration
entity with a source location:

```bash
compass explain AuthMiddleware
```

Read incoming and outgoing relations separately:

```text
incoming
  who imports, calls, contains, or depends on this?

outgoing
  what does this import, call, contain, or depend on?
```

Check provenance. An `INFERRED` cross-file call can be a strong navigation lead,
but it should send you to the supporting sources. An `AMBIGUOUS` edge belongs
in a verification list.

When a label matches several nodes, use the returned stable ID or add context
such as the file/module name.

## 5. Trace an implementation path

Once you know two boundaries, connect them:

```bash
compass path HttpHandler TokenVerifier
```

Useful boundary pairs include:

- endpoint to persistence;
- command handler to side effect;
- public API to internal implementation;
- configuration key to consumer;
- failing test to production symbol;
- parser entry to output renderer.

For each hop, record:

| Hop | Relation | Provenance | Source verified? |
| --- | --- | --- | --- |
| 1 | calls | extracted | yes/no |
| 2 | uses | inferred | yes/no |

A shortest path is a compact explanation of known connectivity. It is not
necessarily the only path, the most frequent path, or one runtime trace.

If no path exists:

1. query each endpoint independently;
2. confirm the right node IDs;
3. check direction and relation types;
4. check ignored/generated/external code;
5. try a subsystem boundary rather than a leaf symbol;
6. record the missing link as a graph limitation or extraction issue.

## 6. Understand structure around files and symbols

Generate the tree view:

```bash
compass tree
```

The default HTML output is `compass-out/GRAPH_TREE.html`. Use it to move between
filesystem structure and symbol-level edges. For a large repository, cap
visible children or outbound edges:

```bash
compass tree --max-children 100 --top-k-edges 8
```

Tree view is useful when a graph neighborhood lacks the repository layout
context a human expects.

## 7. Estimate the review surface

For a symbol you might change:

```bash
compass affected TokenVerifier --depth 3
```

Impact traversal typically follows incoming dependency-like relationships:

```text
caller --CALLS--> target

change target
    |
    `--> inspect caller
```

Use the result to build a review checklist:

- direct callers;
- importing modules;
- implementers or inheritors;
- tests and fixtures;
- configuration consumers;
- cross-community dependencies.

Do not convert the result mechanically into “all files that must change.”
Static graphs can over-include potential dependencies and under-represent
dynamic ones.

## 8. Use CompassQL for a precise inventory

After discovery tells you the pattern, make it explicit:

```bash
compass query --cql \
  "MATCH (caller)-[edge:CALLS]->(target)
   WHERE target.label = 'TokenVerifier'
   RETURN caller.id, edge.confidence, target.id
   ORDER BY caller.id
   LIMIT 200"
```

Examples of useful inventories:

### Cross-community calls

```cypher
MATCH (source)-[edge:CALLS]->(target)
WHERE source.community <> target.community
RETURN source.id, target.id, edge.confidence
LIMIT 200
```

### Implementers of an interface

```cypher
MATCH (implementation)-[:IMPLEMENTS]->(contract)
WHERE contract.label = $contract
RETURN implementation.id
ORDER BY implementation.id
```

### Ambiguous relationships for manual review

```cypher
MATCH (source)-[edge]->(target)
WHERE edge.confidence = 'AMBIGUOUS'
RETURN source.id, type(edge), target.id
LIMIT 200
```

Confirm relation names and labels in your graph; extractor vocabularies can
vary by language and input.

## 9. Verify in source

Compass should shorten source reading, not replace it. For each architectural
claim:

1. open the source locations from the node/edge;
2. confirm direction and conditional behavior;
3. check nearby comments, tests, and configuration;
4. note dynamic behavior static extraction cannot prove;
5. revise the graph question if the hypothesis was wrong.

A useful architecture statement includes evidence:

```text
Token verification enters through AuthMiddleware, which delegates to
TokenVerifier and reads SessionStore. Evidence: graph path at commit abc123,
verified in src/http/auth.rs and src/auth/token.rs. Dynamic provider selection
remains configuration-dependent.
```

## 10. Write a digestible architecture note

Use this template:

```text
Question
  What behavior or boundary did we investigate?

Entry points
  Which public handlers, commands, or jobs start it?

Core path
  What are the 3–7 most important hops?

State and side effects
  Which databases, files, queues, or providers are touched?

Subsystem boundaries
  Which communities/modules participate?

Change impact
  Which direct dependents and tests deserve review?

Uncertainty
  Which ambiguous, dynamic, ignored, or external behavior remains?

Evidence
  Graph revision/profile, commands, and source files verified.
```

This keeps the result useful to a teammate who never saw the raw query output.

## Working with a coding assistant

Give the assistant a focused subgraph and an explicit goal:

```text
Goal: explain token verification failure handling.
Graph evidence: output of the focused Compass query and path.
Constraints: verify claims in the cited source files; treat ambiguous edges as
leads; do not read unrelated directories.
```

Avoid pasting an entire large `graph.json` into a model. The value of Compass is
to choose a smaller, structurally coherent context.

## Completion checklist

- [ ] Graph scope and working-tree/revision identity recorded.
- [ ] Report read and three hypotheses written.
- [ ] Behavior-shaped query run.
- [ ] At least one anchor explained.
- [ ] Core path traced and provenance reviewed.
- [ ] Impact neighborhood inspected.
- [ ] Important claims verified in source.
- [ ] Uncertainty and missing evidence recorded.
- [ ] Short architecture note written for another reader.

## Related pages

- [How Compass works](../concepts/how-it-works.md)
- [Architecture-discovery cookbook](../cookbook/architecture-discovery.md)
- [Impact-analysis cookbook](../cookbook/impact-analysis.md)
- [Assistant setup](assistant-setup.md)

**Next step:** use the [architecture-discovery cookbook](../cookbook/architecture-discovery.md)
for shorter recipes you can repeat during reviews.
