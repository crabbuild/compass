# Compass Code Graph v1 Design

**Status:** Approved for implementation planning
**Date:** 2026-07-27
**Product:** Compass
**Schema:** `compass.graph/1`

## Summary

Compass will introduce a versioned, typed structural code-graph contract for
enterprise code navigation and analysis. The graph retains a
NetworkX-compatible node-link envelope while replacing the current free-form
node attributes and relationship strings with a validated Compass schema.

The first release includes:

- a fixed vocabulary for code and enterprise/domain nodes and relationships;
- stable, portable identity for files, nodes, and relationship sites;
- structured source evidence, provenance, confidence, coverage, and
  diagnostics;
- automatic route detection and handler resolution for the approved framework
  matrix;
- deterministic search, callers, callees, impact, explore, and node-trail
  queries;
- optional Program IR enrichment without duplicating structural symbols;
- one versioned query contract shared by the CLI, MCP server, and VS Code
  extension;
- a hard cutover from the current unversioned graph artifact.

Compass owns the implementation, contract, fixtures, qualification evidence,
and release lifecycle end to end.

## Product goals

1. Make every graph fact interpretable. A consumer can determine what a node or
   relationship means, where it came from, and whether it is exact, inferred,
   ambiguous, or heuristic.
2. Give enterprise users consistent queries across languages and frameworks.
3. Prevent missing or partial extraction from being mistaken for proof that a
   relationship does not exist.
4. Make clean builds, incremental builds, history materialization, CLI, MCP,
   and VS Code agree on one graph model.
5. Preserve graph portability and standard tooling interoperability through a
   NetworkX-compatible envelope.
6. Keep `graph.json` and `program.json` deterministic, auditable source
   artifacts while using rebuildable indexes for query performance.

## Non-goals

- Replacing Program IR with the structural graph.
- Treating numeric heuristic scores as calibrated probabilities.
- Using an LLM to create structural graph facts.
- Supporting pre-contract `graph.json` artifacts through translation or
  compatibility aliases.
- Making SQLite the authoritative graph store.
- Adding vector search to the v1 query contract.

## Architectural approach

Compass will implement native Rust framework and domain intelligence packs.

```text
compass-languages
  AST, configuration, and file-convention detection
            |
            v
Typed extraction facts
  code symbols, routes, events, jobs, schemas, database entities
            |
            v
compass-resolve
  cross-file identity, handler resolution, middleware ordering,
  dynamic-dispatch synthesis, provenance
            |
            v
compass-graph
  validation, stable identity, canonical ordering,
  NetworkX-compatible compass.graph/1 publication
            |
            +------> compass-query derived SQLite/FTS5 index
            |          search, callers, callees, impact, explore
            |
program.json ------> optional stable-symbol enrichment
            |
            v
compass.query/1 responses
  CLI, MCP, VS Code
```

### Component responsibilities

#### `compass-model`

Owns:

- the `compass.graph/1` document contract;
- node, role, and edge vocabularies;
- stable source anchors;
- provenance, evidence, confidence, coverage, and diagnostics;
- strict validation and canonical serialization;
- the versioned `compass.query/1` response model.

#### `compass-languages`

Owns:

- syntax recognition;
- configuration-file recognition;
- deterministic file-convention recognition;
- local definitions and relationship sites;
- framework-specific local facts.

It does not guess cross-file handler or dispatch identity.

#### `compass-resolve`

Owns:

- cross-file symbol resolution;
- route-to-handler resolution;
- middleware ordering;
- event, message, job, and database relationship resolution;
- ambiguity preservation;
- dynamic-dispatch synthesis;
- heuristic rule and wiring-site evidence.

#### `compass-graph`

Owns:

- combining resolved extraction facts;
- assigning and validating identities;
- endpoint-kind validation;
- canonical ordering;
- deterministic graph construction;
- atomic `graph.json` publication.

#### `compass-query`

Owns:

- content-addressed SQLite/FTS5 query indexes;
- deterministic symbol ranking;
- callers and callees;
- transitive impact;
- explore and node-trail assembly;
- Program IR reconciliation;
- bounded results and truncation metadata.

#### CLI, MCP, and VS Code

