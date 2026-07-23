# How Compass works

Compass turns a directory of source code and project artifacts into a directed,
queryable knowledge graph. This page explains the complete flow first in plain
language, then at the level needed to reason about correctness, performance,
and extension points.

> **Who this page is for:** evaluators, users who want to interpret results,
> and contributors building a mental model of the system.
>
> **You will learn:** what happens during discovery, extraction, resolution,
> graph analysis, publication, and querying; which stages are deterministic;
> and where optional semantic providers enter.
>
> **Prerequisites:** none. Familiarity with functions, files, and imports is
> helpful; graph-database experience is not required.
>
> **Reading time:** 12–15 minutes.

![The Compass graph construction and query pipeline](../assets/diagrams/graph-pipeline.svg)

## The short version

Compass does six things:

```text
1. Discover     choose relevant files and project metadata
2. Extract      turn each source into entities and local relationships
3. Resolve      connect facts that cross files, modules, or languages
4. Analyze      group, rank, and diagnose the resulting graph
5. Publish      write a coherent compass-out/ artifact set
6. Query        load indexes and return focused subgraphs or exact matches
```

For source code, the first five stages are structural and local. Parsers
identify syntax; language-specific extractors turn that syntax into facts; the
resolver connects facts across files. No embedding model or vector database is
needed.

Documents, images, audio/video, and other semantic sources are different. They
may require the model provider or native media capability you explicitly
configure. This optional path merges into the same graph rather than creating a
separate search index.

## A working example

Consider:

```python
from payments import PaymentGateway


def checkout(total):
    gateway = PaymentGateway()
    return gateway.charge(total)
```

A simplified graph might contain:

```text
app.py
  |
  +--CONTAINS-----------> checkout()
  |
  `--IMPORTS_FROM-------> payments.py

checkout()
  |
  +--USES---------------> PaymentGateway
  |
  `--CALLS--------------> PaymentGateway.charge()
```

The parser supplies facts such as “this node is a call expression” and “this
identifier occurs in this scope.” A language extractor creates file, function,
class, import, and call facts. Resolution uses import targets, names, members,
and scope information to connect the call to the best known definition.

The graph keeps direction. `checkout() --CALLS--> charge()` means something
different from the reverse. Query and impact commands can therefore choose
whether to follow outgoing, incoming, or both directions.

## Stage 1: discovery

Discovery decides what enters the build. It:

- walks the requested root;
- applies supported extensions and project-file detection;
- respects committed ignore rules by default;
- applies explicit exclusion patterns;
- classifies source code, manifests, documents, media, and integration inputs;
- computes or reuses fingerprints for incremental work.

Discovery is a correctness boundary. A perfect parser cannot represent a file
that was excluded, ignored, generated after the scan, or outside the requested
root.

Useful questions when something is missing:

```text
Was the file under the requested root?
Was its extension or format recognized?
Did an ignore or exclude rule remove it?
Did the build use code-only mode?
Did an optional integration require explicit configuration?
```

`--no-gitignore` changes the ignore behavior and should be used deliberately.
Large generated directories can create noise and cost even when their syntax
is supported.

## Stage 2: structural extraction

Compass contains a deterministic language registry and a vendored
tree-sitter language pack. At build time, supported languages use native
parsers and language-specific extraction rules.

For each file, extraction can produce:

- file and module nodes;
- definitions such as functions, methods, classes, types, variables, and
  database objects;
- containment and declaration edges;
- import, call, inheritance, use, reference, and configuration relations;
- source file and source location attributes;
- language-specific facts needed by the later resolver;
- hyperedges for facts that naturally involve more than two participants.

Per-file extraction is intentionally separable. Independent files can be
parsed in parallel, and unchanged file facts can be reused by an incremental
build.

Structural does not mean simplistic. Language-specific code handles concerns
such as:

- JavaScript and TypeScript re-exports;
- member calls and inheritance;
- namespaces and packages;
- project manifests;
- templates and markup;
- languages whose build metadata changes name resolution.

It does mean that results are based on parseable project evidence, not semantic
similarity.

## Stage 3: cross-file resolution

A per-file extractor often cannot know the final target of a relation. The
resolver merges file facts and connects them using project-wide evidence.

For example:

```text
file A                        file B
------                        ------
imports payments             defines module payments
calls charge()               defines PaymentGateway.charge()
       \                         /
        \                       /
         +---- resolver -------+
                  |
                  v
      checkout --CALLS--> PaymentGateway.charge
```

Resolution performs targeted language and project operations, including
canonicalizing import targets, following re-exports, resolving call and member
facts, disambiguating colliding identifiers, and rewiring unique placeholder
nodes to known definitions.

This distinction explains provenance:

- a relation directly present in a file can be **EXTRACTED**;
- a cross-file relation connected by the resolver can be **INFERRED**;
- a relation with multiple viable targets can remain **AMBIGUOUS**.

“Inferred” in this context is not shorthand for “generated by an LLM.” It means
Compass derived the link from structural evidence rather than copying one
explicit syntax relation.

## Stage 4: merge and graph analysis

The merged result is a directed graph document. Analysis derives navigation
and diagnostic information such as:

- node degree and highly connected “god nodes”;
- communities of densely related nodes;
- cross-file or cross-community connections;
- suggested questions;
- import cycles and other diagnostics;
- summary data for `GRAPH_REPORT.md`.

### Communities

A community is a group with stronger internal connectivity than external
connectivity according to the clustering algorithm. Communities often resemble
features or subsystems, but they are graph-derived—not hand-maintained
architecture labels.

Use a community as a starting hypothesis:

```text
community 4 appears to contain:
  auth middleware
  token parsing
  session storage
  login handlers

hypothesis:
  community 4 is the authentication subsystem
```