These surfaces are adapters over `compass.query/1`. They do not implement
independent ranking, traversal, evidence classification, or framework
semantics.

## Durable artifact

Compass keeps a NetworkX-compatible node-link envelope:

```json
{
  "directed": true,
  "multigraph": true,
  "graph": {
    "schema": "compass.graph/1",
    "build": {
      "builderVersion": "compass/1.0.0",
      "schemaFingerprint": "sha256:example-schema-fingerprint",
      "sourceTreeDigest": "sha256:example-source-tree-digest",
      "configurationDigest": "sha256:example-configuration-digest",
      "generationId": "sha256:example-generation-id",
      "sourceCommit": "0123456789abcdef0123456789abcdef01234567"
    },
    "files": [],
    "coverage": [],
    "diagnostics": []
  },
  "nodes": [],
  "links": []
}
```

The envelope is interoperable; its contents are governed by the strict Compass
schema.

### Envelope rules

- `directed` is always `true`.
- `multigraph` is always `true`.
- `graph.schema` is exactly `compass.graph/1`.
- `nodes` contains typed Compass node records.
- `links` contains typed Compass relationship records.
- Every parallel link has a stable NetworkX `key` equal to its Compass edge
  identity.
- Unknown kinds, roles, fields, provenance values, confidence values, or
  diagnostic severities fail validation.
- Arbitrary flattened attributes are not part of the contract.
- The complete document is validated before atomic publication.
- Build metadata contains no wall-clock timestamp, checkout root, hostname, or
  other machine-specific value. `generationId` derives from the other
  fingerprinted build inputs.

## Source anchors

Every source-backed fact uses a half-open byte range and human-readable
line/column coordinates:

```json
{
  "file": "src/orders/controller.ts",
  "startByte": 1842,
  "endByte": 1881,
  "startLine": 61,
  "startColumn": 8,
  "endLine": 61,
  "endColumn": 47
}
```

Rules:

- Paths are normalized repository-relative paths.
- Byte ranges are half-open: `[startByte, endByte)`.
- Lines are one-based.
- Columns are zero-based Unicode-scalar columns.
- Anchors must fall within the recorded file size.
- A zero-width anchor is permitted only for a convention-derived fact with no
  literal syntax site; such a fact must still identify the convention input
  file.

## Node contract

A node contains:

- `id`;
- `kind`;
- zero or more semantic `roles`;
- `name`;
- `qualifiedName`;
- optional language and framework;
- optional source anchor;
- typed kind-specific details;
- one or more evidence records;
- optional coverage and diagnostics.

### Node kinds

#### Core code kinds

- `file`
- `module`
- `package`
- `namespace`
- `class`
- `struct`
- `interface`
- `trait`
- `protocol`
- `enum`
- `enum_member`
- `type_alias`
- `function`
- `method`
- `constructor`
- `property`
- `field`
- `variable`
- `constant`
- `parameter`
- `import`
- `export`
- `macro`
- `annotation`
- `route`
- `component`

#### Enterprise and domain kinds

- `event`
- `message`
- `topic`
- `queue`
- `job`
- `resource`
- `schema`
- `query`
- `migration`
- `config_key`
- `database`
- `database_schema`
- `database_table`
- `database_view`
- `database_column`
- `database_index`
- `database_constraint`
- `database_procedure`
- `database_trigger`

### Semantic roles

A symbol retains one structural kind and may have multiple semantic roles.
For example, a NestJS method remains `method` while carrying
`controller` and `route_handler`.

The v1 roles are:

- `controller`
- `route_handler`
- `middleware`
- `service`
- `resolver`
- `consumer`
- `producer`
- `subscriber`
- `repository`
- `model`
- `test`
- `fixture`
- `generated`

Roles never replace the node's structural kind and never create duplicate
symbol nodes.

## Edge contract

A link contains:

- stable `id` and matching NetworkX `key`;
- `source` and `target` node IDs;
- exact `kind`;
- optional relationship-site anchor;
- structured provenance;
- one or more evidence records;
- typed kind-specific details;
- optional diagnostics.

### Edge kinds

#### Core relationships

- `contains`
- `calls`
- `imports`
- `exports`
- `extends`
- `implements`
- `references`
- `type_of`
- `returns`
- `instantiates`
- `overrides`
- `decorates`

#### Enterprise and domain relationships

- `routes_to`
- `reads`
- `writes`
- `aliases`
- `registers`
- `handles`
- `publishes`
- `subscribes`
- `produces`
- `consumes`
- `schedules`
- `triggers`
- `tests`
- `depends_on`
- `documents`
- `maps_to`

### Endpoint constraints

Every edge kind has a closed set of permitted source and target kind families.
The initial domain constraints include:

- `route routes_to function|method|class|component`
- callable nodes `calls` callable or constructible nodes
- callable nodes `publishes event|message|topic`
- callable nodes `handles event|message`
- callable nodes `reads|writes
  database|database_schema|database_table|database_view|database_column|config_key`
- type nodes `maps_to database_table|database_view`
- callable or configuration nodes `schedules job`
- `job triggers function|method`
- test-role code symbols `tests` any code or domain symbol
- container nodes `contains` their declared members

The implementation registry defines the complete matrix. Validation rejects a
relationship outside that matrix instead of degrading it to `references`.

## Provenance, evidence, and confidence

Every node and edge has structured provenance:

```json
{
  "origin": "heuristic",
  "extractor": "compass.languages.nestjs",
  "rule": "message-pattern-dispatch",
  "confidence": "inferred",
  "anchors": [],
  "wiringSite": {
    "file": "src/events/gateway.ts",
    "startByte": 900,
    "endByte": 947,
    "startLine": 32,
    "startColumn": 2,
    "endLine": 32,
    "endColumn": 49
  }
}
```

### Origins

- `ast`: direct syntax-tree evidence.
- `config`: direct configuration-file evidence.
- `convention`: deterministic framework or filesystem convention.
- `artifact`: evidence from a versioned external artifact such as SCIP.
- `heuristic`: synthesized dynamic-dispatch evidence.

### Confidence

- `exact`: the evidence determines one fact under the supported contract.
- `inferred`: deterministic evidence supports the fact but static parsing
  cannot prove runtime selection.
- `ambiguous`: bounded evidence supports multiple candidates.

### Provenance invariants

- AST, configuration, and artifact facts identify exact anchors.
- Convention facts identify their input file and convention rule.
- Heuristic facts identify the synthesis rule and wiring site.
- A numeric score, when present, is evidence metadata and is never described as
  probability.
- Ambiguous resolution retains all bounded candidates.
- No resolver silently chooses the first candidate.
- Conflicting evidence is retained and diagnosed.

## Identity

All identities are portable, deterministic, and schema-versioned.

### File identity

File identity derives from:

- graph schema identity;
- normalized repository-relative path.

It never contains an absolute checkout path.

### Code-symbol identity

Code-symbol identity derives from:

- graph schema identity;
- language;
- node kind;
- normalized source path;
- canonical qualified name;
- overload discriminator where the language permits overloads.

### Route identity

Route identity derives from:

- graph schema identity;
- framework;
- normalized HTTP method or protocol operation;
- normalized path or pattern;
- router, controller, or module scope.

### Messaging identity

Event, message, topic, and queue identity derives from:

- graph schema identity;
- transport or framework;
- canonical subject/channel name;
- declaring scope.

### Database identity

Database entity identity derives from:

- graph schema identity;
- logical database;
- database schema;
- entity kind;
- canonical qualified name.

### Edge identity

Edge identity derives from:

- graph schema identity;
- source ID;
- target ID;
- edge kind;
- relationship-site anchor;
- rule discriminator.

Distinct call or reference sites remain distinct links.

### Rename semantics

A rename or move is remove/add unless exact alias evidence establishes
continuity. Compass does not infer continuity from name similarity.

## File inventory and source integrity

Each `graph.files` record contains:

- file identity;
- normalized path;
- language;
- content digest;
- byte size;
- generated status;
- extraction status;
- coverage;
- diagnostics.

Source text is not copied into `graph.json`.

Explore verifies the working-tree digest before returning source. If a digest
does not match, Compass returns a stale-source diagnostic and omits that
source slice. It never combines source from one generation with graph facts
from another.

## Program IR relationship

The structural graph and Program IR remain two intentional artifacts:

- `graph.json` provides broad, language-neutral structural coverage;
- `program.json` provides deeper behavioral evidence where supported.