Verify the hypothesis by looking at source locations and boundary edges.

### God nodes

A god node has unusually high degree. It may be:

- a true architectural hub;
- a broad interface or dispatcher;
- a configuration root;
- a generic type used everywhere;
- a noisy or overly broad extraction target.

Degree identifies connectivity, not quality or business importance. Compass
filters common built-in noise in reports, but interpretation still matters.

## Stage 5: atomic publication

The normal current-tree output lives in `compass-out/`. The public artifacts
include:

| Artifact | Responsibility |
| --- | --- |
| `graph.json` | Complete machine-readable node-link graph |
| `GRAPH_REPORT.md` | Human-readable orientation and diagnostics |
| `graph.html` | Optional interactive visualization |
| `manifest.json` | Incremental build state |

The build pipeline treats output as a set. A failed semantic provider,
validation error, or incomplete build must not publish a graph as if it were a
complete successful result. Write paths use temporary or staged artifacts and
atomic replacement where the contract requires it.

Consumers should still avoid reading files while a writer is replacing an
artifact set. A long-lived integration can either watch for completion, invoke
Compass itself, or open a graph snapshot only after the producing command
returns successfully.

## Stage 6: loading and querying

Read commands load `compass-out/graph.json` by default. The graph layer:

- validates the JSON extension and size guard;
- decodes the node-link document;
- preserves directed and multigraph semantics;
- indexes stable node IDs;
- builds incoming and outgoing adjacency;
- builds query indexes and a schema fingerprint;
- may reuse a local binary cache when it matches the graph signature.

The main read surfaces answer different questions:

| Command | Question |
| --- | --- |
| `query` | Which focused neighborhood is relevant to this phrase? |
| `explain` | What is this node and what connects to it? |
| `path` | How are these two entities connected? |
| `affected` | What may depend on this entity through impact relations? |
| `tree` | What hierarchical structure surrounds this entity? |
| `query --cql` | Which rows exactly match this structural pattern? |

Natural-language discovery is still deterministic graph search. It tokenizes
the question, scores candidate nodes, and traverses from relevant anchors under
a budget. It does not write a prose answer with a model.

CompassQL compiles an explicit structural query into a bounded execution plan.
It is the right choice when automation needs exact patterns, parameters,
typed JSON, limits, or profiles.

## Current tree versus exact Git history

`compass update .` describes the working tree at build time. That can include
uncommitted changes.

Versioned history answers a different question: “What was the complete graph
for this exact Git commit under this exact extraction profile?”

```text
working tree                         exact commit
------------                         ------------
compass update .                     compass history build REV
      |                                       |
      v                                       v
compass-out/                         immutable realization
current mutable artifact set        content-addressed Prolly roots in SQLite
```

Historical materialization resolves a revision, creates a protected offline
worktree, runs the recorded build profile, validates the result, and publishes
an immutable realization. Meaning-affecting inputs are captured in an
extraction fingerprint so a code-only graph is not silently compared with a
semantic graph.

Read [Versioned graph history](../guides/versioned-history.md) before operating
the history store.

## Optional semantic path

Some project knowledge is not represented by program syntax:

- design documents;
- ADRs and RFCs;
- prose requirements;
- images and diagrams;
- office documents;
- audio or video;
- remote workspace or database sources.

When enabled, semantic orchestration:

1. classifies supported non-code inputs;
2. renders or decodes content where needed;
3. chunks content under configured limits;
4. sends permitted content to the configured provider or native model path;
5. validates structured extraction results;
6. merges semantic nodes and edges with the structural graph;
7. prevents incomplete provider work from masquerading as a complete build.

Credentials and provider/model selection affect meaning and therefore affect a
history realization's fingerprint, but secret values are not stored in that
fingerprint.

Use `--code-only` when you need an explicit fully local structural profile.
Compass does not silently downgrade a requested semantic history build because
credentials are missing.

## Determinism and limits

Determinism has practical meanings:

- the same supported inputs and meaning-affecting configuration should produce
  an equivalent structural graph;
- output ordering is stable where the public contract requires it;
- query budgets and resource limits are explicit;
- unsupported CompassQL syntax is rejected, not approximated;
- historical realizations are immutable once published.

It does not mean:

- every filesystem state is stable while being scanned;
- a provider will always return identical semantic content;
- dynamic runtime behavior can always be recovered from static source;
- JSON member ordering is a semantic graph property.

Bounded behavior protects interactive and automated use. Graph loading has a
size cap. CompassQL has row, path-depth, relationship-expansion, memory, and
deadline limits. File/stdin query sources and parameter documents also have
caps. Consult the [CompassQL reference](../COMPASSQL.md) for current values.

## Failure model

Compass distinguishes categories so scripts can respond correctly:

```text
usage or compile mistake
    -> fix the command/query; retrying unchanged input will not help

graph loading or validation failure
    -> inspect path, JSON, size, endpoints, or artifact completeness

provider or integration failure
    -> check credentials/network/source; incomplete builds do not publish

resource limit or cancellation
    -> narrow the work or lower scope; no successful partial result is emitted

history corruption or profile mismatch
    -> use explicit inspection/rebuild/profile commands; do not overwrite silently
```

Exact exit codes differ by command family. Use the
[command reference](../reference/commands.md) and canonical CompassQL/history
documents for automation.

## Related pages

- [Graph model](graph-model.md)
- [Provenance and confidence](provenance.md)
- [System architecture](../design/architecture.md)
- [Extraction pipeline](../implementation/extraction-pipeline.md)

**Next step:** read [Graph model](graph-model.md) to interpret the entities,
relationships, and attributes returned by Compass.