Program IR remains independently versioned under its existing
`http://crab.build/compass/v1` schema. The graph cutover does not rename or
renumber Program IR.

### Reconciliation rules

- Program IR joins structural nodes through `graph_node_id`.
- It enriches matched nodes with exact calls, parameters, types, control flow,
  effects, and capability coverage.
- It does not duplicate structural symbols.
- Program-only facts without a valid structural identity remain visible in
  Program diagnostics but do not invent structural nodes.
- Matching structural and Program IR relationships retain both evidence
  records.
- Conflicts produce structured diagnostics; there is no last-writer-wins
  behavior.
- Absence from Program IR never deletes structural evidence.
- `graph.json`, `program.json`, the manifest, and trusted build-state markers
  are published as one atomic generation with matching fingerprints.

## Derived query index

The authoritative artifacts are `graph.json` and `program.json`.

Compass builds a disposable, content-addressed SQLite database with FTS5 under
`compass-out/cache/`. Its cache key includes:

- graph digest;
- Program IR digest when present;
- graph schema fingerprint;
- Program IR schema fingerprint;
- query-index implementation version.

A missing, stale, or corrupt index is rebuilt automatically. Index corruption
cannot corrupt the authoritative artifacts.

## Shared query contract

CLI, MCP, and VS Code consume versioned `compass.query/1` responses. The
response model includes:

- resolved and alternative nodes;
- typed edges;
- files and source slices;
- connecting paths;
- evidence and provenance;
- Program IR enrichment;
- coverage and diagnostics;
- budgets, limits, and truncation state.

Clients do not independently interpret raw graph attributes.

## Search

FTS5 indexes:

- name;
- qualified name;
- kind;
- roles;
- file path;
- language;
- framework;
- signature;
- documentation.

Ranking order is deterministic:

1. exact name;
2. exact qualified name;
3. prefix match;
4. FTS5 BM25.

Search supports filters for kind, role, language, framework, and path. Vector
or LLM search is outside v1.

## Callers and callees

The default traversal is one hop. Requests may set an explicit bounded depth.

The execution relationship family includes:

- `calls`;
- `routes_to`;
- `handles`;
- `subscribes`;
- `schedules`;
- `triggers`.

Consequences:

- callers of a controller action include each route that binds it;
- callees of a route include ordered middleware and the final handler;
- callers of an event handler include subscriptions or registrations;
- every heuristic hop includes its synthesis rule and wiring site;
- parallel relationship sites remain visible.

## Impact

Impact walks incoming dependency relationships transitively.

Each result includes:

- the reason and path by which it is affected;
- exact, inferred, ambiguous, and heuristic evidence;
- Program IR coverage where available;
- maximum depth and node limits;
- explicit truncation state;
- omitted-count estimates when they can be computed within the request budget.

An `exact-only` mode excludes inferred, ambiguous, and heuristic edges for CI
and policy use.

Container expansion enters the container's own members at the same impact
depth. Traversal does not climb containment and expand unrelated siblings.

## Explore

Explore accepts one or more symbol IDs or names. One bounded response contains:

- disambiguation candidates;
- related symbols grouped by file;
- digest-verified source slices;
- the best connecting execution or dependency paths;
- inline edge provenance and wiring sites;
- Program IR signatures, types, effects, and coverage;
- unresolved and conflicting evidence;
- output-budget and truncation metadata.

If a requested file is stale, its source is omitted and diagnosed.

## Node trail

A node query returns:

- containment ancestors;
- immediate children;
- callers and callees;
- domain relationships;
- source evidence;
- provenance;
- coverage;
- diagnostics.

Route nodes additionally show:

- method or protocol operation;
- normalized and original path patterns;
- framework;
- router or controller scope;
- middleware sequence;
- exact, ambiguous, or unresolved handlers.

## Framework routing intelligence

Route resolution is automatic. Recognized files and syntax emit route nodes
after the next index or sync.

The canonical handler relationship is `routes_to`. Generic traversal may
classify it in a broader reference or dependency family, but the stored edge
kind remains `routes_to`.

### Route fact requirements

Every route records:

- framework;
- canonical method or operation;
- normalized path;
- original path expression or convention;
- declaring scope;
- source anchor;
- stable identity;
- provenance;
- resolution state;
- middleware order where available;
- resolved handler IDs or bounded candidates.

File-based routes use `convention` provenance. Reflective or dynamic bindings
use `heuristic` provenance and include a wiring site.

A route uses one `routes_to` edge per execution stage. Middleware edges carry
`stage: "middleware"` and a zero-based `position`; the final binding carries
`stage: "handler"`. This preserves execution order without reclassifying the
middleware or handler symbol.

### Release-one framework matrix

| Framework | Recognized shapes |
|---|---|
| Django | `path()`, `re_path()`, `url()`, and `include()` in `urls.py`; class-based `.as_view()` and dotted handlers |
| Flask | `@app.route`, blueprint routes, and declared methods |
| FastAPI | `@app` and `@router` decorators for standard HTTP methods |
| Express | `app` and `router` method calls with ordered middleware chains |
| NestJS | controllers and HTTP decorators; GraphQL resolvers, queries, and mutations; message, event, and WebSocket subscription decorators |
| Laravel | route methods, resources, `Controller@action`, and tuple syntax |
| Drupal | `*.routing.yml` controllers, forms, and entity handlers; `hook_*` implementations in supported module/theme/include files |
| Rails | verb routes using `to:` and hash-rocket controller/action syntax |
| Spring | method-level mapping annotations and composed HTTP mappings |
| Play | verb routes in `conf/routes` to Scala and Java controller actions |
| Gin, chi, gorilla, mux | registered HTTP method and handler calls |
| Axum, actix, Rocket | router methods and route attributes/macros |
| ASP.NET | HTTP method attributes on controller actions |
| Vapor | application and route-group method registrations |
| React Router, SvelteKit | route component and file-based route nodes |
| Vue Router, Nuxt | configured routes, file-based pages and server endpoints, and route middleware |
| Astro | file-based pages and endpoints including parameter and rest-segment conventions |

No route is emitted merely because a string resembles a URL.

## Enterprise and domain extraction

The first release includes real producers for every declared kind.

- SQL DDL supplies database, schema, table, view, column, index, constraint,
  procedure, trigger, query, and migration evidence where syntax permits.
- Supported ORM packs supply `maps_to`, `reads`, and `writes` relationships
  only when exact mapping or query evidence exists.
- Framework packs supply event, message, topic, queue, job, publish, subscribe,
  schedule, trigger, produce, and consume facts.
- Manifest and configuration extraction supplies package, resource,
  configuration, and dependency facts.

A kind with no trustworthy producer is removed from the v1 vocabulary before
release rather than shipped as an unused promise.

## VS Code behavior

The extension is a presentation client for `compass.query/1`.

It provides:

- kind-specific icons and filters;
- symbol and route search;
- callers, callees, impact, explore, and node-trail actions;
- distinct styling for exact, convention, ambiguous, and heuristic evidence;
- an evidence inspector showing extractor, rule, source anchors, and wiring
  sites;
- Program IR coverage and conflict indicators;
- stale-artifact and truncated-result states;
- source navigation from nodes and relationship sites.

The extension does not implement its own traversal, ranking, framework
resolution, or provenance classification.

## Validation and failure handling

### Publication failures

Graph publication fails for:

- unknown node, role, or edge values;
- invalid endpoint-kind combinations;
- duplicate stable IDs with conflicting content;
- dangling endpoints;
- missing provenance;
- heuristic relationships without a rule or wiring site;
- invalid source anchors;
- repository-escaping paths;
- mismatched graph, Program IR, manifest, or build fingerprints.

### Partial extraction

An unsupported or unparsable file is represented by failed or indeterminate
coverage and a diagnostic. It does not silently disappear.

The default developer build may publish a partial graph when all structural
invariants remain valid. Strict mode fails publication when required coverage
policy is not met.

### Query outcomes

These expected conditions are successful typed responses with diagnostics:

- no match;
- ambiguous match;
- unresolved handler;
- incomplete coverage;
- stale source;
- bounded truncation.

These are hard failures:

- corrupt authoritative artifacts;
- schema mismatch;
- unsafe path;
- violated graph invariant.

## Hard cutover

`compass.graph/1` is the first supported Compass graph contract. Existing
unversioned graphs are pre-contract artifacts.

Rules:

- no legacy loader;
- no translation adapter;
- no compatibility relationship aliases;
- unversioned graphs are rejected;
- index and update commands rebuild from source;
- all derived caches are invalidated;
- VS Code reports that a rebuild is required;
- history comparisons rematerialize old commits from source into
  `compass.graph/1`.

The Program IR contract is independently versioned and is not renamed by this
cutover.

## Security and resource bounds

- All artifact and source paths are repository-relative and normalized.
- Source reads are confined to the repository and verified against recorded
  digests.
- Graph, Program IR, source-slice, query, and FTS5-input sizes are bounded.
- FTS5 queries use parameterized statements and escaped user input.
- Diagnostics do not copy unrelated environment or configuration secrets.
- Framework configuration parsers reject aliases, expansion, or recursion that
  exceed documented limits.
- Query responses always report limits and truncation.

## Testing strategy

### Contract tests

- Valid v1 serialization and canonical ordering.
- Unknown field, kind, role, provenance, confidence, and severity rejection.
- Endpoint-kind matrix validation.
- Stable file, node, route, domain, and edge identities.
- Portable identity across checkout roots.
- Duplicate and dangling endpoint rejection.
- Anchor bounds and path confinement.

### Extraction and resolution tests

Every node and edge kind requires:

- a positive producer fixture;
- a negative near-match fixture;
- source-anchor assertions;
- provenance assertions;
- clean-build and incremental-build assertions.

Every framework requires:

- multiple methods and routes;
- nested prefixes;
- middleware order where applicable;
- exact, ambiguous, and unresolved handlers;
- add, change, delete, and rename sync cases;
- misleading syntax that emits no route.

### Query tests

- Exact and qualified search precedence.
- FTS5 ranking and filters.
- One-hop callers and callees.
- Route-to-handler, event, message, and job execution traversal.
- Impact paths, exact-only policy, containment behavior, and truncation.
- Explore grouping, path selection, source digest validation, and budgets.
- Node-trail evidence and wiring sites.
- Parallel relationship-site preservation.
- Missing, stale, ambiguous, partial, and conflicting results.

### Cross-layer tests

- Shared CLI and MCP golden query responses.
- Rust response and VS Code decoder parity.
- VS Code source navigation and evidence rendering.
- Program IR joins and conflicts.
- Atomic graph/Program/manifest/build-state publication.
- History rematerialization.
- Derived-cache deletion and corruption recovery.

### Determinism and platform tests

- Repeated clean builds are byte-identical.
- Incremental output equals a clean rebuild.
- Identity is stable across checkout roots.
- macOS, Linux, and Windows path behavior.
- Fuzzing for graph decoding, framework configuration, and query inputs.

### Real-repository qualification

- Small, medium, and large repositories for every supported language family.
- At least three representative route-to-handler flows per framework.
- No bounded ambiguous candidate is reported as exact.
- No falsely resolved handler is reported as exact.
- Query and indexing performance are measured against the release baseline and
  regressions require explicit review.

## Release gates

The first release does not ship until:

1. Every declared node and edge kind has a validated producer.
2. Every routing framework passes its fixture and real-repository matrix.
3. Clean and incremental graphs are canonically equivalent.
4. CLI, MCP, and VS Code expose matching query semantics.
5. Heuristic relationships always display rule and wiring evidence.
6. Strict mode rejects incomplete required coverage.
7. Unversioned artifacts cannot be mistaken for `compass.graph/1`.
8. Program IR enrichment cannot overwrite or silently contradict structural
   evidence.
9. Cross-platform validation and security bounds pass.

## Delivery sequence

The implementation plan will divide the work into independently reviewable
deliverables:

1. Graph v1 model, validation, identity, and hard cutover.
2. Core extraction normalization and provenance.
3. Enterprise/domain producers.
4. Framework routing intelligence packs.
5. Program IR reconciliation and derived query index.
6. Search, callers, callees, impact, explore, and node trail.
7. CLI and MCP adapters.
8. VS Code integration.
9. Cross-platform, real-repository, and release qualification.

All nine deliverables belong to the first Compass Graph v1 release.
